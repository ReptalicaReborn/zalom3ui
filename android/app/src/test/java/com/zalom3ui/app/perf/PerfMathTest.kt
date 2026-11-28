package com.zalom3ui.app.perf

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PerfMathTest {
    private val budget60 = frameBudgetNs(60f)

    private fun ms(value: Double): Long = (value * 1_000_000).toLong()

    // ---- /proc/self/stat and /proc/self/status ----

    @Test
    fun cpuTicksAddsUtimeAndStime() {
        val stat = "4242 (com.zalom3ui.app) S 1 2 3 4 5 6 7 8 9 10 111 222 0 0 20 0 46 0 12345 987654321"
        assertEquals(333L, ProcParse.cpuTicks(stat))
    }

    @Test
    fun cpuTicksCountsFromTheLastClosingParenthesis() {
        // The command name holds a space and a closing parenthesis; counting from the first ')' would misplace every field.
        val stat = "4242 (a b) c) S 1 2 3 4 5 6 7 8 9 10 111 222 0 0 20 0 46 0 12345 987654321"
        assertEquals(333L, ProcParse.cpuTicks(stat))
        assertEquals(333L, ProcParse.cpuTicks("7 ((x)) R 1 2 3 4 5 6 7 8 9 10 111 222 0 0\n"))
    }

    @Test
    fun cpuTicksIsNullForTextThatIsNotAStatLine() {
        assertNull(ProcParse.cpuTicks(""))
        assertNull(ProcParse.cpuTicks("no parenthesis here"))
        assertNull(ProcParse.cpuTicks("4242 (short) S 1 2 3"))
        assertNull(ProcParse.cpuTicks("4242 (x) S 1 2 3 4 5 6 7 8 9 10 abc 222 0 0"))
    }

    @Test
    fun statusReadsRssPeakAndThreads() {
        val text = """
            Name:	zalom3ui.app
            VmPeak:	 9876543 kB
            VmHWM:	  450000 kB
            VmRSS:	  401234 kB
            RssAnon:	  200000 kB
            Threads:	46
            SigQ:	0/12345
        """.trimIndent()
        assertEquals(ProcStatus(rssKb = 401_234, hwmKb = 450_000, threads = 46), ProcParse.status(text))
    }

    @Test
    fun statusReportsMissingFieldsAsMinusOne() {
        assertEquals(ProcStatus(-1, -1, -1), ProcParse.status(""))
        assertEquals(ProcStatus(rssKb = 10, hwmKb = -1, threads = -1), ProcParse.status("VmRSS:\t10 kB\nVmHWM:\tgarbage kB\n"))
    }

    // ---- CPU ----

    @Test
    fun cpuPercentIsPercentOfOneCore() {
        // 50 ticks at 100 per second is half a second of CPU; over one second of wall time that is 50%.
        assertEquals(50f, cpuPercent(50, 1000.0, 100), 0.001f)
        // 250 ticks in one second: 2.5 cores busy.
        assertEquals(250f, cpuPercent(250, 1000.0, 100), 0.001f)
        // The same ticks over a 500 ms window are twice the share.
        assertEquals(100f, cpuPercent(50, 500.0, 100), 0.001f)
        assertEquals(0f, cpuPercent(0, 500.0, 100), 0.001f)
    }

    @Test
    fun cpuPercentIgnoresNonsenseInput() {
        assertEquals(0f, cpuPercent(-1, 1000.0, 100), 0f)
        assertEquals(0f, cpuPercent(10, 0.0, 100), 0f)
        assertEquals(0f, cpuPercent(10, 1000.0, 0), 0f)
    }

    // ---- frames ----

    @Test
    fun frameBudgetIsOneRefreshAndFallsBackTo60Hz() {
        assertEquals(16_666_666L, frameBudgetNs(60f))
        assertEquals(8_333_333L, frameBudgetNs(120f))
        assertEquals(DEFAULT_BUDGET_NS, frameBudgetNs(0f))
        assertEquals(DEFAULT_BUDGET_NS, frameBudgetNs(Float.NaN))
        assertEquals(DEFAULT_BUDGET_NS, frameBudgetNs(1000f))
    }

    @Test
    fun aWindowOfFramesGivesFpsAverageMaxAndJank() {
        val acc = FrameAccumulator(budget60)
        // Two frames within the 16.7 ms budget, two over it.
        for (d in listOf(8.0, 10.0, 20.0, 40.0)) acc.add(ms(d))
        val stats = frameStats(acc.drain(), windowMs = 500.0)
        assertEquals(8f, stats.fps, 0.001f) // 4 frames in half a second
        assertEquals(19.5f, stats.avgMs, 0.001f)
        assertEquals(40f, stats.maxMs, 0.001f)
        assertEquals(2, stats.slow)
        assertEquals(50f, stats.jankPct, 0.001f)
    }

    @Test
    fun aFrameExactlyOnTheBudgetIsNotJank() {
        val acc = FrameAccumulator(budget60)
        acc.add(budget60)
        acc.add(budget60 + 1)
        assertEquals(1, acc.drain().slow)
    }

    @Test
    fun drainStartsANewWindowAndAnEmptyWindowIsAllZero() {
        val acc = FrameAccumulator(budget60)
        acc.add(ms(30.0))
        assertEquals(1, acc.drain().count)
        val empty = acc.drain()
        assertEquals(FrameTotals(), empty)
        val stats = frameStats(empty, 500.0)
        assertEquals(0f, stats.fps, 0f)
        assertEquals(0f, stats.avgMs, 0f)
        assertEquals(0f, stats.jankPct, 0f)
        assertEquals(0f, frameStats(FrameTotals(count = 3, sumNs = 3), 0.0).fps, 0f)
    }

    @Test
    fun aSecondReportForTheSameVsyncIsNotAnotherFrame() {
        val acc = FrameAccumulator(budget60)
        acc.add(ms(5.0), vsyncNs = 1_000)
        acc.add(ms(30.0), vsyncNs = 1_000) // same vsync: the same frame on screen
        acc.add(ms(6.0), vsyncNs = 2_000)
        val totals = acc.drain()
        assertEquals(2, totals.count)
        assertEquals(1, totals.dup)
        assertEquals(ms(11.0), totals.sumNs) // the repeat adds neither to the count nor to the average
        assertEquals(ms(30.0), totals.maxNs) // but a long repeat still counts for the worst frame
        // A window never reports more frames than there were vsyncs.
        assertEquals(4f, frameStats(totals, windowMs = 500.0).fps, 0.001f)
        assertEquals(1, frameStats(totals, windowMs = 500.0).dup)
    }

    @Test
    fun theLastVsyncIsRememberedAcrossWindowsAndUnknownVsyncsNeverMatch() {
        val acc = FrameAccumulator(budget60)
        acc.add(ms(5.0), vsyncNs = 7_000)
        acc.drain()
        acc.add(ms(5.0), vsyncNs = 7_000)
        assertEquals(0, acc.drain().count)
        // Without a vsync (-1) every report is its own frame.
        acc.add(ms(5.0))
        acc.add(ms(5.0))
        assertEquals(2, acc.drain().count)
    }

    @Test
    fun theFramesOwnDeadlineReplacesTheRefreshBudgetWhenBelievable() {
        val acc = FrameAccumulator(budget60) // 16.7 ms
        acc.add(ms(10.0), vsyncNs = 1, deadlineNs = ms(8.3)) // over its deadline (an adaptive display at 120 Hz)
        acc.add(ms(10.0), vsyncNs = 2, deadlineNs = ms(33.3)) // fine against a 30 Hz deadline
        assertEquals(1, acc.drain().slow)
        // Nonsense deadlines (negative, zero, over 100 ms) fall back to the refresh-rate budget.
        for ((i, d) in listOf(-5L, 0L, ms(500.0)).withIndex()) acc.add(ms(20.0), vsyncNs = 10L + i, deadlineNs = d)
        assertEquals(3, acc.drain().slow)
    }

    @Test
    fun theBudgetCanChangeWhileMeasuring() {
        val acc = FrameAccumulator(budget60)
        acc.add(ms(10.0)) // fine at 60 Hz
        acc.budgetNs = frameBudgetNs(120f)
        acc.add(ms(10.0)) // over the 8.3 ms of 120 Hz
        assertEquals(1, acc.drain().slow)
    }

    @Test
    fun windowsAddUp() {
        val a = FrameTotals(count = 2, sumNs = 30, maxNs = 20, slow = 1)
        val b = FrameTotals(count = 3, sumNs = 45, maxNs = 25, slow = 0)
        assertEquals(FrameTotals(count = 5, sumNs = 75, maxNs = 25, slow = 1), a + b)
    }

    @Test
    fun negativeDurationsAreIgnored() {
        val acc = FrameAccumulator(budget60)
        acc.add(-5)
        assertEquals(0, acc.drain().count)
    }

    // ---- ring buffer ----

    @Test
    fun ringKeepsTheNewestValuesOldestFirst() {
        val ring = FloatRing(3)
        assertArrayEquals(floatArrayOf(), ring.toArray(), 0f)
        ring.add(1f)
        ring.add(2f)
        assertArrayEquals(floatArrayOf(1f, 2f), ring.toArray(), 0f)
        ring.add(3f)
        ring.add(4f)
        ring.add(5f)
        assertArrayEquals(floatArrayOf(3f, 4f, 5f), ring.toArray(), 0f)
        assertEquals(3, ring.size)
        assertEquals(5f, ring.max(), 0f)
        ring.add(0f)
        assertArrayEquals(floatArrayOf(4f, 5f, 0f), ring.toArray(), 0f)
    }

    @Test
    fun ringMaxOfNothingIsZero() {
        assertEquals(0f, FloatRing(4).max(), 0f)
    }

    // ---- rate limiter, levels, names ----

    @Test
    fun rateLimiterLetsOneEventThroughPerGap() {
        val limiter = RateLimiter(1_000)
        assertTrue(limiter.allow(5_000))
        assertFalse(limiter.allow(5_500))
        assertFalse(limiter.allow(5_999))
        assertTrue(limiter.allow(6_000))
        assertTrue(RateLimiter(1_000).allow(0))
    }

    @Test
    fun levelsFollowTheirThresholds() {
        assertEquals(Level.GOOD, frameLevel(16.6f, 16.7f))
        assertEquals(Level.WARN, frameLevel(30f, 16.7f))
        assertEquals(Level.BAD, frameLevel(40f, 16.7f))
        assertEquals(Level.GOOD, jankLevel(4.9f))
        assertEquals(Level.WARN, jankLevel(10f))
        assertEquals(Level.BAD, jankLevel(20f))
        assertEquals(Level.GOOD, cpuLevel(59f))
        assertEquals(Level.WARN, cpuLevel(99f))
        assertEquals(Level.BAD, cpuLevel(100f))
        assertEquals(Level.GOOD, latencyLevel(49f))
        assertEquals(Level.WARN, latencyLevel(150f))
        assertEquals(Level.BAD, latencyLevel(200f))
    }

    @Test
    fun memoryLevelIsBadWhenLowAndWarnNearTheThreshold() {
        assertEquals(Level.BAD, memoryLevel(1_000, 100, lowMemory = true))
        assertEquals(Level.WARN, memoryLevel(150, 100, lowMemory = false))
        assertEquals(Level.GOOD, memoryLevel(200, 100, lowMemory = false))
        assertEquals(Level.GOOD, memoryLevel(-1, -1, lowMemory = false))
    }

    @Test
    fun trimLevelsHaveTheirAndroidNames() {
        assertEquals("RUNNING_LOW", trimLevelName(10))
        assertEquals("UI_HIDDEN", trimLevelName(20))
        assertEquals("COMPLETE", trimLevelName(80))
        assertEquals("LEVEL_7", trimLevelName(7))
    }

    @Test
    fun thermalStatusNames() {
        assertEquals("none", thermalName(0))
        assertEquals("severe", thermalName(3))
        assertEquals("n/a", thermalName(-1))
    }

    // ---- dragging ----

    @Test
    fun theCardStaysInsideItsContainer() {
        // 1000 x 2000 container, 400 x 300 card anchored at the top end: it can move 600 left and 1700 down.
        assertEquals(0f to 0f, clampCardOffset(50f, -20f, 1000, 2000, 400, 300))
        assertEquals(-600f to 1700f, clampCardOffset(-900f, 5000f, 1000, 2000, 400, 300))
        assertEquals(-100f to 250f, clampCardOffset(-100f, 250f, 1000, 2000, 400, 300))
        // A card bigger than the container can't move.
        assertEquals(0f to 0f, clampCardOffset(-50f, 50f, 300, 200, 400, 300))
    }

    // ---- log trimming ----

    @Test
    fun trimmingDropsTheHalfLineAtTheCut() {
        assertEquals(8, LogTrim.firstLineStart("partial\nline1\nline2\n".toByteArray()))
        assertEquals(0, LogTrim.firstLineStart("no newline at all".toByteArray()))
        // A newline only as the last byte would leave nothing: keep the whole tail.
        assertEquals(0, LogTrim.firstLineStart("one line\n".toByteArray()))
        assertEquals(0, LogTrim.firstLineStart(ByteArray(0)))
    }
}
