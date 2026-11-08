package dev.penlink

import dev.penlink.protocol.HostAddress
import dev.penlink.protocol.InputMsg
import dev.penlink.protocol.MouseAction
import dev.penlink.protocol.MouseButton
import dev.penlink.protocol.PairRequest
import dev.penlink.protocol.PenAction
import dev.penlink.protocol.TouchAction
import dev.penlink.protocol.WsIn
import dev.penlink.protocol.WsOut
import dev.penlink.protocol.WsParser
import dev.penlink.protocol.describeClose
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.double
import kotlinx.serialization.json.int
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ProtocolTest {
    private fun parse(s: String): JsonObject = Json.parseToJsonElement(s).jsonObject
    private fun JsonObject.str(k: String) = getValue(k).jsonPrimitive.content
    private fun JsonObject.keys2() = keys.toSet()

    @Test
    fun open_shape() {
        val o = parse(WsOut.open("app1", 1440, 3120, 2.0))
        assertEquals(setOf("type", "appId", "width", "height", "scale"), o.keys2())
        assertEquals("open", o.str("type"))
        assertEquals("app1", o.str("appId"))
        assertEquals(1440, o.getValue("width").jsonPrimitive.int)
        assertEquals(3120, o.getValue("height").jsonPrimitive.int)
        assertEquals(2.0, o.getValue("scale").jsonPrimitive.double, 0.0)
    }

    @Test
    fun answer_ice_resize_close_keyframe_shape() {
        val a = parse(WsOut.answer("s1", "v=0"))
        assertEquals(setOf("type", "sessionId", "sdp"), a.keys2())
        assertEquals("answer", a.str("type"))
        assertEquals("v=0", a.str("sdp"))

        val i = parse(WsOut.ice("s1", "candidate:1 1 udp 1 1.2.3.4 5 typ host", "0", 0))
        assertEquals(setOf("type", "sessionId", "candidate", "sdpMid", "sdpMLineIndex"), i.keys2())
        assertEquals("ice", i.str("type"))
        assertEquals("0", i.str("sdpMid"))
        assertEquals(0, i.getValue("sdpMLineIndex").jsonPrimitive.int)

        val r = parse(WsOut.resize("s1", 3120, 1440, 2.5))
        assertEquals(setOf("type", "sessionId", "width", "height", "scale"), r.keys2())
        assertEquals("resize", r.str("type"))
        assertEquals(2.5, r.getValue("scale").jsonPrimitive.double, 0.0)

        val c = parse(WsOut.close("s1"))
        assertEquals(setOf("type", "sessionId"), c.keys2())
        assertEquals("close", c.str("type"))

        val k = parse(WsOut.keyframe("s1"))
        assertEquals(setOf("type", "sessionId"), k.keys2())
        assertEquals("keyframe", k.str("type"))
    }

    @Test
    fun host_messages_parse() {
        assertEquals(
            WsIn.Opened("s1", "app", "Notepad"),
            WsParser.parse("""{"type":"opened","sessionId":"s1","appId":"app","title":"Notepad"}"""),
        )
        assertEquals(WsIn.Offer("s1", "sdp"), WsParser.parse("""{"type":"offer","sessionId":"s1","sdp":"sdp"}"""))
        assertEquals(
            WsIn.Ice("s1", "cand", "0", 1),
            WsParser.parse("""{"type":"ice","sessionId":"s1","candidate":"cand","sdpMid":"0","sdpMLineIndex":1}"""),
        )
        assertEquals(
            WsIn.Closed("s1", "app exited"),
            WsParser.parse("""{"type":"closed","sessionId":"s1","reason":"app exited"}"""),
        )
        assertEquals(WsIn.Error("bad"), WsParser.parse("""{"type":"error","message":"bad"}"""))
        assertNull(WsParser.parse("not json"))
        assertNull(WsParser.parse("""{"type":"mystery"}"""))
    }

    @Test
    fun opened_with_empty_title_then_title_update() {
        assertEquals(
            WsIn.Opened("s1", "app", ""),
            WsParser.parse("""{"type":"opened","sessionId":"s1","appId":"app","title":""}"""),
        )
        assertEquals(
            WsIn.Title("s1", "Untitled - Notepad"),
            WsParser.parse("""{"type":"title","sessionId":"s1","title":"Untitled - Notepad"}"""),
        )
        assertNull(WsParser.parse("""{"type":"title","sessionId":"s1"}"""))
    }

    @Test
    fun close_notice_text() {
        assertEquals("Notepad closed: app exited", describeClose("Notepad", "app exited"))
        assertEquals("Foo could not be opened: timed out waiting for the app window",
            describeClose("Foo", "error: timed out waiting for the app window"))
        assertEquals("Foo closed", describeClose("Foo", ""))
    }

    @Test
    fun pair_request_shape() {
        val s = Json.encodeToString(PairRequest.serializer(), PairRequest("123456", "S26 Ultra"))
        val o = parse(s)
        assertEquals(setOf("code", "deviceName"), o.keys2())
        assertEquals("123456", o.str("code"))
        assertEquals("S26 Ultra", o.str("deviceName"))
    }

    @Test
    fun pen_shape() {
        val o = parse(InputMsg.pen(PenAction.Down, 0.5, 0.25, 0.6, 10.0, -5.0, sideButton = true, eraser = false))
        assertEquals(setOf("t", "a", "x", "y", "p", "tx", "ty", "b", "e"), o.keys2())
        assertEquals("pen", o.str("t"))
        assertEquals("down", o.str("a"))
        assertEquals(0.5, o.getValue("x").jsonPrimitive.double, 1e-9)
        assertEquals(0.25, o.getValue("y").jsonPrimitive.double, 1e-9)
        assertEquals(0.6, o.getValue("p").jsonPrimitive.double, 1e-9)
        assertEquals(10.0, o.getValue("tx").jsonPrimitive.double, 1e-9)
        assertEquals(-5.0, o.getValue("ty").jsonPrimitive.double, 1e-9)
        assertEquals(1, o.getValue("b").jsonPrimitive.int)
        assertEquals(false, o.getValue("e").jsonPrimitive.boolean)
    }

    @Test
    fun pen_actions_and_eraser() {
        for ((a, w) in listOf(
            PenAction.Hover to "hover", PenAction.Down to "down", PenAction.Move to "move",
            PenAction.Up to "up", PenAction.Leave to "leave",
        )) {
            assertEquals(w, parse(InputMsg.pen(a, 0.0, 0.0, 0.0, 0.0, 0.0, false, true)).str("a"))
        }
        val o = parse(InputMsg.pen(PenAction.Hover, 0.0, 0.0, 0.0, 0.0, 0.0, false, true))
        assertEquals(true, o.getValue("e").jsonPrimitive.boolean)
        assertEquals(0, o.getValue("b").jsonPrimitive.int)
    }

    @Test
    fun pen_pressure_is_clamped() {
        assertEquals(1.0, parse(InputMsg.pen(PenAction.Move, 0.0, 0.0, 1.7, 0.0, 0.0, false, false)).getValue("p").jsonPrimitive.double, 0.0)
        assertEquals(0.0, parse(InputMsg.pen(PenAction.Move, 0.0, 0.0, -0.2, 0.0, 0.0, false, false)).getValue("p").jsonPrimitive.double, 0.0)
    }

    @Test
    fun touch_shape() {
        val o = parse(InputMsg.touch(TouchAction.Down, 2, 0.1, 0.9))
        assertEquals(setOf("t", "a", "id", "x", "y"), o.keys2())
        assertEquals("touch", o.str("t"))
        assertEquals("down", o.str("a"))
        assertEquals(2, o.getValue("id").jsonPrimitive.int)
        for ((a, w) in listOf(TouchAction.Move to "move", TouchAction.Up to "up", TouchAction.Cancel to "cancel")) {
            assertEquals(w, parse(InputMsg.touch(a, 0, 0.0, 0.0)).str("a"))
        }
    }

    @Test
    fun mouse_shapes() {
        val o = parse(InputMsg.mouse(MouseAction.Down, 0.3, 0.4, MouseButton.Right))
        assertEquals(setOf("t", "a", "x", "y", "btn"), o.keys2())
        assertEquals("mouse", o.str("t"))
        assertEquals("down", o.str("a"))
        assertEquals("right", o.str("btn"))
        assertEquals("left", parse(InputMsg.mouse(MouseAction.Move, 0.0, 0.0, MouseButton.Left)).str("btn"))
        assertEquals("middle", parse(InputMsg.mouse(MouseAction.Up, 0.0, 0.0, MouseButton.Middle)).str("btn"))

        val r = parse(InputMsg.mouseRel(3.5, -2.0))
        assertEquals(setOf("t", "dx", "dy"), r.keys2())
        assertEquals("mrel", r.str("t"))
        assertEquals(3.5, r.getValue("dx").jsonPrimitive.double, 1e-9)
        assertEquals(-2.0, r.getValue("dy").jsonPrimitive.double, 1e-9)
    }

    @Test
    fun wheel_key_text_shapes() {
        val w = parse(InputMsg.wheel(0, -120))
        assertEquals(setOf("t", "dx", "dy"), w.keys2())
        assertEquals("wheel", w.str("t"))
        assertEquals(-120, w.getValue("dy").jsonPrimitive.int)

        val k = parse(InputMsg.key(true, 17))
        assertEquals(setOf("t", "a", "vk"), k.keys2())
        assertEquals("key", k.str("t"))
        assertEquals("down", k.str("a"))
        assertEquals(17, k.getValue("vk").jsonPrimitive.int)
        assertEquals("up", parse(InputMsg.key(false, 17)).str("a"))

        val t = parse(InputMsg.text("héllo \"q\""))
        assertEquals(setOf("t", "s"), t.keys2())
        assertEquals("text", t.str("t"))
        assertEquals("héllo \"q\"", t.str("s"))
    }

    @Test
    fun host_address_normalisation() {
        assertEquals("10.0.2.2:47800", HostAddress.normalize("10.0.2.2"))
        assertEquals("10.0.2.2:47800", HostAddress.normalize(" http://10.0.2.2/ "))
        assertEquals("pc:5000", HostAddress.normalize("pc:5000"))
        assertEquals("100.64.1.2:47800", HostAddress.normalize("100.64.1.2:47800"))
        assertNull(HostAddress.normalize("   "))
        assertEquals("ws://10.0.2.2:47800/ws?token=abc", HostAddress.wsUrl("10.0.2.2:47800", "abc"))
        assertTrue(HostAddress.httpBase("h:1").startsWith("http://"))
    }
}
