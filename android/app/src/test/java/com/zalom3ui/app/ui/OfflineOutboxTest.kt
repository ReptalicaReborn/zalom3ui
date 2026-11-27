package com.zalom3ui.app.ui

import com.zalom3ui.app.data.ApiException
import com.zalom3ui.app.data.Mention
import com.zalom3ui.app.data.ThreadRef
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.IOException
import java.net.SocketTimeoutException

class OfflineOutboxTest {
    private val threadA = ThreadRef(kind = "user", id = "u1")
    private val threadB = ThreadRef(kind = "group", id = "g1")

    @Test
    fun enqueueAndEntriesFor_preserveFifoOrderPerThread() {
        val outbox = OfflineOutbox()
        val m1 = outbox.enqueue(threadA, "first", localId = "1", createdAt = 10L)
        val m2 = outbox.enqueue(threadB, "other-thread", localId = "2", createdAt = 20L)
        val m3 = outbox.enqueue(
            thread = threadA,
            text = "second",
            quoteId = "q1",
            mentions = listOf(Mention(uid = "u2", pos = 0, len = 4)),
            ttl = 60_000L,
            localId = "3",
            createdAt = 30L,
        )

        assertEquals(listOf(m1, m3), outbox.entriesFor(threadA))
        assertEquals(listOf(m2), outbox.entriesFor(threadB))
        assertEquals(2, outbox.pendingCount(threadA))
        assertEquals(1, outbox.pendingCount(threadB))
        assertEquals(listOf("1", "2", "3"), outbox.entries.value.map { it.localId })
    }

    @Test
    fun flushAll_sendsInFifoOrderAndRemovesSucceededEntries() = runBlocking {
        val outbox = OfflineOutbox()
        outbox.enqueue(threadA, "one", localId = "1")
        outbox.enqueue(threadB, "two", localId = "2")
        outbox.enqueue(threadA, "three", localId = "3")

        val delivered = mutableListOf<String>()
        val count = outbox.flushAll { entry ->
            delivered += entry.localId
            Result.success(Unit)
        }

        assertEquals(3, count)
        assertEquals(listOf("1", "2", "3"), delivered)
        assertTrue(outbox.entries.value.isEmpty())
        assertEquals(0, outbox.pendingCount(threadA))
        assertEquals(0, outbox.pendingCount(threadB))
    }

    @Test
    fun flushAll_stopsEarlyOnOfflineErrorAndPreservesRemainingQueue() = runBlocking {
        val outbox = OfflineOutbox()
        outbox.enqueue(threadA, "msg-1", localId = "1")
        outbox.enqueue(threadA, "msg-2", localId = "2")
        outbox.enqueue(threadA, "msg-3", localId = "3")

        val attempted = mutableListOf<String>()
        val firstPass = outbox.flushAll { entry ->
            attempted += entry.localId
            if (entry.localId == "2") {
                Result.failure(IOException("Network unreachable"))
            } else {
                Result.success(Unit)
            }
        }

        assertEquals(1, firstPass)
        assertEquals(listOf("1", "2"), attempted)
        assertEquals(listOf("2", "3"), outbox.entries.value.map { it.localId })

        val failedEntry = outbox.entries.value.first()
        assertEquals(1, failedEntry.attempts)
        assertEquals("Network unreachable", failedEntry.lastError)

        val untouchedEntry = outbox.entries.value[1]
        assertEquals(0, untouchedEntry.attempts)
        assertNull(untouchedEntry.lastError)

        // Next reconnect drains the remaining entries in FIFO order.
        val reconnected = mutableListOf<String>()
        val secondPass = outbox.flushAll { entry ->
            reconnected += entry.localId
            Result.success(Unit)
        }
        assertEquals(2, secondPass)
        assertEquals(listOf("2", "3"), reconnected)
        assertTrue(outbox.entries.value.isEmpty())
    }

    @Test
    fun flushAll_advancesPastPermanent4xxAndKeepsLastErrorForManualRetry() = runBlocking {
        val outbox = OfflineOutbox()
        outbox.enqueue(threadA, "bad-msg", localId = "1")
        outbox.enqueue(threadA, "good-msg", localId = "2")

        val sent = mutableListOf<String>()
        val count = outbox.flushAll { entry ->
            if (entry.localId == "1") {
                Result.failure(ApiException("HTTP 400 Bad Request", code = 400))
            } else {
                sent += entry.localId
                Result.success(Unit)
            }
        }

        assertEquals(1, count)
        assertEquals(listOf("2"), sent)
        assertEquals(1, outbox.entries.value.size)
        assertEquals("1", outbox.entries.value.single().localId)
        assertEquals(1, outbox.entries.value.single().attempts)
        assertEquals("HTTP 400 Bad Request", outbox.entries.value.single().lastError)
    }

    @Test
    fun removeAndClear_manageIndividualAndAllEntries() {
        val outbox = OfflineOutbox()
        outbox.enqueue(threadA, "a", localId = "1")
        outbox.enqueue(threadA, "b", localId = "2")
        outbox.enqueue(threadB, "c", localId = "3")

        assertTrue(outbox.remove("2"))
        assertFalse(outbox.remove("missing"))
        assertEquals(listOf("1"), outbox.entriesFor(threadA).map { it.localId })
        assertEquals(listOf("3"), outbox.entriesFor(threadB).map { it.localId })

        outbox.markFailed("1", "Temporary error")
        assertEquals(1, outbox.entriesFor(threadA).single().attempts)
        assertEquals("Temporary error", outbox.entriesFor(threadA).single().lastError)

        outbox.clear()
        assertTrue(outbox.entries.value.isEmpty())
        assertEquals(0, outbox.pendingCount(threadA))
        assertEquals(0, outbox.pendingCount(threadB))
    }

    @Test
    fun isLikelyOfflineError_classifiesNetworkSessionAndHttpCodes() {
        assertTrue(isLikelyOfflineError(IOException("Connection reset")))
        assertTrue(isLikelyOfflineError(SocketTimeoutException("Read timed out")))
        assertTrue(isLikelyOfflineError(ApiException("Not connected to the server")))
        assertTrue(isLikelyOfflineError(ApiException("not logged in", code = 409)))
        assertTrue(isLikelyOfflineError(ApiException("HTTP 503", code = 503)))
        assertTrue(isLikelyOfflineError(ApiException("HTTP 409", code = 409)))
        assertTrue(isLikelyOfflineError(IllegalStateException("Device appears offline")))
        assertTrue(isLikelyOfflineError(RuntimeException("Request timeout")))
        assertTrue(isLikelyOfflineError(RuntimeException("wrapped", IOException("broken pipe"))))

        assertFalse(isLikelyOfflineError(ApiException("HTTP 400 Bad Request", code = 400)))
        assertFalse(isLikelyOfflineError(ApiException("Forbidden", code = 403)))
        assertFalse(isLikelyOfflineError(IllegalArgumentException("Message too long")))
    }
}
