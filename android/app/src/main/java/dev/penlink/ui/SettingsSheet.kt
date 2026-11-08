package dev.penlink.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.dp
import dev.penlink.PenlinkApp
import dev.penlink.data.StreamPrefs
import dev.penlink.rtc.HostConnection
import dev.penlink.rtc.RemoteSession
import dev.penlink.ui.theme.Spacing
import kotlinx.coroutines.launch
import kotlin.math.roundToInt

/** Per-session settings as a bottom sheet with grouped sections. Behaviour is unchanged: same prefs, same saving. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsSheet(app: PenlinkApp, conn: HostConnection, session: RemoteSession, screen: Pair<Int, Int>, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    val haptics = rememberHaptics()
    val prefs by session.prefs.collectAsState()
    val (sw, sh) = screen
    var sharp by remember { mutableStateOf(prefs.sharpText) }
    val steps = StreamPrefs.steps(sharp, sw, sh)
    var step by remember { mutableStateOf(steps.indexOf(StreamPrefs.snap(prefs.uiScale, steps)).toFloat()) }

    fun apply(p: StreamPrefs) {
        session.setPrefs(p)
        scope.launch { app.store.setStreamPrefs(conn.host.address, p) }
    }

    val uiPercent = steps[step.roundToInt().coerceIn(0, steps.size - 1)]

    fun setSharp(on: Boolean) {
        if (on == sharp) return
        haptics.toggle(on)
        sharp = on
        val p = prefs.copy(sharpText = on, uiScale = uiPercent).fitted(sw, sh)
        val newSteps = StreamPrefs.steps(on, sw, sh)
        step = newSteps.indexOf(p.uiScale).toFloat()
        apply(p)
    }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(
            Modifier.verticalScroll(rememberScrollState()).padding(horizontal = Spacing.xl).padding(bottom = Spacing.xl),
            verticalArrangement = Arrangement.spacedBy(Spacing.lg),
        ) {
            Text("Session settings", style = MaterialTheme.typography.headlineSmall)

            SettingsGroup("Display") {
                Text("Picture quality", style = MaterialTheme.typography.titleMedium)
                SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth()) {
                    SegmentedButton(
                        selected = !sharp,
                        onClick = { setSharp(false) },
                        shape = SegmentedButtonDefaults.itemShape(0, 2),
                        modifier = Modifier.heightIn(min = 48.dp),
                    ) { Text("Fast") }
                    SegmentedButton(
                        selected = sharp,
                        onClick = { setSharp(true) },
                        shape = SegmentedButtonDefaults.itemShape(1, 2),
                        modifier = Modifier.heightIn(min = 48.dp),
                    ) { Text("Sharp text") }
                }
                Hint(
                    if (sharp) "Full phone resolution: crisp text, but the UI cannot get as big and more pixels are sent."
                    else "Fewer pixels, faster. Text is a little soft when enlarged.",
                )

                Row(Modifier.fillMaxWidth().padding(top = Spacing.sm), verticalAlignment = Alignment.CenterVertically) {
                    Text("UI size", style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                    Surface(shape = MaterialTheme.shapes.small, color = MaterialTheme.colorScheme.primaryContainer) {
                        Text(
                            "$uiPercent%",
                            style = MaterialTheme.typography.labelLarge,
                            color = MaterialTheme.colorScheme.onPrimaryContainer,
                            modifier = Modifier.padding(horizontal = Spacing.md, vertical = Spacing.xs),
                        )
                    }
                }
                Slider(
                    value = step,
                    onValueChange = {
                        if (it.roundToInt() != step.roundToInt()) haptics.tick()
                        step = it
                    },
                    valueRange = 0f..(steps.size - 1).toFloat(),
                    steps = (steps.size - 2).coerceAtLeast(0),
                    onValueChangeFinished = { apply(prefs.copy(sharpText = sharp, uiScale = uiPercent)) },
                    modifier = Modifier.semantics { stateDescription = "$uiPercent percent" },
                )
                Row(Modifier.fillMaxWidth()) {
                    Text("${steps.first()}%", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.weight(1f))
                    Text("${steps.last()}%", style = MaterialTheme.typography.labelSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                Hint(
                    if (sharp) "Windows scales text and buttons. Windows allows at most ${steps.last()}% on this screen."
                    else "The PC draws at lower resolution and this phone enlarges it. Up to ${steps.last()}% on this screen.",
                )
            }

            SettingsGroup("Keyboard") {
                SwitchRow(
                    title = "Resize app above keyboard",
                    checked = prefs.resizeAboveKeyboard,
                    description = if (prefs.resizeAboveKeyboard) "On: the PC window shrinks to the area above the keyboard, so the app reflows."
                    else "Off: the PC window keeps its size and the picture slides up so what you are working on stays visible above the keyboard.",
                    onChange = { haptics.toggle(it); apply(prefs.copy(resizeAboveKeyboard = it)) },
                )
            }

            val win by session.windowSize.collectAsState()
            val size by session.requestedSize.collectAsState()
            SettingsGroup("Connection") {
                val w = win
                when {
                    w == null -> Text("Waiting for the PC...", style = MaterialTheme.typography.bodyMedium)
                    w.isMonitor -> {
                        Text("Virtual monitor: ${w.width} x ${w.height} px", style = MaterialTheme.typography.bodyMedium)
                        val dpi = w.dpi
                        val up = if (w.width > 0) size.first.toDouble() / w.width else 1.0
                        Text(
                            "Windows DPI ${dpi ?: 100}%%, enlarged %.2fx here = UI size about %d%%".format(up, ((dpi ?: 100) * up).roundToInt()),
                            style = MaterialTheme.typography.bodyMedium,
                        )
                        if (dpi != null && sharp && dpi != uiPercent) {
                            Hint("Windows used the nearest DPI step it allows ($dpi%, asked $uiPercent%).")
                        }
                    }
                    else -> {
                        Text("The app's window: ${w.width} x ${w.height} px", style = MaterialTheme.typography.bodyMedium)
                        Hint("No virtual monitor (is the SudoVDA display driver installed on the PC?), so menus and dialogs outside the app window do not show.")
                        if (w.fitted) {
                            Text(
                                fittedHint(size.first, w.width),
                                color = MaterialTheme.colorScheme.error,
                                style = MaterialTheme.typography.bodyMedium,
                            )
                        }
                    }
                }
                Hint("Phone area: ${size.first} x ${size.second} px")
                val stats by app.rtc.latencyOverlay.collectAsState()
                SwitchRow(
                    title = "Show performance stats",
                    checked = stats,
                    description = "A small readout of lag, frame rate and network timing in the corner of the app.",
                    onChange = { haptics.toggle(it); app.rtc.setLatencyOverlay(it) },
                )
            }
        }
    }
}

@Composable
private fun SettingsGroup(title: String, content: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(Spacing.sm)) {
        Text(title, style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(start = Spacing.xs))
        Surface(shape = MaterialTheme.shapes.large, color = MaterialTheme.colorScheme.surfaceContainer) {
            Column(Modifier.fillMaxWidth().padding(Spacing.lg), verticalArrangement = Arrangement.spacedBy(Spacing.xs)) { content() }
        }
    }
}

@Composable
private fun Hint(text: String) {
    Text(text, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

/** A whole-row switch: the label, a description and a Switch; the full row is the 48 dp+ touch target. */
@Composable
private fun SwitchRow(title: String, checked: Boolean, description: String, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = 56.dp).toggleable(value = checked, role = Role.Switch, onValueChange = onChange),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f).padding(end = Spacing.md)) {
            Text(title, style = MaterialTheme.typography.titleMedium)
            Hint(description)
        }
        Switch(checked = checked, onCheckedChange = null)
    }
}
