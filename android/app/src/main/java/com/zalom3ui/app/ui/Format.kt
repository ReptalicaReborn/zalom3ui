package com.zalom3ui.app.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.text.format.DateUtils
import android.util.Base64
import com.zalom3ui.app.data.ChatMessage
import java.text.DateFormat
import java.text.Normalizer
import java.util.Calendar

fun initial(name: String): String = name.trim().firstOrNull()?.uppercase() ?: "?"

/** "14:05" today, "Mon" this week, "12/3/26" before that, like a chat list. */
fun formatListTime(ts: Long, now: Long = System.currentTimeMillis()): String {
    val then = Calendar.getInstance().apply { timeInMillis = ts }
    val today = Calendar.getInstance().apply { timeInMillis = now }
    val sameDay = then.get(Calendar.YEAR) == today.get(Calendar.YEAR) &&
        then.get(Calendar.DAY_OF_YEAR) == today.get(Calendar.DAY_OF_YEAR)
    return when {
        sameDay -> DateFormat.getTimeInstance(DateFormat.SHORT).format(ts)
        now - ts < 6 * DateUtils.DAY_IN_MILLIS -> java.text.SimpleDateFormat("EEE", java.util.Locale.getDefault()).format(ts)
        else -> DateFormat.getDateInstance(DateFormat.SHORT).format(ts)
    }
}

fun formatTime(ts: Long): String =
    DateUtils.formatSameDayTime(ts, System.currentTimeMillis(), DateFormat.SHORT, DateFormat.SHORT).toString()

fun formatClock(ts: Long): String = DateFormat.getTimeInstance(DateFormat.SHORT).format(ts)

/** Header shown between messages from different days. */
fun formatDay(ts: Long, now: Long = System.currentTimeMillis()): String {
    val yesterday = Calendar.getInstance().apply {
        timeInMillis = now
        add(Calendar.DAY_OF_YEAR, -1)
    }.timeInMillis
    return when {
        sameDay(ts, now) -> "Today"
        sameDay(ts, yesterday) -> "Yesterday"
        else -> DateFormat.getDateInstance(DateFormat.MEDIUM).format(ts)
    }
}

fun sameDay(a: Long, b: Long): Boolean {
    val ca = Calendar.getInstance().apply { timeInMillis = a }
    val cb = Calendar.getInstance().apply { timeInMillis = b }
    return ca.get(Calendar.YEAR) == cb.get(Calendar.YEAR) && ca.get(Calendar.DAY_OF_YEAR) == cb.get(Calendar.DAY_OF_YEAR)
}

/** Short presence string: "Active now" / "Active 5m ago" / "Active 2h ago" / "Active yesterday". */
fun formatPresence(lastOnline: Long, now: Long = System.currentTimeMillis()): String {
    val ms = if (lastOnline < 100_000_000_000L) lastOnline * 1000 else lastOnline
    val diff = (now - ms).coerceAtLeast(0L)
    if (diff < 5 * MINUTE_MS) return "Active now"
    val mins = diff / MINUTE_MS
    if (mins < 60) return "Active ${mins}m ago"
    val yesterday = Calendar.getInstance().apply {
        timeInMillis = now
        add(Calendar.DAY_OF_YEAR, -1)
    }.timeInMillis
    val hours = diff / HOUR_MS
    if (hours < 24 && sameDay(ms, now)) return "Active ${hours}h ago"
    if (sameDay(ms, yesterday)) return "Active yesterday"
    if (hours < 24) return "Active ${hours}h ago"
    val days = diff / DAY_MS
    if (days < 7) return "Active ${days}d ago"
    val date = java.text.SimpleDateFormat("MMM d", java.util.Locale.getDefault()).format(ms)
    return "Active $date"
}

private const val MINUTE_MS = 60_000L
private const val HOUR_MS = 60 * MINUTE_MS
private const val DAY_MS = 24 * HOUR_MS

/** Compact time range for grouped call entries, e.g. "12:57–12:59" or "12:59". */
fun formatCallRange(firstTs: Long, lastTs: Long): String {
    val start = formatClock(firstTs)
    val end = formatClock(lastTs)
    return if (start == end) start else "$start–$end"
}

private val SYSTEM_MSG_TYPES = setOf(
    "chat.call.feedback",
    "chat.system",
    "chat.notice",
    "group.event",
    "chat.list.action",
)

private val CONNECTED_WITH_REGEX = Regex("^bạn vừa kết bạn với\\s+(.+)$", RegexOption.IGNORE_CASE)
private val JOINED_GROUP_REGEX = Regex("^(.+?)\\s+đã tham gia nhóm$", RegexOption.IGNORE_CASE)
private val LEFT_GROUP_REGEX = Regex("^(.+?)\\s+đã rời khỏi nhóm$", RegexOption.IGNORE_CASE)

/**
 * If [msg] is a Zalo system/service prompt (such as post-call audio feedback or a connection/group event), returns a
 * localized label to render as a centered system row instead of a chat bubble. Returns null for regular user messages.
 */
fun systemMessageLabel(msg: ChatMessage): String? {
    if (msg.recalled) return null
    if (msg.attachment?.kind == "call") return null
    val raw = msg.text.replace(Regex("\\s+"), " ").trim()
    val lowered = raw.lowercase()
    val folded = foldForSearch(raw)
    if (msg.msgType == "chat.call.feedback" || folded.contains("am thanh cuoc goi co van de")) {
        return "Call audio issue?"
    }
    if (folded == "dang nhap zalo tren may tinh thanh cong") {
        return "Logged in to Zalo on PC"
    }
    if (folded == "ban tro thanh thanh vien cua cong dong") {
        return "You joined the community"
    }
    if (folded == "cac ban da tro thanh ban be" || folded == "cac ban da ket ban tren zalo") {
        return "You are now connected on Zalo"
    }
    if (folded == "ban da roi khoi nhom") {
        return "You left the group"
    }
    CONNECTED_WITH_REGEX.matchEntire(raw)?.let { m ->
        return "You are now connected with ${m.groupValues[1].trim()}"
    }
    JOINED_GROUP_REGEX.matchEntire(raw)?.let { m ->
        return "${m.groupValues[1].trim()} joined the group"
    }
    LEFT_GROUP_REGEX.matchEntire(raw)?.let { m ->
        return "${m.groupValues[1].trim()} left the group"
    }
    if (msg.msgType in SYSTEM_MSG_TYPES || lowered.startsWith("[system]")) {
        return raw.removePrefix("[System]").removePrefix("[system]").trim().ifBlank { null }
    }
    return null
}

/** Lowercase and strip Vietnamese diacritics so "duc" finds "Đức". */
fun foldForSearch(s: String): String =
    Normalizer.normalize(s.lowercase(), Normalizer.Form.NFD)
        .replace(Regex("\\p{Mn}+"), "")
        .replace('đ', 'd')

fun decodeDataUrl(dataUrl: String): Bitmap? = runCatching {
    val bytes = Base64.decode(dataUrl.substringAfter(','), Base64.DEFAULT)
    BitmapFactory.decodeByteArray(bytes, 0, bytes.size)
}.getOrNull()

fun statusLabel(msg: ChatMessage): String? = when {
    !msg.isSelf || msg.recalled -> null
    msg.threadKind == "group" && (msg.seenCount ?: 0) > 0 -> "Seen by ${msg.seenCount}"
    msg.status == "seen" -> "Seen"
    msg.status == "delivered" -> "Delivered"
    msg.status == "sent" -> "Sent"
    else -> null
}

fun muteLabel(mutedUntil: Long?): String = when {
    mutedUntil == null -> ""
    mutedUntil == -1L -> "Muted until turned on"
    else -> "Muted until ${formatTime(mutedUntil)}"
}

/** Preview of a message for the chat list; group chats say who wrote it. */
fun preview(msg: ChatMessage?): String {
    if (msg == null) return ""
    if (msg.recalled) return "Message recalled"
    systemMessageLabel(msg)?.let { return it }
    val raw = msg.text.ifBlank { msg.attachment?.let { "📎 " + (it.title?.takeIf(String::isNotBlank) ?: "Attachment") }.orEmpty() }
    val body = raw.replace(Regex("\\s+"), " ").trim()
    return when {
        msg.threadKind != "group" -> body
        msg.isSelf -> "You: $body"
        msg.fromName.isNotBlank() -> "${msg.fromName}: $body"
        else -> body
    }
}
