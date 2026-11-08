package dev.penlink.rtc

import dev.penlink.data.SavedHost
import dev.penlink.data.StreamPrefs
import dev.penlink.protocol.AppEntry
import dev.penlink.protocol.HostAddress
import dev.penlink.protocol.WsIn
import dev.penlink.protocol.WsParser
import dev.penlink.protocol.describeClose
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asSharedFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.util.concurrent.atomic.AtomicLong

/**
 * The single WebSocket to one host plus the sessions (tabs) opened over it.
 *
 * The link is kept alive: when it drops (host restarted or crashed, network change) the tabs go to
 * [SessionStatus.Reconnecting], the connection retries with backoff ([ReconnectMachine]) and, once
 * the host is back, re-opens every tab with `attach` (same app, new session, no relaunch). A tab
 * whose app has exited meanwhile comes back as `closed: app exited` and is removed like any other
 * closed tab. After 30 s without a host [link] turns [LinkState.Unreachable] (the UI shows Retry).
 */
class HostConnection(
    val host: SavedHost,
    private val http: OkHttpClient,
    private val rtc: Rtc,
    private val clock: () -> Long = { System.currentTimeMillis() },
) {
    private val ids = AtomicLong(1)
    private val lock = Any()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val machine = ReconnectMachine()
    private val wake = Channel<Unit>(Channel.CONFLATED)
    private var loop: Job? = null
    private var ws: WebSocket? = null
    private var current: Socket? = null

    /** True between "socket open and sessions re-submitted" and the socket dropping. Guarded by [lock]. */
    private var connected = false
    private var stopped = false

    private val _link = MutableStateFlow<LinkState>(LinkState.Idle)

    /** State of the WebSocket link (connected / reconnecting / unreachable). */
    val link: StateFlow<LinkState> = _link.asStateFlow()

    private val _sessions = MutableStateFlow<List<RemoteSession>>(emptyList())
    val sessions: StateFlow<List<RemoteSession>> = _sessions.asStateFlow()

    /** Last host-level error message (shown by the UI). */
    val lastError = MutableStateFlow<String?>(null)

    private val _notices = MutableSharedFlow<String>(extraBufferCapacity = 16)

    /** One-line messages for the user (a tab was closed by the host, and why). */
    val notices: SharedFlow<String> = _notices.asSharedFlow()

    private val pendingOpen = ArrayDeque<RemoteSession>()

    private fun sendFrame(text: String): Boolean = ws?.send(text) ?: false

    /** Open an app in a new tab. Works while the link is down too: the open is sent once it is up. */
    fun open(app: AppEntry, width: Int, height: Int, prefs: StreamPrefs): RemoteSession {
        val s = RemoteSession(ids.getAndIncrement(), app.id, app.name, width, height, prefs, rtc, ::sendFrame)
        synchronized(lock) {
            _sessions.update { it + s }
            lastError.value = null
            if (connected) submit(s) else startLoopLocked()
        }
        return s
    }

    /** Close a tab: tell the host, then drop it from the list. */
    fun close(s: RemoteSession) {
        s.close()
        synchronized(pendingOpen) { pendingOpen.remove(s) }
        _sessions.update { it - s }
    }

    fun onScreenSize(width: Int, height: Int) {
        for (s in _sessions.value) {
            if (s.status.value is SessionStatus.Closed) continue
            if (s.requestedSize.value != width to height) s.resize(width, height)
        }
    }

    /** The Retry button: try again right now and restart the 30 s unreachable clock. */
    fun retryNow() {
        synchronized(lock) {
            if (stopped) return
            if (loop?.isActive != true) {
                startLoopLocked()
            } else {
                machine.retryNow(clock())
                _link.value = machine.state
                wake.trySend(Unit)
            }
        }
    }

    fun shutdown() {
        val socket: WebSocket?
        synchronized(lock) {
            stopped = true
            connected = false
            machine.stop()
            _link.value = LinkState.Idle
            socket = ws
            ws = null
            current = null
        }
        _sessions.value.forEach { it.close() }
        _sessions.value = emptyList()
        socket?.close(1000, "bye")
        scope.cancel()
    }

    // ---- connection loop

    private fun startLoopLocked() {
        if (loop?.isActive == true) return
        loop = scope.launch { runLoop() }
    }

    private suspend fun runLoop() {
        var delayMs = synchronized(lock) {
            machine.begin(clock()).also { _link.value = machine.state }
        }
        while (scope.isActive) {
            if (delayMs > 0) withTimeoutOrNull(delayMs) { wake.receive() }
            wake.tryReceive()
            val sock = Socket()
            val req = Request.Builder().url(HostAddress.wsUrl(host.address, host.token)).build()
            synchronized(lock) {
                if (stopped) return
                current = sock
                ws = http.newWebSocket(req, sock)
            }
            if (!sock.opened.await()) {
                delayMs = synchronized(lock) {
                    if (stopped) return
                    machine.onAttemptFailed(clock()).also { _link.value = machine.state }
                }
                continue
            }
            synchronized(lock) {
                if (stopped) return
                machine.onConnected()
                // Everything that was open comes back: same app, new session, no relaunch if it ran before.
                for (s in _sessions.value) {
                    if (s.status.value !is SessionStatus.Closed) submit(s)
                }
                connected = true
                _link.value = LinkState.Connected
            }
            sock.closed.await()
            delayMs = synchronized(lock) {
                if (stopped) return
                connected = false
                if (current === sock) {
                    ws = null
                    current = null
                }
                synchronized(pendingOpen) { pendingOpen.clear() }
                _sessions.value.forEach { it.onLinkLost() }
                machine.onDropped(clock()).also { _link.value = machine.state }
            }
        }
    }

    /** Send `open` for a session and queue it for its `opened`. Call with [lock] held. */
    private fun submit(s: RemoteSession) {
        synchronized(pendingOpen) { pendingOpen.addLast(s) }
        if (!s.sendOpen()) synchronized(pendingOpen) { pendingOpen.remove(s) }
    }

    private fun find(id: String) = _sessions.value.firstOrNull { it.sessionId == id }

    private fun onText(text: String) {
        when (val m = WsParser.parse(text)) {
            is WsIn.Opened -> {
                val s = synchronized(pendingOpen) {
                    val i = pendingOpen.indexOfFirst { it.appId == m.appId }
                    if (i >= 0) pendingOpen.removeAt(i) else pendingOpen.removeFirstOrNull()
                }
                s?.onOpened(m.sessionId, m.title)
            }
            is WsIn.Title -> find(m.sessionId)?.onTitle(m.title)
            is WsIn.Window -> find(m.sessionId)?.onWindow(m.width, m.height, m.fitted, m.mode, m.dpi)
            is WsIn.Offer -> find(m.sessionId)?.onOffer(m.sdp)
            is WsIn.Ice -> find(m.sessionId)?.onRemoteIce(m.candidate, m.sdpMid, m.sdpMLineIndex)
            is WsIn.Closed -> {
                // Host ended the session (launch failed, app exited, ...): drop the tab, say why.
                // A close we asked for already removed the tab, so find() is null and nothing happens.
                val s = find(m.sessionId)
                if (s != null) {
                    s.onClosedByHost(m.reason)
                    _sessions.update { it - s }
                    _notices.tryEmit(describeClose(s.title.value, m.reason))
                }
            }
            is WsIn.Error -> {
                lastError.value = m.message
                // An error while a session still waits for "opened" means that open failed.
                synchronized(pendingOpen) {
                    pendingOpen.removeFirstOrNull()?.onClosedByHost("error: ${m.message}")
                }
            }
            null -> {}
        }
    }

    /** One WebSocket attempt: [opened] says whether it came up, [closed] fires when it is gone. */
    private inner class Socket : WebSocketListener() {
        val opened = CompletableDeferred<Boolean>()
        val closed = CompletableDeferred<Unit>()

        override fun onOpen(webSocket: WebSocket, response: Response) {
            opened.complete(true)
        }

        override fun onMessage(webSocket: WebSocket, text: String) {
            if (current === this) onText(text)
        }

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            runCatching { webSocket.close(code, reason) }
            opened.complete(false)
            closed.complete(Unit)
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
            opened.complete(false)
            closed.complete(Unit)
        }

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            lastError.value = if (response?.code == 401) "unauthorised: pair again" else (t.message ?: "connection lost")
            opened.complete(false)
            closed.complete(Unit)
        }
    }
}
