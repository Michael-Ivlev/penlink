package dev.penlink.input

/** One thing to send for text that came from the phone's keyboard. */
sealed interface ImePiece {
    /** Plain text: goes out as one `text` message. */
    data class Text(val s: String) : ImePiece

    /** A real key press (with whatever sticky modifiers are held), so shortcuts like Ctrl+C work. */
    data class Key(val key: KeyDef) : ImePiece
}

/**
 * Turns text committed by the phone's IME into messages.
 *
 * Without an active sticky modifier everything is `text` (typed as Unicode on the PC, any language).
 * With one (Ctrl, Alt, Win, Shift) every character becomes a key press on the US virtual-key of the
 * physical key it sits on; Hebrew letters map to the key of the standard Hebrew layout, so
 * Ctrl+<ב> is Ctrl+C like on a PC with a Hebrew keyboard.
 */
object ImeMapping {
    /** Standard Hebrew layout (SI 1452): Hebrew char -> the US-layout char on the same physical key. */
    private val hebrewToUs: Map<Char, Char> = buildMap {
        val pairs = listOf(
            'ק' to 'e', 'ר' to 'r', 'א' to 't', 'ט' to 'y', 'ו' to 'u', 'ן' to 'i',
            'ם' to 'o', 'פ' to 'p', 'ש' to 'a', 'ד' to 's', 'ג' to 'd', 'כ' to 'f', 'ע' to 'g', 'י' to 'h',
            'ח' to 'j', 'ל' to 'k', 'ך' to 'l', 'ף' to ';', 'ז' to 'z', 'ס' to 'x', 'ב' to 'c', 'ה' to 'v',
            'נ' to 'b', 'מ' to 'n', 'צ' to 'm', 'ת' to ',', 'ץ' to '.',
        )
        for ((h, u) in pairs) put(h, u)
    }

    /** The key that types [c] on a US layout or sits under it on the Hebrew layout; null if there is none. */
    fun keyFor(c: Char): KeyDef? {
        if (c in 'א'..'ת') return hebrewToUs[c]?.let { KeyLayout.forChar(it) }
        return KeyLayout.forChar(c) ?: KeyLayout.forChar(c.lowercaseChar())?.let { KeyDef(it.label, it.vk, shift = true) }
    }

    /**
     * Split [s] into messages. [modifiersActive]: some sticky modifier is currently down.
     * Newlines and tabs are always real Enter / Tab presses.
     */
    fun plan(s: String, modifiersActive: Boolean): List<ImePiece> {
        val out = ArrayList<ImePiece>()
        val text = StringBuilder()
        fun flush() {
            if (text.isNotEmpty()) {
                out += ImePiece.Text(text.toString())
                text.clear()
            }
        }
        for (c in s) {
            val key = when {
                c == '\n' -> KeyLayout.enter
                c == '\t' -> KeyDef("Tab", Vk.TAB)
                modifiersActive -> keyFor(c)
                else -> null
            }
            if (key != null) {
                flush()
                out += ImePiece.Key(key)
            } else {
                text.append(c)
            }
        }
        flush()
        return out
    }
}
