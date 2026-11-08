package dev.penlink.ui

import android.Manifest
import android.content.pm.PackageManager
import android.graphics.Bitmap
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.calculateEndPadding
import androidx.compose.foundation.layout.calculateStartPadding
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.NotificationsActive
import androidx.compose.material.icons.filled.NotificationsOff
import androidx.compose.material3.Button
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LargeTopAppBar
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.LayoutDirection
import androidx.compose.ui.unit.dp
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import dev.penlink.PenlinkApp
import dev.penlink.Screen
import dev.penlink.data.NotifPrefs
import dev.penlink.data.SavedHost
import dev.penlink.notify.NotificationForwardService
import dev.penlink.notify.NotifyHub
import dev.penlink.notify.NotifyStatus
import dev.penlink.ui.theme.Spacing
import kotlinx.coroutines.launch

/**
 * "Notifications" page of one PC: the master switch for forwarding its Windows notifications to this phone and one
 * switch per app that has sent a notification so far (all on by default). Android's own per-app channel settings
 * (one channel per PC app) tune sound and importance on top of this.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun NotificationsScreen(app: PenlinkApp, host: SavedHost) {
    val ctx = LocalContext.current
    val scope = rememberCoroutineScope()
    val haptics = rememberHaptics()
    val prefs by app.store.notifPrefs(host.address).collectAsState(initial = NotifPrefs())
    val status by NotifyHub.status.collectAsState()
    val scroll = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    val back = { app.navigate(Screen.Apps) }
    BackHandler { back() }

    fun granted() = ContextCompat.checkSelfPermission(ctx, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED
    var permitted by remember { mutableStateOf(granted()) }
    val askPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { permitted = it }
    val owner = LocalLifecycleOwner.current
    DisposableEffect(owner) {
        val o = LifecycleEventObserver { _, e -> if (e == Lifecycle.Event.ON_RESUME) permitted = granted() }
        owner.lifecycle.addObserver(o)
        onDispose { owner.lifecycle.removeObserver(o) }
    }

    val setEnabled: (Boolean) -> Unit = { on ->
        haptics.toggle(on)
        if (on && !permitted) askPermission.launch(Manifest.permission.POST_NOTIFICATIONS)
        scope.launch {
            app.store.updateNotifPrefs(host.address) { it.withEnabled(on) }
            if (on) NotificationForwardService.start(ctx)
        }
    }

    Scaffold(
        modifier = Modifier.nestedScroll(scroll.nestedScrollConnection),
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            LargeTopAppBar(
                title = { Text("Notifications", maxLines = 1, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    IconButton(onClick = { back() }) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back to apps") }
                },
                colors = TopAppBarDefaults.topAppBarColors(
                    containerColor = MaterialTheme.colorScheme.background,
                    scrolledContainerColor = MaterialTheme.colorScheme.surfaceContainer,
                ),
                scrollBehavior = scroll,
            )
        },
    ) { padding ->
        val pad = PaddingValues(
            start = padding.calculateStartPadding(LayoutDirection.Ltr) + Spacing.lg,
            end = padding.calculateEndPadding(LayoutDirection.Ltr) + Spacing.lg,
            top = padding.calculateTopPadding(),
            bottom = padding.calculateBottomPadding() + Spacing.lg,
        )
        LazyColumn(Modifier.fillMaxSize(), contentPadding = pad, verticalArrangement = Arrangement.spacedBy(Spacing.sm)) {
            item(key = "master") {
                val link = status[host.address]
                MasterCard(host, prefs.enabled, link, setEnabled)
            }
            if (prefs.enabled && !permitted) {
                item(key = "perm") {
                    NoticeCard(
                        title = "Android is blocking notifications",
                        body = "Allow notifications for Penlink so PC notifications can appear on this phone.",
                        action = "Allow",
                    ) { askPermission.launch(Manifest.permission.POST_NOTIFICATIONS) }
                }
            }
            val access = (status[host.address] as? NotifyStatus.Connected)?.access
            if (prefs.enabled && access != null && access != "allowed") {
                item(key = "access") {
                    NoticeCard(
                        title = "Windows is not sharing notifications",
                        body = "On ${host.name}: Settings > Privacy & security > Notifications, allow Penlink (or \"Let apps access your notifications\"), then restart penlink-host.",
                    )
                }
            }
            item(key = "apps-title") {
                Text(
                    if (prefs.seen.isEmpty()) "Apps" else "Apps  ·  ${prefs.seen.size}",
                    style = MaterialTheme.typography.titleMedium,
                    modifier = Modifier.padding(top = Spacing.md, bottom = Spacing.xs).semantics { },
                )
            }
            if (prefs.seen.isEmpty()) {
                item(key = "empty") {
                    MessagePanel(
                        icon = Icons.Default.NotificationsOff,
                        title = "No apps yet",
                        body = "Apps show up here the first time they send a notification on ${host.name}. All of them are forwarded until you switch one off.",
                    )
                }
            } else {
                items(prefs.seen, key = { it.appId }) { a ->
                    AppRow(app, host, a.appId, a.name, checked = a.appId !in prefs.off, enabled = prefs.enabled, modifier = Modifier) { on ->
                        haptics.toggle(on)
                        scope.launch { app.store.updateNotifPrefs(host.address) { it.withApp(a.appId, on) } }
                    }
                }
            }
        }
    }
}

@Composable
private fun MasterCard(host: SavedHost, enabled: Boolean, link: NotifyStatus?, onChange: (Boolean) -> Unit) {
    val cs = MaterialTheme.colorScheme
    Card(
        colors = CardDefaults.cardColors(containerColor = if (enabled) cs.primaryContainer else cs.surfaceContainerHigh),
        shape = RoundedCornerShape(24.dp),
    ) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = Spacing.lg, vertical = Spacing.md),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(Spacing.md),
        ) {
            Icon(
                if (enabled) Icons.Default.NotificationsActive else Icons.Default.NotificationsOff,
                contentDescription = null,
                tint = if (enabled) cs.onPrimaryContainer else cs.onSurfaceVariant,
            )
            Column(Modifier.weight(1f)) {
                Text(
                    "Forward notifications from this PC",
                    style = MaterialTheme.typography.titleMedium,
                    color = if (enabled) cs.onPrimaryContainer else cs.onSurface,
                )
                val line = when {
                    !enabled -> "Off. Turn on to see ${host.name}'s notifications here."
                    link is NotifyStatus.Connected -> "Connected to ${host.name}"
                    else -> "Reaching ${host.name}…"
                }
                Text(line, style = MaterialTheme.typography.bodyMedium, color = if (enabled) cs.onPrimaryContainer else cs.onSurfaceVariant)
            }
            Switch(checked = enabled, onCheckedChange = onChange)
        }
    }
}

@Composable
private fun NoticeCard(title: String, body: String, action: String? = null, onAction: () -> Unit = {}) {
    val cs = MaterialTheme.colorScheme
    Card(colors = CardDefaults.cardColors(containerColor = cs.errorContainer), shape = RoundedCornerShape(20.dp)) {
        Column(Modifier.fillMaxWidth().padding(Spacing.lg), verticalArrangement = Arrangement.spacedBy(Spacing.xs)) {
            Text(title, style = MaterialTheme.typography.titleSmall, color = cs.onErrorContainer)
            Text(body, style = MaterialTheme.typography.bodyMedium, color = cs.onErrorContainer)
            if (action != null) Button(onClick = onAction, modifier = Modifier.padding(top = Spacing.xs)) { Text(action) }
        }
    }
}

@Composable
private fun AppRow(
    app: PenlinkApp,
    host: SavedHost,
    appId: String,
    name: String,
    checked: Boolean,
    enabled: Boolean,
    modifier: Modifier,
    onChange: (Boolean) -> Unit,
) {
    val cs = MaterialTheme.colorScheme
    val logo: Bitmap? by produceState<Bitmap?>(null, appId) { value = app.api.notifIcon(host.address, host.token, appId) }
    Row(
        modifier.fillMaxWidth().clip(RoundedCornerShape(16.dp)).background(cs.surfaceContainerLow)
            .padding(horizontal = Spacing.md, vertical = Spacing.sm),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(Spacing.md),
    ) {
        Box(Modifier.size(40.dp).clip(CircleShape).background(cs.secondaryContainer), contentAlignment = Alignment.Center) {
            val b = logo
            if (b != null) {
                Image(b.asImageBitmap(), contentDescription = null, Modifier.size(40.dp), contentScale = ContentScale.Fit)
            } else {
                Text(name.take(1).uppercase(), style = MaterialTheme.typography.titleMedium, color = cs.onSecondaryContainer)
            }
        }
        Column(Modifier.weight(1f)) {
            Text(name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
        Switch(checked = checked, onCheckedChange = onChange, enabled = enabled)
    }
}
