package com.zalom3ui.app.ui

import android.widget.Toast
import androidx.compose.foundation.clickable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.DeleteSweep
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material.icons.outlined.Speed
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import com.zalom3ui.app.perf.PerfLog
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * The performance monitor's rows of Settings → Developer (see docs/performance-monitor.md): the overlay switch, the log
 * switch, and sharing or clearing perf.log. Same place and style as the replay and socket log rows.
 */
fun perfSettingsItems(
    overlay: Boolean,
    onOverlay: (Boolean) -> Unit,
    log: Boolean,
    onLog: (Boolean) -> Unit,
): List<@Composable () -> Unit> = listOf(
    { PerfSwitchRow(Icons.Outlined.Speed, "Performance monitor", "Frames, memory, CPU and core on screen", overlay, onOverlay) },
    { PerfSwitchRow(Icons.Outlined.Description, "Log performance", "Write perf.log, with or without the monitor", log, onLog) },
    { SharePerfLogRow() },
    { ClearPerfLogRow() },
)

@Composable
private fun PerfSwitchRow(
    icon: ImageVector,
    title: String,
    summary: String,
    checked: Boolean,
    onChange: (Boolean) -> Unit,
) {
    ListItem(
        modifier = Modifier.clickable { onChange(!checked) },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        leadingContent = { Icon(icon, null) },
        headlineContent = { Text(title) },
        supportingContent = { Text(summary) },
        trailingContent = { Switch(checked = checked, onCheckedChange = onChange) },
    )
}

@Composable
private fun SharePerfLogRow() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    ListItem(
        modifier = Modifier.clickable {
            scope.launch {
                val report = withContext(Dispatchers.IO) { PerfLog.report(context) }
                QrActions.shareText(context, "zaloo-perf-log.txt", "Share performance log", report)
            }
        },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        leadingContent = { Icon(Icons.Outlined.Share, null) },
        headlineContent = { Text("Share performance log") },
        supportingContent = { Text("Timings and memory only, no messages") },
    )
}

@Composable
private fun ClearPerfLogRow() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    ListItem(
        modifier = Modifier.clickable {
            scope.launch {
                withContext(Dispatchers.IO) { PerfLog.clear(context) }
                Toast.makeText(context, "Performance log cleared", Toast.LENGTH_SHORT).show()
            }
        },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        leadingContent = { Icon(Icons.Outlined.DeleteSweep, null) },
        headlineContent = { Text("Clear performance log") },
    )
}
