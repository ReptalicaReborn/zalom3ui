package com.zalom3ui.app.server

import android.content.Context
import android.content.Intent
import androidx.core.content.ContextCompat
import com.zalom3ui.app.data.SettingsStore

/** Starts and stops the Zalo server that runs on the phone itself. */
object LocalServer {
    /** Loopback port the on-phone server listens on: this install's own (see [SettingsStore.localPort]). */
    fun port(context: Context): Int = SettingsStore(context).localPort()

    fun url(context: Context): String = "http://127.0.0.1:${port(context)}"

    fun start(context: Context) {
        ContextCompat.startForegroundService(context, Intent(context, NodeService::class.java))
    }

    /** Stops the server; its process exits, since Node can only start once per process. */
    fun stop(context: Context) {
        context.stopService(Intent(context, NodeService::class.java))
    }
}
