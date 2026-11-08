package dev.penlink.data

/** `sharp`: the PC draws at the phone's pixel size. `fast`: half of it, upscaled here (bigger UI, lighter stream). */
enum class Quality(val wire: String, val label: String) {
    Sharp("sharp", "Sharp"),
    Fast("fast", "Fast");

    companion object {
        fun fromWire(s: String?): Quality = entries.firstOrNull { it.wire == s } ?: Sharp
    }
}

/** Per-PC stream settings: [quality] and [uiScale], the UI size in percent (Windows' own DPI steps). */
data class StreamPrefs(val quality: Quality = Quality.Sharp, val uiScale: Int = DEFAULT_UI_SCALE) {
    /** The old-style `scale` factor for hosts that only know that field. */
    val scaleFactor: Double get() = uiScale / 100.0

    fun encode(): String = "${quality.wire}:$uiScale"

    companion object {
        /** About the text size of a phone app on a ~500 ppi screen. */
        const val DEFAULT_UI_SCALE = 300

        /** The UI sizes Windows offers (100..400 %), the slider snaps to these. */
        val UI_STEPS = listOf(100, 125, 150, 175, 200, 225, 250, 300, 350, 400)

        fun decode(s: String?): StreamPrefs {
            val parts = s?.split(':') ?: return StreamPrefs()
            if (parts.size != 2) return StreamPrefs()
            val ui = parts[1].toIntOrNull() ?: return StreamPrefs()
            return StreamPrefs(Quality.fromWire(parts[0]), snap(ui))
        }

        /** Nearest step; ties go up (bigger buttons). */
        fun snap(v: Int): Int = UI_STEPS.minWith(compareBy<Int>({ kotlin.math.abs(it - v) }, { -it }))

        /** The DPI the PC is asked for: the UI size, halved in `fast` (the picture is upscaled 2x here). */
        fun dpiAsked(p: StreamPrefs): Int = if (p.quality == Quality.Fast) p.uiScale / 2 else p.uiScale
    }
}
