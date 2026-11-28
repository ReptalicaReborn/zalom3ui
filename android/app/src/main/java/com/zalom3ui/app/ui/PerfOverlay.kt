package com.zalom3ui.app.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.zalom3ui.app.perf.Level
import com.zalom3ui.app.perf.PerfFormat
import com.zalom3ui.app.perf.PerfMonitor
import com.zalom3ui.app.perf.PerfSeries
import com.zalom3ui.app.perf.PerfSnapshot
import com.zalom3ui.app.perf.clampCardOffset
import com.zalom3ui.app.perf.cpuLevel
import com.zalom3ui.app.perf.frameLevel
import com.zalom3ui.app.perf.jankLevel
import com.zalom3ui.app.perf.latencyLevel
import com.zalom3ui.app.perf.memoryLevel
import com.zalom3ui.app.perf.thermalName
import com.zalom3ui.app.security.AppLock
import kotlin.math.roundToInt

/**
 * Where the performance card sits and whether it is folded: kept for the life of the process, so a rotation or a
 * recreated activity doesn't move it back. ([x], [y]) is the distance from its top-end corner (x is zero or negative).
 */
private object PerfOverlayState {
    var x by mutableStateOf(0f)
    var y by mutableStateOf(0f)
    var collapsed by mutableStateOf(false)
}

/**
 * Starts and stops the performance monitor (see docs/performance-monitor.md) and shows its overlay. Hosted once, over
 * every screen of the main window; it costs nothing while "Performance monitor" and "Log performance" are both off.
 * The monitor runs only while the activity is started: a lifecycle observer starts it on the way up and ends it on the
 * way down, so the app in the background is not measured. Full-screen dialog windows (the video player) are separate
 * windows above this one, so the overlay is not visible there.
 */
@Composable
fun PerfMonitorHost(vm: AppViewModel) {
    val overlay by vm.perfOverlay.collectAsStateWithLifecycle()
    val log by vm.perfLog.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner, overlay, log) {
        val activity = AppLock.findActivity(context)
        if (activity == null || (!overlay && !log)) {
            PerfMonitor.setEnabled(log)
            return@DisposableEffect onDispose {}
        }
        val observer = LifecycleEventObserver { _, event ->
            when (event) {
                Lifecycle.Event.ON_START -> PerfMonitor.onForeground(activity, overlay, log)
                Lifecycle.Event.ON_STOP -> PerfMonitor.onBackground()
                else -> Unit
            }
        }
        // Adding it to a started activity delivers ON_START at once.
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose {
            lifecycleOwner.lifecycle.removeObserver(observer)
            PerfMonitor.release()
        }
    }
    if (overlay) PerfOverlay(onClose = { vm.setPerfOverlay(false) })
}

private val CARD_WIDTH = 248.dp

@Composable
private fun PerfOverlay(onClose: () -> Unit) {
    // The only state read here is the newest snapshot, twice a second; the card is small, so it just recomposes.
    val snapshot by PerfMonitor.snapshot.collectAsStateWithLifecycle()
    var containerSize by remember { mutableStateOf(IntSize.Zero) }
    var cardSize by remember { mutableStateOf(IntSize.Zero) }
    val collapsed = PerfOverlayState.collapsed
    val palette = perfPalette()

    // This box covers the screen but takes no touches: only the card does (Surface), the rest stays touchable.
    Box(
        Modifier
            .fillMaxSize()
            .statusBarsPadding()
            .navigationBarsPadding()
            .padding(8.dp)
            .onSizeChanged { containerSize = it },
    ) {
        Surface(
            modifier = Modifier
                .align(Alignment.TopEnd)
                .offset {
                    // Read in the layout phase, so dragging moves the card without recomposing it.
                    val (x, y) = clampCardOffset(
                        PerfOverlayState.x, PerfOverlayState.y,
                        containerSize.width, containerSize.height, cardSize.width, cardSize.height,
                    )
                    IntOffset(x.roundToInt(), y.roundToInt())
                }
                .onSizeChanged { cardSize = it }
                .pointerInput(Unit) {
                    detectDragGestures { change, drag ->
                        change.consume()
                        val (x, y) = clampCardOffset(
                            PerfOverlayState.x + drag.x, PerfOverlayState.y + drag.y,
                            containerSize.width, containerSize.height, cardSize.width, cardSize.height,
                        )
                        PerfOverlayState.x = x
                        PerfOverlayState.y = y
                    }
                }
                .then(if (collapsed) Modifier else Modifier.width(CARD_WIDTH)),
            shape = RoundedCornerShape(12.dp),
            color = MaterialTheme.colorScheme.surfaceContainerHighest.copy(alpha = 0.88f),
            contentColor = MaterialTheme.colorScheme.onSurface,
            shadowElevation = 4.dp,
        ) {
            val s = snapshot
            Column(Modifier.padding(horizontal = 8.dp, vertical = 6.dp)) {
                PerfHeader(
                    summary = if (s != null) PerfFormat.summary(s) else "PERF …",
                    collapsed = collapsed,
                    onToggle = { PerfOverlayState.collapsed = !collapsed },
                    onClose = onClose,
                )
                if (!collapsed && s != null) PerfBody(s, palette)
            }
        }
    }
}

@Composable
private fun PerfHeader(summary: String, collapsed: Boolean, onToggle: () -> Unit, onClose: () -> Unit) {
    Row(
        modifier = (if (collapsed) Modifier else Modifier.fillMaxWidth())
            .clickable(onClickLabel = if (collapsed) "Expand" else "Collapse", role = Role.Button, onClick = onToggle),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Mono(if (collapsed) summary else "PERFORMANCE", bold = true)
        if (collapsed) Spacer(Modifier.width(8.dp)) else Spacer(Modifier.weight(1f))
        Box(
            Modifier.size(28.dp).clickable(onClickLabel = "Close", role = Role.Button, onClick = onClose),
            contentAlignment = Alignment.Center,
        ) {
            Icon(Icons.Outlined.Close, contentDescription = "Close performance monitor", modifier = Modifier.size(16.dp))
        }
    }
}

@Composable
private fun PerfBody(s: PerfSnapshot, p: PerfPalette) {
    val dim = MaterialTheme.colorScheme.onSurfaceVariant

    Section("FRAME", "${s.refreshHz.roundToInt()}Hz  budget ${PerfFormat.oneDecimal(s.budgetMs)}ms", dim)
    Stats(
        "fps ${s.fps.roundToInt()}", Color.Unspecified,
        "avg ${PerfFormat.oneDecimal(s.frameAvgMs)}ms", p.of(frameLevel(s.frameAvgMs, s.budgetMs)),
        "max ${s.frameMaxMs.roundToInt()}ms", p.of(frameLevel(s.frameMaxMs, s.budgetMs)),
    )
    Stats(
        "jank ${s.jankPct.roundToInt()}%", p.of(jankLevel(s.jankPct)),
        "slow ${s.slowFrames}", Color.Unspecified,
        "worst ${s.worstFrameMs.roundToInt()}ms", p.of(frameLevel(s.worstFrameMs, s.budgetMs)),
    )
    Sparkline(s.series.frameMaxMs, scaleMin = s.budgetMs * 3, colorOf = { p.of(frameLevel(it, s.budgetMs)) })

    Section("MEMORY", "MB", dim)
    Stats(
        "pss ${PerfFormat.mbOfKb(s.pssKb)}MB", Color.Unspecified,
        "rss ${PerfFormat.mbOfKb(s.rssKb)}MB", Color.Unspecified,
        "java ${PerfFormat.mbOfBytes(s.javaBytes)}MB", Color.Unspecified,
    )
    Stats(
        "native ${PerfFormat.mbOfBytes(s.nativeBytes)}MB", Color.Unspecified,
        "peak ${PerfFormat.mbOfKb(s.hwmKb)}MB", Color.Unspecified,
        if (s.gcPerSec < 0f) "gc n/a" else "gc ${PerfFormat.rate(s.gcPerSec)}/s", Color.Unspecified,
    )
    val memoryColor = p.of(memoryLevel(s.availBytes, s.thresholdBytes, s.lowMemory))
    Sparkline(s.series.pssMb, scaleMin = 0f, colorOf = { memoryColor }, relative = true)

    Section("CPU", "% of one core", dim)
    Stats(
        "cpu ${s.cpuPct.roundToInt()}%", p.of(cpuLevel(s.cpuPct)),
        "thr ${s.threads}", Color.Unspecified,
        "", Color.Unspecified,
    )
    Sparkline(s.series.cpuPct, scaleMin = 100f, colorOf = { p.of(cpuLevel(it)) })

    Section("CORE", if (s.core.errors > 0) "errors ${s.core.errors}" else "bridge to Zalo", if (s.core.errors > 0) p.bad else dim)
    Stats(
        "${PerfFormat.rate(s.core.reqPerSec)}req/s", Color.Unspecified,
        "avg ${PerfFormat.oneDecimal(s.core.avgMs)}ms", p.of(latencyLevel(s.core.avgMs)),
        "max ${s.core.maxMs.roundToInt()}ms", p.of(latencyLevel(s.core.maxMs)),
    )
    Stats(
        "inflight ${s.core.inFlight}", Color.Unspecified,
        "ev ${PerfFormat.rate(s.core.eventsPerSec)}/s", Color.Unspecified,
        "io ${s.core.upKbPerSec.roundToInt()}/${s.core.downKbPerSec.roundToInt()}K", Color.Unspecified,
    )
    Sparkline(s.series.coreAvgMs, scaleMin = 100f, colorOf = { p.of(latencyLevel(it)) })

    Section("DEVICE", "", dim)
    Row(verticalAlignment = Alignment.CenterVertically) {
        Mono("avail ${PerfFormat.size(s.availBytes)} / ${PerfFormat.size(s.totalBytes)}")
        if (s.lowMemory) {
            Spacer(Modifier.width(6.dp))
            Mono("LOW MEM", color = p.bad, bold = true)
        }
    }
    val temp = if (s.batteryTempC < 0f) "" else " · ${PerfFormat.oneDecimal(s.batteryTempC)}°C"
    Mono("thermal ${thermalName(s.thermal)}$temp")
    Mono("screen ${s.screen.ifBlank { "-" }}")
}

@Composable
private fun Section(title: String, detail: String, detailColor: Color) {
    Spacer(Modifier.height(4.dp))
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Mono(title, bold = true, color = MaterialTheme.colorScheme.primary)
        Spacer(Modifier.weight(1f))
        if (detail.isNotEmpty()) Mono(detail, color = detailColor)
    }
}

/** Three monospace columns of equal width. */
@Composable
private fun Stats(a: String, ca: Color, b: String, cb: Color, c: String, cc: Color) {
    Row(Modifier.fillMaxWidth()) {
        Mono(a, color = ca, modifier = Modifier.weight(1f))
        Mono(b, color = cb, modifier = Modifier.weight(1f))
        Mono(c, color = cc, modifier = Modifier.weight(1f))
    }
}

@Composable
private fun Mono(
    text: String,
    modifier: Modifier = Modifier,
    color: Color = Color.Unspecified,
    bold: Boolean = false,
) {
    Text(
        text,
        modifier = modifier,
        color = color,
        fontFamily = FontFamily.Monospace,
        fontSize = 10.sp,
        lineHeight = 13.sp,
        fontWeight = if (bold) FontWeight.Bold else FontWeight.Normal,
        maxLines = 1,
    )
}

/**
 * The newest [PerfSeries.CAPACITY] values as thin bars, newest at the right. Each bar takes the colour [colorOf] gives
 * its value. The bars are scaled to the tallest value, or to [scaleMin] when that is taller. With [relative] they
 * are scaled between the lowest and highest value instead (a number that moves a little, like memory).
 */
@Composable
private fun Sparkline(
    values: FloatArray,
    scaleMin: Float,
    colorOf: (Float) -> Color,
    relative: Boolean = false,
) {
    Canvas(
        Modifier
            .fillMaxWidth()
            .height(14.dp)
            .padding(top = 2.dp),
    ) {
        if (values.isEmpty()) return@Canvas
        val slots = PerfSeries.CAPACITY
        val gap = 1.dp.toPx()
        val barWidth = ((size.width - gap * (slots - 1)) / slots).coerceAtLeast(1f)
        val minHeight = 1.dp.toPx()
        var hi = if (relative) 0f else scaleMin
        var lo = Float.MAX_VALUE
        for (v in values) {
            if (v > hi) hi = v
            if (v < lo) lo = v
        }
        // Relative: the floor sits a little under the lowest value so the lowest bar is still visible.
        val floor = if (relative) lo - (hi - lo) * 0.3f - 1f else 0f
        val span = (hi - floor).coerceAtLeast(0.001f)
        for (i in values.indices) {
            val h = ((values[i] - floor) / span * size.height).coerceIn(minHeight, size.height)
            val x = (slots - values.size + i) * (barWidth + gap)
            drawRect(color = colorOf(values[i]), topLeft = Offset(x, size.height - h), size = Size(barWidth, h))
        }
    }
}

/** The colours of the three levels, readable on the card in light and dark themes (and AMOLED black). */
private class PerfPalette(val good: Color, val warn: Color, val bad: Color) {
    fun of(level: Level): Color = when (level) {
        Level.GOOD -> good
        Level.WARN -> warn
        Level.BAD -> bad
    }
}

private val LIGHT_PALETTE = PerfPalette(Color(0xFF1B7F45), Color(0xFF9A6200), Color(0xFFC62828))
private val DARK_PALETTE = PerfPalette(Color(0xFF6BD68F), Color(0xFFFFC857), Color(0xFFFF6B6B))

@Composable
private fun perfPalette(): PerfPalette =
    if (MaterialTheme.colorScheme.surface.luminance() < 0.5f) DARK_PALETTE else LIGHT_PALETTE
