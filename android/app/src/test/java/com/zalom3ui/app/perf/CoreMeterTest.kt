package com.zalom3ui.app.perf

import org.junit.Assert.assertEquals
import org.junit.Test

class CoreMeterTest {
    private fun ms(value: Long): Long = value * 1_000_000

    @Test
    fun countsRequestsLatencyAndBytes() {
        val meter = CoreMeter()
        val a = meter.requestStarted(nowNs = ms(100))
        val b = meter.requestStarted(nowNs = ms(100))
        assertEquals(2, meter.drain().inFlight)
        meter.requestFinished(a, timed = true, failed = false, bytesUp = 10, bytesDown = 100, nowNs = ms(104)) // 4 ms
        meter.requestFinished(b, timed = true, failed = false, bytesUp = 0, bytesDown = 50, nowNs = ms(120)) // 20 ms
        val totals = meter.drain()
        assertEquals(2L, totals.requests)
        assertEquals(0L, totals.errors)
        assertEquals(2L, totals.latencyCount)
        assertEquals(ms(24), totals.latencySumNs)
        assertEquals(ms(20), totals.latencyMaxNs)
        assertEquals(10L, totals.bytesUp)
        assertEquals(150L, totals.bytesDown)
        assertEquals(0, totals.inFlight)
    }

    @Test
    fun drainStartsAFreshWindowButKeepsWhatIsInFlight() {
        val meter = CoreMeter()
        val pending = meter.requestStarted(nowNs = 0)
        val done = meter.requestStarted(nowNs = 0)
        meter.requestFinished(done, timed = true, failed = true, bytesUp = 0, bytesDown = 0, nowNs = ms(5))
        val first = meter.drain()
        assertEquals(1L, first.requests)
        assertEquals(1L, first.errors)
        assertEquals(1, first.inFlight)
        val second = meter.drain()
        assertEquals(CoreTotals(inFlight = 1), second)
        meter.requestFinished(pending, timed = true, failed = false, bytesUp = 0, bytesDown = 0, nowNs = ms(7))
        assertEquals(0, meter.drain().inFlight)
    }

    @Test
    fun uploadsCountAsRequestsButNotAsLatency() {
        val meter = CoreMeter()
        val start = meter.requestStarted(nowNs = 0)
        meter.requestFinished(start, timed = false, failed = false, bytesUp = 5_000_000, bytesDown = 20, nowNs = ms(60_000))
        val totals = meter.drain()
        assertEquals(1L, totals.requests)
        assertEquals(0L, totals.latencyCount)
        assertEquals(0L, totals.latencyMaxNs)
        assertEquals(5_000_000L, totals.bytesUp)
    }

    @Test
    fun eventsAreCountedWithTheirSize() {
        val meter = CoreMeter()
        meter.eventReceived(120)
        meter.eventReceived(0)
        val totals = meter.drain()
        assertEquals(2L, totals.events)
        assertEquals(120L, totals.bytesDown)
    }

    @Test
    fun statsAreRatesPerSecondOverTheWindow() {
        val totals = CoreTotals(
            requests = 7, errors = 1, latencyCount = 7, latencySumNs = ms(70), latencyMaxNs = ms(25),
            events = 3, bytesUp = 1024, bytesDown = 4096, inFlight = 2,
        )
        // Half a second: everything is doubled per second.
        val stats = coreStats(totals, windowMs = 500.0)
        assertEquals(14f, stats.reqPerSec, 0.001f)
        assertEquals(10f, stats.avgMs, 0.001f)
        assertEquals(25f, stats.maxMs, 0.001f)
        assertEquals(2, stats.inFlight)
        assertEquals(6f, stats.eventsPerSec, 0.001f)
        assertEquals(1, stats.errors)
        assertEquals(2f, stats.upKbPerSec, 0.001f)
        assertEquals(8f, stats.downKbPerSec, 0.001f)
    }

    @Test
    fun statsOfNothingAreZero() {
        assertEquals(CoreStats(inFlight = 3), coreStats(CoreTotals(inFlight = 3), 0.0))
        assertEquals(0f, coreStats(CoreTotals(), 500.0).avgMs, 0f)
    }

    @Test
    fun windowsAddUpAndKeepTheLatestInFlight() {
        val a = CoreTotals(requests = 1, latencyCount = 1, latencySumNs = 10, latencyMaxNs = 10, inFlight = 4)
        val b = CoreTotals(requests = 2, latencyCount = 2, latencySumNs = 30, latencyMaxNs = 20, inFlight = 1)
        assertEquals(
            CoreTotals(requests = 3, latencyCount = 3, latencySumNs = 40, latencyMaxNs = 20, inFlight = 1),
            a + b,
        )
    }
}
