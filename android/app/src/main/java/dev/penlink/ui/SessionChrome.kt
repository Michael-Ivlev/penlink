package dev.penlink.ui

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Keyboard
import androidx.compose.material.icons.filled.Mouse
import androidx.compose.material.icons.filled.TouchApp
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material.icons.filled.ZoomOutMap
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconToggleButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.onClick
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.penlink.rtc.LinkState
import dev.penlink.rtc.SessionStatus
import dev.penlink.ui.theme.Spacing

/*
 * Chrome drawn over the remote picture: the pill, the control bar and the status cards. Everything here is a small
 * composable that takes plain values and lambdas, so SessionScreen's hot paths (video size, touch, IME) never
 * recompose it. The bar sits on a translucent surface; a real backdrop blur is not possible because the picture is a
 * SurfaceView that Compose cannot sample.
 */

/** Translucent surface used by the bar and the status cards. */
@Composable
private fun frostedColor(alpha: Float = 0.88f): Color = MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = alpha)

/** The floating control bar: window buttons at the left, then title, then the mode / zoom / settings / keyboard buttons. */
@Composable
fun SessionControlBar(
    title: String,
    trackpad: Boolean,
    keyboardVisible: Boolean,
    onClose: () -> Unit,
    onMinimize: () -> Unit,
    onToggleTrackpad: () -> Unit,
    onResetZoom: () -> Unit,
    onSettings: () -> Unit,
    onToggleKeyboard: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val haptics = rememberHaptics()
    Surface(
        modifier = modifier.fillMaxWidth(),
        shape = MaterialTheme.shapes.extraLarge,
        color = frostedColor(),
        contentColor = MaterialTheme.colorScheme.onSurface,
        shadowElevation = 10.dp,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f)),
    ) {
        Row(Modifier.padding(horizontal = Spacing.xs, vertical = 2.dp), verticalAlignment = Alignment.CenterVertically) {
            TrafficLight(TrafficKind.Close, "Close (ends the session and removes the virtual monitor)") { haptics.tap(); onClose() }
            TrafficLight(TrafficKind.Minimize, "Minimize (keeps the app running)") { haptics.tap(); onMinimize() }
            Text(
                title,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f).padding(horizontal = Spacing.sm),
                style = MaterialTheme.typography.labelLarge,
            )
            IconToggleButton(checked = trackpad, onCheckedChange = { haptics.toggle(it); onToggleTrackpad() }) {
                Icon(
                    if (trackpad) Icons.Default.Mouse else Icons.Default.TouchApp,
                    contentDescription = if (trackpad) "Trackpad mode (tap for direct)" else "Direct mode (tap for trackpad)",
                )
            }
            IconButton(onClick = { haptics.tap(); onResetZoom() }) {
                Icon(Icons.Default.ZoomOutMap, contentDescription = "Reset zoom")
            }
            IconButton(onClick = { haptics.tap(); onSettings() }) {
                Icon(Icons.Default.Tune, contentDescription = "Session settings")
            }
            IconToggleButton(checked = keyboardVisible, onCheckedChange = { haptics.toggle(it); onToggleKeyboard() }) {
                Icon(Icons.Default.Keyboard, contentDescription = if (keyboardVisible) "Hide keyboard" else "Show keyboard")
            }
        }
    }
}

/**
 * The always-visible way to the control bar: a thin pill centred in the top strip. The strip (status bar / cutout
 * height) is outside the video, so tapping it never reaches the remote app; when a phone has no such strip the pill is
 * only a few dp tall at the very top. Tap or swipe down opens the bar.
 */
@Composable
fun SessionHandle(modifier: Modifier, stripHeight: Dp, onOpen: () -> Unit) {
    val h = stripHeight.coerceIn(14.dp, 36.dp)
    Box(
        modifier
            .width(140.dp).height(h)
            .semantics {
                role = Role.Button
                contentDescription = "Show session controls"
                onClick(label = "Show session controls") { onOpen(); true }
            }
            .pointerInput(Unit) { detectTapGestures { onOpen() } }
            .pointerInput(Unit) { detectVerticalDragGestures { _, dy -> if (dy > 4f) onOpen() } },
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.clip(CircleShape).background(Color(0x99FFFFFF)).size(width = 56.dp, height = 5.dp))
    }
}

/** A centred card over the picture for connecting / reconnecting / closed states. */
@Composable
private fun StatusCard(modifier: Modifier, content: @Composable () -> Unit) {
    Surface(
        modifier = modifier.padding(Spacing.lg).widthIn(max = 360.dp),
        shape = MaterialTheme.shapes.extraLarge,
        color = frostedColor(0.94f),
        contentColor = MaterialTheme.colorScheme.onSurface,
        shadowElevation = 12.dp,
    ) {
        Column(
            Modifier.padding(Spacing.xl),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(Spacing.sm),
        ) { content() }
    }
}

/** Status overlay while connecting, reconnecting, or after the app closed. [Live] draws nothing. */
@Composable
fun SessionStatusOverlay(
    status: SessionStatus,
    link: LinkState,
    hostName: String,
    title: String,
    onRetry: () -> Unit,
    onCloseTab: () -> Unit,
    modifier: Modifier = Modifier,
) {
    when (status) {
        SessionStatus.Reconnecting ->
            StatusCard(modifier) {
                if (link is LinkState.Unreachable) {
                    Text("Can't reach $hostName", style = MaterialTheme.typography.titleMedium, textAlign = TextAlign.Center)
                    Text(
                        "No answer for 30 seconds. Is the PC on and penlink-host running? Still trying in the background.",
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        textAlign = TextAlign.Center,
                    )
                    Button(onClick = onRetry) { Text("Retry now") }
                } else {
                    CircularProgressIndicator()
                    Text("Reconnecting to $hostName...", style = MaterialTheme.typography.bodyLarge, textAlign = TextAlign.Center)
                }
            }
        SessionStatus.Opening, SessionStatus.Connecting ->
            StatusCard(modifier) {
                CircularProgressIndicator()
                Text(
                    if (status == SessionStatus.Opening) "Opening $title..." else "Connecting...",
                    style = MaterialTheme.typography.bodyLarge,
                    textAlign = TextAlign.Center,
                )
            }
        is SessionStatus.Closed ->
            StatusCard(modifier) {
                Text("$title closed (${status.reason})", style = MaterialTheme.typography.bodyLarge, textAlign = TextAlign.Center)
                FilledTonalButton(onClick = onCloseTab) { Text("Close") }
            }
        SessionStatus.Live -> {}
    }
}
