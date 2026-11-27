package com.zalom3ui.app.ui

import com.zalom3ui.app.data.SessionStatus
import org.junit.Assert.assertEquals
import org.junit.Test

class HomeGateTest {
    private val resuming = SessionStatus(state = "logging_in", resuming = true)
    private val offline = resuming.copy(retrying = true, error = "Can't reach Zalo (no internet connection). Retrying in 5 s…")
    private val rejected = SessionStatus(state = "logged_out", error = "Your Zalo session has ended. Log in again with the QR code.")
    private val qrLogin = SessionStatus(state = "awaiting_qr_scan", qrImage = "data:image/png;base64,AAAA")

    @Test
    fun aSavedSessionThatIsReconnecting_opensIntoTheSavedChats() {
        assertEquals(HomeGate.Home, homeGateFor(resuming, hasCachedThreads = true))
        // Offline: the chats are still readable.
        assertEquals(HomeGate.Home, homeGateFor(offline, hasCachedThreads = true))
    }

    @Test
    fun aSavedSessionThatIsReconnecting_withoutSavedChats_keepsTheLoginScreensLoggingIn() {
        assertEquals(HomeGate.Login, homeGateFor(resuming, hasCachedThreads = false))
        assertEquals(HomeGate.Login, homeGateFor(offline, hasCachedThreads = false))
    }

    @Test
    fun whenTheUserHasToLogIn_theLoginScreenShows_savedChatsOrNot() {
        for (cached in listOf(true, false)) {
            assertEquals("no saved session", HomeGate.Login, homeGateFor(SessionStatus(state = "logged_out"), cached))
            assertEquals("saved session rejected", HomeGate.Login, homeGateFor(rejected, cached))
            assertEquals("QR code showing", HomeGate.Login, homeGateFor(qrLogin, cached))
            // A QR login that was scanned and is signing in: `logging_in`, but not the saved session.
            assertEquals("QR login signing in", HomeGate.Login, homeGateFor(SessionStatus(state = "logging_in"), cached))
            assertEquals("QR login whose code is still showing", HomeGate.Login, homeGateFor(resuming.copy(qrImage = qrLogin.qrImage), cached))
        }
    }

    @Test
    fun signedIn_showsTheHome_whateverTheSavedChats() {
        for (state in listOf("ready", "disconnected", "paused")) {
            for (cached in listOf(true, false)) {
                assertEquals(state, HomeGate.Home, homeGateFor(SessionStatus(state = state), cached))
            }
        }
    }

    @Test
    fun theNoteSaysConnecting_orWaiting_onlyWhileTheSavedSessionReconnects() {
        assertEquals(ConnectionNote.Connecting, connectionNoteFor(resuming))
        assertEquals(ConnectionNote.WaitingForConnection, connectionNoteFor(offline))

        assertEquals(ConnectionNote.None, connectionNoteFor(null))
        assertEquals(ConnectionNote.None, connectionNoteFor(SessionStatus(state = "ready")))
        assertEquals(ConnectionNote.None, connectionNoteFor(SessionStatus(state = "disconnected", retrying = true)))
        assertEquals(ConnectionNote.None, connectionNoteFor(rejected))
        assertEquals(ConnectionNote.None, connectionNoteFor(qrLogin))
        assertEquals(ConnectionNote.None, connectionNoteFor(SessionStatus(state = "logging_in")))
    }
}
