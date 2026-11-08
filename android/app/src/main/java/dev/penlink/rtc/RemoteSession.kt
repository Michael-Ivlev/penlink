package dev.penlink.rtc

import dev.penlink.data.StreamPrefs
import dev.penlink.protocol.WsOut
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.webrtc.DataChannel
import org.webrtc.IceCandidate
import org.webrtc.MediaConstraints
import org.webrtc.MediaStream
import org.webrtc.PeerConnection
import org.webrtc.RTCStatsCollectorCallback
import org.webrtc.RtpReceiver
import org.webrtc.RtpTransceiver
import org.webrtc.SdpObserver
import org.webrtc.SessionDescription
import org.webrtc.VideoTrack
import android.os.Handler
import android.os.Looper
import java.nio.ByteBuffer

/**
 * What the host streams, as last reported: the size in PC pixels, [fitted] = a window shrunk to fit the PC screen,
 * [mode] "window" or "monitor" (a virtual monitor), [dpi] = the Windows DPI percent applied in monitor mode.
 */
data class WindowSize(
    val width: Int, val height: Int, val fitted: Boolean,
    val mode: String = "window", val dpi: Int? = null,
) {
    val isMonitor: Boolean get() = mode == "monitor"
}

sealed interface SessionStatus {
    data object Opening : SessionStatus
    data object Connecting : SessionStatus

    /** The link to the host dropped; the tab comes back by itself when the host does. */
    data object Reconnecting : SessionStatus
    data object Live : SessionStatus
    data class Closed(val reason: String) : SessionStatus
}

/**
 * One opened app: one PeerConnection (host offers, we answer), one video track, one `input` data channel.
 * [send] writes a signalling JSON frame to the host WebSocket.
 */
class RemoteSession(
    val localId: Long,
    val appId: String,
    val appName: String,
    width: Int,
    height: Int,
    prefs: StreamPrefs,
    private val rtc: Rtc,
    private val send: (String) -> Boolean,
) {
    @Volatile
    var sessionId: String? = null
        private set

    private val _title = MutableStateFlow(appName)
    val title: StateFlow<String> = _title.asStateFlow()

    private val _status = MutableStateFlow<SessionStatus>(SessionStatus.Opening)
    val status: StateFlow<SessionStatus> = _status.asStateFlow()

    private val _track = MutableStateFlow<VideoTrack?>(null)
    val videoTrack: StateFlow<VideoTrack?> = _track.asStateFlow()

    /** Last video frame size reported by the renderer (0,0 until the first frame). */
    val videoSize = MutableStateFlow(0 to 0)

    val requestedSize = MutableStateFlow(width to height)

    /** Quality and UI size of this session (changed from the settings sheet). */
    val prefs = MutableStateFlow(prefs)

    /** Real PC window size as last reported by the host (null until the first `window` message). */
    val windowSize = MutableStateFlow<WindowSize?>(null)

    /** Debug readout of where the touch-to-pixel time goes (shown by the session screen's latency overlay). */
    val latency = LatencyStats()

    private var pc: PeerConnection? = null
    private var dc: DataChannel? = null

    /** Unordered, unreliable channel for pointer moves (the host opens it next to `input`). */
    private var pdc: DataChannel? = null
    private var moveSeq = 0L
    private var holdsWifiLock = false
    private val ui = Handler(Looper.getMainLooper())
    private var statsLoop: Runnable? = null
    private val pendingRemoteIce = ArrayList<IceCandidate>()
    private var remoteSet = false

    /** The host has accepted this app at least once, so after a reconnect it is re-attached, not relaunched. */
    @Volatile
    private var launchedBefore = false

    /** Send `open` (first time: launch the app; after a lost link: attach to the running app). */
    fun sendOpen(): Boolean {
        if (_status.value is SessionStatus.Closed) return false
        _status.value = SessionStatus.Opening
        val (w, h) = requestedSize.value
        val p = prefs.value
        return send(WsOut.open(appId, w, h, p.scaleFactor, attach = launchedBefore, quality = p.quality.wire, uiScale = p.uiScale))
    }

    /** The WebSocket dropped: forget the host-side session and peer connection, wait for the link. */
    fun onLinkLost() {
        if (_status.value is SessionStatus.Closed) return
        release()
        sessionId = null
        synchronized(pendingRemoteIce) {
            remoteSet = false
            pendingRemoteIce.clear()
        }
        windowSize.value = null
        _status.value = SessionStatus.Reconnecting
    }

    fun onWindow(width: Int, height: Int, fitted: Boolean, mode: String = "window", dpi: Int? = null) {
        windowSize.value = WindowSize(width, height, fitted, mode, dpi)
    }

    fun onOpened(id: String, title: String) {
        sessionId = id
        if (title.isNotBlank()) _title.value = title
        if (_status.value == SessionStatus.Opening) _status.value = SessionStatus.Connecting
    }

    fun onTitle(title: String) {
        if (title.isNotBlank()) _title.value = title
    }

    fun onOffer(sdp: String) {
        val id = sessionId ?: return
        launchedBefore = true // the host found the app's window: after a reconnect attach to it
        val config = PeerConnection.RTCConfiguration(emptyList()).apply {
            sdpSemantics = PeerConnection.SdpSemantics.UNIFIED_PLAN
            continualGatheringPolicy = PeerConnection.ContinualGatheringPolicy.GATHER_CONTINUALLY
        }
        val conn = rtc.factory.createPeerConnection(config, observer)
        if (conn == null) {
            _status.value = SessionStatus.Closed("error: could not create peer connection")
            return
        }
        pc = conn
        if (!holdsWifiLock) {
            holdsWifiLock = true
            rtc.acquireLowLatencyWifi()
        }
        conn.setRemoteDescription(object : SimpleSdp() {
            override fun onSetSuccess() {
                synchronized(pendingRemoteIce) {
                    remoteSet = true
                    pendingRemoteIce.forEach { conn.addIceCandidate(it) }
                    pendingRemoteIce.clear()
                }
                conn.createAnswer(object : SimpleSdp() {
                    override fun onCreateSuccess(desc: SessionDescription) {
                        conn.setLocalDescription(object : SimpleSdp() {
                            override fun onSetSuccess() {
                                send(WsOut.answer(id, desc.description))
                            }
                        }, desc)
                    }
                }, MediaConstraints())
            }
        }, SessionDescription(SessionDescription.Type.OFFER, sdp))
    }

    fun onRemoteIce(candidate: String, sdpMid: String?, sdpMLineIndex: Int) {
        val ice = IceCandidate(sdpMid, sdpMLineIndex, candidate)
        val conn = pc
        synchronized(pendingRemoteIce) {
            if (conn != null && remoteSet) conn.addIceCandidate(ice) else pendingRemoteIce += ice
        }
    }

    fun onClosedByHost(reason: String) {
        if (_status.value is SessionStatus.Closed) return
        _status.value = SessionStatus.Closed(reason)
        release()
    }

    fun requestKeyframe() {
        sessionId?.let { send(WsOut.keyframe(it)) }
    }

    /** The screen area changed (rotation, keyboard): ask the host for the new size. */
    fun resize(width: Int, height: Int) {
        requestedSize.value = width to height
        sendResize()
    }

    /** Quality or UI size changed in the settings sheet. */
    fun setPrefs(p: StreamPrefs) {
        prefs.value = p
        sendResize()
    }

    private fun sendResize() {
        val (w, h) = requestedSize.value
        val p = prefs.value
        sessionId?.let { send(WsOut.resize(it, w, h, p.scaleFactor, p.quality.wire, p.uiScale)) }
    }

    /** Local close: tell the host and free the peer connection. */
    fun close() {
        sessionId?.let { send(WsOut.close(it)) }
        if (_status.value !is SessionStatus.Closed) _status.value = SessionStatus.Closed("closed")
        release()
    }

    /** Send one `input` data-channel JSON message. Returns false when the channel is not open. */
    fun sendInput(json: String): Boolean {
        // Pointer moves: a lost or late one is replaced by the next, so they take the unreliable channel and
        // never wait behind a retransmission; presses, releases and keys stay reliable and ordered.
        val pch = pdc
        if (pch != null && isPointerMove(json) && pch.state() == DataChannel.State.OPEN) {
            val tagged = "{\"n\":${++moveSeq}," + json.substring(1)
            if (pch.send(DataChannel.Buffer(ByteBuffer.wrap(tagged.toByteArray(Charsets.UTF_8)), false))) return true
        }
        val ch = dc ?: return false
        if (ch.state() != DataChannel.State.OPEN) return false
        return ch.send(DataChannel.Buffer(ByteBuffer.wrap(json.toByteArray(Charsets.UTF_8)), false))
    }

    private fun isPointerMove(j: String) =
        j.startsWith("{\"t\":\"pen\",\"a\":\"move\"") || j.startsWith("{\"t\":\"pen\",\"a\":\"hover\"") ||
            j.startsWith("{\"t\":\"touch\",\"a\":\"move\"") || j.startsWith("{\"t\":\"mouse\",\"a\":\"move\"")

    /** While the overlay is on: ping the host and read the WebRTC stats once a second. */
    private fun startStatsLoop() {
        if (statsLoop != null) return
        val r = object : Runnable {
            override fun run() {
                val conn = pc ?: run { statsLoop = null; return }
                if (rtc.latencyOverlay.value) {
                    sendInput(latency.pingJson())
                    conn.getStats(RTCStatsCollectorCallback { latency.onStats(it) })
                }
                ui.postDelayed(this, 1000)
            }
        }
        statsLoop = r
        ui.postDelayed(r, 1000)
    }

    private val inputObserver = object : DataChannel.Observer {
        override fun onBufferedAmountChange(previousAmount: Long) {}
        override fun onStateChange() {}
        override fun onMessage(buffer: DataChannel.Buffer) {
            if (buffer.binary) return
            val bytes = ByteArray(buffer.data.remaining())
            buffer.data.get(bytes)
            latency.onMessage(String(bytes, Charsets.UTF_8))
        }
    }

    private fun release() {
        statsLoop?.let { ui.removeCallbacks(it) }
        statsLoop = null
        if (holdsWifiLock) {
            holdsWifiLock = false
            rtc.releaseLowLatencyWifi()
        }
        runCatching { pdc?.unregisterObserver(); pdc?.close() }
        pdc = null
        runCatching { dc?.unregisterObserver(); dc?.close() }
        runCatching { pc?.close() }
        dc = null
        pc = null
        _track.value = null
    }

    private val observer = object : PeerConnection.Observer {
        override fun onSignalingChange(state: PeerConnection.SignalingState) {}
        override fun onIceConnectionChange(state: PeerConnection.IceConnectionState) {
            when (state) {
                PeerConnection.IceConnectionState.CONNECTED,
                PeerConnection.IceConnectionState.COMPLETED -> {
                    if (_status.value !is SessionStatus.Closed) _status.value = SessionStatus.Live
                    requestKeyframe()
                    startStatsLoop()
                }
                PeerConnection.IceConnectionState.FAILED ->
                    if (_status.value !is SessionStatus.Closed) {
                        _status.value = SessionStatus.Closed("error: connection failed")
                    }
                else -> {}
            }
        }

        override fun onIceConnectionReceivingChange(receiving: Boolean) {}
        override fun onIceGatheringChange(state: PeerConnection.IceGatheringState) {}
        override fun onIceCandidate(c: IceCandidate) {
            val id = sessionId ?: return
            send(WsOut.ice(id, c.sdp, c.sdpMid, c.sdpMLineIndex))
        }

        override fun onIceCandidatesRemoved(candidates: Array<out IceCandidate>) {}
        override fun onAddStream(stream: MediaStream) {}
        override fun onRemoveStream(stream: MediaStream) {}
        override fun onDataChannel(channel: DataChannel) {
            when (channel.label()) {
                "input" -> {
                    dc = channel
                    channel.registerObserver(inputObserver)
                }
                "pointer" -> pdc = channel
            }
        }

        override fun onRenegotiationNeeded() {}
        override fun onAddTrack(receiver: RtpReceiver, streams: Array<out MediaStream>) {}
        override fun onTrack(transceiver: RtpTransceiver) {
            (transceiver.receiver.track() as? VideoTrack)?.let {
                it.setEnabled(true)
                _track.value = it
            }
        }
    }
}

open class SimpleSdp : SdpObserver {
    override fun onCreateSuccess(desc: SessionDescription) {}
    override fun onSetSuccess() {}
    override fun onCreateFailure(error: String?) {}
    override fun onSetFailure(error: String?) {}
}
