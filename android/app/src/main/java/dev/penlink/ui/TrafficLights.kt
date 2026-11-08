package dev.penlink.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp

/** The macOS "traffic light" window buttons used for Close and Minimize. */
enum class TrafficKind(val fill: Color, val border: Color, val glyph: Color) {
    Close(Color(0xFFFF5F57), Color(0xFFE0443E), Color(0xFF4D0000)),
    Minimize(Color(0xFFFEBC2E), Color(0xFFD89E24), Color(0xFF995700)),
}

/**
 * A round window button: a 24 dp circle (fill, border and glyph in the macOS colours) inside a 48 dp touch target.
 * The glyph is always drawn: a phone has no hover, so there is nothing to reveal it on.
 */
@Composable
fun TrafficLight(kind: TrafficKind, description: String, modifier: Modifier = Modifier, onClick: () -> Unit) {
    Box(
        modifier
            .size(48.dp)
            .clip(CircleShape)
            .semantics { contentDescription = description }
            .clickable(role = Role.Button, onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Box(
            Modifier.size(24.dp).background(kind.fill, CircleShape).border(1.dp, kind.border, CircleShape),
            contentAlignment = Alignment.Center,
        ) {
            Canvas(Modifier.size(24.dp)) {
                val c = center
                val stroke = 1.8.dp.toPx()
                val r = 4.dp.toPx()
                when (kind) {
                    TrafficKind.Close -> {
                        drawLine(kind.glyph, androidx.compose.ui.geometry.Offset(c.x - r, c.y - r), androidx.compose.ui.geometry.Offset(c.x + r, c.y + r), stroke, StrokeCap.Round)
                        drawLine(kind.glyph, androidx.compose.ui.geometry.Offset(c.x - r, c.y + r), androidx.compose.ui.geometry.Offset(c.x + r, c.y - r), stroke, StrokeCap.Round)
                    }
                    TrafficKind.Minimize ->
                        drawLine(kind.glyph, androidx.compose.ui.geometry.Offset(c.x - r - 1.dp.toPx(), c.y), androidx.compose.ui.geometry.Offset(c.x + r + 1.dp.toPx(), c.y), stroke, StrokeCap.Round)
                }
            }
        }
    }
}
