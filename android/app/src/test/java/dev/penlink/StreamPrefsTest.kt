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
import org.junit.Test

class StreamPrefsTest {
    private fun obj(s: String) = Json.parseToJsonElement(s).jsonObject

    @Test
    fun default_is_low_resolution_at_200_percent_without_sharp_text() {
        val p = StreamPrefs()
        assertFalse(p.sharpText)
        assertEquals(Quality.Fast, p.quality)
        assertEquals(200, p.uiScale)
        // 1440x3120 / 2.0 = the 720x1560 monitor the old Fast gave
        assertEquals(720, 1440 * 100 / p.uiScale)
        assertEquals(1560, 3120 * 100 / p.uiScale)
    }

    @Test
    fun encode_decode_roundtrip() {
        val p = StreamPrefs(true, 150)
        assertEquals("v2:sharp:150", p.encode())
        assertEquals(p, StreamPrefs.decode(p.encode()))
        assertEquals(StreamPrefs(false, 300), StreamPrefs.decode("v2:fast:300"))
    }

    @Test
    fun old_saved_values_and_junk_become_the_new_default() {
        for (old in listOf("sharp:300", "fast:250", "sharp:100", "weird:180", "fast:abc", "nonsense", "", null, "v2:fast:x", "v2:fast")) {
            assertEquals("for $old", StreamPrefs(), StreamPrefs.decode(old))
        }
    }

    @Test
    fun low_resolution_range_on_the_s26_is_100_to_350_in_25_steps() {
        val steps = StreamPrefs.steps(false, 1440, 3120)
        assertEquals(100, steps.first())
        assertEquals(350, steps.last())
        assertEquals(11, steps.size)
        // portrait and landscape give the same range (the smaller side is 1440 either way)
        assertEquals(steps, StreamPrefs.steps(false, 3120, 1440))
        // a smaller screen gets a smaller maximum
        assertEquals(250, StreamPrefs.maxLowRes(1080, 2400))
        // every step is a different monitor width
        assertEquals(steps.size, steps.map { 1440 * 100 / it / 2 * 2 }.toSet().size)
    }

    @Test
    fun sharp_text_range_follows_windows_dpi_cap() {
        assertEquals(listOf(100, 125, 150, 175, 200, 225), StreamPrefs.steps(true, 1440, 3120))
        assertEquals(225, StreamPrefs.maxSharp(1440, 3120))
        assertEquals(150, StreamPrefs.maxSharp(1080, 2400)) // 1080 / 6.4 = 168
        assertEquals(100, StreamPrefs.maxSharp(500, 900))
    }

    @Test
    fun fitted_moves_the_ui_size_into_the_range_of_the_mode() {
        assertEquals(225, StreamPrefs(true, 400).fitted(1440, 3120).uiScale)
        assertEquals(200, StreamPrefs(false, 200).fitted(1440, 3120).uiScale)
        assertEquals(350, StreamPrefs(false, 500).fitted(1440, 3120).uiScale)
        assertEquals(300, StreamPrefs.snap(290, listOf(100, 200, 300, 400)))
        assertEquals(300, StreamPrefs.snap(250, listOf(100, 200, 300)))
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
        assertFalse(WsOut.open("a", 1, 2, 1.0).contains("quality"))
        assertFalse(WsOut.resize("s", 1, 2, 1.0).contains("uiScale"))
    }

    @Test
    fun window_message_carries_mode_and_dpi() {
        assertEquals(
            WsIn.Window("s", 1440, 3120, false, "monitor", 300),
            WsParser.parse("""{"type":"window","sessionId":"s","width":1440,"height":3120,"fitted":false,"mode":"monitor","dpi":300}"""),
        )
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
