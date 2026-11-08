package dev.penlink.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.penlink.input.KeyAction
import dev.penlink.input.KeyDef
import dev.penlink.input.KeyLayout
import dev.penlink.input.KeyboardLogic
import dev.penlink.input.ModState
import dev.penlink.input.KeyMod
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private enum class Page { Letters, Symbols, Function }

/**
 * Penlink's own coding keyboard. [onKeys] receives key down/up events (Windows VK codes).
 * [onSystemKeyboard] switches to the phone's normal keyboard.
 */
@Composable
fun KeyboardPanel(
    logic: KeyboardLogic,
    onKeys: (List<KeyAction>) -> Unit,
    onSystemKeyboard: () -> Unit,
    modifier: Modifier = Modifier,
) {
    var page by remember { mutableStateOf(Page.Letters) }
    var version by remember { mutableIntStateOf(0) } // bump to re-read modifier state
    val send = rememberUpdatedState(onKeys)

    fun tapKey(k: KeyDef) {
        send.value(logic.tapKey(k))
        version++
    }

    fun tapMod(m: KeyMod) {
        send.value(logic.tapModifier(m))
        version++
    }

    @Suppress("UNUSED_EXPRESSION") version
    val shiftOn = logic.state(KeyMod.Shift) != ModState.Off

    Column(
        modifier.fillMaxWidth().background(Color(0xE61E1E2E)).padding(4.dp),
        verticalArrangement = Arrangement.spacedBy(3.dp),
    ) {
        // Modifiers and navigation.
        KeyRow {
            Cap("Esc", Modifier.weight(1f)) { tapKey(KeyLayout.navKeys[0]) }
            Cap("Tab", Modifier.weight(1f)) { tapKey(KeyLayout.navKeys[1]) }
            for (m in KeyMod.entries) {
                Cap(m.label, Modifier.weight(1f), state = logic.state(m), repeat = false) { tapMod(m) }
            }
            Cap("Home", Modifier.weight(1f)) { tapKey(KeyLayout.navKeys[2]) }
            Cap("End", Modifier.weight(1f)) { tapKey(KeyLayout.navKeys[3]) }
            Cap("PgUp", Modifier.weight(1f)) { tapKey(KeyLayout.navKeys[4]) }
            Cap("PgDn", Modifier.weight(1f)) { tapKey(KeyLayout.navKeys[5]) }
            Cap("Del", Modifier.weight(1f)) { tapKey(KeyLayout.navKeys[7]) }
        }

        when (page) {
            Page.Letters -> {
                KeyRow { for (c in KeyLayout.digitRow) CharCap(c, shiftOn, Modifier.weight(1f)) { tapKey(it) } }
                for (row in KeyLayout.letterRows) {
                    KeyRow {
                        for (c in row) CharCap(c, shiftOn, Modifier.weight(1f)) { tapKey(it) }
                    }
                }
                KeyRow {
                    for (c in "-=[];',./") CharCap(c, shiftOn, Modifier.weight(1f)) { tapKey(it) }
                }
            }
            Page.Symbols -> {
                for (row in KeyLayout.symbolRows) {
                    KeyRow { for (c in row) CharCap(c, false, Modifier.weight(1f)) { tapKey(it) } }
                }
            }
            Page.Function -> {
                KeyRow { for (k in KeyLayout.functionKeys.take(6)) Cap(k.label, Modifier.weight(1f)) { tapKey(k) } }
                KeyRow { for (k in KeyLayout.functionKeys.drop(6)) Cap(k.label, Modifier.weight(1f)) { tapKey(k) } }
            }
        }

        // Bottom row.
        KeyRow {
            Cap(
                if (page == Page.Letters) "?123" else "ABC",
                Modifier.weight(1.2f), repeat = false,
            ) { page = if (page == Page.Letters) Page.Symbols else Page.Letters }
            Cap("Fn", Modifier.weight(1f), repeat = false) {
                page = if (page == Page.Function) Page.Letters else Page.Function
            }
            Cap("←", Modifier.weight(1f)) { tapKey(KeyLayout.arrowKeys[0]) }
            Cap("↑", Modifier.weight(1f)) { tapKey(KeyLayout.arrowKeys[1]) }
            Cap("↓", Modifier.weight(1f)) { tapKey(KeyLayout.arrowKeys[2]) }
            Cap("→", Modifier.weight(1f)) { tapKey(KeyLayout.arrowKeys[3]) }
            Cap("Space", Modifier.weight(3f)) { tapKey(KeyLayout.space) }
            Cap("⌫", Modifier.weight(1.2f)) { tapKey(KeyLayout.backspace) }
            Cap("Enter", Modifier.weight(1.4f)) { tapKey(KeyLayout.enter) }
            Cap("ABC⌨", Modifier.weight(1.2f), repeat = false) { onSystemKeyboard() }
        }
    }
}

@Composable
private fun KeyRow(content: @Composable RowScope.() -> Unit) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(3.dp), content = content)
}

@Composable
private fun CharCap(c: Char, upper: Boolean, modifier: Modifier, onKey: (KeyDef) -> Unit) {
    val def = KeyLayout.forChar(c) ?: return
    val shown = if (upper && c.isLetter()) c.uppercaseChar().toString() else c.toString()
    Cap(shown, modifier) { onKey(def) }
}

/** One key cap. Press = action; holding repeats unless [repeat] is false. */
@Composable
private fun Cap(
    label: String,
    modifier: Modifier,
    state: ModState = ModState.Off,
    repeat: Boolean = true,
    onTap: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val action = rememberUpdatedState(onTap)
    val bg = when (state) {
        ModState.Off -> Color(0xFF45475A)
        ModState.OneShot -> Color(0xFF89B4FA)
        ModState.Locked -> Color(0xFFA6E3A1)
    }
    val fg = if (state == ModState.Off) Color.White else Color.Black
    Box(
        modifier
            .height(42.dp)
            .clip(RoundedCornerShape(6.dp))
            .background(bg)
            .pointerInput(repeat) {
                detectTapGestures(onPress = {
                    action.value()
                    val job = if (repeat) scope.launch {
                        delay(420)
                        while (true) {
                            action.value()
                            delay(55)
                        }
                    } else null
                    tryAwaitRelease()
                    job?.cancel()
                })
            },
        contentAlignment = Alignment.Center,
    ) {
        Text(label, color = fg, fontSize = 14.sp, style = MaterialTheme.typography.labelLarge, maxLines = 1)
    }
}
