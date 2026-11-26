package com.zalom3ui.app.server

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.net.ConnectivityManager
import android.net.Network
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import com.zalom3ui.app.MainActivity
import com.zalom3ui.app.R
import com.zalom3ui.app.data.ApiClient
import com.zalom3ui.app.data.SettingsStore
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.launch

/**
 * Keeps the Zalo connection, and with it the 1:1 message history and message notifications, going while the app is
 * closed: a foreground service that holds the process alive and owns the "Connected to Zalo" notification.
 *
 * The core itself ([ZaloCore], libzalocore.so) is a process-wide singleton that the app starts on first use, so the UI
 * can use it while it is open with or without this service. Stopping or restarting the service does not stop the core
 * or end the process.
 */
class ZaloService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /**
     * Phones often start this service (at boot, or on a train) before they are online. The server then
     * retries its saved session with a growing delay; tell it to try right away when a network appears.
     */
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            scope.launch {
                runCatching { ApiClient.onDevice(this@ZaloService).retry() }
            }
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onCreate() {
        super.onCreate()
        StartupLog.write(this, "service: created")
        if (!promoteToForeground()) return
        runCatching { getSystemService(ConnectivityManager::class.java).registerDefaultNetworkCallback(networkCallback) }
        // New-message notifications, while the app is closed too.
        MessageNotifier(this).start(scope)
    }

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        StartupLog.write(this, "service: start command")
        if (!promoteToForeground()) {
            return START_NOT_STICKY
        }
        // A start left over from before "Stay connected" was switched away from Always: Android needs the notification
        // above to have been posted, then the service has no business running.
        if (!SettingsStore(this).loadStay().mode.keepsForegroundService) {
            StartupLog.write(this, "service: not wanted in this Stay connected mode, stopping")
            stopSelf()
            return START_NOT_STICKY
        }
        // Idempotent: the core may already be running because the app started it.
        scope.launch { ZaloCore.ensureStarted(this@ZaloService) }
        return START_STICKY
    }

    override fun onDestroy() {
        StartupLog.write(this, "service: destroyed")
        scope.cancel()
        runCatching { getSystemService(ConnectivityManager::class.java).unregisterNetworkCallback(networkCallback) }
        super.onDestroy()
    }

    private fun promoteToForeground(): Boolean {
        return try {
            val notification = buildNotification()
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
                startForeground(NOTIFICATION_ID, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_REMOTE_MESSAGING)
            } else {
                startForeground(NOTIFICATION_ID, notification)
            }
            true
        } catch (e: SecurityException) {
            StartupLog.error(this, "service: startForeground denied", e)
            stopSelf()
            false
        } catch (e: IllegalStateException) {
            // Includes ForegroundServiceStartNotAllowedException (API 31+, subclass of IllegalStateException).
            StartupLog.error(this, "service: startForeground not allowed", e)
            stopSelf()
            false
        }
    }

    private fun buildNotification(): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(
            NotificationChannel(CHANNEL_ID, "Zalo connection", NotificationManager.IMPORTANCE_MIN),
        )
        val open = PendingIntent.getActivity(
            this,
            0,
            Intent(this, MainActivity::class.java),
            PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle("Zalỏ")
            .setContentText("Connected to Zalo")
            .setContentIntent(open)
            .setOngoing(true)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .build()
    }

    private companion object {
        const val CHANNEL_ID = "server"
        const val NOTIFICATION_ID = 1
    }
}
