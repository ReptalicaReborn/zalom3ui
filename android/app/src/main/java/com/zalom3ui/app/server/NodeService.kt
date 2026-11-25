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
import android.system.Os
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
 * Runs the Zalo server (the bundled src/ of this repo) in embedded Node.js.
 *
 * Lives in its own ":node" process: Node can only start once per process, so stopping the service
 * ends the process and the next start gets a fresh one. It runs as a foreground service so the
 * Zalo connection, and with it the 1:1 message history, keeps going while the app is closed.
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
            // Node needs a bigger stack than Android's default thread stack.
            Thread(null, Runnable { runNode() }, "node", NODE_STACK_BYTES).start()
        }
        return START_STICKY
    }

    override fun onDestroy() {
        StartupLog.write(this, "service: destroyed")
        scope.cancel()
        runCatching { getSystemService(ConnectivityManager::class.java).unregisterNetworkCallback(networkCallback) }
        super.onDestroy()
        // Exit after onDestroy() returns so Android records stopSelf() cleanly instead of treating it as a crash.
        Handler(Looper.getMainLooper()).post { Process.killProcess(Process.myPid()) }
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

    private fun runNode() {
        try {
            val script = File(filesDir, "server/server.mjs")
            script.parentFile?.mkdirs()
            // Always refresh: the bundle changes with every app update and is only ~2 MB.
            assets.open("server/server.mjs").use { input -> script.outputStream().use { input.copyTo(it) } }

            val dataDir = File(filesDir, "zalo").apply { mkdirs() }
            Os.setenv("HOST", "127.0.0.1", true)
            Os.setenv("PORT", LocalServer.port(this).toString(), true)
            Os.setenv("ACCESS_TOKEN", SettingsStore(this).localToken(), true)
            Os.setenv("DATA_DIR", dataDir.absolutePath, true)
            Os.setenv("HOME", filesDir.absolutePath, true)
            Os.setenv("TMPDIR", cacheDir.absolutePath, true)
            // The server copies its console output here, so a failed start can be shared (StartupLog.report).
            Os.setenv("LOG_FILE", StartupLog.serverLog(this).absolutePath, true)

            StartupLog.write(this, "service: starting Node on port ${LocalServer.port(this)}")
            val code = NodeRuntime.startNode(arrayOf("node", "--max-old-space-size=$HEAP_MB", script.absolutePath))
            Log.w(TAG, "Node exited with code $code")
            StartupLog.write(this, "service: Node exited with code $code")
        } catch (e: Throwable) {
            Log.e(TAG, "Could not run the Zalo server", e)
            StartupLog.error(this, "service: could not run Node", e)
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
        const val NODE_STACK_BYTES = 8L * 1024 * 1024
        const val HEAP_MB = 192
    }
}
