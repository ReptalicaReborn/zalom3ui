package com.zalom3ui.app.data

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ModelsTest {
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun decodesServerMessageWithParityFields() {
        // Shape produced by ChatStore.toPublic on the server.
        val msg = json.decodeFromString<ChatMessage>(
            """
            {"id":"10","cliMsgId":"c10","threadId":"g1","threadKind":"group","fromId":"me","fromName":"Me",
             "isSelf":true,"ts":1700000000000,"msgType":"webchat","text":"@Bob ok",
             "quote":{"msgId":"9","fromId":"200","fromName":"Bob","text":"lunch?"},
             "mentions":[{"uid":"200","pos":0,"len":4}],
             "reactions":[{"key":"heart","emoji":"❤️","count":2,"mine":true}],
             "mentionsMe":false,"status":"seen","seenCount":3,"futureField":1}
            """.trimIndent(),
        )
        assertEquals("lunch?", msg.quote?.text)
        assertEquals(Mention("200", 0, 4), msg.mentions.single())
        assertTrue(msg.reactions.single().mine)
        assertEquals("seen", msg.status)
        assertEquals(3, msg.seenCount)
        assertTrue(msg.isText)
    }

    @Test
    fun olderPayloadsStillDecode() {
        val msg = json.decodeFromString<ChatMessage>(
            """{"id":"1","threadId":"100","threadKind":"user","ts":1,"text":"[Photo]","attachment":{"href":"https://x"}}""",
        )
        assertTrue(msg.reactions.isEmpty())
        assertFalse(msg.recalled)
        assertNull(msg.status)
        assertFalse("attachments can't be forwarded", msg.isText)
    }

    @Test
    fun decodesThreadSummaryFlags() {
        val t = json.decodeFromString<ThreadSummary>(
            """{"id":"g1","kind":"group","name":"Team","unread":2,"pinned":true,"mutedUntil":-1}""",
        )
        assertTrue(t.pinned)
        assertTrue(t.isMuted)
        assertEquals("group:g1", t.key)
        assertFalse(json.decodeFromString<ThreadSummary>("""{"id":"1","kind":"user","name":"A"}""").isMuted)
    }

    @Test
    fun decodesMessageSearchHitAndRef() {
        val hit = AppJson.decodeFromString<MessageSearchHit>(
            """
            {
              "threadId": "g99",
              "threadKind": "group",
              "threadName": "Weekend Hike",
              "threadAvatar": "https://cdn.example/g99.jpg",
              "message": {
                "id": "m42",
                "threadId": "g99",
                "threadKind": "group",
                "fromId": "u1",
                "fromName": "Lan",
                "isSelf": false,
                "ts": 1700001234567,
                "msgType": "webchat",
                "text": "Meet at 7am",
                "quote": {"msgId": "m40", "fromId": "u2", "fromName": "Minh", "text": "When?"},
                "mentions": [{"uid": "u2", "pos": 0, "len": 4}]
              }
            }
            """.trimIndent(),
        )
        assertEquals(ThreadRef("group", "g99"), hit.ref)
        assertEquals("group:g99", hit.ref.key)
        assertEquals("Weekend Hike", hit.threadName)
        assertEquals("https://cdn.example/g99.jpg", hit.threadAvatar)
        assertEquals("m42", hit.message.id)
        assertEquals("Meet at 7am", hit.message.text)
        assertEquals("When?", hit.message.quote?.text)
        assertEquals(Mention("u2", 0, 4), hit.message.mentions.single())
    }

    @Test
    fun pendingMessageDefaultsAndRetryTransition() {
        val quote: QuotedMessage = Quote(msgId = "m1", fromId = "u1", fromName = "An", text = "Hello")
        val mention: MentionDraft = Mention(uid = "u2", pos = 0, len = 4)
        val pending = PendingMessage(
            localId = "pending-1",
            text = "@Bob hi",
            ts = 1000L,
            quote = quote,
            quoteId = quote.msgId,
            mentions = listOf(mention),
        )
        assertFalse(pending.failed)
        assertNull(pending.error)
        assertEquals("m1", pending.quoteId)
        assertEquals("Hello", pending.quote?.text)
        assertEquals(1, pending.mentions.size)

        val failed = pending.copy(failed = true, error = "Offline")
        assertTrue(failed.failed)
        assertEquals("Offline", failed.error)
    }

    @Test
    fun chatAccentAndDraftJsonCodecsRoundTripAndSanitize() {
        assertEquals(ChatAccent.REPTDOC_GOLD, ChatAccent.fromKeyOrNull("gold"))
        assertNull(ChatAccent.fromKeyOrNull("nonexistent"))
        assertEquals(ChatAccent.ULTRAMARINE, ChatAccent.fromKey("nonexistent"))

        val encodedAccents = SettingsStore.encodeChatAccents(
            mapOf("user:1" to ChatAccent.CRIMSON, "group:2" to ChatAccent.TEAL),
        )
        assertEquals(
            mapOf("user:1" to ChatAccent.CRIMSON, "group:2" to ChatAccent.TEAL),
            SettingsStore.decodeChatAccents(encodedAccents),
        )
        assertEquals(
            mapOf("user:1" to ChatAccent.FOREST),
            SettingsStore.decodeChatAccents("""{"user:1":"forest","group:2":"unknown_accent"}"""),
        )
        assertTrue(SettingsStore.decodeChatAccents("not-json").isEmpty())
        assertTrue(SettingsStore.decodeChatAccents(null).isEmpty())

        val encodedDrafts = SettingsStore.encodeDrafts(
            mapOf("user:1" to "On my way", "group:2" to "   "),
        )
        assertEquals(mapOf("user:1" to "On my way"), SettingsStore.decodeDrafts(encodedDrafts))
        assertEquals(
            mapOf("user:1" to "Hello"),
            SettingsStore.decodeDrafts("""{"user:1":"Hello","group:2":""}"""),
        )
        assertTrue(SettingsStore.decodeDrafts("{").isEmpty())
    }
}
