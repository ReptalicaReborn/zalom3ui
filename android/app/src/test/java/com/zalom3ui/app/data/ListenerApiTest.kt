package com.zalom3ui.app.data

import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

/** The requests for pausing the listener, the notify window and the replay test, against a fake transport. */
class ListenerApiTest {
    private class Recorded(val method: String, val path: String, val body: String)

    private class FakeTransport(private val answers: Map<String, String> = emptyMap()) : Transport {
        val requests = mutableListOf<Recorded>()

        override suspend fun execute(method: String, path: String, body: ApiBody, upload: Boolean): TransportResponse {
            val text = (body as? ApiBody.Bytes)?.bytes?.toString(Charsets.UTF_8).orEmpty()
            requests += Recorded(method, path, text)
            return TransportResponse(200, answers["$method $path"] ?: """{"state":"paused"}""")
        }

        override fun events(): Flow<String> = emptyFlow()
    }

    @Test
    fun pause_and_resume_post_to_the_listener_endpoints_and_return_the_status() = runBlocking {
        val transport = FakeTransport()
        val api = ApiClient(transport)
        val status = api.listenerPause()
        api.listenerResume()
        assertEquals("paused", status.state)
        assertTrue(status.isPaused)
        assertEquals(
            listOf("POST /api/listener/pause", "POST /api/listener/resume"),
            transport.requests.map { "${it.method} ${it.path}" },
        )
    }

    @Test
    fun the_notify_window_sends_the_trigger_and_the_ttl() = runBlocking {
        val transport = FakeTransport()
        val api = ApiClient(transport)
        api.setNotifyWindow(triggerTs = 1_700_000_000_000L, ttlMs = 90_000L)
        api.clearNotifyWindow()
        val (put, delete) = transport.requests
        assertEquals("PUT /api/notify-window", "${put.method} ${put.path}")
        assertEquals("""{"triggerTs":1700000000000,"ttlMs":90000}""", put.body)
        assertEquals("DELETE /api/notify-window", "${delete.method} ${delete.path}")
    }

    @Test
    fun the_replay_log_and_mode_use_the_debug_endpoints() = runBlocking {
        val transport = FakeTransport(
            mapOf(
                "GET /api/debug/replay-log" to "=== connected 2026-01-01T00:00:00Z  mode=null\n",
                "GET /api/debug/replay-mode" to """{"mode":"page"}""",
            ),
        )
        val api = ApiClient(transport)
        assertEquals("=== connected 2026-01-01T00:00:00Z  mode=null\n", api.replayLog())
        assertEquals("page", api.replayMode())
        api.setReplayMode("newest")
        api.clearReplayLog()
        val put = transport.requests.first { it.method == "PUT" }
        assertEquals("/api/debug/replay-mode", put.path)
        assertEquals("""{"mode":"newest"}""", put.body)
        assertTrue(transport.requests.any { it.method == "DELETE" && it.path == "/api/debug/replay-log" })
    }
}
