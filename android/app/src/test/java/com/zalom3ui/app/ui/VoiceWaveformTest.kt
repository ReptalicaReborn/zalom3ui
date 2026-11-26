package com.zalom3ui.app.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class VoiceWaveformTest {

    @Test
    fun waveformBarsAreDeterministicAndRespectExactCount() {
        val url = "https://cdn.zaloapp.com/voice/clip-12345.m4a"
        val first = voiceWaveformBars(url)
        val second = voiceWaveformBars(url)
        assertEquals(28, first.size)
        assertEquals(first, second)

        assertEquals(1, voiceWaveformBars(url, count = 1).size)
        assertEquals(16, voiceWaveformBars(url, count = 16).size)
        assertEquals(40, voiceWaveformBars(url, count = 40).size)
        assertEquals(emptyList<Float>(), voiceWaveformBars(url, count = 0))
        assertEquals(emptyList<Float>(), voiceWaveformBars(url, count = -4))
    }

    @Test
    fun waveformBarsStayInExpectedHeightRangeAcrossSeeds() {
        val seeds = listOf(
            "",
            "a",
            "https://cdn.zaloapp.com/voice/1.m4a",
            "https://cdn.zaloapp.com/voice/2.m4a",
            "https://example.com/very/long/path/with/query?token=abcdef0123456789&sig=xyz",
            "Tin nhắn thoại tiếng Việt",
        )
        for (seed in seeds) {
            val bars = voiceWaveformBars(seed)
            assertEquals("seed='$seed' bar count", 28, bars.size)
            for (bar in bars) {
                assertTrue("bar $bar below 0.22f for seed='$seed'", bar >= 0.22f)
                assertTrue("bar $bar above 1.0f for seed='$seed'", bar <= 1.0f)
            }
            assertTrue("bars should vary naturally for seed='$seed'", bars.distinct().size > 5)
        }
        assertNotEquals(
            voiceWaveformBars("https://cdn.zaloapp.com/voice/1.m4a"),
            voiceWaveformBars("https://cdn.zaloapp.com/voice/2.m4a"),
        )
    }

    @Test
    fun playbackSpeedCyclesAndFormatsCleanly() {
        val s1 = nextPlaybackSpeed(1f)
        assertEquals(1.5f, s1, 0.0001f)
        val s2 = nextPlaybackSpeed(s1)
        assertEquals(2f, s2, 0.0001f)
        val s3 = nextPlaybackSpeed(s2)
        assertEquals(1f, s3, 0.0001f)

        assertEquals("1×", formatSpeed(1f))
        assertEquals("1.5×", formatSpeed(1.5f))
        assertEquals("2×", formatSpeed(2f))
    }
}
