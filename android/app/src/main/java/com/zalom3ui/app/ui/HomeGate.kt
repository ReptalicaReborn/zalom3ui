package com.zalom3ui.app.ui

import com.zalom3ui.app.data.SessionStatus

/** What the home destination shows for the session status: the sign-in or the chat list. */
enum class HomeGate { Login, Home }

/**
 * The saved chats are on the phone, so a saved session that is still reconnecting in the background
 * ([SessionStatus.connectingWithSavedSession]) opens straight into the chat list, but only when there are
 * [hasCachedThreads] to show: an empty list would look like "no chats", so that case keeps the login screen's
 * "Logging in…". A session that has to be signed in again (no saved session, a QR login in progress, a rejected saved
 * session) always gets the login screen.
 */
fun homeGateFor(status: SessionStatus, hasCachedThreads: Boolean): HomeGate = when {
    status.isLoggedIn -> HomeGate.Home
    status.connectingWithSavedSession() && hasCachedThreads -> HomeGate.Home
    else -> HomeGate.Login
}

/** The slim, non-blocking note under the chat list's top bar. */
enum class ConnectionNote { None, Connecting, WaitingForConnection }

/**
 * What the chat list says about the saved session: "Connecting…" while it reconnects in the background, "Waiting for a
 * connection…" when Zalo can't be reached and it keeps retrying, nothing otherwise. Cheap and an enum, so the chat list
 * skips recomposing on status updates that don't change it.
 */
fun connectionNoteFor(status: SessionStatus?): ConnectionNote {
    if (status == null || !status.connectingWithSavedSession()) return ConnectionNote.None
    return if (status.retrying) ConnectionNote.WaitingForConnection else ConnectionNote.Connecting
}
