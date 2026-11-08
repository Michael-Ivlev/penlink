package dev.penlink.ui

import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Keyboard
import androidx.compose.material.icons.filled.Mouse
import androidx.compose.material.icons.filled.TouchApp
import androidx.compose.material.icons.filled.Tune
import androidx.compose.material.icons.filled.ZoomOutMap
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Slider
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import dev.penlink.PenlinkApp
import dev.penlink.Screen
import dev.penlink.input.ImeCaptureView
import dev.penlink.input.InputOverlayView
import dev.penlink.input.KeyAction
import dev.penlink.input.KeyboardLogic
import dev.penlink.protocol.InputMsg
import dev.penlink.rtc.HostConnection
import dev.penlink.rtc.RemoteSession
import dev.penlink.rtc.SessionStatus
import dev.penlink.windowPixels
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import org.webrtc.RendererCommon
import org.webrtc.SurfaceViewRenderer

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SessionScreen(app: PenlinkApp, conn: HostConnection, selectedId: Long) {
    val activity = LocalActivity.current
    val view = LocalView.current
    val sessions by conn.sessions.collectAsState()
    val current = sessions.firstOrNull { it.localId == selectedId } ?: sessions.lastOrNull()
    val scope = rememberCoroutineScope()

    // Immersive: no system bars while this screen is shown.
    DisposableEffect(Unit) {
        val window = activity?.window
        val controller = window?.let { WindowInsetsControllerCompat(it, view) }
        if (window != null) WindowCompat.setDecorFitsSystemWindows(window, false)
        controller?.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        controller?.hide(WindowInsetsCompat.Type.systemBars())
        onDispose { controller?.show(WindowInsetsCompat.Type.systemBars()) }
    }

    // Rotation (or any window size change): tell the host about every open session.
    val config = LocalConfiguration.current
    LaunchedEffect(config.orientation, config.screenWidthDp, config.screenHeightDp) {
        activity?.windowPixels()?.let { (w, h) -> conn.onScreenSize(w, h) }
    }

    if (current == null) {
        LaunchedEffect(Unit) { app.navigate(Screen.Apps) }
        return
    }
    LaunchedEffect(current.localId) { app.navigate(Screen.Session(current.localId)) }

    BackHandler { app.navigate(Screen.Apps) }

    var trackpad by remember { mutableStateOf(false) }
    var stripVisible by remember { mutableStateOf(true) }
    var activity_ticks by remember { mutableIntStateOf(0) }
    var keyboardOpen by remember { mutableStateOf(false) }
    var systemIme by remember { mutableStateOf(false) }
    var showSettings by remember { mutableStateOf(false) }
    val keyLogic = remember(current.localId) { KeyboardLogic() }
    val status by current.status.collectAsState()
    val title by current.title.collectAsState()
    val track by current.videoTrack.collectAsState()
    val videoSize by current.videoSize.collectAsState()

    // Auto-hide the tab strip.
    LaunchedEffect(stripVisible, activity_ticks, showSettings) {
        if (stripVisible && !showSettings) {
            delay(4000)
            stripVisible = false
        }
    }

    var overlayRef by remember(current.localId) { mutableStateOf<InputOverlayView?>(null) }
    var imeRef by remember(current.localId) { mutableStateOf<ImeCaptureView?>(null) }

    fun sendKeys(actions: List<KeyAction>) {
        for (a in actions) current.sendInput(InputMsg.key(a.down, a.vk))
    }

    Box(Modifier.fillMaxSize().background(Color.Black)) {
        // Video + input, recreated per tab so each keeps its own renderer and zoom.
        androidx.compose.runtime.key(current.localId) {
            val rtc = app.rtc
            val renderer = remember {
                SurfaceViewRenderer(view.context).also { r ->
                    r.init(
                        rtc.eglBase.eglBaseContext,
                        object : RendererCommon.RendererEvents {
                            override fun onFirstFrameRendered() {}
                            override fun onFrameResolutionChanged(w: Int, h: Int, rotation: Int) {
                                current.videoSize.value = if (rotation % 180 == 0) w to h else h to w
                            }
                        },
                    )
                    r.setScalingType(RendererCommon.ScalingType.SCALE_ASPECT_FIT)
                    r.setEnableHardwareScaler(true)
                }
            }
            DisposableEffect(track) {
                val t = track
                t?.addSink(renderer)
                if (t != null) current.requestKeyframe()
                onDispose { t?.removeSink(renderer) }
            }
            DisposableEffect(Unit) { onDispose { renderer.release() } }

            AndroidView(factory = { renderer }, modifier = Modifier.fillMaxSize())

            AndroidView(
                factory = { ctx ->
                    InputOverlayView(ctx).also { o ->
                        o.onTransform = { z, px, py ->
                            renderer.scaleX = z
                            renderer.scaleY = z
                            renderer.translationX = px
                            renderer.translationY = py
                        }
                        overlayRef = o
                    }
                },
                update = { o ->
                    o.sink = { json -> current.sendInput(json) }
                    o.trackpad = trackpad
                    o.videoW = videoSize.first
                    o.videoH = videoSize.second
                },
                modifier = Modifier.fillMaxSize(),
            )
        }

        // Hidden text sink for the phone's normal keyboard.
        AndroidView(
            factory = { ctx -> ImeCaptureView(ctx).also { imeRef = it } },
            update = { it.sink = { json -> current.sendInput(json) } },
            modifier = Modifier.size(1.dp),
        )

        // Status overlay while connecting or after the app closed.
        when (val s = status) {
            SessionStatus.Opening, SessionStatus.Connecting ->
                Column(Modifier.align(Alignment.Center), horizontalAlignment = Alignment.CenterHorizontally) {
                    CircularProgressIndicator(color = Color.White)
                    Text(
                        if (s == SessionStatus.Opening) "Opening $title..." else "Connecting...",
                        color = Color.White,
                        modifier = Modifier.padding(top = 12.dp),
                    )
                }
            is SessionStatus.Closed -> Column(
                Modifier.align(Alignment.Center).padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text("$title closed (${s.reason})", color = Color.White)
                Button(onClick = { conn.close(current) }) { Text("Close tab") }
            }
            SessionStatus.Live -> {}
        }

        // Tab strip (auto-hide).
        if (stripVisible) {
            Row(
                Modifier.fillMaxWidth().background(Color(0xE61E1E2E)).padding(horizontal = 4.dp, vertical = 2.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                IconButton(onClick = { app.navigate(Screen.Apps) }) {
                    Icon(Icons.Default.Add, contentDescription = "Open another app", tint = Color.White)
                }
                Row(
                    Modifier.weight(1f).horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    for (s in sessions) {
                        val t by s.title.collectAsState()
                        val st by s.status.collectAsState()
                        val selected = s.localId == current.localId
                        Row(
                            Modifier.clip(RoundedCornerShape(8.dp))
                                .background(if (selected) Color(0xFF89B4FA) else Color(0xFF45475A))
                                .clickable {
                                    activity_ticks++
                                    app.navigate(Screen.Session(s.localId))
                                }
                                .padding(start = 10.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            if (st == SessionStatus.Opening || st == SessionStatus.Connecting) {
                                CircularProgressIndicator(
                                    modifier = Modifier.padding(end = 6.dp).size(14.dp),
                                    strokeWidth = 2.dp,
                                    color = if (selected) Color.Black else Color.White,
                                )
                            }
                            Text(
                                t,
                                color = if (selected) Color.Black else Color.White,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.width(110.dp),
                                style = MaterialTheme.typography.labelLarge,
                            )
                            IconButton(onClick = { conn.close(s) }, modifier = Modifier.size(32.dp)) {
                                Icon(
                                    Icons.Default.Close, contentDescription = "Close tab",
                                    tint = if (selected) Color.Black else Color.White,
                                    modifier = Modifier.size(16.dp),
                                )
                            }
                        }
                    }
                }
                IconButton(onClick = { trackpad = !trackpad; activity_ticks++ }) {
                    Icon(
                        if (trackpad) Icons.Default.Mouse else Icons.Default.TouchApp,
                        contentDescription = if (trackpad) "Trackpad mode (tap for direct)" else "Direct mode (tap for trackpad)",
                        tint = Color.White,
                    )
                }
                IconButton(onClick = { overlayRef?.resetZoom(); activity_ticks++ }) {
                    Icon(Icons.Default.ZoomOutMap, contentDescription = "Reset zoom", tint = Color.White)
                }
                IconButton(onClick = { showSettings = true }) {
                    Icon(Icons.Default.Tune, contentDescription = "Session settings", tint = Color.White)
                }
                IconButton(onClick = {
                    keyboardOpen = !keyboardOpen
                    if (!keyboardOpen) {
                        sendKeys(keyLogic.releaseAll())
                        if (systemIme) imeRef?.hide()
                        systemIme = false
                    }
                    activity_ticks++
                }) {
                    Icon(Icons.Default.Keyboard, contentDescription = "Keyboard", tint = if (keyboardOpen) Color(0xFF89B4FA) else Color.White)
                }
            }
        } else {
            // Small handle to bring the strip back.
            Box(
                Modifier.align(Alignment.TopCenter).padding(top = 2.dp)
                    .clip(RoundedCornerShape(50)).background(Color(0x66FFFFFF))
                    .clickable { stripVisible = true }
                    .size(width = 56.dp, height = 12.dp),
            )
        }

        if (keyboardOpen && !systemIme) {
            KeyboardPanel(
                logic = keyLogic,
                onKeys = { sendKeys(it) },
                onSystemKeyboard = {
                    systemIme = true
                    imeRef?.show()
                },
                modifier = Modifier.align(Alignment.BottomCenter),
            )
        } else if (keyboardOpen && systemIme) {
            Button(
                onClick = {
                    systemIme = false
                    imeRef?.hide()
                },
                modifier = Modifier.align(Alignment.TopEnd).padding(top = 48.dp, end = 8.dp),
            ) { Text("Penlink keyboard") }
        }
    }

    if (showSettings) {
        ScaleSheet(app, current, onDismiss = { showSettings = false })
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ScaleSheet(app: PenlinkApp, session: RemoteSession, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    val activity = LocalActivity.current
    val initial by session.scale.collectAsState()
    var value by remember { mutableStateOf(initial.toFloat()) }
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(Modifier.padding(horizontal = 24.dp, vertical = 8.dp).padding(bottom = 24.dp)) {
            Text("UI scale: ${"%.2f".format(value)}x", style = MaterialTheme.typography.titleMedium)
            Text(
                "How much bigger the PC window is drawn than at 100%. Higher = bigger text and buttons.",
                style = MaterialTheme.typography.bodySmall,
            )
            Slider(
                value = value,
                onValueChange = { value = it },
                valueRange = 1f..4f,
                steps = 11,
                onValueChangeFinished = {
                    val (w, h) = activity?.windowPixels() ?: session.requestedSize.value
                    session.resize(w, h, value.toDouble())
                    scope.launch { app.store.setDefaultScale(value.toDouble()) }
                },
            )
            Text("Video: ${session.requestedSize.value.first} x ${session.requestedSize.value.second} px")
        }
    }
}

