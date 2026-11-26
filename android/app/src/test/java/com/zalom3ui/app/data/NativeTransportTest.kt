package com.zalom3ui.app.data

import kotlinx.coroutines.CoroutineStart
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.async
import kotlinx.coroutines.cancelAndJoin
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.launch
import kotlinx.coroutines.runBlocking
import kotlinx.coroutines.withTimeout
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.File
import java.io.IOException
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/** What the core's `request` answers: `u16 status | u16 content-type length | content type | body`. */
internal fun encodeNative(status: Int, contentType: String, body: String): ByteArray {
    val ct = contentType.toByteArray()
    val b = body.toByteArray()
    return byteArrayOf((status shr 8).toByte(), status.toByte(), (ct.size shr 8).toByte(), ct.size.toByte()) + ct + b
}

internal fun jsonAnswer(status: Int, body: String) = encodeNative(status, "application/json", body)

/** A scripted core: records requests, answers from [answer], hands out events from [events]. */
internal class FakeNatives(
    var answer: (method: String, path: String, contentType: String, body: ByteArray?) -> ByteArray =
        { _, _, _, _ -> jsonAnswer(200, "{}") },
) : CoreNatives {
    class Call(val method: String, val path: String, val contentType: String, val body: ByteArray?, val fileBytes: ByteArray? = null)

    val calls = CopyOnWriteArrayList<Call>()
    val events = LinkedBlockingQueue<String>()
    val eventsClosed = AtomicBoolean(false)
    val closedHandles = CopyOnWriteArrayList<Long>()

    override fun request(method: String, pathAndQuery: String, contentType: String, body: ByteArray?): ByteArray {
        calls += Call(method, pathAndQuery, contentType, body)
        return answer(method, pathAndQuery, contentType, body)
    }

    override fun requestFile(method: String, pathAndQuery: String, contentType: String, filePath: String): ByteArray {
        val bytes = File(filePath).readBytes()
        calls += Call(method, pathAndQuery, contentType, null, bytes)
        return answer(method, pathAndQuery, contentType, bytes)
    }

    override fun eventsOpen(): Long = 7

    override fun eventsNext(handle: Long, timeoutMs: Long): String? {
        if (eventsClosed.get()) return ""
        // Polls in small steps so a close wakes the wait, like the real core.
        val deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMs)
        while (System.nanoTime() < deadline) {
            events.poll(10, TimeUnit.MILLISECONDS)?.let { return it }
            if (eventsClosed.get()) return ""
        }
        return null
    }

    override fun eventsClose(handle: Long) {
        closedHandles += handle
        eventsClosed.set(true)
    }

    override fun audioOpen(): Long = CoreNatives.AUDIO_NO_ACTIVE_CALL

    override fun audioSend(handle: Long, data: ByteArray) = false

    override fun audioNext(handle: Long, timeoutMs: Long): ByteArray? = null

    override fun audioCloseReason(handle: Long): String? = null

    override fun audioClose(handle: Long) = Unit
}

class NativeTransportTest {
    private fun tempDir() = File.createTempFile("zalo-uploads", "").apply { delete(); mkdirs(); deleteOnExit() }

    private fun transport(natives: FakeNatives, ensureStarted: () -> Boolean = { true }) =
        NativeTransport(natives, tempDir(), ensureStarted)

    // ---- Response decoding ----

    @Test
    fun decodeResponse_readsStatusContentTypeAndBody() {
        val r = NativeTransport.decodeResponse(encodeNative(201, "application/json", """{"a":"hỏi"}"""))
        assertEquals(201, r.code)
        assertEquals("""{"a":"hỏi"}""", r.body)
    }

    @Test
    fun decodeResponse_keepsStatusesAbove255AndEmptyParts() {
        val r = NativeTransport.decodeResponse(encodeNative(404, "", ""))
        assertEquals(404, r.code)
        assertEquals("", r.body)
        assertEquals(500, NativeTransport.decodeResponse(encodeNative(500, "text/plain", "x")).code)
    }

    @Test
    fun decodeResponse_statusZeroIsACallFailureWithTheMessage() {
        try {
            NativeTransport.decodeResponse(encodeNative(0, "text/plain", "The Zalo core is not running"))
            fail("expected IOException")
        } catch (e: IOException) {
            assertEquals("The Zalo core is not running", e.message)
        }
    }

    @Test
    fun decodeResponse_rejectsMalformedAnswers() {
        for (raw in listOf(ByteArray(0), byteArrayOf(0, 100, 0), byteArrayOf(0, 100, 0, 9, 1, 2))) {
            try {
                NativeTransport.decodeResponse(raw)
                fail("expected IOException for ${raw.toList()}")
            } catch (_: IOException) {
            }
        }
    }

    // ---- execute ----

    @Test
    fun execute_sendsMethodPathAndBodyThroughRequest() = runBlocking {
        val natives = FakeNatives { _, _, _, _ -> jsonAnswer(200, """{"ok":true}""") }
        val t = transport(natives)

        val get = t.execute("GET", "/api/threads?limit=5")
        assertEquals(200, get.code)
        assertEquals("""{"ok":true}""", get.body)

        t.execute("POST", "/api/x", ApiBody.Bytes("application/json", """{"a":1}""".toByteArray()))
        t.execute("PUT", "/api/presence", ApiBody.None)
        t.execute("DELETE", "/api/threads/user/1")

        val c = natives.calls
        assertEquals(listOf("GET", "POST", "PUT", "DELETE"), c.map { it.method })
        assertEquals("/api/threads?limit=5", c[0].path)
        assertEquals("", c[0].contentType)
        assertNull(c[0].body)
        assertEquals("application/json", c[1].contentType)
        assertEquals("""{"a":1}""", String(c[1].body!!))
        // An empty body on a write goes out as an empty JSON body, as over HTTP.
        assertEquals("application/json", c[2].contentType)
        assertNull(c[2].body)
        assertEquals("", c[3].contentType)
    }

    @Test
    fun execute_streamsAPickedFileThroughATempFileAndCleansUp() = runBlocking {
        val natives = FakeNatives { _, _, _, _ -> jsonAnswer(201, """{"msgIds":["1"]}""") }
        val dir = tempDir()
        val t = NativeTransport(natives, dir) { true }
        val payload = ByteArray(300_000) { (it % 251).toByte() }

        val r = t.execute("POST", "/api/threads/user/9/attachments?name=a.jpg", ApiBody.Stream("image/jpeg", payload.size.toLong()) { ByteArrayInputStream(payload) }, upload = true)

        assertEquals(201, r.code)
        val call = natives.calls.single()
        assertEquals("image/jpeg", call.contentType)
        assertNull("the body never travels as a byte array", call.body)
        assertArrayEquals(payload, call.fileBytes)
        assertEquals("temp copy removed", 0, dir.listFiles()?.size ?: 0)
    }

    @Test
    fun execute_streamWithoutMimeTypeIsOctetStream() = runBlocking {
        val natives = FakeNatives()
        transport(natives).execute("POST", "/api/p", ApiBody.Stream(null, 1) { ByteArrayInputStream(byteArrayOf(1)) })
        assertEquals("application/octet-stream", natives.calls.single().contentType)
    }

    @Test
    fun execute_voiceFileGoesUpBareWithItsNameInTheQuery() = runBlocking {
        val natives = FakeNatives { _, _, _, _ -> jsonAnswer(201, "{}") }
        val file = File.createTempFile("voice note ", ".m4a").apply { writeBytes(byteArrayOf(1, 2, 3)); deleteOnExit() }

        transport(natives).execute("POST", "/api/threads/group/g1/voice", ApiBody.VoiceFile(file, "audio/mp4"), upload = true)

        val call = natives.calls.single()
        assertEquals("audio/mp4", call.contentType)
        assertTrue(call.path, call.path.startsWith("/api/threads/group/g1/voice?name=voice%20note%20"))
        assertTrue(call.path.endsWith(".m4a"))
        assertArrayEquals(byteArrayOf(1, 2, 3), call.fileBytes)
        assertEquals("/p?x=1&name=a%20b.m4a", NativeTransport.voicePath("/p?x=1", File("a b.m4a")))
    }

    @Test
    fun execute_startsTheCoreFirstAndFailsClearlyWhenItCannotStart() = runBlocking {
        val natives = FakeNatives()
        val order = mutableListOf<String>()
        natives.answer = { _, _, _, _ -> order += "request"; jsonAnswer(200, "{}") }
        transport(natives) { order += "start"; true }.execute("GET", "/api/status")
        assertEquals(listOf("start", "request"), order)

        try {
            transport(natives) { false }.execute("GET", "/api/status")
            fail("expected IOException")
        } catch (e: IOException) {
            assertEquals(NativeTransport.CORE_NOT_STARTED, e.message)
        }
    }

    // ---- events ----

    @Test
    fun events_emitTextUntilTheSubscriptionCloses() = runBlocking {
        val natives = FakeNatives()
        natives.events += """{"type":"status"}"""
        natives.events += """{"type":"threads"}"""
        val collected = async { transport(natives).events().toList() }
        withTimeout(5_000) {
            while (natives.events.isNotEmpty()) kotlinx.coroutines.yield()
        }
        natives.eventsClosed.set(true) // the core stopped: "" ends the flow normally
        assertEquals(listOf("""{"type":"status"}""", """{"type":"threads"}"""), withTimeout(5_000) { collected.await() })
        assertEquals(listOf(7L), natives.closedHandles.distinct())
    }

    @Test
    fun events_cancellingClosesTheHandleSoTheBlockedPollReturns() = runBlocking {
        val natives = FakeNatives()
        natives.events += "first"
        val seen = LinkedBlockingQueue<String>()
        val job = launch(Dispatchers.Default) {
            transport(natives).events().collect { seen += it }
        }
        assertEquals("first", seen.poll(5, TimeUnit.SECONDS))
        withTimeout(5_000) { job.cancelAndJoin() }
        assertTrue(natives.closedHandles.isNotEmpty())
    }

    @Test
    fun events_failWhenTheCoreIsNotRunning() = runBlocking {
        val natives = object : CoreNatives by FakeNatives() {
            override fun eventsOpen(): Long = 0
        }
        try {
            NativeTransport(natives, tempDir()).events().first()
            fail("expected IOException")
        } catch (e: IOException) {
            assertEquals(NativeTransport.CORE_NOT_STARTED, e.message)
        }
    }

    // ---- ApiClient over the native transport ----

    @Test
    fun apiClient_decodesModelsAndMapsStatusCodesLikeOverHttp() = runBlocking {
        val natives = FakeNatives { method, path, _, _ ->
            when {
                path == "/api/status" -> jsonAnswer(200, """{"state":"ready","ownId":"u1","ownName":"Me"}""")
                path.startsWith("/api/calls/current") -> jsonAnswer(404, """{"error":"No active call"}""")
                path.startsWith("/api/threads/user/9/read") -> jsonAnswer(409, """{"error":"Not logged in"}""")
                path.startsWith("/api/threads/user/9/typing") -> jsonAnswer(500, "not json")
                method == "DELETE" -> encodeNative(204, "", "")
                else -> jsonAnswer(200, "{}")
            }
        }
        val api = ApiClient(transport(natives))

        assertEquals("ready", api.status().state)
        // 404 on the current call means "no call", as over HTTP.
        assertNull(api.currentCall())

        val e409 = runCatching { api.markRead("user", "9") }.exceptionOrNull() as ApiException
        assertEquals(409, e409.code)
        assertEquals("Not logged in", e409.message)

        val e500 = runCatching { api.typing("user", "9") }.exceptionOrNull() as ApiException
        assertEquals(500, e500.code)
        assertEquals("HTTP 500", e500.message)

        api.deleteConversation("user", "9") // 204 with no body: Unit
        assertEquals("DELETE", natives.calls.last().method)
    }

    @Test
    fun apiClient_sendsTheSameRequestsAsOverHttp() = runBlocking {
        val natives = FakeNatives { _, _, _, _ -> jsonAnswer(200, """{"results":[],"messages":[]}""") }
        val api = ApiClient(transport(natives))

        api.send("user", "u 1", "xin chào", quoteId = "q1")
        api.setPinned("group", "g1", true)
        api.searchMessages("hỏi & đáp", 20)
        api.setPresence(null)
        api.messages("user", "u1", 30, before = 99L)

        val c = natives.calls
        assertEquals("POST" to "/api/threads/user/u%201/messages", c[0].method to c[0].path)
        assertEquals("""{"text":"xin chào","quoteId":"q1"}""", String(c[0].body!!))
        assertEquals("PUT" to "/api/threads/group/g1/pin", c[1].method to c[1].path)
        assertEquals("""{"pinned":true}""", String(c[1].body!!))
        assertEquals("GET" to "/api/search?q=h%E1%BB%8Fi%20%26%20%C4%91%C3%A1p&limit=20", c[2].method to c[2].path)
        assertEquals("PUT" to "/api/presence", c[3].method to c[3].path)
        assertEquals("{}", String(c[3].body!!))
        assertEquals("/api/threads/user/u1/messages?limit=30&before=99", c[4].path)
    }

    @Test
    fun apiClient_eventsParseTheCoreStream() = runBlocking {
        val natives = FakeNatives()
        natives.events += """{"type":"status","status":{"state":"ready"}}"""
        natives.events += """{"type":"mystery"}"""
        natives.events += "not json"
        natives.events += """{"type":"threads"}"""
        val events = ApiClient(transport(natives)).events()
        val got = mutableListOf<ServerEvent>()
        val job = launch(start = CoroutineStart.UNDISPATCHED) { events.collect { got += it } }
        withTimeout(5_000) {
            while (got.size < 2) kotlinx.coroutines.delay(10)
        }
        job.cancelAndJoin()
        assertTrue(got[0] is ServerEvent.Status)
        assertEquals(ServerEvent.ThreadsChanged, got[1])
        assertFalse(got.any { it !is ServerEvent.Status && it !is ServerEvent.ThreadsChanged })
    }
}
