package dev.penlink.notify

import android.app.NotificationChannel
import android.app.NotificationChannelGroup
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.util.LruCache
import androidx.core.app.NotificationCompat
import dev.penlink.MainActivity
import dev.penlink.R
import dev.penlink.data.SavedHost
import dev.penlink.net.HostApi
import dev.penlink.protocol.NotifyIn
import java.security.MessageDigest

/** Turns the PC's notifications into Android notifications (and takes them away again). */
class PcNotifier(private val ctx: Context, private val api: HostApi) {
    private val nm = ctx.getSystemService(NotificationManager::class.java)
    private val icons = object : LruCache<String, Bitmap>(64) {}

    /** Show or update one notification. Suspends while the app logo downloads (once per app). */
    suspend fun post(host: SavedHost, n: NotifyIn.Posted) {
        val channel = ensureChannel(host, n.appId, n.appName)
        val logo = icon(host, n.appId)
        val title = n.title.ifBlank { n.appName }
        val b = NotificationCompat.Builder(ctx, channel)
            .setSmallIcon(R.drawable.ic_stat_pc)
            .setContentTitle(title)
            .setContentText(n.body.ifBlank { null })
            .setSubText(n.appName)
            .setWhen(n.timeMs)
            .setShowWhen(true)
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setGroup(groupKey(host))
            .setContentIntent(openIntent(host, n))
            .setDeleteIntent(dismissIntent(host, n.id))
        if (n.body.isNotBlank()) b.setStyle(NotificationCompat.BigTextStyle().bigText(n.body))
        if (logo != null) b.setLargeIcon(logo)
        nm.notify(tag(host), n.id.toInt(), b.build())
    }

    fun remove(host: SavedHost, id: Long) {
        nm.cancel(tag(host), id.toInt())
    }

    /** After (re)connecting: drop our notifications of this PC that no longer exist there. */
    fun keepOnly(host: SavedHost, ids: Set<Long>) {
        val t = tag(host)
        for (sbn in nm.activeNotifications) {
            if (sbn.tag == t && (sbn.id.toLong() and 0xFFFFFFFFL) !in ids) nm.cancel(t, sbn.id)
        }
    }

    private suspend fun icon(host: SavedHost, appId: String): Bitmap? {
        val key = host.address + "|" + appId
        icons.get(key)?.let { return it }
        return api.notifIcon(host.address, host.token, appId)?.also { icons.put(key, it) }
    }

    private fun ensureChannel(host: SavedHost, appId: String, appName: String): String {
        val group = "pc:" + digest(host.address)
        nm.createNotificationChannelGroup(NotificationChannelGroup(group, host.name))
        val id = "app:" + digest(host.address + "|" + appId)
        // Re-creating a channel only updates its name; the user's own settings stay.
        nm.createNotificationChannel(
            NotificationChannel(id, appName, NotificationManager.IMPORTANCE_DEFAULT).apply {
                this.group = group
                description = "Notifications of $appName on ${host.name}"
            },
        )
        return id
    }

    private fun openIntent(host: SavedHost, n: NotifyIn.Posted): PendingIntent {
        val i = Intent(ctx, MainActivity::class.java).apply {
            action = ACTION_OPEN
            putExtra(EXTRA_HOST, host.address)
            putExtra(EXTRA_APP_ID, n.openAppId)
            putExtra(EXTRA_APP_NAME, n.appName)
            flags = Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_CLEAR_TOP
        }
        return PendingIntent.getActivity(ctx, requestCode(host, n.id), i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    private fun dismissIntent(host: SavedHost, id: Long): PendingIntent {
        val i = Intent(ctx, DismissReceiver::class.java).apply {
            action = ACTION_DISMISS
            putExtra(EXTRA_HOST, host.address)
            putExtra(EXTRA_ID, id)
        }
        return PendingIntent.getBroadcast(ctx, requestCode(host, id), i, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
    }

    companion object {
        const val ACTION_OPEN = "dev.penlink.OPEN_FROM_NOTIFICATION"
        const val ACTION_DISMISS = "dev.penlink.DISMISS_PC_NOTIFICATION"
        const val EXTRA_HOST = "host"
        const val EXTRA_APP_ID = "openAppId"
        const val EXTRA_APP_NAME = "appName"
        const val EXTRA_ID = "id"

        fun tag(host: SavedHost) = "pc:" + host.address
        fun groupKey(host: SavedHost) = "pcgroup:" + host.address
        fun requestCode(host: SavedHost, id: Long) = host.address.hashCode() * 31 + id.toInt()

        /** Short stable id for channel names (addresses and app ids are not valid channel ids themselves). */
        fun digest(s: String): String =
            MessageDigest.getInstance("SHA-1").digest(s.toByteArray()).joinToString("") { "%02x".format(it) }.take(16)
    }
}

/** The user swiped a forwarded notification away on the phone: remove it on the PC as well. */
class DismissReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val host = intent.getStringExtra(PcNotifier.EXTRA_HOST) ?: return
        val id = intent.getLongExtra(PcNotifier.EXTRA_ID, -1L).takeIf { it >= 0 } ?: return
        NotifyHub.dismiss(host, id)
    }
}
