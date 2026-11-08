package dev.penlink.ui

import androidx.compose.animation.core.LinearEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shape
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedback
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import dev.penlink.rtc.SessionStatus
import dev.penlink.ui.theme.Spacing
import dev.penlink.ui.theme.extra

/** Small haptic vocabulary so buttons, toggles and modifiers feel consistent. */
class Haptics(private val fb: HapticFeedback) {
    fun tap() = fb.performHapticFeedback(HapticFeedbackType.ContextClick)
    fun key() = fb.performHapticFeedback(HapticFeedbackType.VirtualKey)
    fun toggle(on: Boolean) = fb.performHapticFeedback(if (on) HapticFeedbackType.ToggleOn else HapticFeedbackType.ToggleOff)
    fun tick() = fb.performHapticFeedback(HapticFeedbackType.SegmentTick)
    fun confirm() = fb.performHapticFeedback(HapticFeedbackType.Confirm)
    fun reject() = fb.performHapticFeedback(HapticFeedbackType.Reject)
}

@Composable
fun rememberHaptics(): Haptics {
    val fb = LocalHapticFeedback.current
    return remember(fb) { Haptics(fb) }
}

/**
 * A loading placeholder block with a moving highlight. The animation is read in the draw phase only, so it never
 * recomposes anything.
 */
@Composable
fun ShimmerBox(modifier: Modifier = Modifier, shape: Shape = MaterialTheme.shapes.medium) {
    val base = MaterialTheme.colorScheme.surfaceContainerHigh
    val highlight = MaterialTheme.colorScheme.surfaceContainerHighest
    val transition = rememberInfiniteTransition(label = "shimmer")
    val progress = transition.animateFloat(
        initialValue = -1f,
        targetValue = 2f,
        animationSpec = infiniteRepeatable(tween(1300, easing = LinearEasing), RepeatMode.Restart),
        label = "shimmerX",
    )
    Box(
        modifier
            .clip(shape)
            .drawBehind {
                val w = size.width.coerceAtLeast(1f)
                val x = progress.value * w
                drawRect(
                    Brush.linearGradient(
                        colors = listOf(base, highlight, base),
                        start = Offset(x, 0f),
                        end = Offset(x + w, size.height),
                    ),
                )
            }
            .clearAndSetSemantics { },
    )
}

/** A status dot with a text label next to it (colour is never the only signal). */
@Composable
fun StatusDot(color: Color, modifier: Modifier = Modifier, size: Dp = 8.dp) {
    Box(modifier.size(size).clip(CircleShape).background(color))
}

enum class ChipTone { Success, Neutral, Warning, Error }

/** Pill shaped status chip for an open session. */
@Composable
fun StatusChip(label: String, tone: ChipTone, busy: Boolean, modifier: Modifier = Modifier) {
    val extra = MaterialTheme.extra
    val cs = MaterialTheme.colorScheme
    val (bg, fg) = when (tone) {
        ChipTone.Success -> extra.successContainer to extra.onSuccessContainer
        ChipTone.Neutral -> cs.secondaryContainer to cs.onSecondaryContainer
        ChipTone.Warning -> extra.warningContainer to extra.onWarningContainer
        ChipTone.Error -> cs.errorContainer to cs.onErrorContainer
    }
    Surface(modifier, shape = CircleShape, color = bg, contentColor = fg) {
        Row(Modifier.padding(horizontal = 10.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
            if (busy) {
                CircularProgressIndicator(Modifier.size(10.dp), strokeWidth = 1.5.dp, color = fg)
            } else {
                StatusDot(if (tone == ChipTone.Success) extra.success else fg, size = 8.dp)
            }
            Text(label, Modifier.padding(start = 6.dp), style = MaterialTheme.typography.labelMedium, maxLines = 1)
        }
    }
}

fun chipToneFor(s: SessionStatus): ChipTone = when (s) {
    SessionStatus.Live -> ChipTone.Success
    SessionStatus.Opening, SessionStatus.Connecting -> ChipTone.Neutral
    SessionStatus.Reconnecting -> ChipTone.Warning
    is SessionStatus.Closed -> ChipTone.Error
}

/** Big friendly message with an icon, used for empty and error states. */
@Composable
fun MessagePanel(
    icon: ImageVector,
    title: String,
    body: String,
    modifier: Modifier = Modifier,
    isError: Boolean = false,
    actions: @Composable () -> Unit = {},
) {
    val cs = MaterialTheme.colorScheme
    Column(
        modifier.fillMaxWidth().padding(horizontal = Spacing.xl, vertical = Spacing.xxl),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(Spacing.sm),
    ) {
        Box(
            Modifier.size(88.dp).clip(CircleShape).background(if (isError) cs.errorContainer else cs.secondaryContainer),
            contentAlignment = Alignment.Center,
        ) {
            Icon(icon, contentDescription = null, Modifier.size(40.dp), tint = if (isError) cs.onErrorContainer else cs.onSecondaryContainer)
        }
        Spacer(Modifier.height(Spacing.sm))
        Text(title, style = MaterialTheme.typography.titleLarge, textAlign = TextAlign.Center)
        Text(body, style = MaterialTheme.typography.bodyMedium, color = cs.onSurfaceVariant, textAlign = TextAlign.Center)
        Spacer(Modifier.height(Spacing.sm))
        actions()
    }
}
