package com.zalom3ui.app.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class SessionStatusTest {
    private val api = ApiClient("127.0.0.1:3000", null)

    private fun status(json: String) = AppJson.decodeFromString(SessionStatus.serializer(), json)

    @Test
    fun resuming_isReadFromTheStatus() {
        val s = status(
            """{"state":"logging_in","qrImage":null,"scannedBy":null,"ownId":null,"ownName":null,"ownAvatar":null,"error":null,"retrying":false,"resuming":true}""",
        )
        assertEquals("logging_in", s.state)
        assertTrue(s.resuming)
        assertFalse(s.retrying)
        // Still not signed in: the places that need a signed-in core keep waiting.
        assertFalse(s.isLoggedIn)
    }

    @Test
    fun resuming_defaultsToFalseForOlderCores() {
        val old = status("""{"state":"logging_in","retrying":true}""")
        assertFalse(old.resuming)
        assertTrue(old.retrying)
        assertFalse("an old core's logging_in keeps showing the login screen", old.connectingWithSavedSession())
        assertFalse(status("""{"state":"ready"}""").resuming)
        // A null (or a field added later) doesn't break the decode either.
        assertFalse(status("""{"state":"logging_in","resuming":null}""").resuming)
        assertTrue(status("""{"state":"logging_in","resuming":true,"somethingNew":1}""").resuming)
    }

    @Test
    fun resuming_isReadFromTheStatusEvent() {
        val event = api.parseEvent(
            """{"type":"status","status":{"state":"logging_in","qrImage":null,"error":"Can't reach Zalo (no internet connection). Retrying in 5 s…","retrying":true,"resuming":true}}""",
        ) as ServerEvent.Status
        assertTrue(event.status.resuming)
        assertTrue(event.status.retrying)
        assertTrue(event.status.connectingWithSavedSession())

        val ready = api.parseEvent("""{"type":"status","status":{"state":"ready","ownId":"1","retrying":false,"resuming":false}}""") as ServerEvent.Status
        assertTrue(ready.status.isLoggedIn)
        assertFalse(ready.status.resuming)
        assertNull(api.parseEvent("""{"type":"nope"}"""))
    }

    @Test
    fun connectingWithSavedSession_isOnlyASavedSessionReconnecting() {
        // The cold start: the saved session logs in while the chats are already on the phone.
        assertTrue(SessionStatus(state = "logging_in", resuming = true).connectingWithSavedSession())
        // Zalo can't be reached and it keeps retrying: still the same saved session.
        assertTrue(SessionStatus(state = "logging_in", resuming = true, retrying = true, error = "Can't reach Zalo").connectingWithSavedSession())

        // A fresh QR login (also `logging_in`, before the QR code or after it was scanned) has to show the login screen.
        assertFalse(SessionStatus(state = "logging_in").connectingWithSavedSession())
        assertFalse(SessionStatus(state = "awaiting_qr_scan", qrImage = "data:image/png;base64,AAAA").connectingWithSavedSession())
        assertFalse(SessionStatus(state = "logging_in", resuming = true, qrImage = "data:image/png;base64,AAAA").connectingWithSavedSession())
        // Rejected, logged out, or signed in: nothing is "connecting" any more, whatever a stale flag says.
        assertFalse(SessionStatus(state = "logged_out", error = "Your Zalo session has ended.").connectingWithSavedSession())
        assertFalse(SessionStatus(state = "logged_out", resuming = true).connectingWithSavedSession())
        for (state in listOf("ready", "disconnected", "paused")) {
            assertFalse(state, SessionStatus(state = state, resuming = true).connectingWithSavedSession())
            assertFalse(state, SessionStatus(state = state).connectingWithSavedSession())
        }
    }
}
