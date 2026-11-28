package com.zalom3ui.app.ui

import com.zalom3ui.app.data.SessionStatus
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

class ConnectionTextTest {
    private val rustls = "Connection closed (1006: IO error: peer closed connection without sending TLS close_notify: " +
        "https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html#unexpected-eof)"

    @Test
    fun rawTransportErrors_neverReachTheUser() {
        val text = humanizeConnectionError(rustls)
        assertEquals(CONNECTION_LOST, text)
        for (raw in listOf("rustls", "IO error", "1006", "http", "close_notify")) assertFalse(text.contains(raw))
        assertEquals(CONNECTION_LOST, humanizeConnectionError("Connection closed (1006)"))
        assertEquals(CONNECTION_LOST, humanizeConnectionError(null))
        assertEquals(CONNECTION_LOST, humanizeConnectionError("  "))
        assertEquals(CONNECTION_LOST, humanizeConnectionError("Something odd (code 42): java.lang.Foo"))
    }

    @Test
    fun offlineAndUnreachableGetTheirOwnShortText() {
        assertEquals(NO_INTERNET, humanizeConnectionError("Can't reach Zalo (no internet connection). Retrying in 5 s…"))
        assertEquals(NO_INTERNET, humanizeConnectionError("Unable to resolve host \"chat.zalo.me\""))
        assertEquals(CANT_REACH_ZALO, humanizeConnectionError("Can't reach Zalo (connection timed out). Retrying in 10 s…"))
        assertEquals(CANT_REACH_ZALO, humanizeConnectionError("Cannot reach server"))
        assertEquals(CANT_REACH_ZALO, humanizeConnectionError("Failed to connect to /127.0.0.1:8080"))
    }

    @Test
    fun anotherWebSessionKeepsItsExplanation() {
        val msg = "Zalo Web was opened somewhere else. Only one web session can listen at a time."
        assertEquals(msg, humanizeConnectionError(msg))
    }

    @Test
    fun bannerPrefersTheAppsOwnFailure() {
        assertEquals(CANT_REACH_ZALO, connectionBannerText(rustls, "Cannot reach server"))
        assertEquals(CONNECTION_LOST, connectionBannerText(rustls, null))
        assertEquals(CONNECTION_LOST, connectionBannerText(null, null))
    }

    @Test
    fun loggedOutNote_followsTheReason() {
        val out = SessionStatus(state = "logged_out")
        assertEquals("You were logged out from another device.", loggedOutNoteFor(out.copy(loggedOutReason = "elsewhere")))
        assertEquals("Your session expired.", loggedOutNoteFor(out.copy(loggedOutReason = "expired")))
        assertNull(loggedOutNoteFor(out))
        assertNull(loggedOutNoteFor(out.copy(loggedOutReason = "something-new")))
    }
}
