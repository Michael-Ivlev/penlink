package dev.penlink.notify

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.content.ContextCompat
import dev.penlink.MainActivity
import dev.penlink.PenlinkApp
import dev.penlink.R
import dev.penlink.data.SavedHost
import dev.penlink.protocol.NotifyIn
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/**
 * Keeps one `/notify` socket open to every PC whose "Forward notifications" switch is on, and shows what the PCs
 * send. Foreground service of type `specialUse` (a long-lived network connection for a companion feature): `dataSync`
 * would be stopped by Android 15 after six hours, and `connectedDevice` needs a Bluetooth/USB/NFC-style link. It
 * shows one low-importance ongoing notification, stops by itself when no PC has the switch on, restarts after
 * the process is killed (sticky) and after a reboot (see [BootReceiver]). It survives the app being swiped away.
 */
class NotificationForwardService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Default)
    private val links = HashMap<String, NotifyLink>()
    private val watchers = HashMap<String, Job>()
    // Events of one PC are handled strictly in order (a logo download must not let a later "removed" overtake its "posted").
    private val queues = HashMap<String, Channel<NotifyIn>>()
    private lateinit var notifier: PcNotifier

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        val app = application as PenlinkApp
        notifier = PcNotifier(this, app.api)
        ensureChannel()
        try {
            startForeground(ONGOING_ID, ongoing(emptyList(), emptyList()), ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } catch (e: Exception) {
            // Android refused a foreground start from the background (e.g. ForegroundServiceStartNotAllowedException):
            // stop quietly; the next app launch starts forwarding again.
            stopSelf()
            return
        }

        scope.launch {
            app.store.notifHosts.collect { hosts ->
                synchronized(links) { sync(app, hosts) }
                if (hosts.isEmpty()) stopSelf()
            }
        }
        scope.launch {
            // Keep the ongoing notification's text current.
            combine(app.store.notifHosts, NotifyHub.status) { h, s -> h to s }.collect { (hosts, status) ->
                val up = hosts.filter { status[it.address] is NotifyStatus.Connected }.map { it.name }
                val waiting = hosts.filter { status[it.address] !is NotifyStatus.Connected }.map { it.name }
                getSystemService(NotificationManager::class.java).notify(ONGOING_ID, ongoing(up, waiting))
            }
        }
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_STICKY

    private fun sync(app: PenlinkApp, hosts: List<SavedHost>) {
        val wanted = hosts.associateBy { it.address }
        for (a in links.keys.toList()) {
            val keep = wanted[a]
            // A re-paired PC has a new token: restart its link.
            if (keep == null || links[a]!!.host.token != keep.token) {
                watchers.remove(a)?.cancel()
                queues.remove(a)?.close()
                links.remove(a)?.stop()
                NotifyHub.unregister(a)
            }
        }
        for (h in hosts) {
            if (links.containsKey(h.address)) continue
            val queue = Channel<NotifyIn>(Channel.UNLIMITED)
            queues[h.address] = queue
            scope.launch { for (ev in queue) runCatching { handle(app, h, ev) } }
            val link = NotifyLink(h, app.api.http, scope) { ev -> queue.trySend(ev) }
            links[h.address] = link
            NotifyHub.register(link)
            watchers[h.address] = scope.launch { link.status.collect { NotifyHub.publish(h.address, it) } }
            link.start()
        }
    }

    private suspend fun handle(app: PenlinkApp, host: SavedHost, ev: NotifyIn) {
        when (ev) {
            is NotifyIn.Ready -> notifier.keepOnly(host, ev.ids)
            is NotifyIn.Removed -> notifier.remove(host, ev.id)
            is NotifyIn.Posted -> {
                app.store.updateNotifPrefs(host.address) { it.withSeen(ev.appId, ev.appName) }
                val prefs = app.store.notifPrefs(host.address).first()
                if (prefs.allows(ev.appId)) notifier.post(host, ev)
            }
        }
    }

    private fun ensureChannel() {
        val nm = getSystemService(NotificationManager::class.java)
        nm.createNotificationChannel(
            NotificationChannel(LINK_CHANNEL, "Notification link", NotificationManager.IMPORTANCE_LOW).apply {
                description = "Shown while Penlink listens for notifications from your PC"
                setShowBadge(false)
            },
        )
    }

    private fun ongoing(up: List<String>, waiting: List<String>): Notification {
        val text = when {
            up.isNotEmpty() && waiting.isEmpty() -> "Connected to ${up.joinToString(", ")}"
            up.isNotEmpty() -> "Connected to ${up.joinToString(", ")}; reaching ${waiting.joinToString(", ")}"
            waiting.isNotEmpty() -> "Reaching ${waiting.joinToString(", ")}"
            else -> "Starting"
        }
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, LINK_CHANNEL)
            .setSmallIcon(R.drawable.ic_stat_pc)
            .setContentTitle("Penlink")
            .setContentText(text)
            .setOngoing(true)
            .setOnlyAlertOnce(true)
            .setShowWhen(false)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .setContentIntent(open)
            .build()
    }

    override fun onDestroy() {
        synchronized(links) {
            for ((a, l) in links) {
                l.stop()
                NotifyHub.unregister(a)
            }
            links.clear()
            queues.values.forEach { it.close() }
            queues.clear()
        }
        scope.cancel()
        super.onDestroy()
    }

    companion object {
        const val LINK_CHANNEL = "link"
        const val ONGOING_ID = 4801

        /** Start (or nudge) the service; it stops itself when no PC has the switch on. Needs a foreground app or a boot broadcast. */
        fun start(ctx: Context) {
            runCatching { ContextCompat.startForegroundService(ctx, Intent(ctx, NotificationForwardService::class.java)) }
        }
    }
}
