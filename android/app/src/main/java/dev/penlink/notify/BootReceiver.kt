package dev.penlink.notify

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import dev.penlink.PenlinkApp
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.launch

/** After a reboot (or an app update) start the notification service if any PC has forwarding on. */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != Intent.ACTION_BOOT_COMPLETED && intent.action != Intent.ACTION_MY_PACKAGE_REPLACED) return
        val app = context.applicationContext as PenlinkApp
        val pending = goAsync()
        CoroutineScope(Dispatchers.Default).launch {
            try {
                if (app.store.notifHosts.first().isNotEmpty()) NotificationForwardService.start(context.applicationContext)
            } finally {
                pending.finish()
            }
        }
    }
}
