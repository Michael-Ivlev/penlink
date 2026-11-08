package dev.penlink.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import android.graphics.Bitmap
import androidx.activity.compose.BackHandler
import androidx.activity.compose.LocalActivity
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.Image
import androidx.compose.foundation.LocalIndication
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.ExitToApp
import androidx.compose.material.icons.filled.Apps
import androidx.compose.material.icons.filled.Clear
import androidx.compose.material.icons.filled.CloudOff
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material.icons.filled.Search
import androidx.compose.material.icons.filled.SearchOff
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ElevatedCard
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LargeTopAppBar
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SearchBar
import androidx.compose.material3.SearchBarDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import dev.penlink.PenlinkApp
import dev.penlink.Screen
import dev.penlink.data.StreamPrefs
import dev.penlink.protocol.AppEntry
import dev.penlink.rtc.HostConnection
import dev.penlink.rtc.RemoteSession
import dev.penlink.rtc.SessionStatus
import dev.penlink.ui.theme.Spacing
import dev.penlink.windowPixels

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AppsScreen(app: PenlinkApp, conn: HostConnection) {
    val activity = LocalActivity.current
    val haptics = rememberHaptics()
    val sessions by conn.sessions.collectAsState()
    val prefs by app.store.streamPrefs(conn.host.address).collectAsState(initial = StreamPrefs())
    var apps by remember(conn) { mutableStateOf<List<AppEntry>?>(null) }
    var error by remember(conn) { mutableStateOf<String?>(null) }
    var reload by remember { mutableIntStateOf(0) }
    var query by remember { mutableStateOf("") }
    var searching by remember { mutableStateOf(false) }
    val scroll = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()

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
    val leave = {
        if (sessions.isNotEmpty()) {
            confirmLeave = true
        } else {
            app.disconnect()
            app.navigate(Screen.Hosts)
        }
    }
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
            icon = { Icon(Icons.AutoMirrored.Filled.ExitToApp, contentDescription = null) },
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

    // An app that is already open is resumed, not started a second time.
    val openEntry: (AppEntry) -> Unit = { a ->
        haptics.tap()
        val running = sessions.lastOrNull { it.appId == a.id && it.status.value !is SessionStatus.Closed }
        if (running != null) {
            app.navigate(Screen.Session(running.localId))
        } else {
            val (w, h) = activity?.windowPixels() ?: (1080 to 2340)
            val s = conn.open(a, w, h, prefs)
            app.navigate(Screen.Session(s.localId))
        }
    }

    Scaffold(
        modifier = Modifier.nestedScroll(scroll.nestedScrollConnection),
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            LargeTopAppBar(
                title = { Text(conn.host.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    IconButton(onClick = leave) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back to PCs") }
                },
                actions = {
                    IconButton(onClick = { haptics.tap(); reload++ }) { Icon(Icons.Default.Refresh, contentDescription = "Reload apps") }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.background,
                    scrolledContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                ),
                scrollBehavior = scroll,
            )
        },
    ) { padding ->
        val list = apps
        val shown = remember(list, query) {
            if (list == null) emptyList() else if (query.isBlank()) list else list.filter { it.name.contains(query, ignoreCase = true) }
        }
        val contentPadding = PaddingValues(
            start = padding.calculateStartPadding(LayoutDirection.Ltr) + Spacing.lg,
            end = padding.calculateEndPadding(LayoutDirection.Ltr) + Spacing.lg,
            top = padding.calculateTopPadding(),
            bottom = padding.calculateBottomPadding() + Spacing.lg,
        )
        LazyVerticalGrid(
            columns = GridCells.Adaptive(92.dp),
            contentPadding = contentPadding,
            verticalArrangement = Arrangement.spacedBy(Spacing.xs),
            horizontalArrangement = Arrangement.spacedBy(Spacing.xs),
            modifier = Modifier.fillMaxSize(),
        ) {
            item(span = { GridItemSpan(maxLineSpan) }, key = "address") {
                Text(
                    conn.host.address,
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.padding(bottom = Spacing.sm),
                )
            }
            item(span = { GridItemSpan(maxLineSpan) }, key = "search") {
                AppSearchBar(
                    query = query,
                    onQueryChange = { query = it },
                    expanded = searching,
                    onExpandedChange = { searching = it },
                    results = shown,
                    app = app,
                    conn = conn,
                    onOpen = { searching = false; openEntry(it) },
                )
            }
            if (sessions.isNotEmpty()) {
                item(span = { GridItemSpan(maxLineSpan) }, key = "open-now") {
                    Column(Modifier.padding(top = Spacing.lg)) {
                        SectionTitle("Open now")
                        LazyRow(
                            horizontalArrangement = Arrangement.spacedBy(Spacing.md),
                            modifier = Modifier.fillMaxWidth(),
                        ) {
                            items(sessions, key = { it.localId }) { s ->
                                OpenNowCard(app, conn, s, Modifier.animateItem())
                            }
                        }
                    }
                }
            }
            item(span = { GridItemSpan(maxLineSpan) }, key = "all-apps") {
                SectionTitle(if (list != null && list.isNotEmpty()) "All apps  ·  ${list.size}" else "All apps", Modifier.padding(top = Spacing.lg))
            }
            when {
                error != null -> item(span = { GridItemSpan(maxLineSpan) }, key = "error") {
                    ErrorPanel(error!!, onRetry = { reload++ }, onBack = leave)
                }
                list == null -> items(12, key = { "skeleton-$it" }) { SkeletonTile() }
                list.isEmpty() -> item(span = { GridItemSpan(maxLineSpan) }, key = "empty") {
                    MessagePanel(
                        icon = Icons.Default.Apps,
                        title = "No apps found",
                        body = "penlink-host did not list any apps on this PC. Check its configuration, then reload.",
                    ) { FilledTonalButton(onClick = { reload++ }) { Text("Reload") } }
                }
                shown.isEmpty() -> item(span = { GridItemSpan(maxLineSpan) }, key = "no-match") {
                    MessagePanel(
                        icon = Icons.Default.SearchOff,
                        title = "No matching apps",
                        body = "Nothing on ${conn.host.name} matches \"$query\".",
                    ) { FilledTonalButton(onClick = { query = "" }) { Text("Clear search") } }
                }
                else -> items(shown, key = { it.id }) { a ->
                    AppTile(app, conn, a, Modifier.animateItem()) { openEntry(a) }
                }
            }
        }
    }
}

@Composable
private fun SectionTitle(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.titleMedium,
        color = MaterialTheme.colorScheme.onSurface,
        modifier = modifier.padding(bottom = Spacing.sm).semantics { },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AppSearchBar(
    query: String,
    onQueryChange: (String) -> Unit,
    expanded: Boolean,
    onExpandedChange: (Boolean) -> Unit,
    results: List<AppEntry>,
    app: PenlinkApp,
    conn: HostConnection,
    onOpen: (AppEntry) -> Unit,
) {
    SearchBar(
        inputField = {
            SearchBarDefaults.InputField(
                query = query,
                onQueryChange = onQueryChange,
                onSearch = { onExpandedChange(false) },
                expanded = expanded,
                onExpandedChange = onExpandedChange,
                placeholder = { Text("Search apps") },
                leadingIcon = { Icon(Icons.Default.Search, contentDescription = null) },
                trailingIcon = {
                    if (query.isNotEmpty()) {
                        IconButton(onClick = { onQueryChange("") }) { Icon(Icons.Default.Clear, contentDescription = "Clear search") }
                    }
                },
            )
        },
        expanded = expanded,
        onExpandedChange = onExpandedChange,
        // The status-bar inset only matters for the full-screen expanded state; the collapsed bar sits in the list.
        windowInsets = if (expanded) SearchBarDefaults.windowInsets else WindowInsets(0, 0, 0, 0),
        modifier = Modifier.fillMaxWidth(),
    ) {
        if (results.isEmpty()) {
            Text(
                "No matching apps",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(Spacing.xl),
            )
        } else {
            LazyColumn {
                items(results, key = { it.id }) { a ->
                    ListItem(
                        headlineContent = { Text(a.name, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                        leadingContent = { AppIcon(app, conn, a.id, a.name, 44.dp) },
                        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
                        modifier = Modifier.clickable { onOpen(a) },
                    )
                }
            }
        }
    }
}

/** The icon of an app, fetched once and cached; a coloured tile with the first letter until (or unless) it arrives. */
@Composable
private fun rememberAppIcon(app: PenlinkApp, conn: HostConnection, appId: String): Bitmap? {
    val bmp by produceState(app.cachedIcon(appId), appId) {
        if (value == null) {
            app.api.icon(conn.host.address, conn.host.token, appId)?.let {
                app.cacheIcon(appId, it)
                value = it
            }
        }
    }
    return bmp
}

@Composable
private fun AppIcon(app: PenlinkApp, conn: HostConnection, appId: String, name: String, size: Dp, modifier: Modifier = Modifier) {
    val bmp = rememberAppIcon(app, conn, appId)
    val cs = MaterialTheme.colorScheme
    val tones = listOf(
        cs.primaryContainer to cs.onPrimaryContainer,
        cs.secondaryContainer to cs.onSecondaryContainer,
        cs.tertiaryContainer to cs.onTertiaryContainer,
    )
    val (container, onContainer) = tones[(name.hashCode() and Int.MAX_VALUE) % tones.size]
    val shape = RoundedCornerShape(size * 0.3f)
    Box(
        modifier.size(size).clip(shape).background(if (bmp != null) cs.surfaceContainerHigh else container).clearAndSetSemantics { },
        contentAlignment = Alignment.Center,
    ) {
        if (bmp != null) {
            Image(bmp.asImageBitmap(), contentDescription = null, modifier = Modifier.size(size * 0.72f))
        } else {
            Text(name.trim().take(1).uppercase(), color = onContainer, style = MaterialTheme.typography.titleLarge)
        }
    }
}

@Composable
private fun AppTile(app: PenlinkApp, conn: HostConnection, entry: AppEntry, modifier: Modifier = Modifier, onClick: () -> Unit) {
    val source = remember { MutableInteractionSource() }
    val pressed by source.collectIsPressedAsState()
    val scale by animateFloatAsState(if (pressed) 0.93f else 1f, label = "tilePress")
    Column(
        modifier
            .fillMaxWidth()
            .graphicsLayer { scaleX = scale; scaleY = scale }
            .clip(MaterialTheme.shapes.large)
            .clickable(interactionSource = source, indication = LocalIndication.current, onClick = onClick)
            .semantics(mergeDescendants = true) { contentDescription = "Open ${entry.name}" }
            .padding(vertical = Spacing.sm, horizontal = Spacing.xs),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(Spacing.xs),
    ) {
        AppIcon(app, conn, entry.id, entry.name, 60.dp)
        Text(
            entry.name,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            textAlign = TextAlign.Center,
            style = MaterialTheme.typography.labelMedium,
            modifier = Modifier.fillMaxWidth(),
        )
    }
}

@Composable
private fun SkeletonTile() {
    Column(
        Modifier.fillMaxWidth().padding(vertical = Spacing.sm),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(Spacing.sm),
    ) {
        ShimmerBox(Modifier.size(60.dp), RoundedCornerShape(18.dp))
        ShimmerBox(Modifier.width(56.dp).height(10.dp), MaterialTheme.shapes.extraSmall)
    }
}

@Composable
private fun ErrorPanel(message: String, onRetry: () -> Unit, onBack: () -> Unit) {
    val unpaired = message.contains("Unauthorised", ignoreCase = true)
    MessagePanel(
        icon = Icons.Default.CloudOff,
        title = if (unpaired) "Pairing expired" else "Can't load apps",
        body = if (unpaired) "This PC no longer recognises the phone. Go back, forget the PC and pair it again."
        else "$message\nCheck that penlink-host is running and Tailscale is connected.",
        isError = true,
    ) {
        Row(horizontalArrangement = Arrangement.spacedBy(Spacing.sm)) {
            if (!unpaired) Button(onClick = onRetry) { Text("Retry") }
            FilledTonalButton(onClick = onBack) { Text("Back to PCs") }
        }
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

/** One running session as an elevated card: icon, title, live state chip; tap resumes it at once, the red button ends it. */
@Composable
private fun OpenNowCard(app: PenlinkApp, conn: HostConnection, s: RemoteSession, modifier: Modifier = Modifier) {
    val title by s.title.collectAsState()
    val status by s.status.collectAsState()
    val busy = status == SessionStatus.Opening || status == SessionStatus.Connecting || status == SessionStatus.Reconnecting
    ElevatedCard(
        onClick = { app.navigate(Screen.Session(s.localId)) },
        modifier = modifier.width(188.dp).semantics { contentDescription = "$title, ${sessionStateLabel(status)}. Tap to resume." },
        shape = MaterialTheme.shapes.large,
    ) {
        Column(Modifier.padding(start = Spacing.md, top = Spacing.sm, bottom = Spacing.md, end = Spacing.xs)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                AppIcon(app, conn, s.appId, title, 44.dp)
                Spacer(Modifier.weight(1f))
                TrafficLight(TrafficKind.Close, "Close $title") { conn.close(s) }
            }
            Text(
                title,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                style = MaterialTheme.typography.titleSmall,
                modifier = Modifier.padding(top = Spacing.xs, end = Spacing.sm),
            )
            StatusChip(sessionStateLabel(status), chipToneFor(status), busy, Modifier.padding(top = Spacing.sm))
        }
    }
}
