package com.zalom3ui.app.server

import android.app.Notification
import android.content.ComponentName
import android.content.Context
import android.service.notification.NotificationListenerService
import android.service.notification.StatusBarNotification
import androidx.core.app.NotificationManagerCompat

/**
 * The notification bridge ("While open + notification bridge"): watches the notifications the official Zalo app posts, and
 * when one is a message, asks [StayConnectedCoordinator] to bring Zalỏ online briefly to fetch it. Disabled in the
 * manifest, and switched on only while that mode is chosen (see `StayConnectedCoordinator.refreshFromSettings`). The
 * system binds it once the user grants notification access, and keeps it bound.
 *
 * Privacy: nothing from a notification's text is stored or logged. The startup log gets the package, the category and
 * what was done. Extras are read defensively (any of them can be missing or of another type), and a notification this
 * class cannot read is ignored.
 *
 * Zalo's incoming-call notifications are never cancelled or acted on ([ZaloNotificationRules.Kind.CALL]): calls are
 * answered in Zalo.
 */
class ZaloNotificationListener : NotificationListenerService() {
    override fun onListenerConnected() {
        instance = this
        StartupLog.write(this, "bridge: notification access connected")
    }

    override fun onListenerDisconnected() {
        if (instance === this) instance = null
        StartupLog.write(this, "bridge: notification access disconnected")
    }

    override fun onNotificationPosted(sbn: StatusBarNotification?) {
        val info = runCatching { sbn?.let(::describe) }.getOrNull() ?: return
        if (info.packageName != MessageNotifier.ZALO_PACKAGE) return
        val kind = ZaloNotificationRules.classify(info)
        // Package, category and timing only: never the title or text.
        StartupLog.write(this, "bridge: ${info.packageName} category=${info.category ?: "none"} -> ${kind.name.lowercase()}")
        if (kind == ZaloNotificationRules.Kind.MESSAGE) {
            StayConnectedCoordinator.onZaloMessage(applicationContext, runCatching { sbn?.postTime }.getOrNull() ?: 0L)
        }
    }

    /** Cancels the Zalo app's message notifications for the chat called [chatName] (never a call). */
    private fun hideFor(chatName: String) {
        val active = runCatching { activeNotifications }.getOrNull() ?: return
        for (sbn in active) {
            val info = runCatching { describe(sbn) }.getOrNull() ?: continue
            if (ZaloNotificationRules.isForChat(info, chatName)) {
                runCatching { cancelNotification(sbn.key) }
            }
        }
    }

    companion object {
        @Volatile
        private var instance: ZaloNotificationListener? = null

        /** Whether the user has granted this app notification access (Settings → Notification access). */
        fun isAccessGranted(context: Context): Boolean =
            context.packageName in NotificationManagerCompat.getEnabledListenerPackages(context)

        /** Hides Zalo's own notifications for [chatName], once Zalỏ has posted its own. No-op when not connected. */
        fun hideZaloNotificationsFor(chatName: String) {
            instance?.hideFor(chatName)
        }

        /** Asks the system to bind the listener again (after it was enabled, or if it was unbound). */
        fun requestRebind(context: Context) {
            runCatching { NotificationListenerService.requestRebind(ComponentName(context, ZaloNotificationListener::class.java)) }
        }

        private fun describe(sbn: StatusBarNotification): NotificationInfo {
            val n = sbn.notification
            val extras = runCatching { n.extras }.getOrNull()
            fun text(key: String): String? = runCatching { extras?.getCharSequence(key)?.toString() }.getOrNull()
            val flags = n.flags
            return NotificationInfo(
                packageName = sbn.packageName.orEmpty(),
                category = runCatching { n.category }.getOrNull(),
                isOngoing = flags and (Notification.FLAG_ONGOING_EVENT or Notification.FLAG_FOREGROUND_SERVICE) != 0,
                hasFullScreenIntent = runCatching { n.fullScreenIntent != null }.getOrDefault(false),
                isGroupSummary = flags and Notification.FLAG_GROUP_SUMMARY != 0,
                title = text(Notification.EXTRA_TITLE),
                conversationTitle = text(Notification.EXTRA_CONVERSATION_TITLE),
            )
        }
    }
}
