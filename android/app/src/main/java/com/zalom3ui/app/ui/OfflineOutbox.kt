package com.zalom3ui.app.ui

import com.zalom3ui.app.data.ApiException
import com.zalom3ui.app.data.Mention
import com.zalom3ui.app.data.ThreadRef
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import java.io.IOException
import java.util.Locale
import java.util.UUID

/** Alias for [Mention] matching the outbox contract. */
typealias MentionInfo = Mention

/** One queued outgoing message held by [OfflineOutbox] while offline or awaiting retry. */
data class OutboxEntry(
    val localId: String,
    val thread: ThreadRef,
    val text: String,
    val quoteId: String? = null,
    val mentions: List<MentionInfo> = emptyList(),
    val ttl: Long? = null,
    val createdAt: Long = System.currentTimeMillis(),
    val attempts: Int = 0,
    val lastError: String? = null,
)

private val OFFLINE_ERROR_TOKENS = listOf(
    "not connected",
    "not logged in",
    "offline",
    "timeout",
    "503",
    "409",
)

/**
 * Returns true if [t] (or one of its causes) looks like a transient connectivity, session-disconnected,
 * or server-unavailable error where retrying on reconnect makes sense.
 */
fun isLikelyOfflineError(t: Throwable): Boolean {
    var cur: Throwable? = t
    var depth = 0
    while (cur != null && depth < 8) {
        if (cur is ApiException) {
            if (cur.code == 503 || cur.code == 409) return true
            val msg = cur.message?.lowercase(Locale.ROOT).orEmpty()
            if (OFFLINE_ERROR_TOKENS.any { it in msg }) return true
            if (cur.code in 400..499) return false
        } else if (cur is IOException) {
            return true
        }
        val msg = cur.message?.lowercase(Locale.ROOT).orEmpty()
        if (OFFLINE_ERROR_TOKENS.any { it in msg }) return true
        cur = cur.cause
        depth++
    }
    return false
}

/**
 * Thread-safe FIFO queue of outgoing messages to be drained when connectivity returns.
 *
 * Exposes an immutable snapshot via [entries] so Compose/ViewModel observers react to every change.
 */
class OfflineOutbox {
    private val lock = Any()
    private val flushMutex = Mutex()
    private val _entries = MutableStateFlow<List<OutboxEntry>>(emptyList())

    /** All queued entries in FIFO insertion order. */
    val entries: StateFlow<List<OutboxEntry>> = _entries.asStateFlow()

    /** Queued entries for [thread] in FIFO insertion order. */
    fun entriesFor(thread: ThreadRef): List<OutboxEntry> =
        _entries.value.filter { it.thread == thread }

    /** Appends [entry] to the outbox (or replaces an existing entry with the same [OutboxEntry.localId]). */
    fun enqueue(entry: OutboxEntry): OutboxEntry {
        synchronized(lock) {
            val current = _entries.value
            val idx = current.indexOfFirst { it.localId == entry.localId }
            _entries.value = if (idx >= 0) {
                current.toMutableList().apply { set(idx, entry) }
            } else {
                current + entry
            }
        }
        return entry
    }

    /** Convenience overload that constructs and enqueues a new [OutboxEntry]. */
    fun enqueue(
        thread: ThreadRef,
        text: String,
        quoteId: String? = null,
        mentions: List<MentionInfo> = emptyList(),
        ttl: Long? = null,
        localId: String = "outbox-${UUID.randomUUID()}",
        createdAt: Long = System.currentTimeMillis(),
    ): OutboxEntry = enqueue(
        OutboxEntry(
            localId = localId,
            thread = thread,
            text = text,
            quoteId = quoteId,
            mentions = mentions,
            ttl = ttl,
            createdAt = createdAt,
        ),
    )

    /** Removes the entry with [localId] from the queue. Returns true if an entry was removed. */
    fun remove(localId: String): Boolean {
        synchronized(lock) {
            val current = _entries.value
            val next = current.filterNot { it.localId == localId }
            if (next.size == current.size) return false
            _entries.value = next
            return true
        }
    }

    /** Records a failed send attempt on [localId], incrementing [OutboxEntry.attempts] and setting [OutboxEntry.lastError]. */
    fun markFailed(localId: String, error: String?) {
        synchronized(lock) {
            val current = _entries.value
            val idx = current.indexOfFirst { it.localId == localId }
            if (idx < 0) return
            val existing = current[idx]
            val updated = existing.copy(
                attempts = existing.attempts + 1,
                lastError = error,
            )
            _entries.value = current.toMutableList().apply { set(idx, updated) }
        }
    }

    /** Number of queued entries waiting for [thread]. */
    fun pendingCount(thread: ThreadRef): Int = _entries.value.count { it.thread == thread }

    /** Total number of queued entries across all threads. */
    fun pendingCount(): Int = _entries.value.size

    /** Removes all queued entries. */
    fun clear() {
        synchronized(lock) {
            _entries.value = emptyList()
        }
    }

    /** Removes all queued entries for [thread]. */
    fun clear(thread: ThreadRef) {
        synchronized(lock) {
            _entries.value = _entries.value.filterNot { it.thread == thread }
        }
    }

    /**
     * Drains queued entries in strict FIFO order using [send].
     *
     * - Successful sends are removed from the queue and counted in the returned total.
     * - Likely-offline/transient failures mark the entry failed and stop the flush immediately,
     *   preserving the failed entry and all remaining entries for the next reconnect.
     * - Non-offline (e.g. permanent 4xx) failures record [OutboxEntry.lastError] on that entry
     *   for manual retry/cancel and continue draining subsequent entries.
     */
    suspend fun flushAll(send: suspend (OutboxEntry) -> Result<Unit>): Int = flushMutex.withLock {
        val snapshot = synchronized(lock) { _entries.value.toList() }
        var sentCount = 0
        for (candidate in snapshot) {
            val current = synchronized(lock) {
                _entries.value.firstOrNull { it.localId == candidate.localId }
            } ?: continue

            val result = try {
                send(current)
            } catch (ce: CancellationException) {
                throw ce
            } catch (t: Throwable) {
                Result.failure(t)
            }

            if (result.isSuccess) {
                remove(current.localId)
                sentCount++
            } else {
                val err = result.exceptionOrNull()
                markFailed(current.localId, err?.message ?: "Send failed")
                if (err != null && isLikelyOfflineError(err)) {
                    break
                }
            }
        }
        sentCount
    }
}
