package dev.penlink

import androidx.compose.runtime.getValue
import android.app.Activity
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Bundle
import android.widget.Toast
import androidx.lifecycle.lifecycleScope
import dev.penlink.data.Quality
import dev.penlink.data.SavedHost
import dev.penlink.notify.NotificationForwardService
import dev.penlink.notify.PcNotifier
import dev.penlink.protocol.AppEntry
import dev.penlink.rtc.SessionStatus
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.SystemBarStyle
import androidx.activity.enableEdgeToEdge
import androidx.core.splashscreen.SplashScreen.Companion.installSplashScreen
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.runtime.DisposableEffect
import dev.penlink.ui.PairScreen
import dev.penlink.ui.theme.PenlinkTheme
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.AnimatedContentTransitionScope
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.ContentTransform
import androidx.compose.animation.SizeTransform
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
import androidx.compose.animation.togetherWith
import dev.penlink.rtc.HostConnection
import androidx.core.view.WindowInsetsCompat
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.ui.platform.LocalContext
import dev.penlink.ui.AppsScreen
import dev.penlink.ui.HostsScreen
import dev.penlink.ui.NotificationsScreen
import dev.penlink.ui.SessionScreen

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        installSplashScreen()
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val app = application as PenlinkApp
        setContent {
            val dark = isSystemInDarkTheme()
            // Transparent system bars; the icon colour follows the theme (the activity handles uiMode changes itself).
            DisposableEffect(dark) {
                enableEdgeToEdge(
                    statusBarStyle = SystemBarStyle.auto(android.graphics.Color.TRANSPARENT, android.graphics.Color.TRANSPARENT) { dark },
                    navigationBarStyle = SystemBarStyle.auto(android.graphics.Color.TRANSPARENT, android.graphics.Color.TRANSPARENT) { dark },
                )
                onDispose { }
            }
            PenlinkTheme(darkTheme = dark) {
                Surface(color = MaterialTheme.colorScheme.background) {
                    Root(app)
                }
            }
        }
        startService(Intent(this, TaskWatchService::class.java))
        // Forwarding stays on across app launches, reboots and process deaths.
        lifecycleScope.launch {
            if (app.store.notifHosts.first().isNotEmpty()) NotificationForwardService.start(this@MainActivity)
        }
        handleDebugIntent(intent)
        handleNotificationTap(intent, coldStart = savedInstanceState == null)
    }

    /**
     * The activity is going away for good (finish, or the task was swiped away and the system destroys it): tell the PC
     * to close every session now so its virtual monitors disappear at once. `shutdown` queues the close messages and
     * the WebSocket goodbye on OkHttp's own threads, which run on after this returns.
     */
    override fun onDestroy() {
        if (isFinishing) (application as PenlinkApp).disconnect()
        super.onDestroy()
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleDebugIntent(intent)
        handleNotificationTap(intent, coldStart = false)
    }

    /**
     * A forwarded PC notification was tapped: connect to that PC and show the app the notification came from
     * full screen (resuming it when it is already open); when the app cannot be mapped, show the PC's Apps screen.
     */
    private fun handleNotificationTap(intent: Intent?, coldStart: Boolean) {
        if (intent?.action != PcNotifier.ACTION_OPEN) return
        val address = intent.getStringExtra(PcNotifier.EXTRA_HOST) ?: return
        val appId = intent.getStringExtra(PcNotifier.EXTRA_APP_ID)?.takeIf { it.isNotBlank() }
        val appName = intent.getStringExtra(PcNotifier.EXTRA_APP_NAME) ?: ""
        intent.action = null
        val app = application as PenlinkApp
        lifecycleScope.launch {
            val host = app.store.hosts.first().firstOrNull { it.address == address } ?: return@launch
            val conn = app.connect(host)
            if (appId == null) {
                app.navigate(Screen.Apps)
                return@launch
            }
            // The window insets (safe area) arrive with the first layout; wait for them on a cold start.
            if (coldStart) kotlinx.coroutines.delay(300)
            val running = conn.sessions.value.lastOrNull { it.appId == appId && it.status.value !is SessionStatus.Closed }
            val s = running ?: run {
                val prefs = app.store.streamPrefs(host.address).first()
                val (w, h) = windowPixels()
                conn.open(AppEntry(appId, appName), w, h, prefs)
            }
            app.navigate(Screen.Session(s.localId))
        }
    }

    /**
     * Debug builds only: `adb shell am start -n dev.penlink/.MainActivity --es openAppId "<appId>"`
     * connects to the first saved PC (or the one whose address equals `--es host 10.0.2.2:47900`) and opens that raw appId (for example an `exe:` id for a
     * test window that is not in the Start Menu).
     */
    private fun handleDebugIntent(intent: Intent?) {
        // `--ez latency true|false` shows or hides the latency overlay (on by default in debug builds).
        if (intent?.hasExtra("latency") == true) (application as PenlinkApp).rtc.setLatencyOverlay(intent.getBooleanExtra("latency", true))
        if (applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE == 0) return
        val appId = intent?.getStringExtra("openAppId")?.takeIf { it.isNotBlank() } ?: return
        intent.removeExtra("openAppId")
        val app = application as PenlinkApp
        lifecycleScope.launch {
            val want = intent.getStringExtra("host")
            // `--es pairCode 123456` (with --es host) pairs a lab host first, so scripted runs need no typing.
            val code = intent.getStringExtra("pairCode")
            if (want != null && code != null) {
                runCatching { app.api.pair(want, code, "debug") }.onSuccess { app.store.upsert(SavedHost(want, it.hostName, it.token)) }
            }
            val hosts = app.store.hosts.first()
            val host = if (want != null) hosts.firstOrNull { it.address == want } else hosts.firstOrNull()
            if (host == null) {
                Toast.makeText(this@MainActivity, "openAppId: no saved PC", Toast.LENGTH_LONG).show()
                return@launch
            }
            val conn = app.connect(host)
            // Optional overrides for scripted tests: --es quality sharp (= "Sharp text" on) --ei uiScale 250
            var prefs = app.store.streamPrefs(host.address).first()
            val q = intent.getStringExtra("quality")
            val ui = if (intent.hasExtra("uiScale")) intent.getIntExtra("uiScale", prefs.uiScale) else null
            val (w, h) = windowPixels()
            if (q != null || ui != null) {
                prefs = prefs.copy(sharpText = if (q != null) Quality.fromWire(q) == Quality.Sharp else prefs.sharpText, uiScale = ui ?: prefs.uiScale).fitted(w, h)
                app.store.setStreamPrefs(host.address, prefs)
            }
            val s = conn.open(AppEntry(appId, "debug app"), w, h, prefs)
            app.navigate(Screen.Session(s.localId))
        }
    }
}

/**
 * The safe area in pixels: the window minus the display cutout and the system bars (as if they were shown,
 * so the size does not change while the session screen hides them). This is what the PC renders for.
 */
fun Activity.windowPixels(): Pair<Int, Int> {
    val b = windowManager.currentWindowMetrics.bounds
    val root = window.decorView.rootWindowInsets ?: return b.width() to b.height()
    val c = WindowInsetsCompat.toWindowInsetsCompat(root, window.decorView)
    val bars = c.getInsetsIgnoringVisibility(WindowInsetsCompat.Type.systemBars())
    val cut = c.getInsets(WindowInsetsCompat.Type.displayCutout())
    val l = maxOf(bars.left, cut.left)
    val r = maxOf(bars.right, cut.right)
    val t = maxOf(bars.top, cut.top)
    val bt = maxOf(bars.bottom, cut.bottom)
    return (b.width() - l - r).coerceAtLeast(1) to (b.height() - t - bt).coerceAtLeast(1)
}

/** What the root shows: the screen plus the connection it needs (each content keeps the connection it was composed with). */
private data class Dest(val screen: Screen, val conn: HostConnection?)

private fun Screen.depth(): Int = when (this) {
    Screen.Hosts -> 0
    Screen.Pair, Screen.Apps -> 1
    Screen.Notifications -> 2
    is Screen.Session -> 2
}

/**
 * Between list-like screens: shared-axis X (a short slide plus fade, direction from the depth). Into a session: the
 * previous screen fades out and the session scales/fades in. Out of a session: the remote picture is removed at once
 * (a SurfaceView cannot be faded reliably) and the Apps screen fades in.
 */
private fun AnimatedContentTransitionScope<Dest>.rootTransition(): ContentTransform {
    val from = initialState.screen
    val to = targetState.screen
    val spec = when {
        to is Screen.Session ->
            (fadeIn(tween(220, delayMillis = 90)) + scaleIn(tween(220, delayMillis = 90), initialScale = 0.94f)) togetherWith fadeOut(tween(90))
        from is Screen.Session -> fadeIn(tween(200)) togetherWith ExitTransition.None
        else -> {
            val forward = to.depth() >= from.depth() && !(to == Screen.Hosts)
            val dir = if (forward) 1 else -1
            (slideInHorizontally(tween(300)) { it / 10 * dir } + fadeIn(tween(210, delayMillis = 90))) togetherWith
                (slideOutHorizontally(tween(300)) { -it / 10 * dir } + fadeOut(tween(90)))
        }
    }
    return spec.using(SizeTransform(clip = false))
}

@Composable
private fun Root(app: PenlinkApp) {
    val screen by app.screen.collectAsState()
    val conn by app.connection.collectAsState()
    val ctx = LocalContext.current
    LaunchedEffect(conn) {
        conn?.notices?.collect { Toast.makeText(ctx, it, Toast.LENGTH_LONG).show() }
    }
    val needsConn = screen is Screen.Apps || screen is Screen.Session || screen is Screen.Notifications
    val shown = if (needsConn && conn == null) Screen.Hosts else screen
    AnimatedContent(
        targetState = Dest(shown, conn),
        transitionSpec = { rootTransition() },
        contentKey = { it.screen::class },
        label = "root",
    ) { d ->
        val c = d.conn
        when (val s = d.screen) {
            Screen.Hosts -> HostsScreen(app)
            Screen.Pair -> PairScreen(app)
            Screen.Apps -> if (c != null) AppsScreen(app, c) else HostsScreen(app)
            Screen.Notifications -> if (c != null) NotificationsScreen(app, c.host) else HostsScreen(app)
            is Screen.Session -> if (c != null) SessionScreen(app, c, s.localId) else HostsScreen(app)
        }
    }
}
