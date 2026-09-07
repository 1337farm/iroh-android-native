package com.example.irohapp

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
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat

class IrohDaemonService : Service(), IrohProgressListener {

    private val CHANNEL_ID = "iroh_sync_channel"
    private val NOTIFICATION_ID = 101
    private lateinit var notificationManager: NotificationManager

    override fun onCreate() {
        super.onCreate()
        notificationManager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        createNotificationChannel()
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_CANCEL) {
            IrohBridge.cancelDownload()
            stopForegroundSafely(detach = false)
            stopSelf()
            return START_NOT_STICKY
        }

        val ticket = intent?.getStringExtra(EXTRA_TICKET) ?: ""
        val privateStoragePath = filesDir.absolutePath

        val initialNotification = buildProgressNotification("Optimizing network routes...", 0, ongoing = true)

        try {
            ServiceCompat.startForeground(
                this,
                NOTIFICATION_ID,
                initialNotification,
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                    ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC
                } else {
                    0
                }
            )
        } catch (e: Exception) {
            stopSelf()
            return START_NOT_STICKY
        }

        IrohBridge.initializeAndDownload(privateStoragePath, ticket, this)
        return START_NOT_STICKY
    }

    override fun onTransferProgress(statusCode: Int, progressPct: Int, message: String) {
        val isFinished = (statusCode == 5 || statusCode < 0)
        val updatedNotification = buildProgressNotification(message, progressPct, ongoing = !isFinished)
        notificationManager.notify(NOTIFICATION_ID, updatedNotification)

        if (isFinished) {
            stopForegroundSafely(detach = true)
            stopSelf()
        }
    }

    override fun onDestroy() {
        IrohBridge.cancelDownload()
        super.onDestroy()
    }

    private fun stopForegroundSafely(detach: Boolean) {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            stopForeground(if (detach) STOP_FOREGROUND_DETACH else STOP_FOREGROUND_REMOVE)
        } else {
            @Suppress("DEPRECATION")
            stopForeground(!detach)
        }
    }

    private fun buildProgressNotification(content: String, progress: Int, ongoing: Boolean): Notification {
        val cancelIntent = Intent(this, IrohDaemonService::class.java).apply {
            action = ACTION_CANCEL
        }
        val cancelPendingIntent = PendingIntent.getService(
            this,
            0,
            cancelIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val builder = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Secure File Sync")
            .setContentText(content)
            .setSmallIcon(android.R.drawable.ic_menu_upload)
            .setOngoing(ongoing)
            .setOnlyAlertOnce(true)

        if (ongoing) {
            builder.setProgress(100, progress, progress == 0 || progress == 50)
                .addAction(android.R.drawable.ic_menu_close_clear_cancel, "Cancel", cancelPendingIntent)
        } else {
            builder.setProgress(0, 0, false)
        }

        return builder.build()
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "Background Storage Sync",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Displays continuous peer-to-peer data synchronization progress"
            }
            notificationManager.createNotificationChannel(channel)
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    companion object {
        const val EXTRA_TICKET = "EXTRA_TICKET"
        const val ACTION_CANCEL = "ACTION_CANCEL_DOWNLOAD"
    }
}
