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
import android.os.Handler
import android.os.IBinder
import android.os.Looper
import android.os.Process
import android.util.Log
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
import java.io.File

/**
 * Runs the embedded Rust Zalo server (libzalocore.so).
 *
 * Kept as NodeService in its own ":node" process for upgrade compatibility with existing manifest,
 * receivers, and LocalServer. Runs as a foreground service so the Zalo connection, and with it the
 * 1:1 message history, keeps going while the app is closed.
 */
class NodeService : Service() {
    private var started = false
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

    /**
     * Phones often start this service (at boot, or on a train) before they are online. The server then
     * retries its saved session with a growing delay; tell it to try right away when a network appears.
     */
    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            scope.launch {
                runCatching { ApiClient(LocalServer.url(this@NodeService), SettingsStore(this@NodeService).localToken()).retry() }
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
        StartupLog.write(this, "service: start command (already running: $started)")
        if (!promoteToForeground()) {
            return START_NOT_STICKY
        }
        if (!started) {
            started = true
            Thread({ runServer() }, "zalocore").apply {
                isDaemon = true
                start()
            }
        }
        return START_STICKY
    }

    override fun onDestroy() {
        StartupLog.write(this, "service: destroyed")
        scope.cancel()
        runCatching { getSystemService(ConnectivityManager::class.java).unregisterNetworkCallback(networkCallback) }
        if (started) {
            runCatching { NodeRuntime.stopServer() }
        }
        super.onDestroy()
        // Allow zalocore a brief window to flush state cleanly, then guarantee the ":node" process exits for clean restarts.
        Handler(Looper.getMainLooper()).postDelayed({ Process.killProcess(Process.myPid()) }, STOP_KILL_DELAY_MS)
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

    private fun runServer() {
        try {
            val dataDir = File(filesDir, "zalo").apply { mkdirs() }
            val port = LocalServer.port(this)
            StartupLog.write(this, "service: starting zalocore on port $port")
            val code = NodeRuntime.startServer(
                host = "127.0.0.1",
                port = port,
                token = SettingsStore(this).localToken(),
                dataDir = dataDir.absolutePath,
                logFile = StartupLog.serverLog(this).absolutePath,
            )
            Log.w(TAG, "zalocore exited with code $code")
            StartupLog.write(this, "service: zalocore exited with code $code")
        } catch (e: Throwable) {
            Log.e(TAG, "Could not run the Zalo server", e)
            StartupLog.error(this, "service: could not run zalocore", e)
        }
        stopSelf()
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
        const val TAG = "ZalooNode"
        const val CHANNEL_ID = "server"
        const val NOTIFICATION_ID = 1
        const val STOP_KILL_DELAY_MS = 300L
    }
}
