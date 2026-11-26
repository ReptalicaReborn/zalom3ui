package com.zalom3ui.app.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.DeleteSweep
import androidx.compose.material.icons.outlined.History
import androidx.compose.material.icons.outlined.Tune
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.zalom3ui.app.data.ApiClient
import kotlinx.coroutines.launch

/** The replay-request modes the experiment can run (see docs/replay-test.md), as the server names them. */
private val REPLAY_MODES = listOf(
    Triple("null", "null (default)", "Ask Zalo for old messages the way the app always has. Used first."),
    Triple("newest", "newest", "Ask for messages after the newest one stored, per chat kind."),
    Triple("page", "page", "Ask like null, then keep paging back (up to 5 requests) while a batch looks full and has no overlap."),
)

/**
 * The "Replay log" and "Replay mode" rows of Settings → Developer, for the replay test in docs/replay-test.md. Same
 * place and style as the call log rows. The log holds ids, counts and timestamps only: no message text.
 */
fun replayDebugItems(): List<@Composable () -> Unit> = listOf(
    { ReplayLogRow() },
    { ReplayModeRow() },
    { ClearReplayLogRow() },
)

@Composable
private fun ReplayLogRow() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var log by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    ListItem(
        modifier = Modifier.clickable {
            scope.launch {
                runCatching { ApiClient.onDevice(context).replayLog() }
                    .onSuccess { log = it; error = null }
                    .onFailure { error = it.message ?: "Couldn't read the replay log" }
            }
        },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        leadingContent = { Icon(Icons.Outlined.History, null) },
        headlineContent = { Text("Replay log") },
        supportingContent = {
            Text(error ?: "What Zalo replayed each time Zalỏ connected, and whether history has a gap (ids and counts only)")
        },
    )
    log?.let { text ->
        AlertDialog(
            onDismissRequest = { log = null },
            title = { Text("Replay log") },
            text = {
                SelectionContainer {
                    Text(
                        text,
                        fontFamily = FontFamily.Monospace,
                        fontSize = 11.sp,
                        modifier = Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState()),
                    )
                }
            },
            confirmButton = {
                Row {
                    TextButton(onClick = { copyToClipboard(context, text) }) { Text("Copy") }
                    TextButton(onClick = { scope.launch { QrActions.shareText(context, "zaloo-replay-log.txt", "Share replay log", text) } }) {
                        Text("Share")
                    }
                    TextButton(onClick = {
                        scope.launch {
                            runCatching { ApiClient.onDevice(context).clearReplayLog() }
                            log = null
                        }
                    }) { Text("Clear") }
                }
            },
            dismissButton = { TextButton(onClick = { log = null }) { Text("Close") } },
        )
    }
}

@Composable
private fun ClearReplayLogRow() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    ListItem(
        modifier = Modifier.clickable { scope.launch { runCatching { ApiClient.onDevice(context).clearReplayLog() } } },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        leadingContent = { Icon(Icons.Outlined.DeleteSweep, null) },
        headlineContent = { Text("Clear replay log") },
    )
}

@Composable
private fun ReplayModeRow() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var mode by remember { mutableStateOf("null") }
    var picking by remember { mutableStateOf(false) }
    LaunchedEffect(Unit) { runCatching { ApiClient.onDevice(context).replayMode() }.onSuccess { mode = it } }
    ListItem(
        modifier = Modifier.clickable { picking = true },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        leadingContent = { Icon(Icons.Outlined.Tune, null) },
        headlineContent = { Text("Replay mode: $mode") },
        supportingContent = { Text("How Zalỏ asks Zalo for missed messages when it connects (for the replay test)") },
    )
    if (picking) {
        AlertDialog(
            onDismissRequest = { picking = false },
            title = { Text("Replay mode") },
            text = {
                Column {
                    REPLAY_MODES.forEach { (key, label, description) ->
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .selectable(selected = mode == key, role = Role.RadioButton, onClick = {
                                    mode = key
                                    picking = false
                                    scope.launch { runCatching { ApiClient.onDevice(context).setReplayMode(key) } }
                                })
                                .padding(vertical = 8.dp),
                            verticalAlignment = Alignment.Top,
                        ) {
                            RadioButton(selected = mode == key, onClick = null)
                            Spacer(Modifier.width(12.dp))
                            Column {
                                Text(label, style = MaterialTheme.typography.bodyLarge)
                                Text(
                                    description,
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                )
                            }
                        }
                    }
                }
            },
            confirmButton = {},
            dismissButton = { TextButton(onClick = { picking = false }) { Text("Cancel") } },
        )
    }
}

private fun copyToClipboard(context: Context, text: String) {
    runCatching {
        context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Replay log", text))
    }
}
