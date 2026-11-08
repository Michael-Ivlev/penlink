package dev.penlink.ui

import android.view.WindowManager
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
import androidx.compose.material3.FilterChip
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
import dev.penlink.data.Quality
import dev.penlink.data.StreamPrefs
import kotlin.math.roundToInt
import dev.penlink.Screen
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import dev.penlink.input.ImeMapping
import dev.penlink.input.ImePiece
import dev.penlink.input.KeyDef
import dev.penlink.rtc.LinkState
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

private const val RESIZE_DEBOUNCE_MS = 250L

/** Hint shown in the settings sheet when the PC had to shrink the window to fit its screen. */
fun fittedHint(videoWidth: Int, windowWidth: Int): String {
    val up = if (windowWidth > 0) videoWidth.toDouble() / windowWidth else 1.0
    return "PC screen too small for this size: the picture is upscaled %.1fx and looks soft. Raise the UI scale until this message goes away.".format(up)
}

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
        // Draw under the camera hole too (both orientations): the video fills the whole display.
        val oldCutout = window?.attributes?.layoutInDisplayCutoutMode
        window?.let {
            val a = it.attributes
            a.layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_ALWAYS
            it.attributes = a
        }
        controller?.systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
        controller?.hide(WindowInsetsCompat.Type.systemBars())
        onDispose {
            controller?.show(WindowInsetsCompat.Type.systemBars())
            if (window != null && oldCutout != null) {
                val a = window.attributes
                a.layoutInDisplayCutoutMode = oldCutout
                window.attributes = a
            }
        }
    }

    // The video area: the window minus the phone keyboard and the extra-keys row. Whenever it changes
    // (rotation, keyboard shown/hidden, IME resized) the PC window follows, debounced so a keyboard
    // animation or a rotation sends one resize, not dozens.
    var areaPx by remember { mutableStateOf(0 to 0) }
    LaunchedEffect(areaPx) {
        val (w, h) = areaPx
        if (w <= 0 || h <= 0) return@LaunchedEffect
        delay(RESIZE_DEBOUNCE_MS)
        conn.onScreenSize(w, h)
    }
    val density = LocalDensity.current
    val imePx = WindowInsets.ime.getBottom(density)
    val imeVisible = imePx > 0
    val reservePx = if (imeVisible) imePx + with(density) { ExtraKeysHeight.roundToPx() } else 0

    if (current == null) {
        LaunchedEffect(Unit) { app.navigate(Screen.Apps) }
        return
    }
    LaunchedEffect(current.localId) { app.navigate(Screen.Session(current.localId)) }

    BackHandler { app.navigate(Screen.Apps) }

    var trackpad by remember { mutableStateOf(false) }
    var stripVisible by remember { mutableStateOf(true) }
    var activity_ticks by remember { mutableIntStateOf(0) }
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

    // Keyboard hidden (or tab switched): let go of any sticky modifier.
    LaunchedEffect(imeVisible, current.localId) {
        if (!imeVisible) sendKeys(keyLogic.releaseAll())
    }

    // Text from the phone keyboard: plain `text`, or real key combos while a sticky modifier is on.
    val onImeText = rememberUpdatedState<(String) -> Unit> { s ->
        for (p in ImeMapping.plan(s, keyLogic.anyActive())) {
            when (p) {
                is ImePiece.Text -> current.sendInput(InputMsg.text(p.s))
                is ImePiece.Key -> sendKeys(keyLogic.tapKey(p.key))
            }
        }
    }
    val onImeVk = rememberUpdatedState<(Int) -> Unit> { vk ->
        sendKeys(keyLogic.tapKey(KeyDef("", vk)))
    }

    Box(Modifier.fillMaxSize().background(Color.Black)) {
        // Video + input, recreated per tab so each keeps its own renderer and zoom. It only
        // occupies the area above the keyboard; the PC window is resized to match.
        Box(
            Modifier.fillMaxSize().padding(bottom = with(density) { reservePx.toDp() })
                .onSizeChanged { areaPx = it.width to it.height },
        ) {
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
        }

        // Hidden text sink for the phone's keyboard.
        AndroidView(
            factory = { ctx -> ImeCaptureView(ctx).also { imeRef = it } },
            update = {
                it.onText = { s -> onImeText.value(s) }
                it.onVk = { vk -> onImeVk.value(vk) }
            },
            modifier = Modifier.size(1.dp),
        )

        // Status overlay while connecting, reconnecting, or after the app closed.
        val linkState by conn.link.collectAsState()
        when (val s = status) {
            SessionStatus.Reconnecting ->
                Column(Modifier.align(Alignment.Center).clip(RoundedCornerShape(12.dp)).background(Color(0xE61E1E2E)).padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    if (linkState is LinkState.Unreachable) {
                        Text("Can't reach ${conn.host.name}", color = Color.White, style = MaterialTheme.typography.titleMedium)
                        Text(
                            "No answer for 30 seconds. Is the PC on and penlink-host running? Still trying in the background.",
                            color = Color.White,
                            modifier = Modifier.padding(top = 6.dp),
                        )
                        Button(onClick = { conn.retryNow() }, modifier = Modifier.padding(top = 12.dp)) { Text("Retry now") }
                    } else {
                        CircularProgressIndicator(color = Color.White)
                        Text("Reconnecting to ${conn.host.name}...", color = Color.White, modifier = Modifier.padding(top = 12.dp))
                    }
                }
            SessionStatus.Opening, SessionStatus.Connecting ->
                Column(Modifier.align(Alignment.Center).clip(RoundedCornerShape(12.dp)).background(Color(0xE61E1E2E)).padding(24.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                    CircularProgressIndicator(color = Color.White)
                    Text(
                        if (s == SessionStatus.Opening) "Opening $title..." else "Connecting...",
                        color = Color.White,
                        modifier = Modifier.padding(top = 12.dp),
                    )
                }
            is SessionStatus.Closed -> Column(
                Modifier.align(Alignment.Center).clip(RoundedCornerShape(12.dp)).background(Color(0xE61E1E2E)).padding(24.dp),
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
                            if (st == SessionStatus.Opening || st == SessionStatus.Connecting || st == SessionStatus.Reconnecting) {
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
                    if (imeVisible) imeRef?.hide() else imeRef?.show()
                    activity_ticks++
                }) {
                    Icon(Icons.Default.Keyboard, contentDescription = "Keyboard", tint = if (imeVisible) Color(0xFF89B4FA) else Color.White)
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

        // Extra keys, docked exactly on top of the phone keyboard (the offset follows the IME
        // animation because it is read from the IME insets every frame).
        if (imeVisible) {
            ExtraKeysRow(
                logic = keyLogic,
                onKeys = { sendKeys(it) },
                modifier = Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.ime),
            )
        }
    }

    if (showSettings) {
        SettingsSheet(app, conn, current, onDismiss = { showSettings = false })
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SettingsSheet(app: PenlinkApp, conn: HostConnection, session: RemoteSession, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    val prefs by session.prefs.collectAsState()
    var quality by remember { mutableStateOf(prefs.quality) }
    var step by remember { mutableStateOf(StreamPrefs.UI_STEPS.indexOf(prefs.uiScale).coerceAtLeast(0).toFloat()) }

    fun apply(q: Quality, stepIndex: Int) {
        val p = StreamPrefs(q, StreamPrefs.UI_STEPS[stepIndex])
        session.setPrefs(p)
        scope.launch { app.store.setStreamPrefs(conn.host.address, p) }
    }

    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(Modifier.padding(horizontal = 24.dp, vertical = 8.dp).padding(bottom = 24.dp)) {
            Text("Quality", style = MaterialTheme.typography.titleMedium)
            Row(Modifier.padding(top = 4.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                for (q in Quality.entries) {
                    FilterChip(
                        selected = quality == q,
                        onClick = {
                            quality = q
                            apply(q, step.roundToInt())
                        },
                        label = { Text(q.label) },
                    )
                }
            }
            Text(
                if (quality == Quality.Sharp) "Full phone resolution: crisp text."
                else "Half resolution, upscaled here: bigger UI, lighter stream.",
                style = MaterialTheme.typography.bodySmall,
            )

            val uiPercent = StreamPrefs.UI_STEPS[step.roundToInt()]
            Text("UI size: $uiPercent%", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(top = 16.dp))
            Text(
                "How big text and buttons are on the PC. Higher = bigger.",
                style = MaterialTheme.typography.bodySmall,
            )
            Slider(
                value = step,
                onValueChange = { step = it },
                valueRange = 0f..(StreamPrefs.UI_STEPS.size - 1).toFloat(),
                steps = StreamPrefs.UI_STEPS.size - 2,
                onValueChangeFinished = { apply(quality, step.roundToInt()) },
            )

            val win by session.windowSize.collectAsState()
            val size by session.requestedSize.collectAsState()
            val w = win
            Text("Showing", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(top = 8.dp))
            when {
                w == null -> Text("Waiting for the PC...", style = MaterialTheme.typography.bodyMedium)
                w.isMonitor -> {
                    Text("Virtual monitor: ${w.width} x ${w.height} px", style = MaterialTheme.typography.bodyMedium)
                    val asked = StreamPrefs.dpiAsked(StreamPrefs(quality, uiPercent))
                    val dpi = w.dpi
                    if (dpi != null) {
                        Text(
                            if (dpi == asked) "Windows DPI: $dpi%" else "Windows DPI: $dpi% (asked $asked%, the nearest step Windows allows)",
                            style = MaterialTheme.typography.bodyMedium,
                        )
                    }
                }
                else -> {
                    Text("The app's window: ${w.width} x ${w.height} px", style = MaterialTheme.typography.bodyMedium)
                    Text(
                        "No virtual monitor (is the SudoVDA display driver installed on the PC?), so menus and dialogs outside the app window do not show.",
                        style = MaterialTheme.typography.bodySmall,
                        modifier = Modifier.padding(top = 2.dp),
                    )
                    if (w.fitted) {
                        Text(
                            fittedHint(size.first, w.width),
                            color = MaterialTheme.colorScheme.error,
                            style = MaterialTheme.typography.bodyMedium,
                            modifier = Modifier.padding(top = 4.dp),
                        )
                    }
                }
            }
            Text("Video: ${size.first} x ${size.second} px requested", style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 4.dp))
        }
    }
}
