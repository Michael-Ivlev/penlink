package dev.penlink

import dev.penlink.input.Gesture
import dev.penlink.input.KeyAction
import dev.penlink.input.KeyDef
import dev.penlink.input.KeyLayout
import dev.penlink.input.KeyMod
import dev.penlink.input.KeyboardLogic
import dev.penlink.input.ModState
import dev.penlink.input.Tilt
import dev.penlink.input.VideoMapper
import dev.penlink.input.Vk
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test
import kotlin.math.PI

class InputMappingTest {
    private val eps = 1e-6

    @Test
    fun exact_fit_maps_corners_and_centre() {
        val m = VideoMapper(1000f, 500f, 2000, 1000)
        assertEquals(0.0, m.toNorm(0f, 0f).x, eps)
        assertEquals(1.0, m.toNorm(1000f, 500f).y, eps)
        val c = m.toNorm(500f, 250f)
        assertEquals(0.5, c.x, eps)
        assertEquals(0.5, c.y, eps)
        assertTrue(c.inside)
    }

    @Test
    fun pillarbox_when_view_is_wider_than_video() {
        // 1000x500 view, 500x500 video: video occupies x in 250..750.
        val m = VideoMapper(1000f, 500f, 500, 500)
        assertEquals(250f, m.fitLeft, 0f)
        assertEquals(500f, m.fitW, 0f)
        assertEquals(0.0, m.toNorm(250f, 0f).x, eps)
        assertEquals(1.0, m.toNorm(750f, 0f).x, eps)
        assertEquals(0.5, m.toNorm(500f, 100f).x, eps)
        val outside = m.toNorm(100f, 100f)
        assertFalse(outside.inside)
        assertEquals(0.0, outside.x, eps) // clamped
        assertEquals(0.2, outside.y, eps)
    }

    @Test
    fun letterbox_when_view_is_taller_than_video() {
        // 400x1000 view, 400x200 video: video occupies y in 400..600.
        val m = VideoMapper(400f, 1000f, 400, 200)
        assertEquals(400f, m.fitTop, 0f)
        assertEquals(200f, m.fitH, 0f)
        assertEquals(0.0, m.toNorm(0f, 400f).y, eps)
        assertEquals(1.0, m.toNorm(0f, 600f).y, eps)
        assertEquals(0.25, m.toNorm(100f, 450f).y, eps)
        assertFalse(m.toNorm(100f, 100f).inside)
    }

    @Test
    fun zoom_about_centre_and_pan() {
        val m2 = VideoMapper(1000f, 500f, 1000, 500, zoom = 2f)
        // The view centre stays the video centre.
        assertEquals(0.5, m2.toNorm(500f, 250f).x, eps)
        // With 2x zoom, the right edge of the view shows normalised x = 0.75.
        assertEquals(0.75, m2.toNorm(1000f, 250f).x, eps)
        assertEquals(0.25, m2.toNorm(0f, 250f).x, eps)
        // Panning the content 100px right shifts what is under the finger to the left.
        val p = VideoMapper(1000f, 500f, 1000, 500, zoom = 2f, panX = 100f)
        assertEquals(0.5 - 100.0 / 2000.0, p.toNorm(500f, 250f).x, eps)
    }

    @Test
    fun pan_is_clamped_to_zoomed_content() {
        val m = VideoMapper(1000f, 500f, 1000, 500, zoom = 2f)
        assertEquals(500f, m.clampPanX(900f), 0f)
        assertEquals(-250f, m.clampPanY(-900f), 0f)
        assertEquals(0f, VideoMapper(1000f, 500f, 1000, 500, zoom = 1f).clampPanX(50f), 0f)
    }

    @Test
    fun unknown_video_size_uses_whole_view() {
        val m = VideoMapper(800f, 400f, 0, 0)
        assertEquals(0.5, m.toNorm(400f, 200f).x, eps)
    }

    @Test
    fun displayed_pixel_size_follows_fit_and_zoom() {
        val m = VideoMapper(1000f, 1000f, 500, 500, zoom = 2f)
        assertEquals(4f, m.displayedPixelSize, 1e-4f)
    }

    @Test
    fun tilt_upright_is_zero() {
        val (tx, ty) = Tilt.toDegrees(0f, 0f)
        assertEquals(0.0, tx, eps)
        assertEquals(0.0, ty, eps)
    }

    @Test
    fun tilt_directions() {
        val t30 = (30 * PI / 180).toFloat()
        // Orientation 0 = top of stylus leans toward the top of the screen (away from the user): ty negative.
        val up = Tilt.toDegrees(t30, 0f)
        assertEquals(0.0, up.first, 1e-4)
        assertTrue(up.second < 0)
        // Orientation pi/2 = leans right: tx positive.
        val right = Tilt.toDegrees(t30, (PI / 2).toFloat())
        assertTrue(right.first > 0)
        assertEquals(0.0, right.second, 1e-4)
        // Orientation pi = leans toward the user (down): ty positive.
        val down = Tilt.toDegrees(t30, PI.toFloat())
        assertTrue(down.second > 0)
        // Orientation -pi/2 = leans left: tx negative.
        val left = Tilt.toDegrees(t30, (-PI / 2).toFloat())
        assertTrue(left.first < 0)
    }

    @Test
    fun tilt_single_axis_equals_tilt_angle() {
        val t = (40 * PI / 180).toFloat()
        val right = Tilt.toDegrees(t, (PI / 2).toFloat())
        assertEquals(40.0, right.first, 1e-3)
    }

    @Test
    fun tilt_is_clamped_to_90() {
        val flat = Tilt.toDegrees((PI / 2).toFloat(), (PI / 2).toFloat())
        assertEquals(90.0, flat.first, 1e-3)
        assertTrue(Tilt.toDegrees(10f, 1f).first in -90.0..90.0)
    }

    @Test
    fun wheel_direction() {
        // Finger drag up (negative dy) -> negative wheel (scroll down), 40px = one notch.
        assertEquals(-120f, Gesture.wheelY(-40f), 1e-4f)
        assertEquals(60f, Gesture.wheelY(20f), 1e-4f)
        // Finger drag left -> positive horizontal wheel.
        assertEquals(120f, Gesture.wheelX(-40f), 1e-4f)
    }

    // ------------------------------------------------------------------ keyboard layout / VK codes

    @Test
    fun letters_and_digits_vk() {
        assertEquals(0x41, KeyLayout.forChar('a')!!.vk)
        assertEquals(0x5A, KeyLayout.forChar('z')!!.vk)
        assertFalse(KeyLayout.forChar('q')!!.shift)
        assertEquals(0x51, KeyLayout.forChar('Q')!!.vk)
        assertTrue(KeyLayout.forChar('Q')!!.shift)
        assertEquals(0x30, KeyLayout.forChar('0')!!.vk)
        assertEquals(0x39, KeyLayout.forChar('9')!!.vk)
    }

    @Test
    fun shifted_symbols_send_shift_plus_base_vk() {
        val expect = mapOf(
            '!' to '1', '@' to '2', '#' to '3', '$' to '4', '%' to '5',
            '^' to '6', '&' to '7', '*' to '8', '(' to '9', ')' to '0',
        )
        for ((sym, digit) in expect) {
            val k = KeyLayout.forChar(sym)!!
            assertEquals("$sym", digit.code, k.vk)
            assertTrue("$sym", k.shift)
        }
        assertEquals(Vk.OEM_1 to true, KeyLayout.forChar(':')!!.let { it.vk to it.shift })
        assertEquals(Vk.OEM_1 to false, KeyLayout.forChar(';')!!.let { it.vk to it.shift })
        assertEquals(Vk.OEM_PLUS to true, KeyLayout.forChar('+')!!.let { it.vk to it.shift })
        assertEquals(Vk.OEM_MINUS to true, KeyLayout.forChar('_')!!.let { it.vk to it.shift })
        assertEquals(Vk.OEM_4 to true, KeyLayout.forChar('{')!!.let { it.vk to it.shift })
        assertEquals(Vk.OEM_6 to true, KeyLayout.forChar('}')!!.let { it.vk to it.shift })
        assertEquals(Vk.OEM_5 to true, KeyLayout.forChar('|')!!.let { it.vk to it.shift })
        assertEquals(Vk.OEM_7 to true, KeyLayout.forChar('"')!!.let { it.vk to it.shift })
        assertEquals(Vk.OEM_3 to true, KeyLayout.forChar('~')!!.let { it.vk to it.shift })
        assertEquals(Vk.OEM_2 to true, KeyLayout.forChar('?')!!.let { it.vk to it.shift })
        assertEquals(Vk.OEM_COMMA to true, KeyLayout.forChar('<')!!.let { it.vk to it.shift })
        assertEquals(Vk.OEM_PERIOD to true, KeyLayout.forChar('>')!!.let { it.vk to it.shift })
    }

    @Test
    fun every_symbol_row_glyph_has_a_key() {
        for (row in KeyLayout.symbolRows + KeyLayout.letterRows + listOf(KeyLayout.digitRow, "-=[];',./")) {
            for (c in row) assertNotNull("missing $c", KeyLayout.forChar(c))
        }
    }

    @Test
    fun special_key_codes() {
        assertEquals(0x1B, Vk.ESCAPE)
        assertEquals(0x09, Vk.TAB)
        assertEquals(0x0D, Vk.RETURN)
        assertEquals(0x08, Vk.BACK)
        assertEquals(0x20, Vk.SPACE)
        assertEquals(0x10, KeyMod.Shift.vk)
        assertEquals(0x11, KeyMod.Ctrl.vk)
        assertEquals(0x12, KeyMod.Alt.vk)
        assertEquals(0x5B, KeyMod.Win.vk)
        assertEquals(listOf(0x25, 0x26, 0x28, 0x27), KeyLayout.arrowKeys.map { it.vk })
        assertEquals(0x24, KeyLayout.navKeys.first { it.label == "Home" }.vk)
        assertEquals(0x23, KeyLayout.navKeys.first { it.label == "End" }.vk)
        assertEquals(0x21, KeyLayout.navKeys.first { it.label == "PgUp" }.vk)
        assertEquals(0x22, KeyLayout.navKeys.first { it.label == "PgDn" }.vk)
        assertEquals((0x70..0x7B).toList(), KeyLayout.functionKeys.map { it.vk })
        assertEquals(12, KeyLayout.functionKeys.size)
    }

    // ------------------------------------------------------------------ sticky modifiers

    private fun a(vk: Int, down: Boolean) = KeyAction(vk, down)

    @Test
    fun plain_key_is_down_then_up() {
        val l = KeyboardLogic()
        assertEquals(listOf(a(0x41, true), a(0x41, false)), l.tapKey(KeyLayout.forChar('a')!!))
    }

    @Test
    fun shifted_symbol_wraps_key_in_shift() {
        val l = KeyboardLogic()
        assertEquals(
            listOf(a(Vk.SHIFT, true), a(0x31, true), a(0x31, false), a(Vk.SHIFT, false)),
            l.tapKey(KeyLayout.forChar('!')!!),
        )
    }

    @Test
    fun oneshot_ctrl_applies_to_next_key_only() {
        val l = KeyboardLogic()
        assertEquals(listOf(a(Vk.CONTROL, true)), l.tapModifier(KeyMod.Ctrl))
        assertEquals(ModState.OneShot, l.state(KeyMod.Ctrl))
        assertEquals(
            listOf(a(Vk.CONTROL, false)).let { listOf(a(0x43, true), a(0x43, false)) + it },
            l.tapKey(KeyLayout.forChar('c')!!),
        )
        assertEquals(ModState.Off, l.state(KeyMod.Ctrl))
        assertEquals(listOf(a(0x43, true), a(0x43, false)), l.tapKey(KeyLayout.forChar('c')!!))
    }

    @Test
    fun double_tap_locks_and_third_tap_releases() {
        val l = KeyboardLogic()
        l.tapModifier(KeyMod.Ctrl)
        assertEquals(emptyList<KeyAction>(), l.tapModifier(KeyMod.Ctrl))
        assertEquals(ModState.Locked, l.state(KeyMod.Ctrl))
        // Locked survives keys.
        assertEquals(listOf(a(0x53, true), a(0x53, false)), l.tapKey(KeyLayout.forChar('s')!!))
        assertEquals(ModState.Locked, l.state(KeyMod.Ctrl))
        assertEquals(listOf(a(Vk.CONTROL, false)), l.tapModifier(KeyMod.Ctrl))
        assertEquals(ModState.Off, l.state(KeyMod.Ctrl))
    }

    @Test
    fun held_shift_does_not_add_a_second_shift() {
        val l = KeyboardLogic()
        l.tapModifier(KeyMod.Shift)
        assertEquals(
            listOf(a(0x31, true), a(0x31, false), a(Vk.SHIFT, false)),
            l.tapKey(KeyLayout.forChar('!')!!),
        )
    }

    @Test
    fun ctrl_alt_combo_releases_both_oneshots() {
        val l = KeyboardLogic()
        l.tapModifier(KeyMod.Ctrl)
        l.tapModifier(KeyMod.Alt)
        val out = l.tapKey(KeyDef("Del", Vk.DELETE))
        assertEquals(a(Vk.DELETE, true), out[0])
        assertEquals(a(Vk.DELETE, false), out[1])
        assertTrue(out.contains(a(Vk.CONTROL, false)))
        assertTrue(out.contains(a(Vk.MENU, false)))
    }

    @Test
    fun release_all_clears_everything() {
        val l = KeyboardLogic()
        l.tapModifier(KeyMod.Win)
        l.tapModifier(KeyMod.Shift); l.tapModifier(KeyMod.Shift)
        val out = l.releaseAll()
        assertEquals(setOf(a(Vk.LWIN, false), a(Vk.SHIFT, false)), out.toSet())
        for (m in KeyMod.entries) assertEquals(ModState.Off, l.state(m))
    }
}
