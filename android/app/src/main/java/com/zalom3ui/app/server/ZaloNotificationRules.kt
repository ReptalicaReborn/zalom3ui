package com.zalom3ui.app.server

/**
 * What the notification bridge needs to know about a notification posted by another app. Built by
 * [ZaloNotificationListener] from the system's notification, defensively; the rules below are plain Kotlin and tested on
 * the JVM. The text of a notification is never part of this: only its package, category and flags, and the title, which
 * is used (not logged) to find the notification of a chat.
 */
data class NotificationInfo(
    val packageName: String,
    /** `Notification.category`, e.g. `"msg"` or `"call"`; null when the app sets none. */
    val category: String?,
    /** Ongoing, or posted by a foreground service (a "running" notification, not news). */
    val isOngoing: Boolean,
    /** An incoming-call style notification (a full-screen intent). */
    val hasFullScreenIntent: Boolean,
    val isGroupSummary: Boolean,
    val title: String? = null,
    val conversationTitle: String? = null,
)

object ZaloNotificationRules {
    enum class Kind {
        /** From another app: not our business. */
        NOT_ZALO,

        /** A Zalo incoming or missed call (or alarm): calls are answered in Zalo; never touched. */
        CALL,

        /** A Zalo message: brings Zalỏ online. */
        MESSAGE,

        /** Zalo, but not news: a running-service, progress or group-summary notification. */
        IGNORED,
    }

    private val CALL_CATEGORIES = setOf("call", "missed_call", "alarm")

    private val NON_MESSAGE_CATEGORIES = setOf(
        "service", "progress", "transport", "status", "sys", "err", "recommendation", "promo", "navigation",
        "location_sharing", "stopwatch", "workout", "reminder", "event",
    )

    fun classify(n: NotificationInfo): Kind {
        if (n.packageName != MessageNotifier.ZALO_PACKAGE) return Kind.NOT_ZALO
        val category = n.category?.lowercase()
        if (category in CALL_CATEGORIES || n.hasFullScreenIntent) return Kind.CALL
        if (n.isOngoing || n.isGroupSummary) return Kind.IGNORED
        if (category in NON_MESSAGE_CATEGORIES) return Kind.IGNORED
        return Kind.MESSAGE
    }

    /**
     * Whether the Zalo notification [n] is for the chat called [chatName] (the title Zalỏ's own notification for that
     * chat carries). Titles are compared ignoring case and spacing; a title that adds the sender around the chat name
     * ("Chat name: Alice", "Alice @ Chat name") counts too. Never true for a call.
     */
    fun isForChat(n: NotificationInfo, chatName: String): Boolean {
        if (classify(n) != Kind.MESSAGE) return false
        val wanted = normalise(chatName)
        if (wanted.isEmpty()) return false
        return listOfNotNull(n.title, n.conversationTitle).map(::normalise).any { title ->
            title == wanted || SEPARATORS.any { sep ->
                title.startsWith(wanted + sep) || title.endsWith(sep + wanted)
            }
        }
    }

    private val SEPARATORS = listOf(":", " - ", " @ ", " | ", " · ", " > ", " in ")

    private fun normalise(s: String): String = s.trim().lowercase().replace(Regex("\\s+"), " ")
}
