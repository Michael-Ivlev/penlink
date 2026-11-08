package dev.penlink.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import android.view.WindowManager
import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.material.icons.filled.Minimize
import androidx.compose.material.icons.filled.Close
import androidx.compose.material.icons.filled.Keyboard
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import dev.penlink.PenlinkApp
import dev.penlink.Screen
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.displayCutout
import androidx.compose.foundation.layout.systemBarsIgnoringVisibility
import androidx.compose.ui.platform.LocalLayoutDirection
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.snapshotFlow
import androidx.compose.foundation.layout.offset
import androidx.compose.ui.unit.IntOffset
import dev.penlink.input.KeyboardPan
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import dev.penlink.input.ImeMapping
import dev.penlink.input.ImePiece
import dev.penlink.input.KeyDef
import dev.penlink.input.ImeCaptureView
import dev.penlink.input.InputOverlayView
import dev.penlink.input.KeyAction
import dev.penlink.input.KeyboardLogic
import dev.penlink.protocol.InputMsg
import dev.penlink.rtc.HostConnection
import dev.penlink.windowPixels
import kotlinx.coroutines.delay
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
    val imeInsets = WindowInsets.ime
    val imeVisible by remember { derivedStateOf { imeInsets.getBottom(density) > 0 } }
    val rowPx = with(density) { ExtraKeysHeight.roundToPx() }

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

    // Keyboard behaviour (saved per host): by default the PC window keeps its size and the picture slides up above
    // the keyboard; with "Resize app above keyboard" the PC window is resized to the area above it instead.
    val sessionPrefs by current.prefs.collectAsState()
    val resizeAbove = sessionPrefs.resizeAboveKeyboard
    val reservePx = if (resizeAbove && imeVisible) imeInsets.getBottom(density) + rowPx else 0

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

    // Drive the keyboard pan from the IME inset, which Compose updates on every frame of the keyboard animation
    // (WindowInsetsAnimation), so the picture moves with the keyboard. This only translates the video view; nothing is
    // measured or laid out and the SurfaceView is never resized.
    val safeBNow by rememberUpdatedState(safeB)
    val panMarginPx = with(density) { 24.dp.toPx() }
    LaunchedEffect(overlayRef, resizeAbove, rowPx) {
        val o = overlayRef ?: return@LaunchedEffect
        if (resizeAbove) {
            o.setKeyboardCovered(0f, panMarginPx)
            return@LaunchedEffect
        }
        snapshotFlow { KeyboardPan.covered(imeInsets.getBottom(density), rowPx, safeBNow) }
            .collect { o.setKeyboardCovered(it, panMarginPx) }
    }

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

        LatencyOverlay(current, app.rtc)

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
        SessionStatusOverlay(
            status = status,
            link = linkState,
            hostName = conn.host.name,
            title = title,
            onRetry = { conn.retryNow() },
            onCloseTab = { conn.close(current); app.navigate(Screen.Apps) },
            modifier = Modifier.align(Alignment.Center),
        )

        // Control bar (hidden by default, so the remote app has the whole screen): Minimize, Close, Keyboard,
        // trackpad, zoom reset, settings. A small pill in the top strip (the black safe-area margin above the video,
        // so it covers no part of the remote app) opens it by tap or swipe down.
        AnimatedVisibility(
            visible = barVisible,
            modifier = Modifier.align(Alignment.TopCenter),
            enter = slideInVertically(tween(240)) { -it } + fadeIn(tween(200)),
            exit = slideOutVertically(tween(200)) { -it } + fadeOut(tween(160)),
        ) {
            SessionControlBar(
                title = title,
                trackpad = trackpad,
                keyboardVisible = imeVisible,
                onClose = {
                    imeRef?.hide()
                    conn.close(current)
                    app.navigate(Screen.Apps)
                },
                onMinimize = { minimize() },
                onToggleTrackpad = { trackpad = !trackpad; activity_ticks++ },
                onResetZoom = { overlayRef?.resetZoom(); activity_ticks++ },
                onSettings = { showSettings = true },
                onToggleKeyboard = {
                    if (imeVisible) imeRef?.hide() else imeRef?.show()
                    activity_ticks++
                },
                modifier = Modifier.padding(
                    top = with(density) { safeT.toDp() } + 4.dp,
                    start = with(density) { safeL.toDp() } + 8.dp,
                    end = with(density) { safeR.toDp() } + 8.dp,
                ),
            )
        }
        AnimatedVisibility(
            visible = !barVisible,
            modifier = Modifier.align(Alignment.TopCenter),
            enter = fadeIn(tween(200)),
            exit = fadeOut(tween(120)),
        ) {
            SessionHandle(
                modifier = Modifier.padding(start = with(density) { safeL.toDp() }, end = with(density) { safeR.toDp() }),
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
                    // Slides up from behind the keyboard during the first row-height of its animation.
                    .offset { IntOffset(0, rowPx - KeyboardPan.rowShown(imeInsets.getBottom(density), rowPx).toInt()) }
                    .padding(start = with(density) { safeL.toDp() }, end = with(density) { safeR.toDp() }),
            )
        }
    }

    if (showSettings) {
        SettingsSheet(app, conn, current, activity?.windowPixels() ?: areaPx, onDismiss = { showSettings = false })
    }
}
