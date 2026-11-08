package dev.penlink

import android.app.Service
import android.content.Intent
import android.os.IBinder

/**
 * Does nothing but listen: when the user swipes Penlink away from the recent apps, the activity may never get an
 * `onDestroy`, but a service with `stopWithTask="false"` gets [onTaskRemoved]. There the PC is told to close every
 * session (so the virtual monitors vanish at once) before the process goes.
 */
class TaskWatchService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int = START_NOT_STICKY

    override fun onTaskRemoved(rootIntent: Intent?) {
        (application as PenlinkApp).disconnect()
        stopSelf()
    }
}
