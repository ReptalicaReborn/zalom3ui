package com.zalom3ui.app.server

import android.content.Context
import android.content.Intent
import androidx.core.content.ContextCompat

/** Starts and stops the foreground service that keeps the on-phone Zalo core alive in the background. */
object LocalServer {
    /** Asks Android for the service. Phones that block it (battery savers, autostart rules) are logged. */
    fun start(context: Context): Throwable? {
        StartupLog.write(context, "app: start service requested")
        return try {
            ContextCompat.startForegroundService(context, Intent(context, ZaloService::class.java))
            null
        } catch (e: Exception) {
            StartupLog.error(context, "app: Android refused to start the service", e)
            e
        }
    }

    /** Stops the service. The process, and the core in it, keep running; see [ZaloCore.shutdown] to stop the core. */
    fun stop(context: Context) {
        context.stopService(Intent(context, ZaloService::class.java))
    }
}
