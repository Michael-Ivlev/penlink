package dev.penlink.rtc

import kotlinx.coroutines.flow.MutableStateFlow
import org.json.JSONObject
import org.webrtc.RTCStatsReport

/**
 * Debug latency readout for one session. The host splits what it can see (input received -> injected ->
 * captured -> encoded -> sent) into `lat` messages on the data channel; this class adds what only the phone
 * can see (data channel round trip from ping/pong, jitter buffer and decode time from the WebRTC stats) and
 * puts both into one short text for the on-screen overlay.
 */
class LatencyStats {
    val text = MutableStateFlow("")

    private val window = 24
    private val host = Array(5) { ArrayDeque<Double>() } // inj, cap, conv, enc, snd (ms)
    private val rtt = ArrayDeque<Double>()
    private var jitterMs = 0.0
    private var decodeMs = 0.0
    private var processingMs = 0.0
    private var fps = 0.0
    private var dropped = 0.0
    private var iceRttMs = 0.0
    private var last: Map<String, Double> = emptyMap()

    /** Microseconds on the phone's monotonic clock; the host echoes it back in `pong`. */
    fun nowUs(): Long = System.nanoTime() / 1000

    fun pingJson(): String = """{"t":"ping","c":${nowUs()}}"""

    /** A message from the host on the data channel; returns true if it was a latency message. */
    fun onMessage(json: String): Boolean {
        val o = try { JSONObject(json) } catch (_: Exception) { return false }
        when (o.optString("t")) {
            "pong" -> {
                push(rtt, (nowUs() - o.optLong("c")) / 1000.0)
            }
            "lat" -> {
                val keys = arrayOf("inj", "cap", "conv", "enc", "snd")
                for (i in keys.indices) push(host[i], o.optLong(keys[i]) / 1000.0)
            }
            else -> return false
        }
        refresh()
        return true
    }

    /** `PeerConnection.getStats` result (called about once a second). */
    fun onStats(report: RTCStatsReport) {
        var inbound: Map<String, Any>? = null
        for (s in report.statsMap.values) {
            if (s.type == "inbound-rtp" && s.members["kind"] == "video") inbound = s.members
            if (s.type == "candidate-pair" && s.members["state"] == "succeeded" && s.members["nominated"] == true) {
                (s.members["currentRoundTripTime"] as? Number)?.let { iceRttMs = it.toDouble() * 1000 }
            }
        }
        val m = inbound ?: return
        fun num(k: String) = (m[k] as? Number)?.toDouble() ?: 0.0
        val cur = mapOf(
            "jbd" to num("jitterBufferDelay"), "jbe" to num("jitterBufferEmittedCount"),
            "dec" to num("totalDecodeTime"), "fd" to num("framesDecoded"),
            "proc" to num("totalProcessingDelay"), "fdrop" to num("framesDropped"),
        )
        val prev = last
        last = cur
        fps = num("framesPerSecond")
        if (prev.isNotEmpty()) {
            val de = cur.getValue("jbe") - prev.getValue("jbe")
            if (de > 0) jitterMs = (cur.getValue("jbd") - prev.getValue("jbd")) / de * 1000
            val df = cur.getValue("fd") - prev.getValue("fd")
            if (df > 0) {
                decodeMs = (cur.getValue("dec") - prev.getValue("dec")) / df * 1000
                processingMs = (cur.getValue("proc") - prev.getValue("proc")) / df * 1000
            }
            dropped = cur.getValue("fdrop") - prev.getValue("fdrop")
        }
        refresh()
    }

    private fun push(q: ArrayDeque<Double>, v: Double) {
        q.addLast(v)
        while (q.size > window) q.removeFirst()
    }

    private fun avg(q: ArrayDeque<Double>) = if (q.isEmpty()) 0.0 else q.sum() / q.size

    private fun refresh() {
        val r = avg(rtt)
        val h = host.map { avg(it) }
        val hostTotal = h.sum()
        val phone = jitterMs + decodeMs
        // input -> host (half the round trip) + host + video back (half) + jitter buffer + decode + a display frame
        val est = r + hostTotal + phone + 8.0
        text.value = "net rtt %.1f (ice %.1f)  fps %.0f drop %.0f\nhost %.0f = inj %.1f cap %.1f conv %.1f enc %.1f snd %.1f\nphone jitter %.1f dec %.1f (proc %.1f)\nest. touch->pixel %.0f ms"
            .format(r, iceRttMs, fps, dropped, hostTotal, h[0], h[1], h[2], h[3], h[4], jitterMs, decodeMs, processingMs, est)
    }
}
