package com.zalom3ui.app.data

import kotlinx.serialization.Serializable

/** What an import added; mirrors `counts` of the server's `HistorySyncStatus` (src/historysync.ts). */
@Serializable
data class HistorySyncCounts(
    val imported: Int = 0,
    val duplicates: Int = 0,
    val skipped: Int = 0,
    val recalled: Int = 0,
    val conversations: Int = 0,
)

/**
 * "Sync messages from the phone": `GET /api/history-sync/status` and the `history_sync` event on /ws.
 * [state] is idle, waiting_for_phone, downloading, importing, done or failed.
 */
@Serializable
data class HistorySyncStatus(
    val state: String = "idle",
    /** 0-100. */
    val progress: Int = 0,
    val error: String? = null,
    /** A short sentence to show under the progress bar. */
    val detail: String = "",
    val expectedMessages: Int? = null,
    val counts: HistorySyncCounts? = null,
) {
    val isActive: Boolean get() = state == "waiting_for_phone" || state == "downloading" || state == "importing"
    val isFinished: Boolean get() = state == "done" || state == "failed"
}

/** A `history_sync` event from the server. */
data class HistorySyncEvent(val status: HistorySyncStatus) : ServerEvent
