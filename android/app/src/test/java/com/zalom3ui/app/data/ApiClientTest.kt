package com.zalom3ui.app.data

import okhttp3.MultipartBody
import okio.Buffer
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.File

class ApiClientTest {
    @Test
    fun normalizeServerUrl_addsSchemeAndTrimsSlash() {
        assertEquals("http://192.168.1.5:3000/", ApiClient.normalizeServerUrl(" 192.168.1.5:3000/ ").toString())
        assertEquals("https://zalo.example.com/", ApiClient.normalizeServerUrl("https://zalo.example.com").toString())
    }

    @Test
    fun normalizeServerUrl_rejectsBlankAndGarbage() {
        assertNull(ApiClient.normalizeServerUrl(""))
        assertNull(ApiClient.normalizeServerUrl("   "))
        assertNull(ApiClient.normalizeServerUrl("ftp://example.com"))
    }

    @Test
    fun serverSettings_onDeviceNeedsNoAddress() {
        assertTrue(ServerSettings(onDevice = true, serverUrl = "", token = "").isConfigured)
        assertFalse(ServerSettings(onDevice = false, serverUrl = "", token = "").isConfigured)
        assertTrue(ServerSettings(onDevice = false, serverUrl = "192.168.1.5:3000", token = "").isConfigured)
    }

    @Test
    fun eventsRequest_usesAuthorizationHeaderInsteadOfQueryParam() {
        val withToken = HttpTransport("https://zalo.example.com", "secret-123").eventsRequest()
        assertEquals("Bearer secret-123", withToken.header("Authorization"))
        assertNull(withToken.url.queryParameter("token"))
        assertEquals("/ws", withToken.url.encodedPath)

        val withoutToken = HttpTransport("http://127.0.0.1:45000", null).eventsRequest()
        assertNull(withoutToken.header("Authorization"))
        assertNull(withoutToken.url.queryParameter("token"))
    }

    @Test
    fun url_encodesEachSegmentWithoutSplittingOrDoubleEncoding() {
        val path = ApiClient.path("calls", "a/b c%20d", "answer")
        assertEquals("/api/calls/a%2Fb%20c%2520d/answer", path)
        val built = HttpTransport("http://127.0.0.1:45000", "tok").url(path)
        assertEquals("http://127.0.0.1:45000/api/calls/a%2Fb%20c%2520d/answer", built.toString())
    }

    @Test
    fun path_encodesQueryParametersAndKeepsAServerPathPrefix() {
        assertEquals(
            "/api/search?q=h%E1%BB%8Fi%20%26%20%2B&limit=5",
            ApiClient.path("search", query = listOf("q" to "hỏi & +", "limit" to "5")),
        )
        assertEquals("/api/status", ApiClient.path("status"))
        val prefixed = HttpTransport("https://example.com/zalo/", null).url("/api/status")
        assertEquals("https://example.com/zalo/api/status", prefixed.toString())
    }

    @Test
    fun parseSearchResults_decodesResultsArray() {
        val api = ApiClient("http://127.0.0.1:45000", "tok")
        val hits = api.parseSearchResults(
            """
            {
              "results": [
                {
                  "threadId": "u7",
                  "threadKind": "user",
                  "threadName": "Alice",
                  "threadAvatar": "",
                  "message": {
                    "id": "m101",
                    "threadId": "u7",
                    "threadKind": "user",
                    "fromId": "u7",
                    "fromName": "Alice",
                    "isSelf": false,
                    "ts": 1700000001000,
                    "msgType": "webchat",
                    "text": "See you tomorrow!"
                  }
                }
              ]
            }
            """.trimIndent(),
        )
        assertEquals(1, hits.size)
        val hit = hits.single()
        assertEquals(ThreadRef("user", "u7"), hit.ref)
        assertEquals("Alice", hit.threadName)
        assertEquals("m101", hit.message.id)
        assertEquals("See you tomorrow!", hit.message.text)
    }

    @Test
    fun sendVoiceRequest_buildsMultipartAudioMp4Post() {
        val transport = HttpTransport("http://127.0.0.1:45000", "tok")
        val tmp = File.createTempFile("note-", ".m4a").apply {
            writeBytes(byteArrayOf(1, 2, 3, 4))
            deleteOnExit()
        }
        val req = transport.buildRequest("POST", ApiClient.path("threads", "group", "g42", "voice"), ApiBody.VoiceFile(tmp, "audio/mp4")).build()
        assertEquals("POST", req.method)
        assertEquals("http://127.0.0.1:45000/api/threads/group/g42/voice", req.url.toString())
        val body = req.body as MultipartBody
        assertEquals(1, body.size)
        val part = body.part(0)
        assertEquals("audio/mp4", part.body.contentType()?.toString())
        val disposition = part.headers?.get("Content-Disposition").orEmpty()
        assertTrue(disposition.contains("""name="voice""""))
        assertTrue(disposition.contains("""filename="${tmp.name}""""))
        val sink = Buffer()
        part.body.writeTo(sink)
        assertEquals(4L, sink.size)
        tmp.delete()
    }

    @Test
    fun swarm2Routes_sendExpectedMethodsPathsBodiesAndDecodeResponses() = kotlinx.coroutines.runBlocking {
        val calls = mutableListOf<Triple<String, String, String?>>()
        val responses = ArrayDeque<TransportResponse>()
        val transport = object : Transport {
            override suspend fun execute(method: String, path: String, body: ApiBody, upload: Boolean): TransportResponse {
                val textBody = (body as? ApiBody.Bytes)?.bytes?.toString(Charsets.UTF_8)
                calls += Triple(method, path, textBody)
                return if (responses.isEmpty()) TransportResponse(204, "") else responses.removeFirst()
            }

            override fun events(): kotlinx.coroutines.flow.Flow<String> = kotlinx.coroutines.flow.emptyFlow()
        }
        val api = ApiClient(transport)

        // Friend alias, unfriend, block/unblock
        api.changeFriendAlias("u42", "Bestie")
        api.removeFriendAlias("u42")
        api.removeFriend("u42")
        api.blockUser("u99")
        api.unblockUser("u99")

        // Group link, join-link, deputies, board, notes
        responses += TransportResponse(200, """{"link":"https://zalo.me/g/abc","enabled":true,"expirationDate":1800000000}""")
        val linkInfo = api.groupLink("g1")
        assertEquals("https://zalo.me/g/abc", linkInfo.link)
        assertTrue(linkInfo.enabled)
        assertEquals(1800000000L, linkInfo.expirationDate)

        responses += TransportResponse(200, """{"link":"https://zalo.me/g/new1","enabled":true,"expirationDate":1900000000}""")
        val enabled = api.enableGroupLink("g1")
        assertEquals("https://zalo.me/g/new1", enabled.link)

        api.disableGroupLink("g1")

        responses += TransportResponse(200, """{"groupId":"g_new"}""")
        val joined = api.joinGroupLink("https://zalo.me/g/joinme")
        assertEquals("g_new", joined.groupId)

        api.addGroupDeputy("g1", "u5")
        api.removeGroupDeputy("g1", "u5")

        responses += TransportResponse(
            200,
            """{"items":[{"id":"n1","kind":"note","title":"Sprint goals","creatorId":"u1","createTime":1700000001,"pinned":true}]}""",
        )
        val board = api.groupBoard("g1")
        assertEquals(1, board.items.size)
        assertEquals("Sprint goals", board.items.single().title)
        assertTrue(board.items.single().pinned)

        responses += TransportResponse(
            201,
            """{"id":"n2","kind":"note","title":"Release checklist","creatorId":"me","createTime":1700000099,"pinned":true}""",
        )
        val createdNote = api.createGroupNote("g1", "Release checklist", pin = true)
        assertEquals("n2", createdNote.id)

        // Messages with TTL, cards, links, userInfo + lastOnline delegation
        responses += TransportResponse(201, """{"msgId":"501"}""")
        api.send("user", "u1", "Self destruct", ttl = 60_000L)

        responses += TransportResponse(201, """{}""")
        api.sendCard("group", "g1", "u7", phone = "0901234567", ttl = 60_000L)

        responses += TransportResponse(201, """{}""")
        api.sendLink("user", "u1", "https://example.com", message = "Read", ttl = 300_000L)

        responses += TransportResponse(
            200,
            """{"lastOnline":1700001234,"commonGroups":[{"id":"g_alpha","kind":"group","name":"Alpha Team","avatar":"https://a/alpha.jpg"}]}""",
        )
        val info = api.userInfo("u10")
        assertEquals(1700001234L, info.lastOnline)
        assertEquals("Alpha Team", info.commonGroups.single().name)

        responses += TransportResponse(200, """{"lastOnline":1700009999,"commonGroups":[]}""")
        assertEquals(1700009999L, api.lastOnline("u10"))

        assertEquals("PUT" to "/api/friends/u42/alias", calls[0].first to calls[0].second)
        assertTrue(calls[0].third.orEmpty().contains(""""alias":"Bestie""""))
        assertEquals("DELETE" to "/api/friends/u42/alias", calls[1].first to calls[1].second)
        assertEquals("DELETE" to "/api/friends/u42", calls[2].first to calls[2].second)
        assertEquals("POST" to "/api/users/u99/block", calls[3].first to calls[3].second)
        assertEquals("DELETE" to "/api/users/u99/block", calls[4].first to calls[4].second)
        assertEquals("GET" to "/api/groups/g1/link", calls[5].first to calls[5].second)
        assertEquals("POST" to "/api/groups/g1/link", calls[6].first to calls[6].second)
        assertEquals("DELETE" to "/api/groups/g1/link", calls[7].first to calls[7].second)
        assertEquals("POST" to "/api/groups/join-link", calls[8].first to calls[8].second)
        assertEquals("POST" to "/api/groups/g1/deputies/u5", calls[9].first to calls[9].second)
        assertEquals("DELETE" to "/api/groups/g1/deputies/u5", calls[10].first to calls[10].second)
        assertEquals("GET" to "/api/groups/g1/board?page=1&count=20", calls[11].first to calls[11].second)
        assertEquals("POST" to "/api/groups/g1/notes", calls[12].first to calls[12].second)
        assertEquals("POST" to "/api/threads/user/u1/messages", calls[13].first to calls[13].second)
        assertTrue(calls[13].third.orEmpty().contains(""""ttl":60000"""))
        assertEquals("POST" to "/api/threads/group/g1/card", calls[14].first to calls[14].second)
        assertTrue(calls[14].third.orEmpty().contains(""""userId":"u7""""))
        assertEquals("POST" to "/api/threads/user/u1/link", calls[15].first to calls[15].second)
        assertTrue(calls[15].third.orEmpty().contains(""""link":"https://example.com""""))
        assertEquals("GET" to "/api/threads/user/u10/info", calls[16].first to calls[16].second)
        assertEquals("GET" to "/api/threads/user/u10/info", calls[17].first to calls[17].second)
    }
}
