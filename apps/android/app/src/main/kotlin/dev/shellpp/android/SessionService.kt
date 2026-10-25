package dev.shellpp.android

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder

/**
 * Keeps the process, and so the open SSH sessions, alive while the app is
 * in the background (architecture 3): a foreground service of type
 * specialUse with a notification that counts the sessions and offers
 * "Disconnect all". While SFTP transfers run, it shows the current one
 * instead, with its progress (UI spec 10).
 */
class SessionService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val app = application as App
        if (intent?.action == ACTION_DISCONNECT_ALL) {
            app.closeAll()
            stopSelf()
            return START_NOT_STICKY
        }
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL, getString(R.string.channel_sessions), NotificationManager.IMPORTANCE_LOW),
        )
        val notification = notification(this, app)
        if (Build.VERSION.SDK_INT >= 34) {
            startForeground(NOTIFICATION, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_SPECIAL_USE)
        } else {
            startForeground(NOTIFICATION, notification)
        }
        // The last session may have gone between the start and now: the
        // service stops itself only after it went to the foreground, which
        // Android requires (review of run 13).
        if (app.sessions.isEmpty()) stopSelf()
        return START_NOT_STICKY
    }

    companion object {
        const val CHANNEL = "sessions"
        const val NOTIFICATION = 1
        const val ACTION_DISCONNECT_ALL = "dev.shellpp.android.DISCONNECT_ALL"

        /**
         * The service's notification: the count of open sessions, or the
         * running transfer with the bytes so far (the size is not known
         * ahead, so the bar only moves) and how many wait behind it.
         */
        fun notification(context: Context, app: App): Notification {
            val count = app.sessions.size
            val open = PendingIntent.getActivity(
                context, 0, Intent(context, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE,
            )
            val disconnect = PendingIntent.getService(
                context, 1, Intent(context, SessionService::class.java).setAction(ACTION_DISCONNECT_ALL),
                PendingIntent.FLAG_IMMUTABLE,
            )
            val active = app.transfers.value.values
                .filter { it.optString("state") in setOf("queued", "running") }
                .sortedBy { it.optLong("id") }
            val current = active.firstOrNull { it.optString("state") == "running" }
            val builder = Notification.Builder(context, CHANNEL)
                .setSmallIcon(R.drawable.ic_launcher)
                .setContentTitle(context.getString(R.string.app_name))
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setContentIntent(open)
                .addAction(Notification.Action.Builder(null, context.getString(R.string.disconnect_all), disconnect).build())
            if (current != null) {
                val name = current.optString("name")
                val size = humanSize(current.optLong("done"))
                builder.setContentText(
                    if (current.optString("direction") == "upload") {
                        context.getString(R.string.transfer_uploading, name, size)
                    } else {
                        context.getString(R.string.transfer_downloading, name, size)
                    },
                )
                if (active.size > 1) builder.setSubText(context.getString(R.string.transfers_waiting, active.size - 1))
                builder.setProgress(0, 0, true)
            } else {
                builder.setContentText(
                    if (count == 1) context.getString(R.string.session_open) else context.getString(R.string.sessions_open, count),
                )
            }
            return builder.build()
        }
    }
}
