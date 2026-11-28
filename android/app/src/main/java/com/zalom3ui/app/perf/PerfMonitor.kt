package com.zalom3ui.app.perf

import android.app.Activity
import android.app.ActivityManager
import android.app.Application
import android.content.ComponentCallbacks2
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.content.pm.ApplicationInfo
import android.content.res.Configuration
import android.os.BatteryManager
import android.os.Build
import android.os.Debug
import android.os.Handler
import android.os.HandlerThread
import android.os.PowerManager
import android.os.SystemClock
import android.system.Os
import android.system.OsConstants
import android.view.FrameMetrics
import android.view.Window
import androidx.core.content.pm.PackageInfoCompat
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import java.io.File

/**
 * The performance monitor (see docs/performance-monitor.md): frame times, memory, CPU and the traffic over the bridge to
 * the Zalo core, shown by the overlay (ui/PerfOverlay.kt) and written to `perf.log` ([PerfLog]).
 *
 * It runs only while the app is in the foreground and the overlay or the log is switched on: [onForeground] starts a
 * sampler thread and a frame-metrics listener, [onBackground] ends both. With both switched off nothing is registered
 * and no thread exists; the only always-on cost is the counters in the transport layer ([PerfCounters]).
 */
object PerfMonitor {
    private val _snapshot = MutableStateFlow<PerfSnapshot?>(null)

    /** The newest window, or null while the monitor is not running. Updated twice a second while the overlay is on. */
    val snapshot: StateFlow<PerfSnapshot?> = _snapshot.asStateFlow()

    /** The screen the user is on (the navigation route), set by the app's navigation. */
    @Volatile
    var screen: String = ""

    /** Whether the log is on; also known while the app is in the background, so a trim-memory call there is logged. */
    @Volatile
    private var logEnabled = false

    /** True until a header has been written for the current "log on" period; set again when the log is switched off. */
    private var headerPending = true
    private var sampler: PerfSampler? = null
    private var app: Application? = null

    /** Called once from `Application.onCreate`: listens for the system's memory warnings (only logs when the log is on). */
    fun attach(app: Application, logEnabled: Boolean) {
        if (this.app != null) return
        this.app = app
        this.logEnabled = logEnabled
        app.registerComponentCallbacks(object : ComponentCallbacks2 {
            override fun onTrimMemory(level: Int) {
                if (this@PerfMonitor.logEnabled) PerfLog.write(app, PerfFormat.trimLine(level))
            }

            override fun onLowMemory() {
                if (this@PerfMonitor.logEnabled) PerfLog.write(app, "LOWMEM onLowMemory")
            }

            override fun onConfigurationChanged(newConfig: Configuration) = Unit
        })
    }

    /** The settings changed (or the app starts): the log being off means its next switch-on writes a new header. */
    @Synchronized
    fun setEnabled(log: Boolean) {
        logEnabled = log
        if (!log) headerPending = true
    }

    /** The app's window is on screen: start measuring if the overlay or the log is on. Main thread. */
    @Synchronized
    fun onForeground(activity: Activity, overlay: Boolean, log: Boolean) {
        stopLocked(logBackground = false)
        setEnabled(log)
        if (!overlay && !log) return
        val writeHeader = log && headerPending
        if (log) headerPending = false
        val s = PerfSampler(activity, overlay, log, writeHeader)
        sampler = s
        s.start()
    }

    /** The app's window left the screen (or the monitor was switched off): stop measuring. Main thread. */
    @Synchronized
    fun onBackground() {
        stopLocked(logBackground = true)
    }

    /** Like [onBackground], but without a "BG" line: the monitor is restarted or switched off, the app did not leave. */
    @Synchronized
    fun release() {
        stopLocked(logBackground = false)
    }

    private fun stopLocked(logBackground: Boolean) {
        sampler?.stop(logBackground)
        sampler = null
        _snapshot.value = null
    }

    internal fun publish(snapshot: PerfSnapshot) {
        _snapshot.value = snapshot
    }
}

/**
 * One run of the monitor, from the app coming to the foreground to it leaving (or a setting changing). Everything
 * runs on its own background thread: the frame-metrics callbacks and a sample every [tickMs].
 */
internal class PerfSampler(
    activity: Activity,
    private val overlay: Boolean,
    private val log: Boolean,
    private val writeHeader: Boolean,
) {
    private val app: Context = activity.applicationContext
    private val window: Window = activity.window
    private var activityRef: Activity? = activity
    private val thread = HandlerThread("perf-monitor", android.os.Process.THREAD_PRIORITY_BACKGROUND)
    private var handler: Handler? = null

    @Volatile
    private var running = false

    /** Half a second while the overlay shows, otherwise one sample is the log's 2-second line. */
    private val tickMs: Long = if (overlay) OVERLAY_TICK_MS else LOG_EVERY_MS

    private val frames = FrameAccumulator(DEFAULT_BUDGET_NS)
    private var frameListener: Window.OnFrameMetricsAvailableListener? = null

    private val clkTck: Long = runCatching { Os.sysconf(OsConstants._SC_CLK_TCK) }.getOrDefault(100L).takeIf { it > 0 } ?: 100L
    private val memoryInfo = Debug.MemoryInfo()
    private val activityMemory = ActivityManager.MemoryInfo()
    private val activityManager: ActivityManager? = app.getSystemService(ActivityManager::class.java)

    // Window state: only touched on the sampler thread.
    private var lastNs = 0L
    private var lastTicks = -1L
    private val fpsRing = FloatRing(PerfSeries.CAPACITY)
    private val frameMaxRing = FloatRing(PerfSeries.CAPACITY)
    private val cpuRing = FloatRing(PerfSeries.CAPACITY)
    private val pssRing = FloatRing(PerfSeries.CAPACITY)
    private val coreAvgRing = FloatRing(PerfSeries.CAPACITY)
    private val worstRing = FloatRing((WORST_WINDOW_MS / tickMs).toInt().coerceAtLeast(1))

    // What the log line adds up: every window since the last line.
    private var logStartTicks = -1L
    private var logWallMs = 0.0
    private var logFrames = FrameTotals()
    private var logCore = CoreTotals()

    // Slower-changing numbers, refreshed about every 2 seconds ([refreshSlow]).
    private var lastSlowNs = 0L
    private var pssKb = -1L
    private var availBytes = -1L
    private var totalBytes = -1L
    private var thresholdBytes = -1L
    private var lowMemory = false
    private var gcPerSec = -1f
    private var lastGcCount = -1L
    private var thermal = -1
    private var batteryTempC = -1f
    private var refreshHz = 60f
    private var budgetMs = DEFAULT_BUDGET_NS / 1_000_000f

    private val jankLimiter = RateLimiter(JANK_EVENT_GAP_MS)
    private val lowMemLimiter = RateLimiter(LOWMEM_EVENT_GAP_MS)
    private val gcLimiter = RateLimiter(GC_EVENT_GAP_MS)

    fun start() {
        running = true
        thread.start()
        val h = Handler(thread.looper)
        handler = h
        val listener = Window.OnFrameMetricsAvailableListener { _, metrics, _ ->
            onFrame(metrics.getMetric(FrameMetrics.TOTAL_DURATION))
        }
        // Only drawn frames are reported (unlike a Choreographer callback, which ticks on every vsync).
        if (runCatching { window.addOnFrameMetricsAvailableListener(listener, h) }.isSuccess) frameListener = listener
        PerfCounters.core.drain() // what piled up while nobody was reading
        h.post { begin() }
    }

    fun stop(logBackground: Boolean) {
        running = false
        val listener = frameListener
        frameListener = null
        if (listener != null) runCatching { window.removeOnFrameMetricsAvailableListener(listener) }
        handler?.removeCallbacksAndMessages(null)
        thread.quitSafely()
        activityRef = null
        if (log && logBackground) PerfLog.write(app, "BG")
    }

    private val tickRunnable = object : Runnable {
        override fun run() {
            if (!running) return
            runCatching { sample() }
            if (running) handler?.postDelayed(this, tickMs)
        }
    }

    /** On the sampler thread: takes the baselines every later window is measured from. */
    private fun begin() {
        if (!running) return
        val now = System.nanoTime()
        lastNs = now
        lastTicks = readCpuTicks()
        logStartTicks = lastTicks
        refreshSlow(now)
        if (log) {
            if (writeHeader) PerfLog.writeHeader(app, headerText())
            PerfLog.write(app, "FG")
        }
        handler?.postDelayed(tickRunnable, tickMs)
    }

    /** On the frame-metrics thread: one frame was drawn. */
    private fun onFrame(totalNs: Long) {
        frames.add(totalNs)
        if (log && totalNs >= JANK_EVENT_NS && jankLimiter.allow(SystemClock.elapsedRealtime())) {
            PerfLog.write(app, PerfFormat.jankLine(totalNs / 1_000_000f, PerfMonitor.screen))
        }
    }

    private fun sample() {
        val now = System.nanoTime()
        val windowMs = (now - lastNs) / 1_000_000.0
        lastNs = now

        val frameTotals = frames.drain()
        val coreTotals = PerfCounters.core.drain()
        val ticks = readCpuTicks()
        val cpu = if (ticks >= 0 && lastTicks >= 0) cpuPercent(ticks - lastTicks, windowMs, clkTck) else 0f
        lastTicks = ticks
        val status = ProcParse.status(readText("/proc/self/status"))
        val runtime = Runtime.getRuntime()
        val javaBytes = runtime.totalMemory() - runtime.freeMemory()
        val nativeBytes = Debug.getNativeHeapAllocatedSize()

        val slowDue = now - lastSlowNs >= SLOW_DUE_NS
        if (slowDue) refreshSlow(now)

        logFrames += frameTotals
        logCore += coreTotals
        logWallMs += windowMs

        val frame = frameStats(frameTotals, windowMs)
        val core = coreStats(coreTotals, windowMs)
        worstRing.add(frame.maxMs)
        fpsRing.add(frame.fps)
        frameMaxRing.add(frame.maxMs)
        cpuRing.add(cpu)
        pssRing.add(if (pssKb >= 0) pssKb / 1024f else 0f)
        coreAvgRing.add(core.avgMs)

        if (overlay) {
            PerfMonitor.publish(
                snapshot(
                    frame, core, cpu, status, javaBytes, nativeBytes,
                    series = PerfSeries(
                        fpsRing.toArray(), frameMaxRing.toArray(), cpuRing.toArray(), pssRing.toArray(), coreAvgRing.toArray(),
                    ),
                ),
            )
        }

        if (slowDue && log) {
            // The log line covers every window since the previous one.
            val logCpu = if (ticks >= 0 && logStartTicks >= 0) cpuPercent(ticks - logStartTicks, logWallMs, clkTck) else 0f
            val line = snapshot(
                frameStats(logFrames, logWallMs), coreStats(logCore, logWallMs), logCpu, status, javaBytes, nativeBytes,
                series = PerfSeries.EMPTY,
            )
            PerfLog.write(app, PerfFormat.logLine(line))
            if (lowMemory && lowMemLimiter.allow(SystemClock.elapsedRealtime())) {
                PerfLog.write(app, PerfFormat.lowMemLine(availBytes, thresholdBytes))
            }
            if (gcPerSec >= GC_HEAVY_PER_SEC && gcLimiter.allow(SystemClock.elapsedRealtime())) {
                PerfLog.write(app, PerfFormat.gcHeavyLine(gcPerSec, javaBytes))
            }
            logStartTicks = ticks
            logWallMs = 0.0
            logFrames = FrameTotals()
            logCore = CoreTotals()
        }
    }

    private fun snapshot(
        frame: FrameStats,
        core: CoreStats,
        cpu: Float,
        status: ProcStatus,
        javaBytes: Long,
        nativeBytes: Long,
        series: PerfSeries,
    ) = PerfSnapshot(
        timeMs = System.currentTimeMillis(),
        fps = frame.fps,
        frameAvgMs = frame.avgMs,
        frameMaxMs = frame.maxMs,
        jankPct = frame.jankPct,
        slowFrames = frame.slow,
        worstFrameMs = worstRing.max(),
        budgetMs = budgetMs,
        refreshHz = refreshHz,
        cpuPct = cpu,
        pssKb = pssKb,
        rssKb = status.rssKb,
        hwmKb = status.hwmKb,
        javaBytes = javaBytes,
        nativeBytes = nativeBytes,
        availBytes = availBytes,
        totalBytes = totalBytes,
        thresholdBytes = thresholdBytes,
        lowMemory = lowMemory,
        gcPerSec = gcPerSec,
        threads = status.threads,
        core = core,
        thermal = thermal,
        batteryTempC = batteryTempC,
        screen = PerfMonitor.screen,
        series = series,
    )

    /** The numbers that cost more to read (PSS walks the process's memory map) or change slowly: about every 2 seconds. */
    private fun refreshSlow(now: Long) {
        val elapsedMs = (now - lastSlowNs) / 1_000_000.0
        lastSlowNs = now
        runCatching {
            Debug.getMemoryInfo(memoryInfo)
            pssKb = memoryInfo.totalPss.toLong()
        }
        runCatching {
            val manager = activityManager ?: return@runCatching
            manager.getMemoryInfo(activityMemory)
            availBytes = activityMemory.availMem
            totalBytes = activityMemory.totalMem
            thresholdBytes = activityMemory.threshold
            lowMemory = activityMemory.lowMemory
        }
        runCatching {
            val count = Debug.getRuntimeStat("art.gc.gc-count")?.trim()?.toLongOrNull()
            if (count != null && lastGcCount >= 0 && elapsedMs > 0) {
                gcPerSec = ((count - lastGcCount) * 1000.0 / elapsedMs).toFloat().coerceAtLeast(0f)
            }
            lastGcCount = count ?: -1L
        }
        runCatching {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                thermal = app.getSystemService(PowerManager::class.java)?.currentThermalStatus ?: -1
            }
        }
        runCatching {
            // The sticky battery broadcast: asking with no receiver registers nothing.
            val battery = app.registerReceiver(null, IntentFilter(Intent.ACTION_BATTERY_CHANGED))
            val tenths = battery?.getIntExtra(BatteryManager.EXTRA_TEMPERATURE, Int.MIN_VALUE) ?: Int.MIN_VALUE
            batteryTempC = if (tenths == Int.MIN_VALUE) -1f else tenths / 10f
        }
        runCatching {
            val hz = refreshRateOf(activityRef)
            if (hz > 0f) {
                refreshHz = hz
                frames.budgetNs = frameBudgetNs(hz)
                budgetMs = frames.budgetNs / 1_000_000f
            }
        }
    }

    @Suppress("DEPRECATION")
    private fun refreshRateOf(activity: Activity?): Float {
        if (activity == null) return 0f
        val display = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) activity.display else activity.windowManager.defaultDisplay
        return display?.refreshRate ?: 0f
    }

    private fun headerText(): String {
        val info = runCatching { app.packageManager.getPackageInfo(app.packageName, 0) }.getOrNull()
        val debug = (app.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE) != 0
        return PerfFormat.headerText(
            appName = "Zalỏ",
            versionName = info?.versionName ?: "?",
            versionCode = info?.let { PackageInfoCompat.getLongVersionCode(it) } ?: -1L,
            buildType = if (debug) "debug" else "release",
            device = "${Build.MANUFACTURER} ${Build.MODEL}",
            hardware = Build.HARDWARE,
            androidRelease = Build.VERSION.RELEASE,
            sdk = Build.VERSION.SDK_INT,
            abis = Build.SUPPORTED_ABIS.joinToString(),
            totalRamBytes = totalBytes,
            cores = Runtime.getRuntime().availableProcessors(),
            refreshHz = refreshHz,
        )
    }

    private fun readCpuTicks(): Long = ProcParse.cpuTicks(readText("/proc/self/stat")) ?: -1L

    private fun readText(path: String): String = runCatching { File(path).readText() }.getOrDefault("")

    private companion object {
        const val OVERLAY_TICK_MS = 500L
        const val LOG_EVERY_MS = 2_000L

        /** A sample is "due" for the slow numbers and the log line a little before a full 2 seconds: ticks drift later. */
        const val SLOW_DUE_NS = 1_900L * 1_000_000L
        const val WORST_WINDOW_MS = 30_000L
        const val JANK_EVENT_NS = 100L * 1_000_000L
        const val JANK_EVENT_GAP_MS = 1_000L
        const val LOWMEM_EVENT_GAP_MS = 30_000L
        const val GC_EVENT_GAP_MS = 10_000L
        const val GC_HEAVY_PER_SEC = 4f
    }
}
