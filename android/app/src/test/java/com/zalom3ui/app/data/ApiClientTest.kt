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
        val withToken = ApiClient("https://zalo.example.com", "secret-123").eventsRequest()
        assertEquals("Bearer secret-123", withToken.header("Authorization"))
        assertNull(withToken.url.queryParameter("token"))
        assertEquals("/ws", withToken.url.encodedPath)

        val withoutToken = ApiClient("http://127.0.0.1:45000", null).eventsRequest()
        assertNull(withoutToken.header("Authorization"))
        assertNull(withoutToken.url.queryParameter("token"))
    }

    @Test
    fun url_encodesEachSegmentWithoutSplittingOrDoubleEncoding() {
        val api = ApiClient("http://127.0.0.1:45000", "tok")
        val built = api.url("calls", "a/b c%20d", "answer")
        assertEquals("http://127.0.0.1:45000/api/calls/a%2Fb%20c%2520d/answer", built.toString())
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
        val api = ApiClient("http://127.0.0.1:45000", "tok")
        val tmp = File.createTempFile("note-", ".m4a").apply {
            writeBytes(byteArrayOf(1, 2, 3, 4))
            deleteOnExit()
        }
        val req = api.sendVoiceRequest(ThreadRef("group", "g42"), tmp).build()
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
}
