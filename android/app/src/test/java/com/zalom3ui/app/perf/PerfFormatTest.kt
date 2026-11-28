package com.zalom3ui.app.perf

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PerfFormatTest {
    private val snapshot = PerfSnapshot(
        fps = 57.6f,
        frameAvgMs = 9.14f,
        frameMaxMs = 22.4f,
        jankPct = 3.2f,
        slowFrames = 2,
        cpuPct = 12.4f,
        pssKb = 319_488, // 312 MB
        rssKb = 410_000,
        javaBytes = 48L * 1024 * 1024,
        nativeBytes = 120L * 1024 * 1024,
        availBytes = (1.1 * 1024 * 1024 * 1024).toLong(),
        core = CoreStats(
            reqPerSec = 13.6f, avgMs = 3.24f, maxMs = 18.4f, inFlight = 0, eventsPerSec = 3.2f,
            upKbPerSec = 0.8f, downKbPerSec = 33.7f,
        ),
        gcPerSec = 0.5f,
        threads = 46,
        thermal = 0,
        batteryTempC = 31.24f,
        screen = "chat",
    )

    @Test
    fun logLineHasEveryKeyInAFixedOrder() {
        assertEquals(
            "fps=58 hz=60 frame=9.1/22ms jank=3% slow=2 cpu=12% pss=312MB rss=400MB java=48MB native=120MB avail=1.1GB " +
                "core=14req/s lat=3.2/18ms inflight=0 ev=3.2/s io=1/34KB/s gc=0.5 thr=46 therm=0 temp=31.2 screen=chat",
            PerfFormat.logLine(snapshot),
        )
    }

    @Test
    fun theRefreshRateIsOnEveryLineAndAChangeHasItsOwnLine() {
        assertTrue(" hz=120 " in PerfFormat.logLine(snapshot.copy(refreshHz = 120f)))
        assertTrue(" hz=90 " in PerfFormat.logLine(snapshot.copy(refreshHz = 90.2f)))
        assertEquals("HZ 120->90 screen=chat", PerfFormat.hzLine(120f, 90f, "chat"))
        assertEquals("HZ 60->120 screen=-", PerfFormat.hzLine(60f, 119.97f, ""))
        // The header lists the key in the same place as the line.
        assertTrue(PerfFormat.logLine(snapshot).startsWith("fps=58 hz=60 frame="))
    }

    @Test
    fun dupIsOnlyLoggedWhenFramesWereReportedTwice() {
        assertTrue(" dup=" !in PerfFormat.logLine(snapshot))
        val line = PerfFormat.logLine(snapshot.copy(dupFrames = 40))
        assertTrue(line, " slow=2 dup=40 cpu=12% " in line)
    }

    @Test
    fun logLineKeysDoNotChangeWithTheValues() {
        fun keys(line: String) = line.split(' ').map { it.substringBefore('=') }
        val busy = snapshot.copy(
            fps = 0f, cpuPct = 180f, jankPct = 100f, screen = "",
            core = CoreStats(reqPerSec = 120f, inFlight = 9),
        )
        assertEquals(keys(PerfFormat.logLine(snapshot)), keys(PerfFormat.logLine(busy)))
        assertEquals(keys(PerfFormat.logLine(snapshot)), keys(PerfFormat.logLine(PerfSnapshot())))
        // Every token is key=value, so the line can be split and plotted.
        assertTrue(PerfFormat.logLine(PerfSnapshot()).split(' ').all { '=' in it })
    }

    @Test
    fun unknownValuesReadMinusOne() {
        val line = PerfFormat.logLine(PerfSnapshot())
        assertTrue(line, "pss=-1MB rss=-1MB java=-1MB native=-1MB avail=-1MB" in line)
        assertTrue(line, "gc=-1 thr=-1 therm=-1 temp=-1 screen=-" in line)
    }

    @Test
    fun valuesAreRoundedNotCut() {
        val line = PerfFormat.logLine(snapshot.copy(fps = 59.5f, jankPct = 99.6f, cpuPct = 0.4f))
        assertTrue(line, line.startsWith("fps=60 "))
        assertTrue(line, " jank=100% " in line)
        assertTrue(line, " cpu=0% " in line)
    }

    @Test
    fun rateKeepsADecimalBelowTen() {
        assertEquals("0.0", PerfFormat.rate(0f))
        assertEquals("3.2", PerfFormat.rate(3.24f))
        assertEquals("10", PerfFormat.rate(10f))
        assertEquals("14", PerfFormat.rate(13.6f))
    }

    @Test
    fun sizesSwitchToGigabytesAtOneGigabyte() {
        assertEquals("812MB", PerfFormat.size(812L * 1024 * 1024))
        assertEquals("1.0GB", PerfFormat.size(1024L * 1024 * 1024))
        assertEquals("5.6GB", PerfFormat.size((5.6 * 1024 * 1024 * 1024).toLong()))
        assertEquals("-1MB", PerfFormat.size(-1))
    }

    @Test
    fun collapsedSummaryIsFpsFrameMemoryAndCpu() {
        assertEquals("58fps 9.1ms · 312MB · 12%", PerfFormat.summary(snapshot))
    }

    @Test
    fun eventLines() {
        assertEquals("JANK 142ms screen=chat", PerfFormat.jankLine(141.8f, "chat"))
        assertEquals("JANK 100ms screen=-", PerfFormat.jankLine(100f, ""))
        assertEquals("TRIM RUNNING_LOW", PerfFormat.trimLine(10))
        assertEquals("LOWMEM avail=300MB threshold=150MB", PerfFormat.lowMemLine(300L * 1024 * 1024, 150L * 1024 * 1024))
        assertEquals("GC-HEAVY gc=6.5/s java=48MB", PerfFormat.gcHeavyLine(6.5f, 48L * 1024 * 1024))
    }

    @Test
    fun headerNamesThePhoneAndTheApp() {
        val text = PerfFormat.headerText(
            appName = "Zalỏ", versionName = "1.0.0", versionCode = 7, buildType = "release",
            device = "samsung SM-A135F", hardware = "mt6765", androidRelease = "14", sdk = 34,
            abis = "arm64-v8a, armeabi-v7a", totalRamBytes = (3.6 * 1024 * 1024 * 1024).toLong(), cores = 8, refreshHz = 60.0f,
        )
        val lines = text.lines()
        assertEquals("app: Zalỏ 1.0.0 (code 7, release)", lines[0])
        assertEquals("device: samsung SM-A135F (hw mt6765), Android 14 (API 34), arm64-v8a, armeabi-v7a", lines[1])
        assertEquals("ram: 3.6GB, cores: 8, display: 60Hz", lines[2])
        assertTrue(lines[3].startsWith("fields: fps hz frame=avg/max"))
        assertFalse(text.endsWith("\n"))
    }
}
