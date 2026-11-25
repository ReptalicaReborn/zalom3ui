package com.zalom3ui.app.ui

import com.zalom3ui.app.data.ApiClient
import com.zalom3ui.app.data.ChatMessage
import com.zalom3ui.app.data.HistorySyncCounts
import com.zalom3ui.app.data.HistorySyncEvent
import com.zalom3ui.app.data.HistorySyncStatus
import com.zalom3ui.app.data.ThreadSummary
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class HistorySyncTest {
    private fun thread(id: String, last: ChatMessage? = null) = ThreadSummary(id = id, kind = "user", name = id, lastMessage = last)

    private val message = ChatMessage(id = "1", threadId = "a", threadKind = "user", ts = 1L, text = "hi")

    @Test
    fun offer_whenChatsExistButNoneHasAMessage() {
        val idle = HistorySyncStatus()
        assertTrue(shouldOfferHistorySync(true, listOf(thread("a"), thread("b")), idle))
        assertFalse("some history is already there", shouldOfferHistorySync(true, listOf(thread("a", message), thread("b")), idle))
        assertFalse("nothing to sync into yet", shouldOfferHistorySync(true, emptyList(), idle))
        assertFalse("not signed in", shouldOfferHistorySync(false, listOf(thread("a")), idle))
        assertFalse("already running", shouldOfferHistorySync(true, listOf(thread("a")), HistorySyncStatus(state = "importing")))
    }

    @Test
    fun parseEvent_readsHistorySyncProgress() {
        val api = ApiClient("http://127.0.0.1:45000", null)
        val event = api.parseEvent(
            """{"type":"history_sync","state":"importing","progress":72,"error":null,"detail":"Importing messages (3/10 chats)...","expectedMessages":1200}""",
        )
        val status = (event as HistorySyncEvent).status
        assertEquals("importing", status.state)
        assertEquals(72, status.progress)
        assertNull(status.error)
        assertEquals(1200, status.expectedMessages)
        assertTrue(status.isActive)
        assertFalse(status.isFinished)
    }

    @Test
    fun parseEvent_readsAFailureAndTheFinalCounts() {
        val api = ApiClient("http://127.0.0.1:45000", null)
        val failed = (api.parseEvent("""{"type":"history_sync","state":"failed","progress":0,"error":"The sync request was declined on the phone.","detail":""}""") as HistorySyncEvent).status
        assertTrue(failed.isFinished)
        assertEquals("The sync request was declined on the phone.", failed.error)

        val done = (api.parseEvent(
            """{"type":"history_sync","state":"done","progress":100,"error":null,"detail":"","counts":{"imported":11,"duplicates":2,"skipped":1,"recalled":1,"conversations":2}}""",
        ) as HistorySyncEvent).status
        assertEquals(11, done.counts?.imported)
        assertEquals("Added 11 messages from 2 chats.", historySyncSummary(done))
    }

    @Test
    fun summary_prefersTheServersSentence() {
        assertEquals("Hello", historySyncSummary(HistorySyncStatus(state = "done", detail = "Hello")))
        assertEquals("Done.", historySyncSummary(HistorySyncStatus(state = "done")))
        assertEquals(
            "No new messages were found on your phone.",
            historySyncSummary(HistorySyncStatus(state = "done", counts = HistorySyncCounts())),
        )
    }
}
