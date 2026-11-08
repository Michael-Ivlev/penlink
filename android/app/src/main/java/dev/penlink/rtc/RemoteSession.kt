package dev.penlink.rtc

import dev.penlink.protocol.WsOut
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import org.webrtc.DataChannel
import org.webrtc.IceCandidate
import org.webrtc.MediaConstraints
import org.webrtc.MediaStream
import org.webrtc.PeerConnection
import org.webrtc.RtpReceiver
import org.webrtc.RtpTransceiver
import org.webrtc.SdpObserver
import org.webrtc.SessionDescription
import org.webrtc.VideoTrack
import java.nio.ByteBuffer

sealed interface SessionStatus {
    data object Opening : SessionStatus
    data object Connecting : SessionStatus
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
    scale: Double,
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
    val scale = MutableStateFlow(scale)

    private var pc: PeerConnection? = null
    private var dc: DataChannel? = null
    private val pendingRemoteIce = ArrayList<IceCandidate>()
    private var remoteSet = false

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

    fun resize(width: Int, height: Int, scale: Double) {
        requestedSize.value = width to height
        this.scale.value = scale
        sessionId?.let { send(WsOut.resize(it, width, height, scale)) }
    }

    /** Local close: tell the host and free the peer connection. */
    fun close() {
        sessionId?.let { send(WsOut.close(it)) }
        if (_status.value !is SessionStatus.Closed) _status.value = SessionStatus.Closed("closed")
        release()
    }

    /** Send one `input` data-channel JSON message. Returns false when the channel is not open. */
    fun sendInput(json: String): Boolean {
        val ch = dc ?: return false
        if (ch.state() != DataChannel.State.OPEN) return false
        return ch.send(DataChannel.Buffer(ByteBuffer.wrap(json.toByteArray(Charsets.UTF_8)), false))
    }

    private fun release() {
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
            if (channel.label() == "input") dc = channel
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
