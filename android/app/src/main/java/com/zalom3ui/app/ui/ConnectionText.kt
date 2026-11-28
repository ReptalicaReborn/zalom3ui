package com.zalom3ui.app.ui

import com.zalom3ui.app.data.SessionStatus

/** Shown when nothing more specific is known: the usual state of a flaky connection, and it fixes itself. */
const val CONNECTION_LOST = "Connection lost. Reconnecting…"
const val NO_INTERNET = "No internet connection"
const val CANT_REACH_ZALO = "Can't reach Zalo"

private const val OPENED_ELSEWHERE = "Zalo Web was opened somewhere else. Only one web session can listen at a time."

/**
 * Short, human text for a connection error from the core (`status.error`, e.g. "Connection closed (1006: IO error:
 * peer closed connection without sending TLS close_notify: https://docs.rs/...)") or from the app's own requests.
 * Never returns transport detail (close codes, "IO error", library URLs); that goes to the log. Pure, for JVM tests.
 */
fun humanizeConnectionError(raw: String?): String {
    val text = raw?.trim().orEmpty()
    if (text.isEmpty()) return CONNECTION_LOST
    val t = text.lowercase()
    return when {
        // The core's own plain-language message about a second web session.
        t.contains("opened somewhere else") -> OPENED_ELSEWHERE
        listOf(
            "no internet", "unknownhost", "unable to resolve", "failed to lookup", "dns error", "name or service not known",
            "network is unreachable", "no address associated", "temporary failure in name resolution",
        ).any { t.contains(it) } -> NO_INTERNET
        listOf("timed out", "timeout", "can't reach", "cannot reach", "connection refused", "failed to connect")
            .any { t.contains(it) } -> CANT_REACH_ZALO
        else -> CONNECTION_LOST
    }
}

/** The chat list's red banner: the app's own request failure wins, else the core's status error. */
fun connectionBannerText(statusError: String?, connectionError: String?): String =
    humanizeConnectionError(connectionError ?: statusError)

/** One line above the login screen's QR when Zalo ended the session; null when there is nothing to explain. */
fun loggedOutNoteFor(status: SessionStatus): String? = when (status.loggedOutReason) {
    "elsewhere" -> "You were logged out from another device."
    "expired" -> "Your session expired."
    else -> null
}
