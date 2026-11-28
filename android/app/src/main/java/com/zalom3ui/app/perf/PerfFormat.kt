package com.zalom3ui.app.perf

import java.util.Locale
import kotlin.math.roundToInt

/** The newest values of each graph on the overlay, oldest first. */
class PerfSeries(
    val fps: FloatArray,
    /** The longest frame of each window, in ms. */
    val frameMaxMs: FloatArray,
    val cpuPct: FloatArray,
    val pssMb: FloatArray,
    /** Average latency of a call into the core, per window. */
    val coreAvgMs: FloatArray,
) {
    companion object {
        /** How many windows the graphs show. */
        const val CAPACITY = 30
        val EMPTY = PerfSeries(FloatArray(0), FloatArray(0), FloatArray(0), FloatArray(0), FloatArray(0))
    }
}

/**
 * Everything the monitor knows after one window: the overlay shows the newest, the log gets one for each 2 seconds.
 * Memory sizes are -1 when they could not be read.
 */
data class PerfSnapshot(
    val timeMs: Long = 0L,
    val fps: Float = 0f,
    val frameAvgMs: Float = 0f,
    val frameMaxMs: Float = 0f,
    val jankPct: Float = 0f,
    val slowFrames: Int = 0,
    /** The longest frame in the last 30 seconds. */
    val worstFrameMs: Float = 0f,
    val budgetMs: Float = 16.7f,
    val refreshHz: Float = 60f,
    val cpuPct: Float = 0f,
    val pssKb: Long = -1L,
    val rssKb: Long = -1L,
    val hwmKb: Long = -1L,
    val javaBytes: Long = -1L,
    val nativeBytes: Long = -1L,
    val availBytes: Long = -1L,
    val totalBytes: Long = -1L,
    val thresholdBytes: Long = -1L,
    val lowMemory: Boolean = false,
    /** Garbage collections per second; -1 when the runtime doesn't say. */
    val gcPerSec: Float = -1f,
    val threads: Int = -1,
    val core: CoreStats = CoreStats(),
    /** `PowerManager` thermal status 0 to 6, or -1. */
    val thermal: Int = -1,
    val batteryTempC: Float = -1f,
    val screen: String = "",
    val series: PerfSeries = PerfSeries.EMPTY,
)

/** The text the monitor writes and shows. Numbers only, in a fixed order, so the log can be grepped and plotted. */
object PerfFormat {
    private const val KB_PER_MB = 1024.0
    private const val BYTES_PER_MB = 1024.0 * 1024.0
    private const val BYTES_PER_GB = 1024.0 * 1024.0 * 1024.0

    /** Whole megabytes of [kb] kilobytes; -1 stays -1. */
    fun mbOfKb(kb: Long): Int = if (kb < 0) -1 else (kb / KB_PER_MB).roundToInt()

    /** Whole megabytes of [bytes]; -1 stays -1. */
    fun mbOfBytes(bytes: Long): Int = if (bytes < 0) -1 else (bytes / BYTES_PER_MB).roundToInt()

    /** `1.1GB` from one gigabyte up, `812MB` below. */
    fun size(bytes: Long): String = when {
        bytes < 0 -> "-1MB"
        bytes >= BYTES_PER_GB -> String.format(Locale.US, "%.1fGB", bytes / BYTES_PER_GB)
        else -> "${mbOfBytes(bytes)}MB"
    }

    /** `14` from ten up, `3.2` below: enough digits to see a small rate, short enough for a column. */
    fun rate(value: Float): String =
        if (value >= 10f) value.roundToInt().toString() else String.format(Locale.US, "%.1f", value)

    fun oneDecimal(value: Float): String = String.format(Locale.US, "%.1f", value)

    /**
     * The 2-second log line, after its time stamp:
     * `fps=58 frame=9.1/22ms jank=3% slow=2 cpu=12% pss=312MB rss=401MB java=48MB native=120MB avail=1.1GB core=14req/s
     * lat=3.2/18ms inflight=0 ev=3.2/s io=1/34KB/s gc=0.0 thr=46 therm=0 temp=31.2 screen=chat`.
     * `frame` is average/maximum, `lat` is the average/maximum latency of a call into the core, `io` is KB/s up/down
     * through the bridge. Every key is always there, in this order; unknown values read -1.
     */
    fun logLine(s: PerfSnapshot): String = buildString {
        append("fps=").append(s.fps.roundToInt())
        append(" frame=").append(oneDecimal(s.frameAvgMs)).append('/').append(s.frameMaxMs.roundToInt()).append("ms")
        append(" jank=").append(s.jankPct.roundToInt()).append('%')
        append(" slow=").append(s.slowFrames)
        append(" cpu=").append(s.cpuPct.roundToInt()).append('%')
        append(" pss=").append(mbOfKb(s.pssKb)).append("MB")
        append(" rss=").append(mbOfKb(s.rssKb)).append("MB")
        append(" java=").append(mbOfBytes(s.javaBytes)).append("MB")
        append(" native=").append(mbOfBytes(s.nativeBytes)).append("MB")
        append(" avail=").append(size(s.availBytes))
        append(" core=").append(rate(s.core.reqPerSec)).append("req/s")
        append(" lat=").append(oneDecimal(s.core.avgMs)).append('/').append(s.core.maxMs.roundToInt()).append("ms")
        append(" inflight=").append(s.core.inFlight)
        append(" ev=").append(rate(s.core.eventsPerSec)).append("/s")
        append(" io=").append(s.core.upKbPerSec.roundToInt()).append('/').append(s.core.downKbPerSec.roundToInt()).append("KB/s")
        append(" gc=").append(if (s.gcPerSec < 0f) "-1" else oneDecimal(s.gcPerSec))
        append(" thr=").append(s.threads)
        append(" therm=").append(s.thermal)
        append(" temp=").append(if (s.batteryTempC < 0f) "-1" else oneDecimal(s.batteryTempC))
        append(" screen=").append(s.screen.ifBlank { "-" })
    }

    /** The collapsed overlay: `58fps 9.1ms · 312MB · 14%` (frames per second, average frame, PSS, CPU of one core). */
    fun summary(s: PerfSnapshot): String =
        "${s.fps.roundToInt()}fps ${oneDecimal(s.frameAvgMs)}ms · ${mbOfKb(s.pssKb)}MB · ${s.cpuPct.roundToInt()}%"

    /** Written for a frame that took [ms]. */
    fun jankLine(ms: Float, screen: String): String = "JANK ${ms.roundToInt()}ms screen=${screen.ifBlank { "-" }}"

    fun trimLine(level: Int): String = "TRIM ${trimLevelName(level)}"

    fun lowMemLine(availBytes: Long, thresholdBytes: Long): String =
        "LOWMEM avail=${size(availBytes)} threshold=${size(thresholdBytes)}"

    fun gcHeavyLine(gcPerSec: Float, javaBytes: Long): String =
        "GC-HEAVY gc=${oneDecimal(gcPerSec)}/s java=${mbOfBytes(javaBytes)}MB"

    /** The lines under the "start" line of a log: what the phone and the app are, so two logs can be compared. */
    fun headerText(
        appName: String,
        versionName: String,
        versionCode: Long,
        buildType: String,
        device: String,
        hardware: String,
        androidRelease: String,
        sdk: Int,
        abis: String,
        totalRamBytes: Long,
        cores: Int,
        refreshHz: Float,
    ): String = buildString {
        appendLine("app: $appName $versionName (code $versionCode, $buildType)")
        appendLine("device: $device (hw $hardware), Android $androidRelease (API $sdk), $abis")
        appendLine("ram: ${size(totalRamBytes)}, cores: $cores, display: ${refreshHz.roundToInt()}Hz")
        append(
            "fields: fps frame=avg/max jank slow cpu(% of one core) pss rss java native avail core lat=avg/max " +
                "inflight ev io=up/down gc thr therm temp screen",
        )
    }
}
