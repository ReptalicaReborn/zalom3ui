package com.zalom3ui.app.ui

import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.text.format.DateUtils
import android.util.Base64
import com.zalom3ui.app.data.ChatMessage
import java.text.DateFormat
import java.text.Normalizer
import java.time.Instant
import java.time.ZoneId
import java.time.format.DateTimeFormatter
import java.time.format.DecimalStyle
import java.util.Calendar
import java.util.Date
import java.util.Locale
import java.util.TimeZone

/** Built once rather than on every row and recomposition. */
private val SPACE_RUN_REGEX = Regex("\\s+")
private val COMBINING_MARKS_REGEX = Regex("\\p{Mn}+")

/**
 * A fixed date pattern built once per locale instead of on every row. java.time formatters are immutable and
 * thread-safe. The locale's digits are kept so the text matches SimpleDateFormat, and the zone is read on every
 * call so a time zone change still applies.
 */
private class PatternFormatter(private val pattern: String) {
    private class Entry(val locale: Locale, val formatter: DateTimeFormatter)

    @Volatile private var cached: Entry? = null

    fun format(ms: Long): String {
        val locale = Locale.getDefault()
        val entry = cached?.takeIf { it.locale == locale }
            ?: Entry(locale, DateTimeFormatter.ofPattern(pattern, locale).withDecimalStyle(DecimalStyle.of(locale)))
                .also { cached = it }
        return entry.formatter.format(Instant.ofEpochMilli(ms).atZone(ZoneId.systemDefault()))
    }
}

/**
 * Per-thread cached [DateFormat] that refreshes whenever the default [Locale] or [TimeZone] changes,
 * avoiding per-row formatter and [Date] allocations while remaining 100% thread-safe.
 */
private class ThreadDateFormat(private val create: (Locale) -> DateFormat) {
    private class Entry(val locale: Locale, var zone: TimeZone, val format: DateFormat, val date: Date)

    private val local = ThreadLocal<Entry>()

    fun format(ms: Long): String {
        val locale = Locale.getDefault()
        val zone = TimeZone.getDefault()
        var entry = local.get()
        if (entry == null || entry.locale != locale) {
            val df = create(locale).apply { timeZone = zone }
            entry = Entry(locale, zone, df, Date(ms))
            local.set(entry)
        } else {
            if (entry.zone != zone) {
                entry.format.timeZone = zone
                entry.zone = zone
            }
            entry.date.time = ms
        }
        return entry.format.format(entry.date)
    }
}

private val WEEKDAY_FORMAT = PatternFormatter("EEE")
private val MONTH_DAY_FORMAT = PatternFormatter("MMM d")
private val TIME_SHORT_FORMAT = ThreadDateFormat { DateFormat.getTimeInstance(DateFormat.SHORT, it) }
private val DATE_SHORT_FORMAT = ThreadDateFormat { DateFormat.getDateInstance(DateFormat.SHORT, it) }
private val DATE_MEDIUM_FORMAT = ThreadDateFormat { DateFormat.getDateInstance(DateFormat.MEDIUM, it) }

private val THREAD_CALENDAR = ThreadLocal<Calendar>()

private fun threadCalendar(ts: Long): Calendar {
    val zone = TimeZone.getDefault()
    var cal = THREAD_CALENDAR.get()
    if (cal == null) {
        cal = Calendar.getInstance(zone)
        THREAD_CALENDAR.set(cal)
    } else if (cal.timeZone != zone) {
        cal.timeZone = zone
    }
    cal.timeInMillis = ts
    return cal
}

/** Compact local-timezone calendar day bucket (`year * 1000 + dayOfYear`) for allocation-free same-day checks. */
fun dayKey(ts: Long): Long {
    val cal = threadCalendar(ts)
    return cal.get(Calendar.YEAR) * 1000L + cal.get(Calendar.DAY_OF_YEAR)
}

private fun yesterdayDayKey(now: Long): Long {
    val cal = threadCalendar(now)
    cal.add(Calendar.DAY_OF_YEAR, -1)
    return cal.get(Calendar.YEAR) * 1000L + cal.get(Calendar.DAY_OF_YEAR)
}

fun initial(name: String): String = name.trim().firstOrNull()?.uppercase() ?: "?"

/** "14:05" today, "Mon" this week, "12/3/26" before that, like a chat list. */
fun formatListTime(ts: Long, now: Long = System.currentTimeMillis()): String = when {
    dayKey(ts) == dayKey(now) -> TIME_SHORT_FORMAT.format(ts)
    now - ts < 6 * DAY_MS -> WEEKDAY_FORMAT.format(ts)
    else -> DATE_SHORT_FORMAT.format(ts)
}

fun formatTime(ts: Long): String =
    DateUtils.formatSameDayTime(ts, System.currentTimeMillis(), DateFormat.SHORT, DateFormat.SHORT).toString()

fun formatClock(ts: Long): String = TIME_SHORT_FORMAT.format(ts)

/** Header shown between messages from different days. */
fun formatDay(ts: Long, now: Long = System.currentTimeMillis()): String = when (dayKey(ts)) {
    dayKey(now) -> "Today"
    yesterdayDayKey(now) -> "Yesterday"
    else -> DATE_MEDIUM_FORMAT.format(ts)
}

/** Header shown for a [dayKey] bucket, matching [formatDay] without needing an exact millisecond timestamp. */
fun formatDayKey(key: Long, now: Long = System.currentTimeMillis()): String = when (key) {
    dayKey(now) -> "Today"
    yesterdayDayKey(now) -> "Yesterday"
    else -> {
        val cal = threadCalendar(now)
        cal.clear()
        cal.set(Calendar.YEAR, (key / 1000L).toInt())
        cal.set(Calendar.DAY_OF_YEAR, (key % 1000L).toInt())
        cal.set(Calendar.HOUR_OF_DAY, 12)
        DATE_MEDIUM_FORMAT.format(cal.timeInMillis)
    }
}

fun sameDay(a: Long, b: Long): Boolean = dayKey(a) == dayKey(b)

/** Short presence string: "Active now" / "Active 5m ago" / "Active 2h ago" / "Active yesterday". */
fun formatPresence(lastOnline: Long, now: Long = System.currentTimeMillis()): String {
    val ms = if (lastOnline < 100_000_000_000L) lastOnline * 1000 else lastOnline
    val diff = (now - ms).coerceAtLeast(0L)
    if (diff < 5 * MINUTE_MS) return "Active now"
    val mins = diff / MINUTE_MS
    if (mins < 60) return "Active ${mins}m ago"
    val hours = diff / HOUR_MS
    val msDay = dayKey(ms)
    if (hours < 24 && msDay == dayKey(now)) return "Active ${hours}h ago"
    if (msDay == yesterdayDayKey(now)) return "Active yesterday"
    if (hours < 24) return "Active ${hours}h ago"
    val days = diff / DAY_MS
    if (days < 7) return "Active ${days}d ago"
    return "Active ${MONTH_DAY_FORMAT.format(ms)}"
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
 * Zero-allocation pre-filter for [systemMessageLabel] on ordinary [ChatMessage.msgType] rows so normal messages
 * skip whitespace collapsing, ICU NFD normalization, and regex matching entirely.
 */
private fun couldBeSystemMessageText(text: String, start: Int, end: Int): Boolean {
    val len = end - start
    if (len < 8) return false
    val first = text[start]
    if (first == '[' && text.regionMatches(start, "[system]", 0, 8, ignoreCase = true)) return true
    if (len < 18) return false
    if (text.regionMatches(end - 4, "nhóm", 0, 4, ignoreCase = true)) return true
    if (len >= 21 && text.regionMatches(start, "bạn", 0, 3, ignoreCase = true)) return true
    val last = text[end - 1]
    if (len >= 20 && (first == 'b' || first == 'B') && (last == 'm' || last == 'M')) return true
    if (len >= 28 && (first == 'c' || first == 'C') && (last == 'o' || last == 'O')) return true
    if (len >= 27 && containsThanhStem(text, start, end)) return true
    return false
}

/** True if `text[start until end]` contains `th<1..3 chars>nh` (case-insensitive), shared by four Vietnamese prompts. */
private fun containsThanhStem(text: String, start: Int, end: Int): Boolean {
    val limit = end - 5
    var i = start
    while (i <= limit) {
        val c0 = text[i]
        if ((c0 == 't' || c0 == 'T') && (text[i + 1] == 'h' || text[i + 1] == 'H')) {
            val maxGapEnd = minOf(i + 5, end - 2)
            var j = i + 3
            while (j <= maxGapEnd) {
                val cn = text[j]
                if ((cn == 'n' || cn == 'N') && (text[j + 1] == 'h' || text[j + 1] == 'H')) {
                    return true
                }
                j++
            }
        }
        i++
    }
    return false
}

/**
 * If [msg] is a Zalo system/service prompt (such as post-call audio feedback or a connection/group event), returns a
 * localized label to render as a centered system row instead of a chat bubble. Returns null for regular user messages.
 */
fun systemMessageLabel(msg: ChatMessage): String? {
    if (msg.recalled) return null
    if (msg.attachment?.kind == "call") return null
    if (msg.msgType == "chat.call.feedback") return "Call audio issue?"
    msg.attachment?.poll?.takeIf { it.event != null }?.let { return pollEventLabel(msg, it.question) }
    val text = msg.text
    var start = 0
    var end = text.length
    while (start < end && text[start].isWhitespace()) start++
    while (end > start && text[end - 1].isWhitespace()) end--
    if (start == end) return null
    val isSystemType = msg.msgType in SYSTEM_MSG_TYPES
    if (!isSystemType && !couldBeSystemMessageText(text, start, end)) return null

    val raw = text.substring(start, end).replace(SPACE_RUN_REGEX, " ")
    val folded = foldForSearch(raw)
    if (folded.contains("am thanh cuoc goi co van de")) {
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
    if (isSystemType || raw.startsWith("[system]", ignoreCase = true)) {
        return raw.removePrefix("[System]").removePrefix("[system]").trim().ifBlank { null }
    }
    return null
}

/** A vote or close on a poll: a system line, not a bubble (the poll itself is the card it points to). */
private fun pollEventLabel(msg: ChatMessage, question: String): String {
    val who = when {
        msg.isSelf -> "You"
        msg.fromName.isNotBlank() -> msg.fromName.trim()
        else -> null
    }
    val q = question.replace(SPACE_RUN_REGEX, " ").trim()
    return when {
        who != null && q.isNotEmpty() -> "$who updated the poll: $q"
        who != null -> "$who updated a poll"
        q.isNotEmpty() -> "Poll updated: $q"
        else -> "Poll updated"
    }
}

/** Lowercase and strip Vietnamese diacritics so "duc" finds "Đức". */
fun foldForSearch(s: String): String =
    Normalizer.normalize(s.lowercase(), Normalizer.Form.NFD)
        .replace(COMBINING_MARKS_REGEX, "")
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
    val body = raw.replace(SPACE_RUN_REGEX, " ").trim()
    return when {
        msg.threadKind != "group" -> body
        msg.isSelf -> "You: $body"
        msg.fromName.isNotBlank() -> "${msg.fromName}: $body"
        else -> body
    }
}

private const val UNSUPPORTED_LABEL = "[Unsupported message]"

/**
 * Text shown in a message bubble: an unsupported message names its Zalo `msgType`
 * ("[Unsupported message · chat.xyz]") so an unknown record can be identified from a screenshot.
 * The thread list keeps the short server text.
 */
fun bubbleText(msg: ChatMessage): String =
    if (msg.text == UNSUPPORTED_LABEL && msg.msgType.isNotBlank()) "[Unsupported message · ${msg.msgType}]" else msg.text
