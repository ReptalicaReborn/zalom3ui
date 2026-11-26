package com.zalom3ui.app.server

import com.zalom3ui.app.data.ChatMessage
import com.zalom3ui.app.data.StayConnected
import com.zalom3ui.app.ui.extractOtpCode

/*
 * The plain-Kotlin pieces of "codes in notifications" (no Android classes), tested on the JVM:
 * what a code notification says, which messages carry a code, the de-duplication that keeps one code from being
 * announced twice, what to read from the official Zalo app's notification, and which of the notification listener's
 * two jobs (the bridge, the code rescue) runs for a given notification.
 */

/** The words of a code notification. The code is first, so it is the first thing seen on a heads-up or collapsed row. */
object OtpText {
    const val PUBLIC_TITLE = "Zalỏ"
    const val PUBLIC_TEXT = "New code received"

    /** "Code 482910 · Chat name" (just "Code 482910" without a chat). */
    fun title(code: String, chat: String?): String =
        if (chat.isNullOrBlank()) "Code $code" else "Code $code · ${chat.trim()}"

    /** A line of the conversation for a message that carries [code]: the code first, then the message. */
    fun line(code: String, text: String): String = "$code · $text"

    fun copyLabel(code: String): String = "Copy $code"

    fun copiedToast(code: String): String = "Copied $code"

    /** A code is 4-8 digits; the copy receiver refuses anything else. */
    fun isValidCode(code: String?): Boolean = code != null && code.length in 4..8 && code.all { it in '0'..'9' }

    /** The code a message carries: the server's `otp` first, then the card's, then a local scan of the text. */
    fun messageCode(msg: ChatMessage): String? {
        if (msg.isSelf || msg.recalled) return null
        return (msg.otp ?: msg.attachment?.otp ?: extractOtpCode(msg.text))?.takeIf(::isValidCode)
    }
}

/**
 * Remembers which codes Zalỏ already announced, so a code is not posted twice: once by the web connection (a chat
 * notification) and once from the official Zalo app's notification, and once for each update Zalo makes to a notification
 * it already showed. Entries expire; the map is bounded. Not persisted, and holds codes only in memory.
 */
class OtpDedupe(
    private val webWindowMs: Long = WEB_WINDOW_MS,
    private val rescueMemoryMs: Long = RESCUE_MEMORY_MS,
) {
    private val web = LinkedHashMap<String, Long>()
    private val rescued = LinkedHashMap<String, Long>()

    /** Zalỏ posted a notification for [code] from a message it received itself. */
    @Synchronized
    fun noteNotified(code: String, now: Long) {
        web[code] = now
        trim(web)
    }

    /**
     * Whether to post [code] read from the Zalo app's notification, and if so remember that it was. False when Zalỏ's
     * own notification for the same code is under [webWindowMs] old, or this code was already rescued within
     * [rescueMemoryMs] (Zalo re-posting the same notification with more lines, or an old line still in its history).
     */
    @Synchronized
    fun tryRescue(code: String, now: Long): Boolean {
        web[code]?.let { if (now - it in 0..webWindowMs) return false }
        rescued[code]?.let { if (now - it in 0..rescueMemoryMs) return false }
        rescued[code] = now
        trim(rescued)
        return true
    }

    private fun trim(map: LinkedHashMap<String, Long>) {
        while (map.size > MAX_ENTRIES) map.remove(map.keys.first())
    }

    companion object {
        /** Skip a rescue when Zalỏ itself announced the same code this recently. */
        const val WEB_WINDOW_MS = 2 * 60_000L

        /** A rescued code is not rescued again for this long (OTPs expire in a few minutes). */
        const val RESCUE_MEMORY_MS = 10 * 60_000L
        private const val MAX_ENTRIES = 50
    }
}

/** Everything text-like in the Zalo app's notification, read in memory and never stored or logged. */
data class ZaloNotificationText(
    val title: String?,
    /** `EXTRA_TEXT`, `EXTRA_BIG_TEXT`, each of `EXTRA_TEXT_LINES`, and the (recent) MessagingStyle messages. */
    val texts: List<String>,
)

object OtpRescue {
    /** A MessagingStyle message older than this is history, not news. */
    const val FRESH_MS = 5 * 60_000L

    /** Whether a MessagingStyle message stamped [timestamp] is recent enough to hold a code worth announcing. */
    fun isFresh(timestamp: Long, now: Long): Boolean = timestamp <= 0L || now - timestamp <= FRESH_MS

    /**
     * The code in the notification, if any. Each text is scanned alone, then the title with each text (a title
     * "Mã xác thực OTP" over a text that is only the code).
     */
    fun findCode(t: ZaloNotificationText): String? {
        val texts = t.texts.map { it.trim() }.filter { it.isNotEmpty() }.distinct()
        for (text in texts) extractOtpCode(text)?.let { return it }
        val title = t.title?.trim().orEmpty()
        if (title.isNotEmpty()) {
            extractOtpCode(title)?.let { return it }
            for (text in texts) extractOtpCode("$title\n$text")?.let { return it }
        }
        return null
    }
}

/** Which of the notification listener's two jobs run for a Zalo notification, and when the component is on at all. */
object ListenerJobs {
    enum class Job { BRIDGE, OTP_RESCUE }

    /**
     * The listener component is enabled when the bridge mode needs it (the on-phone core only) or the code rescue is
     * on. The two are independent: each switch alone is enough.
     */
    fun componentEnabled(onDevice: Boolean, mode: StayConnected, otpRescue: Boolean): Boolean =
        (onDevice && mode.usesBridge) || otpRescue

    /**
     * What to do for a notification of [kind]: the bridge only brings the core online (never in other modes, never for a
     * call), the code rescue only reads the text (never brings the core online, never for a call).
     */
    fun jobsFor(kind: ZaloNotificationRules.Kind, mode: StayConnected, otpRescue: Boolean): Set<Job> {
        if (kind != ZaloNotificationRules.Kind.MESSAGE) return emptySet()
        return buildSet {
            if (mode.usesBridge) add(Job.BRIDGE)
            if (otpRescue) add(Job.OTP_RESCUE)
        }
    }
}
