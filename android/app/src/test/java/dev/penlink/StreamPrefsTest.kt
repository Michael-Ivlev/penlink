package dev.penlink

import dev.penlink.data.Quality
import dev.penlink.data.StreamPrefs
import dev.penlink.protocol.WsIn
import dev.penlink.protocol.WsOut
import dev.penlink.protocol.WsParser
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class StreamPrefsTest {
    private fun obj(s: String) = Json.parseToJsonElement(s).jsonObject

    @Test
    fun defaults_are_sharp_and_phone_app_sized() {
        val p = StreamPrefs()
        assertEquals(Quality.Sharp, p.quality)
        assertEquals(300, p.uiScale)
        assertEquals(3.0, p.scaleFactor, 0.0)
    }

    @Test
    fun encode_decode_roundtrip_and_junk() {
        val p = StreamPrefs(Quality.Fast, 250)
        assertEquals("fast:250", p.encode())
        assertEquals(p, StreamPrefs.decode(p.encode()))
        assertEquals(StreamPrefs(), StreamPrefs.decode(null))
        assertEquals(StreamPrefs(), StreamPrefs.decode("nonsense"))
        assertEquals(StreamPrefs(), StreamPrefs.decode("fast:abc"))
        // an unknown quality falls back to sharp, an odd size snaps to a step
        assertEquals(StreamPrefs(Quality.Sharp, 175), StreamPrefs.decode("weird:180"))
    }

    @Test
    fun snapping_goes_to_the_nearest_windows_step_ties_up() {
        assertEquals(100, StreamPrefs.snap(10))
        assertEquals(400, StreamPrefs.snap(900))
        assertEquals(300, StreamPrefs.snap(275))
        assertEquals(250, StreamPrefs.snap(260))
        assertEquals(125, StreamPrefs.snap(115))
        assertTrue(StreamPrefs.UI_STEPS.all { StreamPrefs.snap(it) == it })
    }

    @Test
    fun fast_asks_windows_for_half_the_dpi() {
        assertEquals(300, StreamPrefs.dpiAsked(StreamPrefs(Quality.Sharp, 300)))
        assertEquals(150, StreamPrefs.dpiAsked(StreamPrefs(Quality.Fast, 300)))
    }

    @Test
    fun open_and_resize_carry_quality_and_ui_scale() {
        val o = obj(WsOut.open("a", 1440, 3120, 3.0, quality = "fast", uiScale = 250))
        assertEquals("fast", o["quality"]!!.jsonPrimitive.content)
        assertEquals(250, o["uiScale"]!!.jsonPrimitive.content.toInt())
        assertEquals(3.0, o["scale"]!!.jsonPrimitive.content.toDouble(), 0.0)
        val r = obj(WsOut.resize("s", 1440, 1500, 3.0, "sharp", 300))
        assertEquals("sharp", r["quality"]!!.jsonPrimitive.content)
        assertEquals(300, r["uiScale"]!!.jsonPrimitive.content.toInt())
        // old shape is unchanged when the fields are not given
        assertFalse(WsOut.open("a", 1, 2, 1.0).contains("quality"))
        assertFalse(WsOut.resize("s", 1, 2, 1.0).contains("uiScale"))
    }

    @Test
    fun window_message_carries_mode_and_dpi() {
        assertEquals(
            WsIn.Window("s", 1440, 3120, false, "monitor", 300),
            WsParser.parse("""{"type":"window","sessionId":"s","width":1440,"height":3120,"fitted":false,"mode":"monitor","dpi":300}"""),
        )
        // a host without virtual monitors says window (or nothing at all)
        assertEquals(
            WsIn.Window("s", 812, 1760, true, "window", null),
            WsParser.parse("""{"type":"window","sessionId":"s","width":812,"height":1760,"fitted":true,"mode":"window"}"""),
        )
        assertEquals(
            WsIn.Window("s", 812, 1760, true),
            WsParser.parse("""{"type":"window","sessionId":"s","width":812,"height":1760,"fitted":true}"""),
        )
    }
}
