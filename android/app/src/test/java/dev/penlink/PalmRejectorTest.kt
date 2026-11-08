package dev.penlink

import dev.penlink.input.PalmRejector
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PalmRejectorTest {
    private fun r() = PalmRejector(palmMajorPx = 300f, graceMs = 600)

    @Test fun fingersWorkWithoutPen() {
        assertFalse(r().rejectFinger(1000, 100f, false))
    }

    @Test fun hoverRejectsFingersAsLongAsItLasts() {
        val p = r()
        assertTrue(p.penHover(1000))
        assertTrue(p.rejectFinger(1010, 50f, false))
        assertTrue(p.rejectFinger(60_000, 50f, false)) // a still hover sends no events, it stays active
    }

    @Test fun penArrivalIsReportedOnlyOnce() {
        val p = r()
        assertTrue(p.penHover(1000))
        assertFalse(p.penHover(1010))
        assertFalse(p.penTouch(1020))
    }

    @Test fun touchRejectsFingers() {
        val p = r()
        p.penTouch(1000)
        assertTrue(p.rejectFinger(1500, 50f, false))
    }

    @Test fun graceAfterHoverExit() {
        val p = r()
        p.penHover(1000)
        p.penHoverEnd(2000)
        assertTrue(p.rejectFinger(2599, 50f, false))
        assertFalse(p.rejectFinger(2600, 50f, false))
        assertFalse(p.penActive(5000))
    }

    @Test fun graceAfterLiftBetweenStrokes() {
        val p = r()
        p.penTouch(1000)
        p.penTouchEnd(1200)
        assertTrue(p.rejectFinger(1700, 50f, false))
        assertFalse(p.rejectFinger(1801, 50f, false))
    }

    @Test fun liftWhileStillHoveringStaysActive() {
        val p = r()
        p.penHover(1000)
        p.penTouch(1100)
        p.penTouchEnd(1200)
        p.penHover(1210)
        assertTrue(p.rejectFinger(30_000, 50f, false))
    }

    @Test fun penArrivingInGraceCountsAsAlreadyActive() {
        val p = r()
        p.penTouch(1000)
        p.penTouchEnd(1100)
        assertFalse(p.penHover(1200)) // no new cancel needed: fingers were already being ignored
    }

    @Test fun penArrivalAfterGraceIsNewAgain() {
        val p = r()
        p.penTouch(1000)
        p.penTouchEnd(1100)
        assertTrue(p.penHover(5000))
    }

    @Test fun largeContactIsPalmWithoutPen() {
        val p = r()
        assertFalse(p.rejectFinger(0, 300f, false))
        assertTrue(p.rejectFinger(0, 301f, false))
    }

    @Test fun systemCancelledContactIsRejected() {
        assertTrue(r().rejectFinger(0, 10f, true))
    }

    @Test fun disabledKeepsOldBehaviour() {
        val p = r()
        p.enabled = false
        assertFalse(p.penHover(1000))
        assertFalse(p.penActive(1000))
        assertFalse(p.rejectFinger(1000, 9999f, true))
    }

    @Test fun resetForgetsThePen() {
        val p = r()
        p.penHover(1000)
        p.reset()
        assertFalse(p.rejectFinger(1001, 10f, false))
    }

    @Test fun mixedPointersPalmRejectedWhilePenDown() {
        // Pen and palm in one event: the palm is checked per pointer, the pen is what makes it active.
        val p = r()
        p.penTouch(1000)
        assertTrue(p.rejectFinger(1000, 20f, false))
        assertTrue(p.rejectFinger(1010, 400f, false))
    }
}
