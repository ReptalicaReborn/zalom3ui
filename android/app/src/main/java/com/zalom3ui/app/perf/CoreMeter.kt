package com.zalom3ui.app.perf

import java.util.concurrent.atomic.AtomicInteger
import java.util.concurrent.atomic.AtomicLong

/** What went through the bridge to the Zalo core since the last [CoreMeter.drain]. [inFlight] is the count right now. */
data class CoreTotals(
    val requests: Long = 0L,
    val errors: Long = 0L,
    /** Requests that count for latency: everything except uploads, which last as long as the file takes. */
    val latencyCount: Long = 0L,
    val latencySumNs: Long = 0L,
    val latencyMaxNs: Long = 0L,
    val events: Long = 0L,
    val bytesUp: Long = 0L,
    val bytesDown: Long = 0L,
    val inFlight: Int = 0,
) {
    operator fun plus(other: CoreTotals): CoreTotals = CoreTotals(
        requests + other.requests,
        errors + other.errors,
        latencyCount + other.latencyCount,
        latencySumNs + other.latencySumNs,
        maxOf(latencyMaxNs, other.latencyMaxNs),
        events + other.events,
        bytesUp + other.bytesUp,
        bytesDown + other.bytesDown,
        other.inFlight,
    )
}

/** [CoreTotals] over a window of known length, in the units the overlay and the log show. */
data class CoreStats(
    val reqPerSec: Float = 0f,
    val avgMs: Float = 0f,
    val maxMs: Float = 0f,
    val inFlight: Int = 0,
    val eventsPerSec: Float = 0f,
    val errors: Int = 0,
    val upKbPerSec: Float = 0f,
    val downKbPerSec: Float = 0f,
)

fun coreStats(totals: CoreTotals, windowMs: Double): CoreStats {
    if (windowMs <= 0.0) return CoreStats(inFlight = totals.inFlight)
    val perSec = 1000.0 / windowMs
    return CoreStats(
        reqPerSec = (totals.requests * perSec).toFloat(),
        avgMs = if (totals.latencyCount > 0) (totals.latencySumNs / totals.latencyCount / 1_000_000.0).toFloat() else 0f,
        maxMs = (totals.latencyMaxNs / 1_000_000.0).toFloat(),
        inFlight = totals.inFlight,
        eventsPerSec = (totals.events * perSec).toFloat(),
        errors = totals.errors.toInt(),
        upKbPerSec = (totals.bytesUp * perSec / 1024.0).toFloat(),
        downKbPerSec = (totals.bytesDown * perSec / 1024.0).toFloat(),
    )
}

/**
 * Always-on counters for the bridge between the app and the Zalo core (JNI in this process, or HTTP to a server): one
 * pair of [System.nanoTime] calls and a few atomic additions per request, nothing else. The monitor reads and resets
 * them each window with [drain]; while nobody reads they only grow.
 */
class CoreMeter {
    private val requests = AtomicLong()
    private val errors = AtomicLong()
    private val latencyCount = AtomicLong()
    private val latencySumNs = AtomicLong()
    private val latencyMaxNs = AtomicLong()
    private val events = AtomicLong()
    private val bytesUp = AtomicLong()
    private val bytesDown = AtomicLong()
    private val inFlight = AtomicInteger()

    /** A request begins; pass the returned time to [requestFinished]. */
    fun requestStarted(nowNs: Long = System.nanoTime()): Long {
        inFlight.incrementAndGet()
        return nowNs
    }

    /**
     * A request ended, with an answer or without ([failed]). [timed] false leaves it out of the latency numbers (an
     * upload); [bytesUp] and [bytesDown] are the sizes of what was sent and what came back.
     */
    fun requestFinished(
        startNs: Long,
        timed: Boolean,
        failed: Boolean,
        bytesUp: Long,
        bytesDown: Long,
        nowNs: Long = System.nanoTime(),
    ) {
        inFlight.decrementAndGet()
        requests.incrementAndGet()
        if (failed) errors.incrementAndGet()
        if (timed) {
            val took = nowNs - startNs
            latencyCount.incrementAndGet()
            latencySumNs.addAndGet(took)
            raiseMax(took)
        }
        if (bytesUp > 0) this.bytesUp.addAndGet(bytesUp)
        if (bytesDown > 0) this.bytesDown.addAndGet(bytesDown)
    }

    /** One event from the core's event stream arrived ([size] is its text length). */
    fun eventReceived(size: Int) {
        events.incrementAndGet()
        if (size > 0) bytesDown.addAndGet(size.toLong())
    }

    /** The totals since the last call, and a fresh start. */
    fun drain(): CoreTotals = CoreTotals(
        requests = requests.getAndSet(0),
        errors = errors.getAndSet(0),
        latencyCount = latencyCount.getAndSet(0),
        latencySumNs = latencySumNs.getAndSet(0),
        latencyMaxNs = latencyMaxNs.getAndSet(0),
        events = events.getAndSet(0),
        bytesUp = bytesUp.getAndSet(0),
        bytesDown = bytesDown.getAndSet(0),
        inFlight = inFlight.get(),
    )

    private fun raiseMax(value: Long) {
        while (true) {
            val current = latencyMaxNs.get()
            if (value <= current || latencyMaxNs.compareAndSet(current, value)) return
        }
    }
}

/** The one meter of this process; `data/MeteredTransport.kt` feeds it and the monitor reads it. */
object PerfCounters {
    val core = CoreMeter()
}
