package com.zalom3ui.app.server

import java.util.concurrent.atomic.AtomicBoolean

/**
 * Cold-start timing written to the startup log (one line per cold start), so a slow start can be measured on a phone or
 * watch without adb. The arithmetic lives here, free of Android classes, so it can be unit tested.
 */
object StartupTiming {
    /** An activity created this soon after the process started is a cold start. */
    const val COLD_START_WINDOW_MS = 5_000L

    private val claimed = AtomicBoolean(false)

    /** [android.os.SystemClock.uptimeMillis] when the core finished starting in this process; 0 until then. */
    @Volatile
    var coreReadyUptimeMs: Long = 0L
        private set

    fun markCoreReady(uptimeMs: Long) {
        if (coreReadyUptimeMs == 0L) coreReadyUptimeMs = uptimeMs
    }

    /** A fresh launch (no saved state) that began right after the process did. */
    fun isColdStart(processStartUptimeMs: Long, activityCreateUptimeMs: Long, hasSavedState: Boolean): Boolean =
        !hasSavedState && activityCreateUptimeMs - processStartUptimeMs in 0..COLD_START_WINDOW_MS

    /** True for the first caller in this process only, so one process logs one cold start. */
    fun claim(): Boolean = claimed.compareAndSet(false, true)

    /** Milliseconds since process start, or null when the moment has not happened (0 or negative). */
    fun sinceStart(processStartUptimeMs: Long, eventUptimeMs: Long): Long? =
        (eventUptimeMs - processStartUptimeMs).takeIf { eventUptimeMs > 0L && it >= 0L }

    /** Named moments on the way to the chat list (see [mark]), by uptime. Only the first time each is seen is kept. */
    private val marks = java.util.concurrent.ConcurrentHashMap<String, Long>()

    /** Records that [name] happened at [uptimeMs] ([android.os.SystemClock.uptimeMillis]); later calls are ignored. */
    fun mark(name: String, uptimeMs: Long) {
        marks.putIfAbsent(name, uptimeMs)
    }

    fun snapshotMarks(): Map<String, Long> = HashMap(marks)

    /** "vm-created 190, refresh-start 205, ..." in time order, each as ms since process start. */
    fun formatMarks(processStartUptimeMs: Long, marks: Map<String, Long>): String =
        marks.entries
            .mapNotNull { (name, at) -> sinceStart(processStartUptimeMs, at)?.let { name to it } }
            .sortedBy { it.second }
            .joinToString(", ") { (name, ms) -> "$name $ms" }

    fun format(
        firstFrameMs: Long?,
        coreReadyMs: Long?,
        listDataMs: Long,
        threads: Int,
        steps: String = "",
    ): String {
        fun part(v: Long?) = if (v == null) "n/a" else "$v ms"
        val base = "startup: process→first-frame ${part(firstFrameMs)}, process→core-ready ${part(coreReadyMs)}, " +
            "process→list-data ${part(listDataMs)} (threads=$threads)"
        return if (steps.isEmpty()) base else "$base; at ms: $steps"
    }
}
