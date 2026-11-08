package dev.penlink.input

/** Windows virtual-key codes used by the on-screen keyboard. */
object Vk {
    const val BACK = 0x08
    const val TAB = 0x09
    const val RETURN = 0x0D
    const val SHIFT = 0x10
    const val CONTROL = 0x11
    const val MENU = 0x12 // Alt
    const val ESCAPE = 0x1B
    const val SPACE = 0x20
    const val PRIOR = 0x21 // PgUp
    const val NEXT = 0x22 // PgDn
    const val END = 0x23
    const val HOME = 0x24
    const val LEFT = 0x25
    const val UP = 0x26
    const val RIGHT = 0x27
    const val DOWN = 0x28
    const val INSERT = 0x2D
    const val DELETE = 0x2E
    const val LWIN = 0x5B
    const val F1 = 0x70
    const val OEM_1 = 0xBA // ; :
    const val OEM_PLUS = 0xBB // = +
    const val OEM_COMMA = 0xBC // , <
    const val OEM_MINUS = 0xBD // - _
    const val OEM_PERIOD = 0xBE // . >
    const val OEM_2 = 0xBF // / ?
    const val OEM_3 = 0xC0 // ` ~
    const val OEM_4 = 0xDB // [ {
    const val OEM_5 = 0xDC // \ |
    const val OEM_6 = 0xDD // ] }
    const val OEM_7 = 0xDE // ' "

    fun f(n: Int): Int {
        require(n in 1..24)
        return F1 + n - 1
    }
}

/** One key cap. [vk] is the base key; [shift] means the produced glyph needs Shift on a US layout. */
data class KeyDef(val label: String, val vk: Int, val shift: Boolean = false)

enum class KeyMod(val vk: Int, val label: String) {
    Ctrl(Vk.CONTROL, "Ctrl"), Alt(Vk.MENU, "Alt"), Shift(Vk.SHIFT, "Shift"), Win(Vk.LWIN, "Win"),
}

/** US layout lookup: character -> key. Used for the keyboard pages and tests. */
object KeyLayout {
    private val symbols: Map<Char, KeyDef> = buildMap {
        for (c in 'a'..'z') put(c, KeyDef(c.toString(), c.uppercaseChar().code))
        for (c in 'A'..'Z') put(c, KeyDef(c.toString(), c.code, shift = true))
        for (c in '0'..'9') put(c, KeyDef(c.toString(), c.code))
        // Shifted digits (US layout)
        val shiftedDigits = ")!@#$%^&*("
        shiftedDigits.forEachIndexed { i, ch -> put(ch, KeyDef(ch.toString(), '0'.code + i, shift = true)) }
        fun pair(base: Char, shifted: Char, vk: Int) {
            put(base, KeyDef(base.toString(), vk))
            put(shifted, KeyDef(shifted.toString(), vk, shift = true))
        }
        pair(';', ':', Vk.OEM_1)
        pair('=', '+', Vk.OEM_PLUS)
        pair(',', '<', Vk.OEM_COMMA)
        pair('-', '_', Vk.OEM_MINUS)
        pair('.', '>', Vk.OEM_PERIOD)
        pair('/', '?', Vk.OEM_2)
        pair('`', '~', Vk.OEM_3)
        pair('[', '{', Vk.OEM_4)
        pair('\\', '|', Vk.OEM_5)
        pair(']', '}', Vk.OEM_6)
        pair('\'', '"', Vk.OEM_7)
        put(' ', KeyDef("Space", Vk.SPACE))
    }

    /** The key (and whether it needs Shift) that types [c] on a US layout, or null if none. */
    fun forChar(c: Char): KeyDef? = symbols[c]

    val letterRows = listOf("qwertyuiop", "asdfghjkl", "zxcvbnm")
    val digitRow = "1234567890"

    /** Symbol page rows, each entry the glyph typed (some need Shift). */
    val symbolRows = listOf(
        "!@#$%^&*()",
        "-_=+[]{}\\|",
        ";:'\",.<>/?",
        "`~",
    )

    val navKeys = listOf(
        KeyDef("Esc", Vk.ESCAPE), KeyDef("Tab", Vk.TAB),
        KeyDef("Home", Vk.HOME), KeyDef("End", Vk.END),
        KeyDef("PgUp", Vk.PRIOR), KeyDef("PgDn", Vk.NEXT),
        KeyDef("Ins", Vk.INSERT), KeyDef("Del", Vk.DELETE),
    )
    val arrowKeys = listOf(
        KeyDef("←", Vk.LEFT), KeyDef("↑", Vk.UP), KeyDef("↓", Vk.DOWN), KeyDef("→", Vk.RIGHT),
    )
    val functionKeys = (1..12).map { KeyDef("F$it", Vk.f(it)) }
    val backspace = KeyDef("⌫", Vk.BACK)
    val enter = KeyDef("Enter", Vk.RETURN)
    val space = KeyDef("Space", Vk.SPACE)
}

/** One key event the keyboard asks to send. */
data class KeyAction(val vk: Int, val down: Boolean)

enum class ModState { Off, OneShot, Locked }

/**
 * Sticky-modifier state machine for the on-screen keyboard.
 *
 * KeyMod tap cycle: Off -> OneShot (next key only) -> Locked -> Off. A modifier's key-down is sent
 * as soon as it becomes active and its key-up as soon as it is released, so it also applies to
 * pointer clicks made while it is held. After a normal key, all OneShot modifiers are released.
 */
class KeyboardLogic {
    private val states = KeyMod.entries.associateWith { ModState.Off }.toMutableMap()

    fun state(m: KeyMod): ModState = states.getValue(m)

    fun tapModifier(m: KeyMod): List<KeyAction> {
        val prev = states.getValue(m)
        val next = when (prev) {
            ModState.Off -> ModState.OneShot
            ModState.OneShot -> ModState.Locked
            ModState.Locked -> ModState.Off
        }
        states[m] = next
        return when {
            prev == ModState.Off -> listOf(KeyAction(m.vk, true))
            next == ModState.Off -> listOf(KeyAction(m.vk, false))
            else -> emptyList()
        }
    }

    /** Press and release a normal key, honouring active modifiers and the key's own Shift need. */
    fun tapKey(key: KeyDef): List<KeyAction> {
        val out = ArrayList<KeyAction>()
        val shiftHeld = states.getValue(KeyMod.Shift) != ModState.Off
        val extraShift = key.shift && !shiftHeld
        if (extraShift) out += KeyAction(Vk.SHIFT, true)
        out += KeyAction(key.vk, true)
        out += KeyAction(key.vk, false)
        if (extraShift) out += KeyAction(Vk.SHIFT, false)
        for (m in KeyMod.entries) {
            if (states.getValue(m) == ModState.OneShot) {
                states[m] = ModState.Off
                out += KeyAction(m.vk, false)
            }
        }
        return out
    }

    /** Release everything (used when a session closes or the keyboard is dismissed). */
    fun releaseAll(): List<KeyAction> {
        val out = ArrayList<KeyAction>()
        for (m in KeyMod.entries) {
            if (states.getValue(m) != ModState.Off) out += KeyAction(m.vk, false)
            states[m] = ModState.Off
        }
        return out
    }
}
