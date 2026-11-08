package dev.penlink.rtc

import android.content.Context
import android.content.pm.ApplicationInfo
import android.net.wifi.WifiManager
import kotlinx.coroutines.flow.MutableStateFlow
import org.webrtc.LowLatencyDecoderFactory
import org.webrtc.DefaultVideoEncoderFactory
import org.webrtc.EglBase
import org.webrtc.PeerConnectionFactory

/** Process-wide WebRTC factory and EGL context (shared by all sessions and renderers). */
class Rtc(context: Context) {
    private val appContext = context.applicationContext
    private var lowLatencyLock: WifiManager.WifiLock? = null
    private var lockUsers = 0

    /** Whether the latency readout is shown on the session screen (debug builds by default; `--ez latency false` hides it). */
    val latencyOverlay = MutableStateFlow(
        (appContext.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0,
    )

    /**
     * Holds the Wi-Fi radio in its low-latency mode (no power-save batching of packets, which on a phone is what
     * turns a 3 ms link into 30-70 ms pings) while at least one stream is open.
     */
    @Synchronized
    fun acquireLowLatencyWifi() {
        if (lockUsers++ > 0) return
        runCatching {
            val wm = appContext.getSystemService(Context.WIFI_SERVICE) as WifiManager
            lowLatencyLock = wm.createWifiLock(WifiManager.WIFI_MODE_FULL_LOW_LATENCY, "penlink:stream").apply {
                setReferenceCounted(false)
                acquire()
            }
        }
    }

    @Synchronized
    fun releaseLowLatencyWifi() {
        if (lockUsers == 0 || --lockUsers > 0) return
        runCatching { lowLatencyLock?.release() }
        lowLatencyLock = null
    }

    val eglBase: EglBase = EglBase.create()
    val factory: PeerConnectionFactory

    init {
        PeerConnectionFactory.initialize(
            PeerConnectionFactory.InitializationOptions.builder(context.applicationContext)
                // The host sends playout-delay 0..0; with that the receiver decodes each frame as soon
                // as it is complete instead of pacing it.
                .setFieldTrials("WebRTC-ZeroPlayoutDelay/min_pacing:0ms,max_decode_queue_size:4/")
                .createInitializationOptions(),
        )
        factory = PeerConnectionFactory.builder()
            .setVideoDecoderFactory(LowLatencyDecoderFactory(eglBase.eglBaseContext))
            .setVideoEncoderFactory(DefaultVideoEncoderFactory(eglBase.eglBaseContext, true, true))
            .createPeerConnectionFactory()
    }
}
