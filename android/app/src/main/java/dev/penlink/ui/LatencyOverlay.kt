package dev.penlink.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import dev.penlink.rtc.RemoteSession
import dev.penlink.rtc.Rtc

/** Debug readout of where the touch-to-pixel time goes (see [dev.penlink.rtc.LatencyStats]). Off in release builds. */
@Composable
fun BoxScope.LatencyOverlay(session: RemoteSession, rtc: Rtc) {
    val on by rtc.latencyOverlay.collectAsState()
    val text by session.latency.text.collectAsState()
    if (!on || text.isEmpty()) return
    Text(
        text,
        color = Color(0xFF9CFF9C),
        fontSize = 10.sp,
        lineHeight = 12.sp,
        modifier = Modifier.align(Alignment.BottomStart).padding(4.dp).background(Color(0x99000000)).padding(4.dp),
    )
}
