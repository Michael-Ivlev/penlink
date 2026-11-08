package dev.penlink.data

import kotlin.math.abs

/** The wire value of `quality`: `fast` = UI size by low resolution, `sharp` = full resolution with Windows DPI scaling. */
enum class Quality(val wire: String) {
    Sharp("sharp"),
    Fast("fast");

    companion object {
        fun fromWire(s: String?): Quality = entries.firstOrNull { it.wire == s } ?: Fast
    }
}

/**
 * Per-PC stream settings.
 *
 * [uiScale] is the UI size in percent. By default the PC renders the virtual monitor at LOWER resolution
 * (screen / uiScale, Windows DPI 100%) and this phone upscales the video: 200% on a 1440x3120 phone is a
 * 720x1560 monitor. With [sharpText] the monitor keeps the full phone resolution and Windows' own DPI scaling
 * makes the UI bigger (crisp, but Windows caps it at about 225% on this phone).
 */
data class StreamPrefs(val sharpText: Boolean = false, val uiScale: Int = DEFAULT_UI_SCALE) {
    val quality: Quality get() = if (sharpText) Quality.Sharp else Quality.Fast

    /** The old-style `scale` factor for hosts that only know that field. */
    val scaleFactor: Double get() = uiScale / 100.0

    fun encode(): String = "v2:${quality.wire}:$uiScale"

    /** The same settings with the UI size moved to the nearest step the screen allows in this mode. */
    fun fitted(width: Int, height: Int): StreamPrefs = copy(uiScale = snap(uiScale, steps(sharpText, width, height)))

    companion object {
        /** What the old "Fast" gave on a 1440x3120 phone: a 720x1560 monitor (UI size 200%). */
        const val DEFAULT_UI_SCALE = 200

        /** Windows' own DPI steps (used in sharp-text mode). */
        val DPI_STEPS = listOf(100, 125, 150, 175, 200, 225, 250, 300, 350, 400, 450, 500)

        /** Smallest monitor side that is still useful for apps; Windows itself accepts down to 200 px. */
        const val USABLE_MONITOR_SIDE = 400

        /** Windows keeps (smaller side / DPI scale) at 640 px or more. */
        const val DPI_MIN_LOGICAL_SIDE = 640

        /**
         * Saved values from before the low-resolution design (`sharp:300`, `fast:250`, ...) were test values:
         * they, and anything unreadable, become the new default.
         */
        fun decode(s: String?): StreamPrefs {
            val parts = s?.split(':') ?: return StreamPrefs()
            if (parts.size != 3 || parts[0] != "v2") return StreamPrefs()
            val ui = parts[2].toIntOrNull() ?: return StreamPrefs()
            return StreamPrefs(parts[1] == Quality.Sharp.wire, ui.coerceIn(100, 500))
        }

        /** Highest low-resolution UI size for a screen: the smaller monitor side stays >= [USABLE_MONITOR_SIDE], in steps of 25. */
        fun maxLowRes(width: Int, height: Int): Int =
            ((minOf(width, height) * 100 / USABLE_MONITOR_SIDE) / 25 * 25).coerceAtLeast(100)

        /** Highest Windows DPI step for a screen (smaller side / scale >= 640). */
        fun maxSharp(width: Int, height: Int): Int {
            val cap = minOf(width, height) * 100 / DPI_MIN_LOGICAL_SIDE
            return DPI_STEPS.lastOrNull { it <= cap } ?: 100
        }

        /** Every UI size the slider offers for this screen and mode; each one gives a visibly different UI. */
        fun steps(sharpText: Boolean, width: Int, height: Int): List<Int> =
            if (sharpText) DPI_STEPS.filter { it <= maxSharp(width, height) }
            else (100..maxLowRes(width, height) step 25).toList()

        /** Nearest step; ties go up (bigger buttons). */
        fun snap(v: Int, steps: List<Int>): Int = steps.minWith(compareBy<Int>({ abs(it - v) }, { -it }))
    }
}
