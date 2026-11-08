package dev.penlink

import dev.penlink.data.StreamPrefs
import dev.penlink.input.KeyboardPan
import dev.penlink.input.VideoMapper
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class KeyboardPanTest {
    // Portrait phone: area 1440 x 3000, video fills it. Keyboard 1200 high + 144 row.
    private val areaH = 3000f
    private val covered = 1344f
    private val margin = 60f

    @Test
    fun covered_is_zero_without_keyboard_and_subtracts_the_safe_bottom() {
        assertEquals(0f, KeyboardPan.covered(0, 144, 100), 0f)
        assertEquals(1200f + 144f - 100f, KeyboardPan.covered(1200, 144, 100), 0f)
        assertEquals(0f, KeyboardPan.covered(10, 144, 200), 0f) // nav bar alone is not "covered"
    }

    @Test
    fun row_slides_in_over_the_first_row_height_of_the_ime_animation() {
        assertEquals(0f, KeyboardPan.rowShown(0, 144), 0f)
        assertEquals(50f, KeyboardPan.rowShown(50, 144), 0f)
        assertEquals(144f, KeyboardPan.rowShown(144, 144), 0f)
        assertEquals(144f, KeyboardPan.rowShown(900, 144), 0f)
        // grows by exactly 1px per px of IME animation at the start: no jump
        assertEquals(KeyboardPan.covered(11, 144, 0) - KeyboardPan.covered(10, 144, 0), 2f, 0f)
    }

    @Test
    fun no_pan_when_the_focus_is_already_visible() {
        assertEquals(0f, KeyboardPan.panUp(500f, areaH, areaH, covered, margin), 0f)
        // exactly at the margin line
        assertEquals(0f, KeyboardPan.panUp(areaH - covered - margin, areaH, areaH, covered, margin), 0f)
    }

    @Test
    fun no_pan_without_keyboard_or_focus() {
        assertEquals(0f, KeyboardPan.panUp(2900f, areaH, areaH, 0f, margin), 0f)
        assertEquals(0f, KeyboardPan.panUp(Float.NaN, areaH, areaH, covered, margin), 0f)
    }

    @Test
    fun pans_just_enough_to_bring_the_focus_above_the_keyboard() {
        val focus = 2800f
        val pan = KeyboardPan.panUp(focus, areaH, areaH, covered, margin)
        assertEquals(focus + margin - (areaH - covered), pan, 0.001f)
        assertEquals(areaH - covered - margin, focus - pan, 0.001f)
    }

    @Test
    fun pan_grows_smoothly_with_the_keyboard() {
        var last = 0f
        for (c in 0..1344 step 56) {
            val p = KeyboardPan.panUp(2800f, areaH, areaH, c.toFloat(), margin)
            assertTrue(p >= last)
            assertTrue(p - last < 100f)
            last = p
        }
    }

    @Test
    fun never_pans_past_the_bottom_of_the_picture() {
        // video only 1000 px tall at the top of a 3000 px area (letterboxed): contentBottom 1000, visible bottom 1656
        val p = KeyboardPan.panUp(990f, 1000f, areaH, covered, margin)
        assertEquals(0f, p, 0f) // 990 + 60 < 1656, visible
        // taller picture: contentBottom 2900, focus at the very bottom: pan only until the picture bottom meets the keyboard
        val q = KeyboardPan.panUp(2900f, 2900f, areaH, covered, margin)
        assertEquals(2900f - (areaH - covered), q, 0.001f)
    }

    @Test
    fun never_pans_the_focus_past_the_top_of_the_area() {
        // landscape: area 1000 high, keyboard covers 900 (visible strip 100, less than two margins). Focus at 900.
        val p = KeyboardPan.panUp(900f, 1000f, 1000f, 900f, 60f)
        assertEquals(900f - 60f, p, 0.001f) // the focus ends at the top margin, not above the area
        val screenY = 900f - p
        assertTrue(screenY >= 60f - 0.001f)
    }

    // ---------------------------------------------------------------- mapping with pan + zoom

    @Test
    fun mapper_with_pan_still_lands_taps_on_the_same_video_point() {
        val rest = VideoMapper(1440f, 3000f, 720, 1500)
        val ny = 0.93
        val y = rest.toViewY(ny)
        val pan = -KeyboardPan.panUp(y, rest.toViewY(1.0), 3000f, covered, margin)
        assertTrue(pan < 0f)
        val panned = VideoMapper(1440f, 3000f, 720, 1500, panY = pan)
        // the same video point is now drawn `pan` higher, and a tap there maps back to it
        assertEquals(y + pan, panned.toViewY(ny), 0.01f)
        assertEquals(ny, panned.toNorm(720f, y + pan).y, 1e-6)
        assertEquals(0.5, panned.toNorm(720f, y + pan).x, 1e-6)
    }

    @Test
    fun toViewY_is_the_inverse_of_toNorm_with_zoom_and_pan() {
        val m = VideoMapper(1440f, 3000f, 1000, 1500, zoom = 2.5f, panX = 120f, panY = -340f)
        for (ny in listOf(0.0, 0.1, 0.5, 0.77, 1.0)) {
            for (nx in listOf(0.0, 0.3, 1.0)) {
                val n = m.toNorm(m.toViewX(nx), m.toViewY(ny))
                assertEquals(nx, n.x, 1e-5)
                assertEquals(ny, n.y, 1e-5)
            }
        }
    }

    @Test
    fun zoomed_view_composes_with_the_keyboard_pan() {
        // zoomed 2x, user panned up 200; keyboard pan adds on top and taps still resolve to the same video point
        val user = VideoMapper(1440f, 3000f, 720, 1500, zoom = 2f, panY = -200f)
        val ny = 0.8
        val y = user.toViewY(ny)
        val up = KeyboardPan.panUp(y, user.toViewY(1.0), 3000f, covered, margin)
        val shown = VideoMapper(1440f, 3000f, 720, 1500, zoom = 2f, panY = -200f - up)
        assertEquals(ny, shown.toNorm(300f, y - up).y, 1e-5)
        assertTrue(y - up <= 3000f - covered - margin + 0.01f || up == 0f)
    }

    // ---------------------------------------------------------------- the setting

    @Test
    fun resize_above_keyboard_is_off_by_default_and_saved() {
        assertFalse(StreamPrefs().resizeAboveKeyboard)
        assertEquals("v2:fast:200", StreamPrefs().encode())
        val on = StreamPrefs(true, 150, true)
        assertEquals("v2:sharp:150:k", on.encode())
        assertEquals(on, StreamPrefs.decode(on.encode()))
        assertEquals(StreamPrefs(false, 200, false), StreamPrefs.decode("v2:fast:200:x"))
        assertTrue(on.fitted(1440, 3120).resizeAboveKeyboard)
    }
}
