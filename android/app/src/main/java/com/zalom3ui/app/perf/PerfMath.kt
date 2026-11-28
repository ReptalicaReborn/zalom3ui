package com.zalom3ui.app.perf

import kotlin.math.max
import kotlin.math.min

/*
 * The arithmetic and parsing behind the performance monitor (see docs/performance-monitor.md). Pure Kotlin on purpose:
 * nothing here touches the Android framework, so the JVM unit tests run it as it is. The Android side that feeds it
 * is PerfMonitor.kt.
 */

/** What the frames drawn in one window add up to. Several windows add up with [plus]. */
data class FrameTotals(
    val count: Int = 0,
    val sumNs: Long = 0L,
    val maxNs: Long = 0L,
    /** Frames that took longer than the display's frame budget. */
    val slow: Int = 0,
) {
    operator fun plus(other: FrameTotals): FrameTotals =
        FrameTotals(count + other.count, sumNs + other.sumNs, max(maxNs, other.maxNs), slow + other.slow)
}

/**
 * Collects the duration of every drawn frame. [add] runs on the frame-metrics thread, [drain] on the sampler thread;
 * both only hold the lock for a few additions.
 */
class FrameAccumulator(initialBudgetNs: Long) {
    private val lock = Any()
    private var count = 0
    private var sumNs = 0L
    private var maxNs = 0L
    private var slow = 0

    /** A frame longer than this counts as slow ("jank"): the time one display refresh lasts. */
    @Volatile
    var budgetNs: Long = initialBudgetNs

    fun add(durationNs: Long) {
        if (durationNs < 0) return
        val over = durationNs > budgetNs
        synchronized(lock) {
            count++
            sumNs += durationNs
            if (durationNs > maxNs) maxNs = durationNs
            if (over) slow++
        }
    }

    /** The totals since the last call; starts the next window. */
    fun drain(): FrameTotals = synchronized(lock) {
        val totals = FrameTotals(count, sumNs, maxNs, slow)
        count = 0
        sumNs = 0L
        maxNs = 0L
        slow = 0
        totals
    }
}

/** What the frame totals of a window of [windowMs] mean: fps is frames actually drawn per second, not the refresh rate. */
data class FrameStats(
    val fps: Float,
    val avgMs: Float,
    val maxMs: Float,
    val slow: Int,
    /** Slow frames as a percentage of the frames drawn (0 when none were drawn). */
    val jankPct: Float,
)

fun frameStats(totals: FrameTotals, windowMs: Double): FrameStats {
    if (totals.count <= 0 || windowMs <= 0.0) return FrameStats(0f, 0f, 0f, 0, 0f)
    return FrameStats(
        fps = (totals.count * 1000.0 / windowMs).toFloat(),
        avgMs = (totals.sumNs / totals.count / 1_000_000.0).toFloat(),
        maxMs = (totals.maxNs / 1_000_000.0).toFloat(),
        slow = totals.slow,
        jankPct = (totals.slow * 100.0 / totals.count).toFloat(),
    )
}

/** The frame budget used when the display's refresh rate is unknown: 60 Hz. */
const val DEFAULT_BUDGET_NS = 16_700_000L

/** The time one refresh of a display at [refreshHz] lasts, in nanoseconds; 60 Hz when the rate is missing or absurd. */
fun frameBudgetNs(refreshHz: Float): Long =
    if (refreshHz in 20f..240f) (1_000_000_000.0 / refreshHz).toLong() else DEFAULT_BUDGET_NS

/** Process CPU time as a percentage of ONE core: [deltaTicks] of CPU time over [wallMs] of wall time. 150 means 1.5 cores. */
fun cpuPercent(deltaTicks: Long, wallMs: Double, ticksPerSec: Long): Float {
    if (deltaTicks < 0 || wallMs <= 0.0 || ticksPerSec <= 0) return 0f
    val cpuMs = deltaTicks * 1000.0 / ticksPerSec
    return (cpuMs / wallMs * 100.0).toFloat()
}

/** What the monitor reads from `/proc/self/status`; -1 for a field that was not there. */
data class ProcStatus(val rssKb: Long = -1L, val hwmKb: Long = -1L, val threads: Int = -1)

object ProcParse {
    /**
     * utime + stime (clock ticks) from the text of `/proc/<pid>/stat`, or null when it doesn't parse. The command name
     * (field 2) sits in parentheses and can itself hold spaces and parentheses, so the fields are counted from the
     * LAST `)`: after it come state (field 3), then ppid and so on, with utime as field 14 and stime as field 15.
     */
    fun cpuTicks(stat: String): Long? {
        val close = stat.lastIndexOf(')')
        if (close < 0) return null
        val fields = stat.substring(close + 1).trim().split(' ')
        // fields[0] is field 3 (state), so field 14 is index 11 and field 15 is index 12.
        if (fields.size < 13) return null
        val user = fields[11].toLongOrNull() ?: return null
        val system = fields[12].toLongOrNull() ?: return null
        return user + system
    }

    /** `VmRSS`, `VmHWM` (peak RSS) and `Threads` from the text of `/proc/self/status`. */
    fun status(text: String): ProcStatus {
        var rss = -1L
        var hwm = -1L
        var threads = -1
        for (line in text.lineSequence()) {
            when {
                line.startsWith("VmRSS:") -> rss = firstNumber(line, "VmRSS:") ?: rss
                line.startsWith("VmHWM:") -> hwm = firstNumber(line, "VmHWM:") ?: hwm
                line.startsWith("Threads:") -> threads = firstNumber(line, "Threads:")?.toInt() ?: threads
            }
        }
        return ProcStatus(rss, hwm, threads)
    }

    private fun firstNumber(line: String, prefix: String): Long? =
        line.substring(prefix.length).trim().split(' ').firstOrNull()?.toLongOrNull()
}

/** A fixed-size window of the newest values, for the sparklines. Only used from the sampler thread. */
class FloatRing(val capacity: Int) {
    private val data = FloatArray(capacity)
    private var next = 0

    var size = 0
        private set

    fun add(value: Float) {
        data[next] = value
        next = (next + 1) % capacity
        if (size < capacity) size++
    }

    /** The values from oldest to newest. */
    fun toArray(): FloatArray {
        val out = FloatArray(size)
        val start = if (size < capacity) 0 else next
        for (i in 0 until size) out[i] = data[(start + i) % capacity]
        return out
    }

    fun max(): Float {
        var best = 0f
        for (i in 0 until size) if (data[i] > best) best = data[i]
        return best
    }
}

/** Lets an event through at most once per [minGapMs], so a burst of slow frames is one log line, not hundreds. */
class RateLimiter(private val minGapMs: Long) {
    private var last = Long.MIN_VALUE

    @Synchronized
    fun allow(nowMs: Long): Boolean {
        if (last != Long.MIN_VALUE && nowMs - last < minGapMs) return false
        last = nowMs
        return true
    }
}

/** How a number reads on the overlay: the colour it gets. */
enum class Level { GOOD, WARN, BAD }

/** A frame within the budget is good, up to two budgets is a visible hitch, beyond that a stall. */
fun frameLevel(ms: Float, budgetMs: Float): Level = when {
    ms <= budgetMs -> Level.GOOD
    ms <= budgetMs * 2 -> Level.WARN
    else -> Level.BAD
}

fun jankLevel(pct: Float): Level = when {
    pct < 5f -> Level.GOOD
    pct < 20f -> Level.WARN
    else -> Level.BAD
}

/** CPU in percent of one core: above 100 the process keeps more than a whole core busy. */
fun cpuLevel(pct: Float): Level = when {
    pct < 60f -> Level.GOOD
    pct < 100f -> Level.WARN
    else -> Level.BAD
}

/** Latency of a call into the core. */
fun latencyLevel(ms: Float): Level = when {
    ms < 50f -> Level.GOOD
    ms < 200f -> Level.WARN
    else -> Level.BAD
}

/** The device is short of memory when Android says so, and close to it when free memory is under twice its threshold. */
fun memoryLevel(availBytes: Long, thresholdBytes: Long, lowMemory: Boolean): Level = when {
    lowMemory -> Level.BAD
    thresholdBytes > 0 && availBytes in 0 until thresholdBytes * 2 -> Level.WARN
    else -> Level.GOOD
}

/** The name of a `ComponentCallbacks2.onTrimMemory` level, for the log. */
fun trimLevelName(level: Int): String = when (level) {
    5 -> "RUNNING_MODERATE"
    10 -> "RUNNING_LOW"
    15 -> "RUNNING_CRITICAL"
    20 -> "UI_HIDDEN"
    40 -> "BACKGROUND"
    60 -> "MODERATE"
    80 -> "COMPLETE"
    else -> "LEVEL_$level"
}

/** The name of a `PowerManager` thermal status (0 to 6); "n/a" when the phone can't tell (-1). */
fun thermalName(status: Int): String = when (status) {
    0 -> "none"
    1 -> "light"
    2 -> "moderate"
    3 -> "severe"
    4 -> "critical"
    5 -> "emergency"
    6 -> "shutdown"
    else -> "n/a"
}

/**
 * Keeps a dragged card on screen. The card is anchored at the top end of a container of [containerW] x [containerH]
 * pixels; ([x], [y]) is how far it was dragged from there (x is zero or negative: toward the start). Returns the
 * offset limited so the whole card, [cardW] x [cardH], stays inside the container.
 */
fun clampCardOffset(x: Float, y: Float, containerW: Int, containerH: Int, cardW: Int, cardH: Int): Pair<Float, Float> {
    val minX = min(0f, (cardW - containerW).toFloat())
    val maxY = max(0f, (containerH - cardH).toFloat())
    return x.coerceIn(minX, 0f) to y.coerceIn(0f, maxY)
}

/** Where a log file keeps its lines when it is cut down to its newest part. */
object LogTrim {
    /**
     * Index in [tail] (the last bytes of a file) where the first whole line starts: just after the first newline, or 0
     * when there is none or it is the last byte. The cut can land mid-line, and that half line is dropped.
     */
    fun firstLineStart(tail: ByteArray): Int {
        val nl = tail.indexOf('\n'.code.toByte())
        return if (nl in 0 until tail.size - 1) nl + 1 else 0
    }
}
