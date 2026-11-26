package com.zalom3ui.app.server

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import com.zalom3ui.app.data.SettingsStore

/** Reconnects to Zalo after the phone restarts ("Stay connected: Always"), so messages keep arriving into the history. */
class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        // Only "Stay connected: Always" keeps a foreground service; the other modes connect when the app is opened.
        val store = SettingsStore(context)
        if (intent.action == Intent.ACTION_BOOT_COMPLETED && store.load().onDevice && store.loadStay().mode.keepsForegroundService) {
            LocalServer.start(context)
        }
    }
}
