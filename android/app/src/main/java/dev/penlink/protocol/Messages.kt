package dev.penlink.protocol

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.contentOrNull
import kotlinx.serialization.json.intOrNull
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.put
import kotlinx.serialization.Serializable

const val DEFAULT_PORT = 47800

/** One installed app on the host (GET /apps). */
@Serializable
data class AppEntry(val id: String, val name: String)

@Serializable
data class PairRequest(val code: String, val deviceName: String)

@Serializable
data class PairResponse(val token: String, val hostName: String)

@Serializable
data class InfoResponse(val hostName: String, val version: String)

/** Phone -> host WebSocket messages (SPEC "Protocol"). Each returns the JSON text frame. */
object WsOut {
    /**
     * [attach]: re-open after a lost connection; the host attaches to the running app instead of launching it.
     * [quality] ("sharp" | "fast") and [uiScale] (percent) ask for a virtual monitor of the phone's size
     * (or half of it) with that UI size; a host without the driver answers with window capture.
     */
    fun open(
        appId: String, width: Int, height: Int, scale: Double, attach: Boolean = false,
        quality: String? = null, uiScale: Int? = null,
    ): String = obj {
        put("type", "open")
        put("appId", appId)
        put("width", width)
        put("height", height)
        put("scale", scale)
        if (attach) put("attach", true)
        if (quality != null) put("quality", quality)
        if (uiScale != null) put("uiScale", uiScale)
    }

    fun answer(sessionId: String, sdp: String): String = obj {
        put("type", "answer")
        put("sessionId", sessionId)
        put("sdp", sdp)
    }

    fun ice(sessionId: String, candidate: String, sdpMid: String?, sdpMLineIndex: Int): String = obj {
        put("type", "ice")
        put("sessionId", sessionId)
        put("candidate", candidate)
        put("sdpMid", sdpMid ?: "0")
        put("sdpMLineIndex", sdpMLineIndex)
    }

    fun resize(
        sessionId: String, width: Int, height: Int, scale: Double,
        quality: String? = null, uiScale: Int? = null,
    ): String = obj {
        put("type", "resize")
        put("sessionId", sessionId)
        put("width", width)
        put("height", height)
        put("scale", scale)
        if (quality != null) put("quality", quality)
        if (uiScale != null) put("uiScale", uiScale)
    }

    fun close(sessionId: String): String = obj {
        put("type", "close")
        put("sessionId", sessionId)
    }

    fun keyframe(sessionId: String): String = obj {
        put("type", "keyframe")
        put("sessionId", sessionId)
    }
}

/** Host -> phone WebSocket messages. */
sealed interface WsIn {
    data class Opened(val sessionId: String, val appId: String, val title: String) : WsIn
    /** Window title became known or changed (sent after `opened`). */
    data class Title(val sessionId: String, val title: String) : WsIn
    /**
     * What the video shows: [mode] "window" (the app's window; real size, [fitted] = shrunk to fit the PC screen,
     * video upscaled) or "monitor" (a virtual monitor of that size, [dpi] = the Windows DPI percent applied).
     */
    data class Window(
        val sessionId: String, val width: Int, val height: Int, val fitted: Boolean,
        val mode: String = "window", val dpi: Int? = null,
    ) : WsIn
    data class Offer(val sessionId: String, val sdp: String) : WsIn
    data class Ice(val sessionId: String, val candidate: String, val sdpMid: String?, val sdpMLineIndex: Int) : WsIn
    data class Closed(val sessionId: String, val reason: String) : WsIn
    data class Error(val message: String) : WsIn
}

object WsParser {
    fun parse(text: String): WsIn? {
        val o = try {
            Json.parseToJsonElement(text).jsonObject
        } catch (e: Exception) {
            return null
        }
        fun s(k: String) = (o[k] as? JsonPrimitive)?.contentOrNull
        return when (s("type")) {
            "opened" -> WsIn.Opened(s("sessionId") ?: return null, s("appId") ?: "", s("title") ?: "")
            "title" -> WsIn.Title(s("sessionId") ?: return null, s("title") ?: return null)
            "window" -> WsIn.Window(
                s("sessionId") ?: return null,
                (o["width"] as? JsonPrimitive)?.intOrNull ?: return null,
                (o["height"] as? JsonPrimitive)?.intOrNull ?: return null,
                (o["fitted"] as? JsonPrimitive)?.booleanOrNull ?: false,
                s("mode") ?: "window",
                (o["dpi"] as? JsonPrimitive)?.intOrNull,
            )
            "offer" -> WsIn.Offer(s("sessionId") ?: return null, s("sdp") ?: return null)
            "ice" -> WsIn.Ice(
                s("sessionId") ?: return null,
                s("candidate") ?: return null,
                s("sdpMid"),
                (o["sdpMLineIndex"] as? JsonPrimitive)?.intOrNull ?: 0,
            )
            "closed" -> WsIn.Closed(s("sessionId") ?: return null, s("reason") ?: "")
            "error" -> WsIn.Error(s("message") ?: "")
            else -> null
        }
    }
}

/** Toast text when the host ends a session ("Notepad closed: app exited"). */
fun describeClose(title: String, reason: String): String {
    val why = reason.removePrefix("error:").trim()
    return when {
        reason.isBlank() -> "$title closed"
        reason.startsWith("error:") -> "$title could not be opened: $why"
        else -> "$title closed: $why"
    }
}

enum class PenAction(val wire: String) { Hover("hover"), Down("down"), Move("move"), Up("up"), Leave("leave") }
enum class TouchAction(val wire: String) { Down("down"), Move("move"), Up("up"), Cancel("cancel") }
enum class MouseAction(val wire: String) { Move("move"), Down("down"), Up("up") }
enum class MouseButton(val wire: String) { Left("left"), Right("right"), Middle("middle") }

/** Data channel `input` messages (SPEC "Data channel input"). */
object InputMsg {
    /** Coordinates are rounded to 5 decimals (well below one pixel) to keep frames small. */
    private fun r5(v: Double): Double = Math.round(v * 100000.0) / 100000.0
    private fun r2(v: Double): Double = Math.round(v * 100.0) / 100.0

    fun pen(
        a: PenAction, x: Double, y: Double, pressure: Double,
        tiltX: Double, tiltY: Double, sideButton: Boolean, eraser: Boolean,
    ): String = obj {
        put("t", "pen")
        put("a", a.wire)
        put("x", r5(x))
        put("y", r5(y))
        put("p", r5(pressure.coerceIn(0.0, 1.0)))
        put("tx", r2(tiltX))
        put("ty", r2(tiltY))
        put("b", if (sideButton) 1 else 0)
        put("e", eraser)
    }

    fun touch(a: TouchAction, id: Int, x: Double, y: Double): String = obj {
        put("t", "touch")
        put("a", a.wire)
        put("id", id)
        put("x", r5(x))
        put("y", r5(y))
    }

    fun mouse(a: MouseAction, x: Double, y: Double, btn: MouseButton): String = obj {
        put("t", "mouse")
        put("a", a.wire)
        put("x", r5(x))
        put("y", r5(y))
        put("btn", btn.wire)
    }

    fun mouseRel(dx: Double, dy: Double): String = obj {
        put("t", "mrel")
        put("dx", r2(dx))
        put("dy", r2(dy))
    }

    fun wheel(dx: Int, dy: Int): String = obj {
        put("t", "wheel")
        put("dx", dx)
        put("dy", dy)
    }

    fun key(down: Boolean, vk: Int): String = obj {
        put("t", "key")
        put("a", if (down) "down" else "up")
        put("vk", vk)
    }

    fun text(s: String): String = obj {
        put("t", "text")
        put("s", s)
    }
}

private inline fun obj(block: kotlinx.serialization.json.JsonObjectBuilder.() -> Unit): String =
    buildJsonObject(block).toString()

