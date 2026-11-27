package com.zalom3ui.app.server

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class StartupTimingTest {
    @Test
    fun cold_start_needs_fresh_launch_soon_after_process_start() {
        assertTrue(StartupTiming.isColdStart(1_000, 1_300, hasSavedState = false))
        assertTrue(StartupTiming.isColdStart(1_000, 6_000, hasSavedState = false))
        assertFalse(StartupTiming.isColdStart(1_000, 6_001, hasSavedState = false))
        assertFalse(StartupTiming.isColdStart(1_000, 1_300, hasSavedState = true))
        assertFalse(StartupTiming.isColdStart(1_000, 900, hasSavedState = false))
    }

    @Test
    fun since_start_is_null_for_moments_that_have_not_happened() {
        assertEquals(250L, StartupTiming.sinceStart(1_000, 1_250))
        assertNull(StartupTiming.sinceStart(1_000, 0))
        assertNull(StartupTiming.sinceStart(1_000, 900))
    }

    @Test
    fun format_matches_the_documented_line() {
        assertEquals(
            "startup: process→first-frame 600 ms, process→core-ready 450 ms, process→list-data 900 ms (threads=1)",
            StartupTiming.format(600, 450, 900, 1),
        )
        assertEquals(
            "startup: process→first-frame n/a, process→core-ready n/a, process→list-data 900 ms (threads=3)",
            StartupTiming.format(null, null, 900, 3),
        )
    }

    @Test
    fun marks_are_listed_in_time_order_since_process_start() {
        val marks = mapOf("state-applied" to 1_410L, "vm-created" to 1_190L, "status-ready" to 1_230L, "never" to 0L)
        assertEquals("vm-created 190, status-ready 230, state-applied 410", StartupTiming.formatMarks(1_000, marks))
        assertEquals("", StartupTiming.formatMarks(1_000, emptyMap()))
    }

    @Test
    fun format_appends_the_steps_when_there_are_some() {
        assertEquals(
            "startup: process→first-frame 105 ms, process→core-ready 78 ms, process→list-data 510 ms (threads=102); " +
                "at ms: vm-created 190",
            StartupTiming.format(105, 78, 510, 102, steps = "vm-created 190"),
        )
    }

    @Test
    fun a_mark_keeps_its_first_time() {
        StartupTiming.mark("test-only-mark", 100)
        StartupTiming.mark("test-only-mark", 900)
        assertEquals(100L, StartupTiming.snapshotMarks()["test-only-mark"])
    }
}
