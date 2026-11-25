package com.zalom3ui.app.data

import com.zalom3ui.app.ui.buildPollRequest
import com.zalom3ui.app.ui.isShown
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.jsonArray
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class PollModelsTest {
    private val api = ApiClient("127.0.0.1:3000", null)
    private val json = Json { ignoreUnknownKeys = true }

    private val detailJson = """
        {"pollId":55,"question":"Lunch?","creatorId":"alice","creatorName":"Alice","closed":false,"expired":false,
         "expiresAt":0,"createdAt":1000,"allowMultiple":true,"allowAddOptions":true,"anonymous":false,
         "resultsHidden":false,"totalVotes":3,"voterCount":3,"myVotes":[1],"canVote":true,"canAddOptions":true,
         "canClose":false,"extra":1,
         "options":[{"id":1,"text":"Pho","votes":2,"voted":true,"voters":[{"id":"me","name":"Me"},{"id":"bob","name":"Bob"}]},
                    {"id":2,"text":"Bun","votes":1,"voted":false,"voters":[{"id":"carol","name":"Carol"}]}]}
    """.trimIndent()

    @Test
    fun pollDetail_decodesServerView() {
        val poll = json.decodeFromString<PollDetail>(detailJson)
        assertEquals(55L, poll.pollId)
        assertEquals(listOf(1L), poll.myVotes)
        assertEquals("Bob", poll.options[0].voters[1].name)
        assertTrue(poll.allowMultiple)
        assertEquals("3 votes", poll.votesLabel)
        assertEquals(2f / 3f, poll.share(poll.options[0]), 0.001f)
    }

    @Test
    fun votesLabel_coversEmptySingularAndHidden() {
        val base = PollDetail(pollId = 1)
        assertEquals("No votes yet", base.votesLabel)
        assertEquals("1 vote", base.copy(totalVotes = 1).votesLabel)
        val hidden = base.copy(totalVotes = 4, resultsHidden = true, options = listOf(PollOption(1, "a", votes = 4)))
        assertEquals("Vote to see results", hidden.votesLabel)
        assertEquals(0f, hidden.share(hidden.options[0]), 0f)
    }

    @Test
    fun pollAttachment_decodesAndIsShown() {
        val msg = json.decodeFromString<ChatMessage>(
            """{"id":"1","threadId":"g1","threadKind":"group","ts":1,"text":"[Poll] Lunch?",
                "attachment":{"kind":"poll","title":"Lunch?","poll":{"pollId":55,"question":"Lunch?"}}}""",
        )
        val poll = msg.attachment?.poll
        assertEquals(PollRef(55L, "Lunch?", null), poll)
        assertTrue(msg.attachment!!.isShown())
        assertNull(json.decodeFromString<Attachment>("""{"kind":"photo"}""").poll)
    }

    @Test
    fun pollEvent_isParsed() {
        assertEquals(ServerEvent.PollChanged(55L), api.parseEvent("""{"type":"poll","pollId":55}"""))
        assertNull(api.parseEvent("""{"type":"poll"}"""))
    }

    @Test
    fun createPollBody_trimsDedupesAndSkipsEmptyDeadline() {
        val body = api.createPollBody(CreatePollRequest(" Lunch? ", listOf("Pho ", "", "Pho", "Bun"), allowMultiple = true))
        assertEquals("Lunch?", body["question"]!!.jsonPrimitive.content)
        assertEquals(listOf("Pho", "Bun"), body["options"]!!.jsonArray.map { it.jsonPrimitive.content })
        assertTrue(body["allowMultiple"]!!.jsonPrimitive.boolean)
        assertFalse(body["allowAddOptions"]!!.jsonPrimitive.boolean)
        assertFalse(body.jsonObject.containsKey("expiresAt"))
        val timed = api.createPollBody(CreatePollRequest("q", listOf("a", "b"), expiresAt = 1700000000000L))
        assertEquals(1700000000000L, timed["expiresAt"]!!.jsonPrimitive.content.toLong())
    }

    @Test
    fun createPollRequest_needsQuestionAndTwoDifferentOptions() {
        assertFalse(CreatePollRequest("", listOf("a", "b")).isValid)
        assertFalse(CreatePollRequest("q", listOf("a", " a ")).isValid)
        assertFalse(CreatePollRequest("q", listOf("a", "")).isValid)
        assertTrue(CreatePollRequest("q", listOf("a", "b")).isValid)
    }

    @Test
    fun dialogRequest_countsDeadlineFromNow() {
        val request = buildPollRequest(" q ", listOf("a", "b", "a"), allowMultiple = false, allowAddOptions = true, deadlineMs = 3_600_000L, now = 1_000L)
        assertEquals(CreatePollRequest("q", listOf("a", "b"), false, true, 3_601_000L), request)
        assertEquals(0L, buildPollRequest("q", listOf("a", "b"), false, false, 0L, 5L).expiresAt)
    }
}
