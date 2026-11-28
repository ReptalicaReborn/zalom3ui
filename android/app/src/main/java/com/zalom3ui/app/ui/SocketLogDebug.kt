package com.zalom3ui.app.ui

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.DeleteSweep
import androidx.compose.material.icons.outlined.Cable
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.zalom3ui.app.data.ApiClient
import kotlinx.coroutines.launch

/**
 * The "Socket log" rows of Settings → Developer: one structure-only line group per WebSocket frame Zalỏ decoded (cmd,
 * subCmd, whether a handler took it, key names, array lengths, and for each message item its type, the shape of its
 * content, who sent it, and whether it became a message or why it was dropped). No message text, names or numbers. It
 * answers whether a one-time code (an OA / ZNS message) reaches Zalỏ at all. Same place and style as the replay log.
 */
fun socketLogItems(): List<@Composable () -> Unit> = listOf(
    { SocketLogRow() },
    { ClearSocketLogRow() },
)

@Composable
private fun SocketLogRow() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var log by remember { mutableStateOf<String?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    ListItem(
        modifier = Modifier.clickable {
            scope.launch {
                runCatching { ApiClient.onDevice(context).socketLog() }
                    .onSuccess { log = it; error = null }
                    .onFailure { error = it.message ?: "Couldn't read the socket log" }
            }
        },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        leadingContent = { Icon(Icons.Outlined.Cable, null) },
        headlineContent = { Text("Socket log") },
        supportingContent = {
            Text(error ?: "Frames Zalo sent and what Zalỏ did with them (no text)")
        },
    )
    log?.let { text ->
        AlertDialog(
            onDismissRequest = { log = null },
            title = { Text("Socket log") },
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
                    TextButton(onClick = { copySocketLog(context, text) }) { Text("Copy") }
                    TextButton(onClick = { scope.launch { QrActions.shareText(context, "zaloo-socket-log.txt", "Share socket log", text) } }) {
                        Text("Share")
                    }
                    TextButton(onClick = {
                        scope.launch {
                            runCatching { ApiClient.onDevice(context).clearSocketLog() }
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
private fun ClearSocketLogRow() {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    ListItem(
        modifier = Modifier.clickable { scope.launch { runCatching { ApiClient.onDevice(context).clearSocketLog() } } },
        colors = ListItemDefaults.colors(containerColor = Color.Transparent),
        leadingContent = { Icon(Icons.Outlined.DeleteSweep, null) },
        headlineContent = { Text("Clear socket log") },
    )
}

private fun copySocketLog(context: Context, text: String) {
    runCatching {
        context.getSystemService(ClipboardManager::class.java).setPrimaryClip(ClipData.newPlainText("Socket log", text))
    }
}
