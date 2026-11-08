package dev.penlink.input

/**
 * Palm rejection decisions, with no Android types so they can be unit tested.
 *
 * The pen is "active" while it hovers or touches, and for [graceMs] after its last event (writers lift between strokes).
 * While it is active every finger is ignored. Without a pen a finger is still ignored when its contact is bigger than
 * [palmMajorPx] or the system marked it cancelled (MotionEvent.FLAG_CANCELED = the system decided it was a palm).
 * All times are `MotionEvent.eventTime` milliseconds.
 */
class PalmRejector(
    /** Contact length (px) above which a finger is a palm. */
    private val palmMajorPx: Float = Float.MAX_VALUE,
    private val graceMs: Long = GRACE_MS,
) {
    /** The setting. Off = fingers are never rejected and the pen never starts from a later pointer (the old behaviour). */
    var enabled = true

    private var hovering = false
    private var touching = false
    private var lastPenT = Long.MIN_VALUE / 2

    /** The pen hovers (enter or move). Returns true if it just became active, so the caller cancels a finger gesture. */
    fun penHover(t: Long): Boolean = pen(t, hover = true, touch = false)

    /** The pen left hover range. The grace period starts now. */
    fun penHoverEnd(t: Long) { hovering = false; lastPenT = t }

    /** The pen is touching (down or move). Returns true if it just became active. */
    fun penTouch(t: Long): Boolean = pen(t, hover = hovering, touch = true)

    /** The pen lifted. The grace period starts now. */
    fun penTouchEnd(t: Long) { touching = false; lastPenT = t }

    private fun pen(t: Long, hover: Boolean, touch: Boolean): Boolean {
        val was = penActive(t)
        hovering = hover
        touching = touch
        lastPenT = t
        return enabled && !was
    }

    /** Forget the pen (window lost focus, view detached). */
    fun reset() { hovering = false; touching = false; lastPenT = Long.MIN_VALUE / 2 }

    fun penActive(t: Long): Boolean = enabled && (hovering || touching || t - lastPenT < graceMs)

    /** Whether a finger contact is a palm and must send nothing: the pen is active, the system cancelled it, or it is big. */
    fun rejectFinger(t: Long, contactMajorPx: Float, systemCanceled: Boolean): Boolean =
        enabled && (penActive(t) || systemCanceled || contactMajorPx > palmMajorPx)

    companion object {
        const val GRACE_MS = 600L

        /** A contact longer than this (dp) is a palm: a fingertip is about 60-90 dp, a resting palm well over 150. */
        const val PALM_MAJOR_DP = 120f
    }
}
