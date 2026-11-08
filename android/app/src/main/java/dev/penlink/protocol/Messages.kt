package dev.penlink.protocol

import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.buildJsonObject
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
    fun open(appId: String, width: Int, height: Int, scale: Double): String = obj {
        put("type", "open")
        put("appId", appId)
        put("width", width)
        put("height", height)
        put("scale", scale)
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

    fun resize(sessionId: String, width: Int, height: Int, scale: Double): String = obj {
        put("type", "resize")
        put("sessionId", sessionId)
        put("width", width)
        put("height", height)
        put("scale", scale)
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

