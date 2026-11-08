package dev.penlink.rtc

import dev.penlink.data.SavedHost
import dev.penlink.protocol.AppEntry
import dev.penlink.protocol.HostAddress
import dev.penlink.protocol.WsIn
import dev.penlink.protocol.WsOut
import dev.penlink.protocol.WsParser
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.util.concurrent.atomic.AtomicLong

/** The single WebSocket to one host plus the sessions (tabs) opened over it. */
class HostConnection(
    val host: SavedHost,
    private val http: OkHttpClient,
    private val rtc: Rtc,
) {
    private val ids = AtomicLong(1)
    private val lock = Any()
    private var ws: WebSocket? = null

    private val _sessions = MutableStateFlow<List<RemoteSession>>(emptyList())
    val sessions: StateFlow<List<RemoteSession>> = _sessions.asStateFlow()

    /** Last host-level error message (shown by the UI). */
    val lastError = MutableStateFlow<String?>(null)

    private val pendingOpen = ArrayDeque<RemoteSession>()

    private fun ensureSocket() {
        synchronized(lock) {
            if (ws != null) return
            val req = Request.Builder().url(HostAddress.wsUrl(host.address, host.token)).build()
            ws = http.newWebSocket(req, listener)
        }
    }

    private fun sendFrame(text: String): Boolean = ws?.send(text) ?: false

    fun open(app: AppEntry, width: Int, height: Int, scale: Double): RemoteSession {
        ensureSocket()
        val s = RemoteSession(ids.getAndIncrement(), app.id, app.name, width, height, scale, rtc, ::sendFrame)
        synchronized(pendingOpen) { pendingOpen.addLast(s) }
        _sessions.value = _sessions.value + s
        lastError.value = null
        sendFrame(WsOut.open(app.id, width, height, scale))
        return s
    }

    /** Close a tab: tell the host, then drop it from the list. */
    fun close(s: RemoteSession) {
        s.close()
        _sessions.value = _sessions.value - s
    }

    fun onScreenSize(width: Int, height: Int) {
        for (s in _sessions.value) {
            if (s.status.value is SessionStatus.Closed) continue
            if (s.requestedSize.value != width to height) s.resize(width, height, s.scale.value)
        }
    }

    fun shutdown() {
        _sessions.value.forEach { it.close() }
        _sessions.value = emptyList()
        synchronized(lock) {
            ws?.close(1000, "bye")
            ws = null
        }
    }

    private fun find(id: String) = _sessions.value.firstOrNull { it.sessionId == id }

    private val listener = object : WebSocketListener() {
        override fun onMessage(webSocket: WebSocket, text: String) {
            when (val m = WsParser.parse(text)) {
                is WsIn.Opened -> {
                    val s = synchronized(pendingOpen) {
                        val i = pendingOpen.indexOfFirst { it.appId == m.appId }
                        if (i >= 0) pendingOpen.removeAt(i) else pendingOpen.removeFirstOrNull()
                    }
                    s?.onOpened(m.sessionId, m.title)
                }
                is WsIn.Offer -> find(m.sessionId)?.onOffer(m.sdp)
                is WsIn.Ice -> find(m.sessionId)?.onRemoteIce(m.candidate, m.sdpMid, m.sdpMLineIndex)
                is WsIn.Closed -> find(m.sessionId)?.onClosedByHost(m.reason)
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

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) =
            dropSocket(webSocket, "host closed the connection")

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
            val why = if (response?.code == 401) "unauthorised: pair again" else (t.message ?: "connection lost")
            lastError.value = why
            dropSocket(webSocket, "error: $why")
        }
    }

    private fun dropSocket(webSocket: WebSocket, reason: String) {
        synchronized(lock) { if (ws === webSocket) ws = null }
        _sessions.value.forEach { it.onClosedByHost(reason) }
        synchronized(pendingOpen) { pendingOpen.clear() }
    }
}
