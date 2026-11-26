package com.zalom3ui.app.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class VoiceRecorderTest {

    @Test
    fun normalizeAmplitudeMapsZeroMidMaxAndClampsOutOfRange() {
        assertEquals(0.08f, normalizeAmplitude(0), 0.0001f)
        assertEquals(0.08f, normalizeAmplitude(-1), 0.0001f)
        assertEquals(0.08f, normalizeAmplitude(-50_000), 0.0001f)

        val quiet = normalizeAmplitude(1000)
        val quarter = normalizeAmplitude(8192)
        val half = normalizeAmplitude(16384)
        assertTrue("quiet speech should lift above floor: $quiet", quiet > 0.15f && quiet < quarter)
        assertTrue("quarter amplitude should be mid-range: $quarter", quarter > 0.45f && quarter < 0.65f)
        assertTrue("half amplitude should exceed quarter: $half", half > quarter && half < 1.0f)

        assertEquals(1.0f, normalizeAmplitude(32767), 0.0001f)
        assertEquals(1.0f, normalizeAmplitude(50_000), 0.0001f)
    }

    @Test
    fun formatRecordingDurationFormatsZeroSubSecondSecondsAndMinutes() {
        assertEquals("0:00", formatRecordingDuration(0L))
        assertEquals("0:00", formatRecordingDuration(-250L))
        assertEquals("0:00", formatRecordingDuration(950L))
        assertEquals("0:01", formatRecordingDuration(1000L))
        assertEquals("0:05", formatRecordingDuration(5000L))
        assertEquals("0:14", formatRecordingDuration(14_999L))
        assertEquals("1:05", formatRecordingDuration(65_000L))
        assertEquals("10:00", formatRecordingDuration(600_000L))
    }
}
