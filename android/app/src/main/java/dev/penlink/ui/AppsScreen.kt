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
import dev.penlink.protocol.AppEntry
import dev.penlink.rtc.HostConnection
import dev.penlink.windowPixels

@Composable
fun AppsScreen(app: PenlinkApp, conn: HostConnection) {
    val activity = LocalActivity.current
    val sessions by conn.sessions.collectAsState()
    val scale by app.store.defaultScale.collectAsState(initial = 2.0)
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

    BackHandler {
        app.disconnect()
        app.navigate(Screen.Hosts)
    }

    Column(Modifier.fillMaxSize().safeDrawingPadding().padding(horizontal = 12.dp)) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            IconButton(onClick = {
                app.disconnect()
                app.navigate(Screen.Hosts)
            }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") }
            Column(Modifier.weight(1f)) {
                Text(conn.host.name, style = MaterialTheme.typography.titleLarge)
                Text(conn.host.address, style = MaterialTheme.typography.bodySmall)
            }
            if (sessions.isNotEmpty()) {
                Button(onClick = { app.navigate(Screen.Session(sessions.last().localId)) }) {
                    Text("Open apps (${sessions.size})")
                }
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
                    items(shown, key = { it.id }) { a ->
                        AppTile(app, conn, a) {
                            val (w, h) = activity?.windowPixels() ?: (1080 to 2340)
                            val s = conn.open(a, w, h, scale)
                            app.navigate(Screen.Session(s.localId))
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
