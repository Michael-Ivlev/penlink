package dev.penlink

import android.app.Activity
import android.content.Intent
import android.content.pm.ApplicationInfo
import android.os.Bundle
import android.widget.Toast
import androidx.lifecycle.lifecycleScope
import dev.penlink.protocol.AppEntry
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.ui.platform.LocalContext
import dev.penlink.ui.AppsScreen
import dev.penlink.ui.HostsScreen
import dev.penlink.ui.SessionScreen

class MainActivity : ComponentActivity() {
    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        val app = application as PenlinkApp
        setContent {
            MaterialTheme(colorScheme = darkColorScheme()) {
                Surface(color = MaterialTheme.colorScheme.background) {
                    Root(app)
                }
            }
        }
        handleDebugIntent(intent)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleDebugIntent(intent)
    }

    /**
     * Debug builds only: `adb shell am start -n dev.penlink/.MainActivity --es openAppId "<appId>"`
     * connects to the first saved PC and opens that raw appId (for example an `exe:` id for a
     * test window that is not in the Start Menu).
     */
    private fun handleDebugIntent(intent: Intent?) {
        if (applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE == 0) return
        val appId = intent?.getStringExtra("openAppId")?.takeIf { it.isNotBlank() } ?: return
        intent.removeExtra("openAppId")
        val app = application as PenlinkApp
        lifecycleScope.launch {
            val host = app.store.hosts.first().firstOrNull()
            if (host == null) {
                Toast.makeText(this@MainActivity, "openAppId: no saved PC", Toast.LENGTH_LONG).show()
                return@launch
            }
            val conn = app.connect(host)
            val scale = app.store.defaultScale.first()
            val (w, h) = windowPixels()
            val s = conn.open(AppEntry(appId, "debug app"), w, h, scale)
            app.navigate(Screen.Session(s.localId))
        }
    }
}

/** Full window size in pixels (system bars included; the session screen is immersive). */
fun Activity.windowPixels(): Pair<Int, Int> {
    val b = windowManager.currentWindowMetrics.bounds
    return b.width() to b.height()
}

@Composable
private fun Root(app: PenlinkApp) {
    val screen by app.screen.collectAsState()
    val conn by app.connection.collectAsState()
    val ctx = LocalContext.current
    LaunchedEffect(conn) {
        conn?.notices?.collect { Toast.makeText(ctx, it, Toast.LENGTH_LONG).show() }
    }
    when (val s = screen) {
        Screen.Hosts -> HostsScreen(app)
        Screen.Apps -> if (conn != null) AppsScreen(app, conn!!) else HostsScreen(app)
        is Screen.Session -> if (conn != null) SessionScreen(app, conn!!, s.localId) else HostsScreen(app)
    }
}
