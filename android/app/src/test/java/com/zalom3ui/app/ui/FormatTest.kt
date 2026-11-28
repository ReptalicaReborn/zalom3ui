package com.zalom3ui.app.ui

import com.zalom3ui.app.data.Attachment
import com.zalom3ui.app.data.ChatMessage
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import java.text.DateFormat
import java.text.Normalizer
import java.text.SimpleDateFormat
import java.util.Calendar
import java.util.Date
import java.util.Locale
import java.util.TimeZone

/** The cached weekday / month-day formatters print what SimpleDateFormat printed, and follow locale and zone changes. */
class FormatTest {
    private lateinit var savedLocale: Locale
    private lateinit var savedZone: TimeZone

    @Before
    fun saveDefaults() {
        savedLocale = Locale.getDefault()
        savedZone = TimeZone.getDefault()
        TimeZone.setDefault(TimeZone.getTimeZone("Asia/Ho_Chi_Minh"))
    }

    @After
    fun restoreDefaults() {
        Locale.setDefault(savedLocale)
        TimeZone.setDefault(savedZone)
    }

    private fun msg(
        text: String,
        msgType: String = "webchat",
        recalled: Boolean = false,
        attachment: Attachment? = null,
    ) = ChatMessage(
        id = "1",
        threadId = "t",
        threadKind = "user",
        ts = 1L,
        msgType = msgType,
        text = text,
        recalled = recalled,
        attachment = attachment,
    )

    @Test
    fun weekdayAndMonthDayMatchSimpleDateFormatPerLocale() {
        // Switching the locale between rounds also catches a formatter cached for the previous locale.
        for (tag in listOf("en-US", "vi-VN", "de-DE", "fr-FR", "en-US")) {
            val locale = Locale.forLanguageTag(tag)
            Locale.setDefault(locale)
            val now = Calendar.getInstance().apply {
                set(2026, Calendar.NOVEMBER, 26, 15, 0, 0)
                set(Calendar.MILLISECOND, 0)
            }.timeInMillis
            val threeDaysAgo = now - 3 * 86_400_000L
            val tenDaysAgo = now - 10 * 86_400_000L
            assertEquals(tag, SimpleDateFormat("EEE", locale).format(threeDaysAgo), formatListTime(threeDaysAgo, now))
            assertEquals(tag, "Active " + SimpleDateFormat("MMM d", locale).format(tenDaysAgo), formatPresence(tenDaysAgo, now))
        }
    }

    @Test
    fun monthDayFollowsTimeZoneChanges() {
        Locale.setDefault(Locale.US)
        // 2026-11-15 20:00 UTC: already Nov 16 in Ho Chi Minh City, still Nov 15 in Los Angeles.
        val ts = 1_794_772_800_000L
        val now = ts + 10 * 86_400_000L
        assertEquals("Active Nov 16", formatPresence(ts, now))
        TimeZone.setDefault(TimeZone.getTimeZone("America/Los_Angeles"))
        assertEquals("Active Nov 15", formatPresence(ts, now))
    }

    @Test
    fun clockListTimeAndDayFormattersFollowLocaleAndTimeZoneChanges() {
        val ts = 1_794_772_800_000L // 2026-11-15 20:00 UTC
        val now = ts + 10 * 86_400_000L
        for (zoneId in listOf("Asia/Ho_Chi_Minh", "America/Los_Angeles", "UTC")) {
            val tz = TimeZone.getTimeZone(zoneId)
            TimeZone.setDefault(tz)
            for (tag in listOf("en-US", "vi-VN", "de-DE")) {
                val locale = Locale.forLanguageTag(tag)
                Locale.setDefault(locale)
                val expectedClock = DateFormat.getTimeInstance(DateFormat.SHORT, locale).apply { timeZone = tz }.format(Date(ts))
                val expectedShortDate = DateFormat.getDateInstance(DateFormat.SHORT, locale).apply { timeZone = tz }.format(Date(ts))
                val expectedMedDate = DateFormat.getDateInstance(DateFormat.MEDIUM, locale).apply { timeZone = tz }.format(Date(ts))

                assertEquals("$zoneId/$tag clock", expectedClock, formatClock(ts))
                assertEquals("$zoneId/$tag list same day", expectedClock, formatListTime(ts, ts + 60_000L))
                assertEquals("$zoneId/$tag list older", expectedShortDate, formatListTime(ts, now))
                assertEquals("$zoneId/$tag day older", expectedMedDate, formatDay(ts, now))
                assertEquals("$zoneId/$tag dayKey older", expectedMedDate, formatDayKey(dayKey(ts), now))
            }
        }
    }

    @Test
    fun dayKeyAndFormatDayHandleTodayYesterdayAndZoneBoundaries() {
        Locale.setDefault(Locale.US)
        val now = Calendar.getInstance().apply {
            set(2026, Calendar.JANUARY, 1, 10, 0, 0)
            set(Calendar.MILLISECOND, 0)
        }.timeInMillis
        val earlierToday = now - 3_600_000L
        val yesterday = now - 15 * 3_600_000L // Dec 31, 2025
        val twoDaysAgo = now - 48 * 3_600_000L

        assertTrue(sameDay(earlierToday, now))
        assertEquals(dayKey(earlierToday), dayKey(now))
        assertFalse(sameDay(yesterday, now))
        assertNotEquals(dayKey(yesterday), dayKey(now))

        assertEquals("Today", formatDay(earlierToday, now))
        assertEquals("Today", formatDayKey(dayKey(earlierToday), now))
        assertEquals("Yesterday", formatDay(yesterday, now))
        assertEquals("Yesterday", formatDayKey(dayKey(yesterday), now))
        assertEquals(formatDay(twoDaysAgo, now), formatDayKey(dayKey(twoDaysAgo), now))

        // 2026-11-15 20:00 UTC (Nov 16 03:00 in ICT) vs 2026-11-15 15:00 UTC (Nov 15 22:00 in ICT):
        // Different days in ICT, same day in UTC.
        val a = 1_794_772_800_000L
        val b = a - 5 * 3_600_000L
        assertFalse(sameDay(a, b))
        TimeZone.setDefault(TimeZone.getTimeZone("UTC"))
        assertTrue(sameDay(a, b))
    }

    @Test
    fun systemMessageLabelCoversAllPatternsAndFastPathRejectsNormalText() {
        assertEquals("Call audio issue?", systemMessageLabel(msg("", msgType = "chat.call.feedback")))
        assertEquals("Call audio issue?", systemMessageLabel(msg("  Âm   thanh cuộc gọi có vấn đề?  ")))
        assertEquals(
            "Call audio issue?",
            systemMessageLabel(msg(Normalizer.normalize("Âm thanh cuộc gọi có vấn đề", Normalizer.Form.NFD))),
        )
        assertEquals("Logged in to Zalo on PC", systemMessageLabel(msg("Đăng nhập Zalo trên máy tính thành công")))
        assertEquals("You joined the community", systemMessageLabel(msg("Bạn trở thành thành viên của cộng đồng")))
        assertEquals("You are now connected on Zalo", systemMessageLabel(msg("Các bạn đã trở thành bạn bè")))
        assertEquals("You are now connected on Zalo", systemMessageLabel(msg("Các bạn đã kết bạn trên Zalo")))
        assertEquals("You left the group", systemMessageLabel(msg("Bạn đã rời khỏi nhóm")))
        assertEquals("You left the group", systemMessageLabel(msg("BAN DA ROI KHOI NHOM")))
        assertEquals("You are now connected with Minh", systemMessageLabel(msg("Bạn vừa kết bạn với   Minh ")))
        assertEquals("An joined the group", systemMessageLabel(msg("An đã tham gia nhóm")))
        assertEquals("Bình left the group", systemMessageLabel(msg("Bình đã rời khỏi nhóm")))
        assertEquals("Group renamed", systemMessageLabel(msg("[System] Group renamed")))
        assertEquals("Notice text", systemMessageLabel(msg("[system] Notice text")))
        assertEquals("Server notice", systemMessageLabel(msg("Server notice", msgType = "chat.system")))
        assertNull(systemMessageLabel(msg("   ", msgType = "chat.system")))
        assertNull(systemMessageLabel(msg("[system]   ")))
        assertNull(systemMessageLabel(msg("Âm thanh cuộc gọi có vấn đề?", recalled = true)))
        assertNull(systemMessageLabel(msg("Âm thanh cuộc gọi có vấn đề?", attachment = Attachment(kind = "call"))))

        // Ordinary user messages (short, long, English, Vietnamese) must return null.
        for (normal in listOf(
            "",
            "   ",
            "ok",
            "Hello there!",
            "Are we still meeting at 7pm tonight for dinner?",
            "Hôm nay đi ăn phở không bạn ơi, mình đói quá rồi",
            "Bạn ơi tối nay rảnh không qua nhà mình chơi nhé",
            "Các bạn nhớ mang theo laptop để họp chiều nay nhé",
            "[Photo]",
        )) {
            assertNull("expected null for '$normal'", systemMessageLabel(msg(normal)))
        }
    }
}
