package dev.penlink

import android.app.Activity
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
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
    when (val s = screen) {
        Screen.Hosts -> HostsScreen(app)
        Screen.Apps -> if (conn != null) AppsScreen(app, conn!!) else HostsScreen(app)
        is Screen.Session -> if (conn != null) SessionScreen(app, conn!!, s.localId) else HostsScreen(app)
    }
}
