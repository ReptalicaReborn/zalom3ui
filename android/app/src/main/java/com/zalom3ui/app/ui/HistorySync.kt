package com.zalom3ui.app.ui

import android.content.Context
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.zalom3ui.app.data.HistorySyncStatus
import com.zalom3ui.app.data.ThreadSummary
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/**
 * "Sync messages from phone": the offer after sign-in, and the progress dialog. The server does the work
 * (docs/history-sync.md); this only asks, shows progress and refreshes the chat list when it is done.
 */

/** Wait after sign-in before offering: the server replays recent messages first, which may fill the list on its own. */
private const val OFFER_DELAY_MS = 8_000L

/**
 * Whether to offer the sync, like Zalos does when a fresh sign-in has no history: signed in, chats exist (groups come
 * from the contact list) but none shows a message.
 */
fun shouldOfferHistorySync(loggedIn: Boolean, threads: List<ThreadSummary>, sync: HistorySyncStatus): Boolean =
    loggedIn && !sync.isActive && threads.isNotEmpty() && threads.none { it.lastMessage != null }

/** Remembers, per account, that the offer was answered (synced, or "don't ask again"). */
class HistorySyncPrefs(context: Context) {
    private val prefs = context.getSharedPreferences("history_sync", Context.MODE_PRIVATE)

    fun isOfferDismissed(ownId: String?): Boolean = prefs.getBoolean(key(ownId), false)

    fun dismissOffer(ownId: String?) {
        prefs.edit().putBoolean(key(ownId), true).apply()
    }

    private fun key(ownId: String?) = "offer_dismissed_${ownId.orEmpty()}"
}

/** One line for the end of a sync. */
fun historySyncSummary(status: HistorySyncStatus): String {
    val counts = status.counts
    return when {
        status.detail.isNotBlank() -> status.detail
        counts == null -> "Done."
        counts.imported == 0 -> "No new messages were found on your phone."
        else -> "Added ${counts.imported} messages from ${counts.conversations} chats."
    }
}

@Composable
fun HistorySyncHost(vm: AppViewModel) {
    val state by vm.state.collectAsStateWithLifecycle()
    val context = LocalContext.current
    val prefs = remember { HistorySyncPrefs(context) }
    val scope = rememberCoroutineScope()
    val ownId = state.status?.ownId
    var dismissed by remember(ownId) { mutableStateOf(prefs.isOfferDismissed(ownId)) }
    val candidate = !dismissed && shouldOfferHistorySync(state.status?.isLoggedIn == true, state.threads, state.historySync)
    var offer by remember { mutableStateOf(false) }
    LaunchedEffect(candidate) {
        if (candidate) {
            delay(OFFER_DELAY_MS)
            offer = true
        } else {
            offer = false
        }
    }
    // A finished sync answers the offer for good.
    LaunchedEffect(state.historySync.state, ownId) {
        if (state.historySync.state == "done") {
            prefs.dismissOffer(ownId)
            dismissed = true
        }
    }

    if (offer && candidate) {
        AlertDialog(
            onDismissRequest = { offer = false },
            title = { Text("Sync messages from your phone?") },
            text = {
                Text(
                    "This account has no chat history here yet. Zalo on your phone can send it: the phone asks you to confirm, " +
                        "then the messages are copied to this app. Open Zalo on your phone first.",
                )
            },
            confirmButton = {
                TextButton(onClick = {
                    offer = false
                    vm.startHistorySync()
                }) { Text("Sync now") }
            },
            dismissButton = {
                Column {
                    TextButton(onClick = { offer = false }) { Text("Not now") }
                    TextButton(onClick = {
                        prefs.dismissOffer(ownId)
                        dismissed = true
                        offer = false
                    }) { Text("Don't ask again") }
                }
            },
        )
    }

    if (state.historySyncShown && state.historySync.state != "idle") {
        HistorySyncDialog(
            status = state.historySync,
            onCancel = vm::cancelHistorySync,
            onHide = vm::hideHistorySync,
            onRetry = vm::startHistorySync,
            onShareLog = { vm.shareHistorySyncLog { log -> scope.launch { QrActions.shareSyncLog(context, log) } } },
        )
    }
}

@Composable
fun HistorySyncDialog(
    status: HistorySyncStatus,
    onCancel: () -> Unit,
    onHide: () -> Unit,
    onRetry: () -> Unit,
    onShareLog: () -> Unit = {},
) {
    val waiting = status.state == "waiting_for_phone"
    AlertDialog(
        onDismissRequest = { if (!status.isActive) onHide() },
        title = {
            Text(
                when (status.state) {
                    "done" -> "Messages synced"
                    "failed" -> "Sync didn't finish"
                    else -> "Syncing from your phone"
                },
            )
        },
        text = {
            Column(Modifier.fillMaxWidth()) {
                when {
                    status.state == "failed" -> Text(status.error ?: "Something went wrong.")
                    status.state == "done" -> Text(historySyncSummary(status))
                    else -> {
                        if (waiting && status.progress <= 15) {
                            LinearProgressIndicator(Modifier.fillMaxWidth())
                        } else {
                            LinearProgressIndicator(progress = { status.progress.coerceIn(0, 100) / 100f }, modifier = Modifier.fillMaxWidth())
                        }
                        Spacer(Modifier.height(12.dp))
                        Text(status.detail.ifBlank { "Working..." })
                        if (waiting) {
                            Spacer(Modifier.height(8.dp))
                            Text(
                                "On your phone, open Zalo and tap Allow when it asks to transfer messages.",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
        },
        confirmButton = {
            when (status.state) {
                "failed" -> TextButton(onClick = onRetry) { Text("Try again") }
                "done" -> TextButton(onClick = onHide) { Text("Done") }
                else -> TextButton(onClick = onHide) { Text("Hide") }
            }
        },
        dismissButton = {
            when {
                status.state == "failed" -> Row {
                    TextButton(onClick = onShareLog) { Text("Share log") }
                    TextButton(onClick = onHide) { Text("Close") }
                }
                status.isActive -> TextButton(onClick = onCancel) { Text("Cancel sync") }
            }
        },
    )
}
