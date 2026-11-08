package dev.penlink.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.animateFloatAsState
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
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
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
val ExtraKeysHeight: Dp = 56.dp

private val KeyHeight = 48.dp

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
    val haptics = rememberHaptics()

    fun tapKey(k: KeyDef) {
        send.value(logic.tapKey(k))
        version++
    }

    fun tapMod(m: KeyMod) {
        send.value(logic.tapModifier(m))
        version++
        haptics.toggle(logic.state(m) != ModState.Off)
    }

    @Suppress("UNUSED_EXPRESSION") version
    val bar = MaterialTheme.colorScheme.surfaceContainer.copy(alpha = 0.96f)
    val hairline = MaterialTheme.colorScheme.outlineVariant
    Row(
        modifier
            .fillMaxWidth()
            .height(ExtraKeysHeight)
            .drawBehind {
                // Frosted-looking bar: translucent surface with a hairline on top.
                drawRect(bar)
                drawLine(hairline, Offset(0f, 0f), Offset(size.width, 0f), 1.dp.toPx())
            }
            .horizontalScroll(rememberScrollState())
            .padding(horizontal = 6.dp, vertical = 4.dp),
        horizontalArrangement = Arrangement.spacedBy(6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Cap("Fn", "Function keys", active = fn, repeat = false, haptic = false) {
            fn = !fn
            haptics.toggle(fn)
        }
        if (fn) {
            for (k in KeyLayout.functionKeys) Cap(k.label) { tapKey(k) }
        } else {
            Cap("Esc", "Escape") { tapKey(KeyLayout.navKeys[0]) }
            Cap("Tab") { tapKey(KeyLayout.navKeys[1]) }
            for (m in KeyMod.entries) {
                Cap(m.label, "${m.label} modifier", state = logic.state(m), repeat = false, haptic = false) { tapMod(m) }
            }
            Cap("←", "Left arrow") { tapKey(KeyLayout.arrowKeys[0]) }
            Cap("↑", "Up arrow") { tapKey(KeyLayout.arrowKeys[1]) }
            Cap("↓", "Down arrow") { tapKey(KeyLayout.arrowKeys[2]) }
            Cap("→", "Right arrow") { tapKey(KeyLayout.arrowKeys[3]) }
            Cap("Home") { tapKey(KeyLayout.navKeys[2]) }
            Cap("End") { tapKey(KeyLayout.navKeys[3]) }
            Cap("PgUp", "Page up") { tapKey(KeyLayout.navKeys[4]) }
            Cap("PgDn", "Page down") { tapKey(KeyLayout.navKeys[5]) }
            Cap("Del", "Delete") { tapKey(KeyLayout.navKeys[7]) }
        }
    }
}

/**
 * One key cap: rounded, with a pressed state. Press = action; holding repeats unless [repeat] is false.
 * Sticky modifiers: next-key-only is tinted with a border, locked is a solid accent fill.
 */
@Composable
private fun Cap(
    label: String,
    description: String = label,
    state: ModState = ModState.Off,
    active: Boolean = false,
    repeat: Boolean = true,
    haptic: Boolean = true,
    onTap: () -> Unit,
) {
    val scope = rememberCoroutineScope()
    val action = rememberUpdatedState(onTap)
    val haptics = rememberHaptics()
    var pressed by remember { mutableStateOf(false) }
    val cs = MaterialTheme.colorScheme

    val container by animateColorAsState(
        when {
            state == ModState.Locked || active -> cs.primary
            state == ModState.OneShot -> cs.primaryContainer
            pressed -> cs.secondaryContainer
            else -> cs.surfaceContainerHighest
        },
        label = "capBg",
    )
    val content by animateColorAsState(
        when {
            state == ModState.Locked || active -> cs.onPrimary
            state == ModState.OneShot -> cs.onPrimaryContainer
            pressed -> cs.onSecondaryContainer
            else -> cs.onSurface
        },
        label = "capFg",
    )
    val outline = cs.primary
    val scale by animateFloatAsState(if (pressed) 0.94f else 1f, label = "capScale")
    val stateText = when {
        active -> "On"
        state == ModState.OneShot -> "On for the next key"
        state == ModState.Locked -> "Locked"
        else -> null
    }

    Box(
        Modifier
            .widthIn(min = 48.dp)
            .height(KeyHeight)
            .graphicsLayer { scaleX = scale; scaleY = scale }
            .drawBehind {
                val r = CornerRadius(12.dp.toPx())
                drawRoundRect(container, cornerRadius = r)
                if (state == ModState.OneShot) {
                    val w = 2.dp.toPx()
                    drawRoundRect(outline, topLeft = Offset(w / 2, w / 2), size = Size(size.width - w, size.height - w), cornerRadius = r, style = Stroke(w))
                }
            }
            .semantics {
                role = Role.Button
                contentDescription = description
                if (stateText != null) stateDescription = stateText
                onClick(label = description) { action.value(); true }
            }
            .pointerInput(repeat) {
                detectTapGestures(onPress = {
                    pressed = true
                    if (haptic) haptics.key()
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
                    pressed = false
                })
            }
            .padding(horizontal = 10.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(label, color = content, fontSize = 15.sp, style = MaterialTheme.typography.labelLarge, maxLines = 1)
    }
}
