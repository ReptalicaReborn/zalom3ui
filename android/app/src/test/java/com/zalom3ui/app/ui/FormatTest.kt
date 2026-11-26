package com.zalom3ui.app.ui

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import java.text.SimpleDateFormat
import java.util.Calendar
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
}
