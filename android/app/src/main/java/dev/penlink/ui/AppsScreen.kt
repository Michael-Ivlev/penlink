package dev.penlink.ui

import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.compose.foundation.Image
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.background
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.filled.Close
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.TextButton
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import dev.penlink.rtc.RemoteSession
import dev.penlink.rtc.SessionStatus
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import dev.penlink.PenlinkApp
import dev.penlink.Screen
import dev.penlink.data.StreamPrefs
import dev.penlink.protocol.AppEntry
import dev.penlink.rtc.HostConnection
import dev.penlink.windowPixels

@Composable
fun AppsScreen(app: PenlinkApp, conn: HostConnection) {
    val activity = LocalActivity.current
    val sessions by conn.sessions.collectAsState()
    val prefs by app.store.streamPrefs(conn.host.address).collectAsState(initial = StreamPrefs())
    var apps by remember(conn) { mutableStateOf<List<AppEntry>?>(null) }
    var error by remember(conn) { mutableStateOf<String?>(null) }
    var reload by remember { mutableStateOf(0) }
    var query by remember { mutableStateOf("") }

    LaunchedEffect(conn, reload) {
        error = null
        apps = null
        try {
            apps = app.api.apps(conn.host.address, conn.host.token)
        } catch (e: Exception) {
            error = e.message ?: ("Could not load apps (" + e.javaClass.simpleName + ")")
        }
    }

    var confirmLeave by remember { mutableStateOf(false) }
    // Back with apps open keeps them (and the connection) and just sends Penlink to the background;
    // leaving the PC for good is the arrow in the top bar.
    BackHandler {
        if (sessions.isNotEmpty()) {
            activity?.moveTaskToBack(true)
        } else {
            app.disconnect()
            app.navigate(Screen.Hosts)
        }
    }

    if (confirmLeave) {
        AlertDialog(
            onDismissRequest = { confirmLeave = false },
            title = { Text("Leave ${conn.host.name}?") },
            text = { Text("${sessions.size} open app(s) will be closed and their virtual monitors removed. The apps stay open on the PC.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmLeave = false
                    app.disconnect()
                    app.navigate(Screen.Hosts)
                }) { Text("Leave") }
            },
            dismissButton = { TextButton(onClick = { confirmLeave = false }) { Text("Stay") } },
        )
    }

    Column(Modifier.fillMaxSize().safeDrawingPadding().padding(horizontal = 12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = {
                if (sessions.isNotEmpty()) {
                    confirmLeave = true
                } else {
                    app.disconnect()
                    app.navigate(Screen.Hosts)
                }
            }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back to PCs") }
            Column(Modifier.weight(1f)) {
                Text(conn.host.name, style = MaterialTheme.typography.titleLarge)
                Text(conn.host.address, style = MaterialTheme.typography.bodySmall)
            }
        }
        OutlinedTextField(
            value = query,
            onValueChange = { query = it },
            label = { Text("Search apps") },
            singleLine = true,
            modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp),
        )
        val list = apps
        when {
            error != null -> Column(
                Modifier.fillMaxWidth().padding(24.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                Text(error!!, color = MaterialTheme.colorScheme.error)
                Spacer(Modifier.height(8.dp))
                Button(onClick = { reload++ }) { Text("Retry") }
            }
            list == null -> Text("Loading apps...", Modifier.padding(24.dp))
            else -> {
                val shown = remember(list, query) {
                    if (query.isBlank()) list else list.filter { it.name.contains(query, ignoreCase = true) }
                }
                LazyVerticalGrid(
                    columns = GridCells.Adaptive(96.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    modifier = Modifier.fillMaxSize(),
                ) {
                    if (sessions.isNotEmpty()) {
                        item(span = { GridItemSpan(maxLineSpan) }, key = "open-now") {
                            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                                Text("Open now", style = MaterialTheme.typography.titleMedium)
                                for (s in sessions) OpenNowRow(app, conn, s)
                                Spacer(Modifier.height(4.dp))
                                Text("All apps", style = MaterialTheme.typography.titleMedium)
                            }
                        }
                    }
                    items(shown, key = { it.id }) { a ->
                        AppTile(app, conn, a) {
                            // An app that is already open is resumed, not started a second time.
                            val running = sessions.lastOrNull { it.appId == a.id && it.status.value !is SessionStatus.Closed }
                            if (running != null) {
                                app.navigate(Screen.Session(running.localId))
                            } else {
                                val (w, h) = activity?.windowPixels() ?: (1080 to 2340)
                                val s = conn.open(a, w, h, prefs)
                                app.navigate(Screen.Session(s.localId))
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun AppTile(app: PenlinkApp, conn: HostConnection, entry: AppEntry, onClick: () -> Unit) {
    val bmp by produceState(app.cachedIcon(entry.id), entry.id) {
        if (value == null) {
            app.api.icon(conn.host.address, conn.host.token, entry.id)?.let {
                app.cacheIcon(entry.id, it)
                value = it
            }
        }
    }
    Column(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(8.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Box(Modifier.size(56.dp), contentAlignment = Alignment.Center) {
            val b = bmp
            if (b != null) {
                Image(b.asImageBitmap(), contentDescription = null, modifier = Modifier.size(56.dp))
            } else {
                Text(entry.name.take(1).uppercase(), style = MaterialTheme.typography.headlineSmall)
            }
        }
        Text(
            entry.name,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            textAlign = TextAlign.Center,
            style = MaterialTheme.typography.labelMedium,
        )
    }
}

/** Plain-language state of an open session, for the "Open now" row. */
fun sessionStateLabel(s: SessionStatus): String = when (s) {
    SessionStatus.Live -> "Connected"
    SessionStatus.Opening -> "Opening..."
    SessionStatus.Connecting -> "Connecting..."
    SessionStatus.Reconnecting -> "Reconnecting..."
    is SessionStatus.Closed -> "Closed"
}

/** One running session: icon, title and live state; tap resumes it at once, X ends it. */
@Composable
private fun OpenNowRow(app: PenlinkApp, conn: HostConnection, s: RemoteSession) {
    val title by s.title.collectAsState()
    val status by s.status.collectAsState()
    val bmp by produceState(app.cachedIcon(s.appId), s.appId) {
        if (value == null) {
            app.api.icon(conn.host.address, conn.host.token, s.appId)?.let {
                app.cacheIcon(s.appId, it)
                value = it
            }
        }
    }
    val live = status == SessionStatus.Live
    Row(
        Modifier.fillMaxWidth().clip(RoundedCornerShape(12.dp))
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .clickable { app.navigate(Screen.Session(s.localId)) }
            .padding(start = 12.dp, top = 6.dp, bottom = 6.dp, end = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(40.dp), contentAlignment = Alignment.Center) {
            val b = bmp
            if (b != null) Image(b.asImageBitmap(), contentDescription = null, modifier = Modifier.size(40.dp))
            else Text(title.take(1).uppercase(), style = MaterialTheme.typography.titleLarge)
        }
        Column(Modifier.weight(1f).padding(horizontal = 12.dp)) {
            Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.titleSmall)
            Row(verticalAlignment = Alignment.CenterVertically) {
                if (live) {
                    Box(Modifier.size(8.dp).clip(CircleShape).background(Color(0xFFA6E3A1)))
                } else if (status !is SessionStatus.Closed) {
                    CircularProgressIndicator(Modifier.size(10.dp), strokeWidth = 1.5.dp)
                }
                Text(sessionStateLabel(status), Modifier.padding(start = 6.dp), style = MaterialTheme.typography.bodySmall)
            }
        }
        TrafficLight(TrafficKind.Close, "Close $title") { conn.close(s) }
    }
}