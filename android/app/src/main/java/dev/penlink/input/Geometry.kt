package dev.penlink.input

import kotlin.math.PI
import kotlin.math.atan2
import kotlin.math.cos
import kotlin.math.max
import kotlin.math.min
import kotlin.math.sin
import kotlin.math.sqrt

/** A point normalised over the video frame (0,0 = top-left, 1,1 = bottom-right). */
data class NormPoint(val x: Double, val y: Double, val inside: Boolean)

/**
 * Maps view pixels to normalised video coordinates.
 *
 * The video is drawn aspect-fit (letterboxed) in a [viewW] x [viewH] view, and then the whole
 * view content is locally zoomed by [zoom] about the view centre and translated by
 * ([panX], [panY]) view pixels (the same thing as setting scaleX/scaleY/translationX/translationY
 * on a view with a centred pivot). Zoom/pan are display-only; the host never sees them.
 */
data class VideoMapper(
    val viewW: Float,
    val viewH: Float,
    val videoW: Int,
    val videoH: Int,
    val zoom: Float = 1f,
    val panX: Float = 0f,
    val panY: Float = 0f,
) {
    /** The aspect-fit rectangle of the video in un-zoomed view coordinates. */
    val fitLeft: Float
    val fitTop: Float
    val fitW: Float
    val fitH: Float

    init {
        if (videoW <= 0 || videoH <= 0 || viewW <= 0f || viewH <= 0f) {
            fitLeft = 0f; fitTop = 0f; fitW = viewW; fitH = viewH
        } else {
            val s = min(viewW / videoW, viewH / videoH)
            fitW = videoW * s
            fitH = videoH * s
            fitLeft = (viewW - fitW) / 2f
            fitTop = (viewH - fitH) / 2f
        }
    }

    /** Undo local zoom/pan: view point -> un-zoomed view point. */
    fun unzoomX(px: Float): Float = (px - panX - viewW / 2f) / zoom + viewW / 2f
    fun unzoomY(py: Float): Float = (py - panY - viewH / 2f) / zoom + viewH / 2f

    /** View pixels -> normalised video coordinates, clamped to 0..1 (inside tells if it was within). */
    fun toNorm(px: Float, py: Float): NormPoint {
        val ux = unzoomX(px)
        val uy = unzoomY(py)
        val nx = ((ux - fitLeft) / fitW).toDouble()
        val ny = ((uy - fitTop) / fitH).toDouble()
        val inside = nx in 0.0..1.0 && ny in 0.0..1.0
        return NormPoint(nx.coerceIn(0.0, 1.0), ny.coerceIn(0.0, 1.0), inside)
    }

    /** Displayed size (view pixels) of one video pixel. */
    val displayedPixelSize: Float
        get() = if (videoW > 0) fitW * zoom / videoW else 1f

    /** Largest pan magnitude that still keeps the zoomed view covering the screen. */
    fun clampPanX(p: Float): Float = clampSym(p, (zoom - 1f) * viewW / 2f)
    fun clampPanY(p: Float): Float = clampSym(p, (zoom - 1f) * viewH / 2f)

    private fun clampSym(p: Float, lim: Float) = max(-lim, min(lim, p))
}

object Tilt {
    /**
     * Converts Android stylus axes into Windows pen tilt degrees.
     *
     * [tilt] is MotionEvent.AXIS_TILT: angle between the stylus and the screen normal, radians,
     * 0 = perpendicular, pi/2 = flat on the screen.
     * [orientation] is MotionEvent.AXIS_ORIENTATION for a stylus: direction (radians, -pi..pi,
     * clockwise from the top of the screen, 0 = up) the stylus top leans toward.
     *
     * Result: tiltX positive = top leans right, tiltY positive = top leans toward the user
     * (down the screen), each in degrees clamped to -90..90 (Windows pen convention).
     */
    fun toDegrees(tilt: Float, orientation: Float): Pair<Double, Double> {
        val t = tilt.toDouble().coerceIn(0.0, PI / 2)
        val o = orientation.toDouble()
        val sinT = sin(t)
        val dx = sin(o) * sinT          // lean to the right
        val dy = -cos(o) * sinT         // lean down the screen (toward the user)
        val dz = cos(t)
        val tx = Math.toDegrees(atan2(dx, dz))
        val ty = Math.toDegrees(atan2(dy, dz))
        return Pair(clean(tx), clean(ty))
    }

    private fun clean(v: Double): Double = if (kotlin.math.abs(v) < 1e-9) 0.0 else v.coerceIn(-90.0, 90.0)
}

/** Helpers for two-finger gestures. */
object Gesture {
    fun distance(x0: Float, y0: Float, x1: Float, y1: Float): Float {
        val dx = x1 - x0
        val dy = y1 - y0
        return sqrt(dx * dx + dy * dy)
    }

    /**
     * Wheel units (Windows: 120 = one notch) for a vertical finger drag of [deltaPx] view pixels.
     * Natural scrolling: dragging up reveals content below, which is a negative wheel value.
     */
    fun wheelY(deltaPx: Float, pxPerNotch: Float = 40f): Float = deltaPx / pxPerNotch * 120f

    /** Horizontal: dragging left reveals content on the right, which is a positive wheel value. */
    fun wheelX(deltaPx: Float, pxPerNotch: Float = 40f): Float = -deltaPx / pxPerNotch * 120f
}
