package dev.penlink.input

import kotlin.math.max
import kotlin.math.min

/**
 * The maths of "the keyboard overlays the picture and the picture slides up" (what Windows App, Jump Desktop and
 * Splashtop do) instead of resizing the PC window. Pure, so it can be unit tested. All values are pixels in the
 * video area's own coordinates (y = 0 at the top of the area, which ends at the bottom safe area).
 */
object KeyboardPan {
    /**
     * How much of the bottom of the area the keyboard and the extra-keys row cover right now.
     *
     * [imePx] is the IME inset (it includes the navigation bar and is animated frame by frame), [rowPx] the full
     * height of the extra-keys row and [safeBottomPx] the bottom safe area (the area already ends above it). The row
     * slides up from behind the keyboard during the first [rowPx] pixels of the IME animation, so the covered height
     * grows smoothly instead of jumping by a whole row.
     */
    fun covered(imePx: Int, rowPx: Int, safeBottomPx: Int): Float {
        if (imePx <= 0) return 0f
        return max(0f, imePx + rowShown(imePx, rowPx) - safeBottomPx)
    }

    /** How much of the row is shown (0..[rowPx]) while the IME is [imePx] high. */
    fun rowShown(imePx: Int, rowPx: Int): Float {
        if (imePx <= 0 || rowPx <= 0) return 0f
        return min(rowPx.toFloat(), imePx.toFloat())
    }

    /**
     * How far (>= 0, pixels) the picture must move UP so [focusY] ends up above the covered strip with [margin] to spare.
     *
     * - 0 when the focus is already visible, or nothing is covered, or there is no focus ([focusY] NaN).
     * - Never more than needed to bring the bottom of the picture ([contentBottom]) up to the keyboard: panning further
     *   would only drag black below the picture into view.
     * - Never so far that the focus itself would leave the top of the area (it keeps [margin] from the top). When the
     *   visible strip is too small for both (phone in landscape), the top wins: the focus stays visible, just not
     *   above the margin of the keyboard.
     *
     * [focusY] and [contentBottom] are the un-panned positions (zoom already applied).
     */
    fun panUp(focusY: Float, contentBottom: Float, areaH: Float, covered: Float, margin: Float): Float {
        if (covered <= 0f || focusY.isNaN()) return 0f
        val visibleBottom = areaH - covered
        val need = focusY + margin - visibleBottom
        if (need <= 0f) return 0f
        val byContent = max(0f, contentBottom - visibleBottom)
        val byTop = max(0f, focusY - margin)
        return min(need, min(byContent, byTop))
    }
}
