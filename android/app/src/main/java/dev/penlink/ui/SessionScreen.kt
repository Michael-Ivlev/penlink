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
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Minimize
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
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.ui.input.pointer.pointerInput
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
import dev.penlink.data.StreamPrefs
import kotlin.math.roundToInt
import dev.penlink.Screen
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.systemBarsIgnoringVisibility
import androidx.compose.material3.Switch
import androidx.compose.ui.platform.LocalLayoutDirection
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

@OptIn(ExperimentalMaterial3Api::class, androidx.compose.foundation.layout.ExperimentalLayoutApi::class)
@Composable
fun SessionScreen(app: PenlinkApp, conn: HostConnection, selectedId: Long) {
    val activity = LocalActivity.current
    val view = LocalView.current
    val sessions by conn.sessions.collectAsState()
    val current = sessions.firstOrNull { it.localId == selectedId }
    val scope = rememberCoroutineScope()

    // Immersive: no system bars while this screen is shown.
    DisposableEffect(Unit) {
        val window = activity?.window
        val controller = window?.let { WindowInsetsControllerCompat(it, view) }
        if (window != null) WindowCompat.setDecorFitsSystemWindows(window, false)
        // The window may extend into the cutout (the unsafe strips are painted black); the video itself
        // sits inside the safe area, see safeL..safeB below.
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

    // Safe area: display cutout and system bars (as if shown, so it does not jump when the bars are hidden).
    val ld = LocalLayoutDirection.current
    val cutout = WindowInsets.displayCutout
    val bars = WindowInsets.systemBarsIgnoringVisibility
    val safeL = maxOf(cutout.getLeft(density, ld), bars.getLeft(density, ld))
    val safeR = maxOf(cutout.getRight(density, ld), bars.getRight(density, ld))
    val safeT = maxOf(cutout.getTop(density), bars.getTop(density))
    val safeB = maxOf(cutout.getBottom(density), bars.getBottom(density))

    if (current == null) {
        LaunchedEffect(Unit) { app.navigate(Screen.Apps) }
        return
    }
    var imeRef by remember(current.localId) { mutableStateOf<ImeCaptureView?>(null) }

    // Minimize: back to the Apps screen. The session, the connection, the PC app and its virtual monitor all stay.
    val minimize = {
        imeRef?.hide()
        app.navigate(Screen.Apps)
    }
    BackHandler { minimize() }

    var trackpad by remember { mutableStateOf(false) }
    var barVisible by remember { mutableStateOf(false) }
    var activity_ticks by remember { mutableIntStateOf(0) }
    var showSettings by remember { mutableStateOf(false) }
    val keyLogic = remember(current.localId) { KeyboardLogic() }
    val status by current.status.collectAsState()
    val title by current.title.collectAsState()
    val track by current.videoTrack.collectAsState()
    val videoSize by current.videoSize.collectAsState()

    // Auto-hide the control bar.
    LaunchedEffect(barVisible, activity_ticks, showSettings) {
        if (barVisible && !showSettings) {
            delay(4000)
            barVisible = false
        }
    }

    var overlayRef by remember(current.localId) { mutableStateOf<InputOverlayView?>(null) }

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
            Modifier.fillMaxSize().padding(
                start = with(density) { safeL.toDp() },
                end = with(density) { safeR.toDp() },
                top = with(density) { safeT.toDp() },
                // the IME insets already include the navigation bar
                bottom = with(density) { maxOf(safeB, reservePx).toDp() },
            )
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

            // Aspect-fit by sizing the renderer itself: a SurfaceViewRenderer given exact bounds
            // stretches the picture to them, which would not match VideoMapper's letterbox maths
            // whenever the video (a monitor's work area, say) differs a little from the screen.
            val (vw, vh) = videoSize
            AndroidView(
                factory = { renderer },
                modifier = if (vw > 0 && vh > 0) Modifier.align(Alignment.Center).aspectRatio(vw.toFloat() / vh) else Modifier.fillMaxSize(),
            )

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
                Button(onClick = { conn.close(current); app.navigate(Screen.Apps) }) { Text("Close") }
            }
            SessionStatus.Live -> {}
        }

        // Control bar (hidden by default, so the remote app has the whole screen): Minimize, Close, Keyboard,
        // trackpad, zoom reset, settings. A small pill in the top strip (the black safe-area margin above the video,
        // so it covers no part of the remote app) opens it by tap or swipe down.
        if (barVisible) {
            Row(
                Modifier.fillMaxWidth().padding(top = with(density) { safeT.toDp() }, start = with(density) { safeL.toDp() }, end = with(density) { safeR.toDp() })
                    .background(Color(0xE61E1E2E)).padding(horizontal = 4.dp, vertical = 2.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                BarButton(Icons.Default.Minimize, "Minimize (keeps the app running)", "Minimize") { minimize() }
                Text(
                    title, color = Color.White, maxLines = 1, overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f).padding(horizontal = 8.dp),
                    style = MaterialTheme.typography.labelLarge,
                )
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
                BarButton(Icons.Default.Close, "Close (ends the session and removes the virtual monitor)", "Close", tint = Color(0xFFF38BA8)) {
                    imeRef?.hide()
                    conn.close(current)
                    app.navigate(Screen.Apps)
                }
            }
        } else {
            SessionHandle(
                modifier = Modifier.align(Alignment.TopCenter).padding(start = with(density) { safeL.toDp() }, end = with(density) { safeR.toDp() }),
                stripHeight = with(density) { safeT.toDp() },
                onOpen = { barVisible = true },
            )
        }

        // Extra keys, docked exactly on top of the phone keyboard (the offset follows the IME
        // animation because it is read from the IME insets every frame).
        if (imeVisible) {
            ExtraKeysRow(
                logic = keyLogic,
                onKeys = { sendKeys(it) },
                modifier = Modifier.align(Alignment.BottomCenter).windowInsetsPadding(WindowInsets.ime)
                    .padding(start = with(density) { safeL.toDp() }, end = with(density) { safeR.toDp() }),
            )
        }
    }

    if (showSettings) {
        SettingsSheet(app, conn, current, activity?.windowPixels() ?: areaPx, onDismiss = { showSettings = false })
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SettingsSheet(app: PenlinkApp, conn: HostConnection, session: RemoteSession, screen: Pair<Int, Int>, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope()
    val prefs by session.prefs.collectAsState()
    val (sw, sh) = screen
    var sharp by remember { mutableStateOf(prefs.sharpText) }
    val steps = StreamPrefs.steps(sharp, sw, sh)
    var step by remember { mutableStateOf(steps.indexOf(StreamPrefs.snap(prefs.uiScale, steps)).toFloat()) }

    fun apply(p: StreamPrefs) {
        session.setPrefs(p)
        scope.launch { app.store.setStreamPrefs(conn.host.address, p) }
    }

    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(Modifier.padding(horizontal = 24.dp, vertical = 8.dp).padding(bottom = 24.dp)) {
            val uiPercent = steps[step.roundToInt().coerceIn(0, steps.size - 1)]
            Text("UI size: $uiPercent%", style = MaterialTheme.typography.titleMedium)
            Text(
                if (sharp) "Windows scales text and buttons. Windows allows at most ${steps.last()}% on this screen."
                else "The PC draws at lower resolution and this phone enlarges it. Up to ${steps.last()}% on this screen.",
                style = MaterialTheme.typography.bodySmall,
            )
            Slider(
                value = step,
                onValueChange = { step = it },
                valueRange = 0f..(steps.size - 1).toFloat(),
                steps = (steps.size - 2).coerceAtLeast(0),
                onValueChangeFinished = { apply(StreamPrefs(sharp, uiPercent)) },
            )
            Row(Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.CenterVertically) {
                Column(Modifier.weight(1f)) {
                    Text("Sharp text", style = MaterialTheme.typography.titleMedium)
                    Text(
                        if (sharp) "Full phone resolution: crisp text, but the UI cannot get as big and more pixels are sent."
                        else "Off: fewer pixels, faster, text is a little soft when enlarged.",
                        style = MaterialTheme.typography.bodySmall,
                    )
                }
                Switch(checked = sharp, onCheckedChange = { on ->
                    sharp = on
                    val p = StreamPrefs(on, uiPercent).fitted(sw, sh)
                    val newSteps = StreamPrefs.steps(on, sw, sh)
                    step = newSteps.indexOf(p.uiScale).toFloat()
                    apply(p)
                })
            }

            val win by session.windowSize.collectAsState()
            val size by session.requestedSize.collectAsState()
            val w = win
            Text("Showing", style = MaterialTheme.typography.titleMedium, modifier = Modifier.padding(top = 16.dp))
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
                        Text("Windows used the nearest DPI step it allows ($dpi%, asked $uiPercent%).", style = MaterialTheme.typography.bodySmall)
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
            Text("Phone area: ${size.first} x ${size.second} px", style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(top = 4.dp))
        }
    }
}

@Composable
private fun BarButton(icon: androidx.compose.ui.graphics.vector.ImageVector, description: String, label: String, tint: Color = Color.White, onClick: () -> Unit) {
    Row(
        Modifier.clip(RoundedCornerShape(8.dp)).clickable(onClick = onClick).padding(horizontal = 8.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, contentDescription = description, tint = tint, modifier = Modifier.size(20.dp))
        Text(label, color = tint, modifier = Modifier.padding(start = 4.dp), style = MaterialTheme.typography.labelLarge)
    }
}

/**
 * The always-visible way to the control bar: a thin pill centred in the top strip. The strip (status bar / cutout
 * height) is outside the video, so tapping it never reaches the remote app; when a phone has no such strip the pill is
 * only a few dp tall at the very top. Tap or swipe down opens the bar.
 */
@Composable
private fun SessionHandle(modifier: Modifier, stripHeight: androidx.compose.ui.unit.Dp, onOpen: () -> Unit) {
    val h = stripHeight.coerceIn(14.dp, 36.dp)
    Box(
        modifier
            .width(140.dp).height(h)
            .pointerInput(Unit) { detectTapGestures { onOpen() } }
            .pointerInput(Unit) { detectVerticalDragGestures { _, dy -> if (dy > 4f) onOpen() } },
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.clip(RoundedCornerShape(50)).background(Color(0x99FFFFFF)).size(width = 56.dp, height = 6.dp))
    }
}
