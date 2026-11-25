package com.zalom3ui.app.data

import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class CallEventsTest {
    private val api = ApiClient("127.0.0.1:3000", null)
    private val json = Json { ignoreUnknownKeys = true }

    @Test
    fun connected_isParsed() {
        val event = api.parseEvent(
            """{"type":"call","call":{"type":"connected","callId":"c1","callerId":"u9","callerName":"Bảo","video":false,"connectedAt":1700000000000}}""",
        )
        assertEquals(ServerEvent.CallConnected("c1", "u9", "Bảo", false, 1700000000000L), event)
    }

    @Test
    fun connected_missingFieldsDefault() {
        val event = api.parseEvent("""{"type":"call","call":{"type":"connected","callId":"c1"}}""")
        assertEquals(ServerEvent.CallConnected("c1", "", "", false, 0L), event)
    }

    @Test
    fun ended_andIncoming_stillParse() {
        assertEquals(
            ServerEvent.CallEnded("c1"),
            api.parseEvent("""{"type":"call","call":{"type":"ended","callId":"c1","act":"hangup"}}"""),
        )
        val incoming = api.parseEvent(
            """{"type":"call","call":{"type":"incoming","call":{"callId":"c2","callerId":"u1","callerName":"An","video":true}}}""",
        )
        assertEquals("c2", (incoming as ServerEvent.CallIncoming).call.callId)
    }

    @Test
    fun unknownCallType_isIgnored() {
        assertNull(api.parseEvent("""{"type":"call","call":{"type":"ringing"}}"""))
    }

    @Test
    fun currentCall_decodes() {
        val call = json.decodeFromString(
            CurrentCall.serializer(),
            """{"callId":"c1","callerId":"u9","callerName":"Bảo","video":false,"state":"connected","connectedAt":1700000000000,"extra":1}""",
        )
        assertEquals(CurrentCall("c1", "u9", "Bảo", false, "connected", 1700000000000L), call)
    }

    @Test
    fun currentCall_connectingHasNoConnectedAt() {
        val call = json.decodeFromString(CurrentCall.serializer(), """{"callId":"c1","state":"connecting"}""")
        assertEquals(0L, call.connectedAt)
    }

    @Test
    fun peer_isParsed() {
        val event = api.parseEvent("""{"type":"call","call":{"type":"peer","callId":"c1","cameraOn":true,"muted":false}}""")
        assertEquals(ServerEvent.CallPeer("c1", cameraOn = true, muted = false), event)
    }

    @Test
    fun peer_mutedAndMissingFields() {
        assertEquals(
            ServerEvent.CallPeer("c1", cameraOn = false, muted = true),
            api.parseEvent("""{"type":"call","call":{"type":"peer","callId":"c1","muted":true}}"""),
        )
    }
}
