package dev.penlink.notify

import dev.penlink.data.SavedHost
import dev.penlink.protocol.NotifyIn
import dev.penlink.protocol.NotifyOut
import dev.penlink.protocol.NotifyParser
import dev.penlink.protocol.notifyWsUrl
import kotlinx.coroutines.CompletableDeferred
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import java.util.concurrent.TimeUnit

/** What the notification socket to one PC is doing. */
sealed interface NotifyStatus {
    data object Connecting : NotifyStatus

    /** Up. [access] is the host's report on Windows' permission: allowed, denied, unspecified or unavailable. */
    data class Connected(val access: String) : NotifyStatus

    /** The PC refused the token (unpaired or revoked): retrying is pointless until the user pairs again. */
    data object Unauthorized : NotifyStatus
}

/**
 * The long-lived `/notify` WebSocket to one PC. Reconnects forever with [NotifyBackoff] until [stop].
 * [onEvent] runs on a background thread, in order.
 */
class NotifyLink(
    val host: SavedHost,
    http: OkHttpClient,
    private val scope: CoroutineScope,
    private val onEvent: (NotifyIn) -> Unit,
) {
    // No read timeout: the host pings every 4 s, OkHttp pings every 20 s, and a dead link ends the socket.
    private val client = http.newBuilder().readTimeout(0, TimeUnit.MILLISECONDS).pingInterval(20, TimeUnit.SECONDS).build()
    private val _status = MutableStateFlow<NotifyStatus>(NotifyStatus.Connecting)
    val status: StateFlow<NotifyStatus> = _status.asStateFlow()

    @Volatile
    private var ws: WebSocket? = null
    private var job: Job? = null

    fun start() {
        if (job?.isActive == true) return
        job = scope.launch {
            var failures = 0
            while (isActive) {
                _status.value = NotifyStatus.Connecting
                val closed = CompletableDeferred<Boolean>() // true = it had been ready
                var ready = false
                var unauthorized = false
                val req = Request.Builder().url(notifyWsUrl(host.address, host.token)).build()
                val socket = client.newWebSocket(req, object : WebSocketListener() {
                    override fun onMessage(webSocket: WebSocket, text: String) {
                        val m = NotifyParser.parse(text) ?: return
                        if (m is NotifyIn.Ready) {
                            ready = true
                            _status.value = NotifyStatus.Connected(m.access)
                        }
                        runCatching { onEvent(m) }
                    }

                    override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                        webSocket.close(1000, null)
                    }

                    override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                        closed.complete(ready)
                    }

                    override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                        if (response?.code == 401) unauthorized = true
                        closed.complete(ready)
                    }
                })
                ws = socket
                val wasReady = try {
                    closed.await()
                } finally {
                    ws = null
                    socket.cancel()
                }
                if (unauthorized) {
                    _status.value = NotifyStatus.Unauthorized
                    return@launch
                }
                failures = if (wasReady) 0 else failures + 1
                _status.value = NotifyStatus.Connecting
                delay(NotifyBackoff.delayMs(if (wasReady) 0 else failures - 1))
            }
        }
    }

    /** Ask the PC to remove one notification. False when the socket is not up. */
    fun dismiss(id: Long): Boolean = ws?.send(NotifyOut.dismiss(id)) ?: false

    fun stop() {
        job?.cancel()
        job = null
        ws?.close(1000, null)
        ws = null
    }
}
