package dev.penlink.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
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
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.penlink.input.KeyAction
import dev.penlink.input.KeyDef
import dev.penlink.input.KeyLayout
import dev.penlink.input.KeyMod
import dev.penlink.input.KeyboardLogic
import dev.penlink.input.ModState
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Height of the row; the session screen reserves this much above the phone keyboard. */
val ExtraKeysHeight: Dp = 48.dp

/**
 * One row of extra keys docked on top of the phone's keyboard: Esc, Tab, sticky Ctrl/Alt/Shift/Win,
 * arrows, Home/End/PgUp/PgDn/Del and an Fn toggle that swaps the row to F1-F12.
 * Modifiers: tap = next key only, tap again = locked, tap again = released.
 */
@Composable
fun ExtraKeysRow(
    logic: KeyboardLogic,
    onKeys: (List<KeyAction>) -> Unit,
    modifier: Modifier = Modifier,
) {
    var fn by remember { mutableStateOf(false) }
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
    Row(
        modifier.fillMaxWidth().height(ExtraKeysHeight).background(Color(0xF01E1E2E))
            .horizontalScroll(rememberScrollState()).padding(horizontal = 4.dp, vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Cap("Fn", active = fn, repeat = false) { fn = !fn }
        if (fn) {
            for (k in KeyLayout.functionKeys) Cap(k.label) { tapKey(k) }
        } else {
            Cap("Esc") { tapKey(KeyLayout.navKeys[0]) }
            Cap("Tab") { tapKey(KeyLayout.navKeys[1]) }
            for (m in KeyMod.entries) {
                Cap(m.label, state = logic.state(m), repeat = false) { tapMod(m) }
            }
            Cap("←") { tapKey(KeyLayout.arrowKeys[0]) }
            Cap("↑") { tapKey(KeyLayout.arrowKeys[1]) }
            Cap("↓") { tapKey(KeyLayout.arrowKeys[2]) }
            Cap("→") { tapKey(KeyLayout.arrowKeys[3]) }
            Cap("Home") { tapKey(KeyLayout.navKeys[2]) }
            Cap("End") { tapKey(KeyLayout.navKeys[3]) }
            Cap("PgUp") { tapKey(KeyLayout.navKeys[4]) }
            Cap("PgDn") { tapKey(KeyLayout.navKeys[5]) }
            Cap("Del") { tapKey(KeyLayout.navKeys[7]) }
        }
    }
}

/** One key cap. Press = action; holding repeats unless [repeat] is false. */
@Composable
private fun Cap(
    label: String,
    state: ModState = ModState.Off,
    active: Boolean = false,
    repeat: Boolean = true,
    onTap: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val action = rememberUpdatedState(onTap)
    val bg = when {
        state == ModState.OneShot || active -> Color(0xFF89B4FA)
        state == ModState.Locked -> Color(0xFFA6E3A1)
        else -> Color(0xFF45475A)
    }
    val fg = if (bg == Color(0xFF45475A)) Color.White else Color.Black
    Box(
        Modifier
            .widthIn(min = 46.dp)
            .height(40.dp)
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
            }
            .padding(horizontal = 8.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(label, color = fg, fontSize = 15.sp, style = MaterialTheme.typography.labelLarge, maxLines = 1)
    }
}
