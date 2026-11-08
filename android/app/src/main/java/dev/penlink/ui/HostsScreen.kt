package dev.penlink.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.setValue
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Add
import androidx.compose.material.icons.filled.Computer
import androidx.compose.material.icons.filled.Delete
import androidx.compose.material.icons.filled.MoreVert
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExtendedFloatingActionButton
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LargeTopAppBar
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.draw.clip
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import dev.penlink.PenlinkApp
import dev.penlink.Screen
import dev.penlink.data.SavedHost
import dev.penlink.ui.theme.Spacing
import dev.penlink.ui.theme.extra
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.ensureActive
import kotlinx.coroutines.isActive
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/** Whether a saved PC answered a quick /info probe. */
sealed interface HostStatus {
    data object Checking : HostStatus
    data object Online : HostStatus
    data object Offline : HostStatus
}

private const val PROBE_TIMEOUT_MS = 2_500L
private const val PROBE_EVERY_MS = 20_000L

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun HostsScreen(app: PenlinkApp) {
    val hostsOrNull by app.store.hosts.collectAsState(initial = null)
    val scope = rememberCoroutineScope()
    val haptics = rememberHaptics()
    var forgetting by remember { mutableStateOf<SavedHost?>(null) }
    var probeTick by remember { mutableIntStateOf(0) }
    val statuses = remember { mutableStateMapOf<String, HostStatus>() }
    val scroll = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    val lifecycle = LocalLifecycleOwner.current.lifecycle

    // Cheap reachability probe: GET /info with a short timeout, now and every 20 s while this screen is shown.
    LaunchedEffect(hostsOrNull, probeTick) {
        val hosts = hostsOrNull ?: return@LaunchedEffect
        statuses.keys.retainAll(hosts.map { it.address }.toSet())
        for (h in hosts) if (h.address !in statuses) statuses[h.address] = HostStatus.Checking
        // Only while the app is in the foreground: leaving the app stops the loop, coming back probes at once.
        lifecycle.repeatOnLifecycle(Lifecycle.State.STARTED) {
            while (isActive) {
                coroutineScope {
                    for (h in hosts) launch {
                        val info = withTimeoutOrNull(PROBE_TIMEOUT_MS) { runCatching { app.api.info(h.address) }.getOrNull() }
                        // runCatching swallows cancellation: a cancelled (refreshed / left) probe must not write "Offline".
                        ensureActive()
                        statuses[h.address] = if (info != null) HostStatus.Online else HostStatus.Offline
                    }
                }
                delay(PROBE_EVERY_MS)
            }
        }
    }

    Scaffold(
        modifier = Modifier.nestedScroll(scroll.nestedScrollConnection),
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            LargeTopAppBar(
                title = { Text("Penlink") },
                actions = {
                    IconButton(onClick = {
                        haptics.tap()
                        for (k in statuses.keys.toList()) statuses[k] = HostStatus.Checking
                        probeTick++
                    }) { Icon(Icons.Default.Refresh, contentDescription = "Check which PCs are online") }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.background,
                    scrolledContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                ),
                scrollBehavior = scroll,
            )
        },
        floatingActionButton = {
            ExtendedFloatingActionButton(
                onClick = { haptics.tap(); app.navigate(Screen.Pair) },
                icon = { Icon(Icons.Default.Add, contentDescription = null) },
                text = { Text("Add PC") },
            )
        },
    ) { padding ->
        val hosts = hostsOrNull
        val gridPadding = PaddingValues(
            start = padding.calculateStartPadding(LayoutDirection.Ltr) + Spacing.lg,
            end = padding.calculateEndPadding(LayoutDirection.Ltr) + Spacing.lg,
            top = padding.calculateTopPadding() + Spacing.xs,
            bottom = padding.calculateBottomPadding() + 96.dp, // room for the FAB
        )
        when {
            hosts == null -> LazyVerticalGrid(
                columns = GridCells.Adaptive(320.dp),
                contentPadding = gridPadding,
                verticalArrangement = Arrangement.spacedBy(Spacing.md),
                horizontalArrangement = Arrangement.spacedBy(Spacing.md),
                modifier = Modifier.fillMaxSize(),
                userScrollEnabled = false,
            ) {
                items(2) { ShimmerBox(Modifier.fillMaxWidth().height(92.dp), MaterialTheme.shapes.large) }
            }
            hosts.isEmpty() -> Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.Center) {
                MessagePanel(
                    icon = Icons.Default.Computer,
                    title = "No PCs yet",
                    body = "Run penlink-host on your PC, then add it here with the 6-digit code it prints.",
                ) {
                    Button(onClick = { haptics.tap(); app.navigate(Screen.Pair) }) {
                        Icon(Icons.Default.Add, contentDescription = null, Modifier.size(18.dp))
                        Text("Add PC", Modifier.padding(start = Spacing.sm))
                    }
                }
            }
            else -> LazyVerticalGrid(
                columns = GridCells.Adaptive(320.dp),
                contentPadding = gridPadding,
                verticalArrangement = Arrangement.spacedBy(Spacing.md),
                horizontalArrangement = Arrangement.spacedBy(Spacing.md),
                modifier = Modifier.fillMaxSize(),
            ) {
                item(span = { GridItemSpan(maxLineSpan) }, key = "intro") {
                    Text(
                        "Pick a PC to control",
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(bottom = Spacing.xs),
                    )
                }
                items(hosts, key = { it.address }) { h ->
                    HostCard(
                        host = h,
                        status = statuses[h.address] ?: HostStatus.Checking,
                        onOpen = {
                            haptics.tap()
                            app.connect(h)
                            app.navigate(Screen.Apps)
                        },
                        onForget = { forgetting = h },
                        modifier = Modifier.animateItem(),
                    )
                }
            }
        }
    }

    forgetting?.let { h ->
        AlertDialog(
            onDismissRequest = { forgetting = null },
            icon = { Icon(Icons.Default.Delete, contentDescription = null) },
            title = { Text("Forget ${h.name}?") },
            text = { Text("This phone will be unpaired from the PC. You will need a new pairing code to add it again.") },
            confirmButton = {
                TextButton(onClick = {
                    haptics.confirm()
                    forgetting = null
                    scope.launch { app.store.remove(h.address) }
                }) { Text("Forget", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { forgetting = null }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun HostCard(host: SavedHost, status: HostStatus, onOpen: () -> Unit, onForget: () -> Unit, modifier: Modifier = Modifier) {
    val cs = MaterialTheme.colorScheme
    var menu by remember { mutableStateOf(false) }
    val label = when (status) {
        HostStatus.Online -> "Online"
        HostStatus.Offline -> "Offline"
        HostStatus.Checking -> "Checking..."
    }
    Card(
        onClick = onOpen,
        modifier = modifier.fillMaxWidth().semantics(mergeDescendants = true) { contentDescription = "${host.name}, ${host.address}, $label" },
        shape = MaterialTheme.shapes.large,
        colors = CardDefaults.cardColors(containerColor = cs.surfaceContainerHigh),
    ) {
        Row(Modifier.padding(start = Spacing.lg, top = Spacing.md, bottom = Spacing.md, end = Spacing.xs), verticalAlignment = Alignment.CenterVertically) {
            Box(Modifier.size(52.dp).clip(CircleShape).background(cs.primaryContainer), contentAlignment = Alignment.Center) {
                Icon(Icons.Default.Computer, contentDescription = null, tint = cs.onPrimaryContainer)
            }
            Column(Modifier.weight(1f).padding(horizontal = Spacing.lg), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(host.name, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(
                    host.address,
                    style = MaterialTheme.typography.bodyMedium,
                    color = cs.onSurfaceVariant,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Row(verticalAlignment = Alignment.CenterVertically, modifier = Modifier.padding(top = 2.dp)) {
                    HostStatusDot(status)
                    Text(label, Modifier.padding(start = 6.dp), style = MaterialTheme.typography.labelMedium, color = cs.onSurfaceVariant)
                }
            }
            Box {
                IconButton(onClick = { menu = true }) { Icon(Icons.Default.MoreVert, contentDescription = "More options for ${host.name}") }
                DropdownMenu(expanded = menu, onDismissRequest = { menu = false }) {
                    DropdownMenuItem(
                        text = { Text("Forget this PC") },
                        leadingIcon = { Icon(Icons.Default.Delete, contentDescription = null) },
                        onClick = { menu = false; onForget() },
                    )
                }
            }
        }
    }
}

@Composable
private fun HostStatusDot(status: HostStatus) {
    val target = when (status) {
        HostStatus.Online -> MaterialTheme.extra.success
        HostStatus.Offline -> MaterialTheme.colorScheme.outline
        HostStatus.Checking -> MaterialTheme.colorScheme.tertiary
    }
    val color by animateColorAsState(target, label = "hostDot")
    val dot = Modifier.size(10.dp).clearAndSetSemantics { }
    if (status == HostStatus.Checking) {
        // Only the checking state animates; online/offline dots are static (no idle animation running).
        val pulse = rememberInfiniteTransition(label = "pulse").animateFloat(
            initialValue = 1f,
            targetValue = 0.3f,
            animationSpec = infiniteRepeatable(tween(700), RepeatMode.Reverse),
            label = "pulseAlpha",
        )
        Box(dot.graphicsLayer { alpha = pulse.value }.clip(CircleShape).background(color))
    } else {
        Box(dot.clip(CircleShape).background(color))
    }
}
