package com.zalom3ui.app.ui

import com.zalom3ui.app.data.ServerEvent

/** Bytes of one upload that were sent so far, out of [total]. */
data class UploadBytes(val sent: Long, val total: Long)

/** Live uploads by chat ([com.zalom3ui.app.data.ThreadRef.key]) and then by the core's opaque upload id. */
typealias UploadsByChat = Map<String, Map<String, UploadBytes>>

/**
 * Folds one `upload_progress` event into [uploads]. A `done` event drops that upload; a chat with no uploads left
 * disappears from the map. Returns [uploads] itself when nothing changes.
 */
fun applyUploadProgress(uploads: UploadsByChat, event: ServerEvent.UploadProgress): UploadsByChat {
    val key = event.thread.key
    val chat = uploads[key].orEmpty()
    val newChat = if (event.done) {
        chat - event.uploadId
    } else {
        val bytes = UploadBytes(event.sent.coerceAtLeast(0), event.total.coerceAtLeast(0))
        if (chat[event.uploadId] == bytes) return uploads
        chat + (event.uploadId to bytes)
    }
    if (newChat == chat) return uploads
    return if (newChat.isEmpty()) uploads - key else uploads + (key to newChat)
}

/** Forgets everything known about [chatKey]'s uploads (its upload count dropped to zero, so nothing can still be live). */
fun clearUploads(uploads: UploadsByChat, chatKey: String): UploadsByChat =
    if (chatKey in uploads) uploads - chatKey else uploads

/** Overall progress 0..1 of a chat's live uploads (sum sent / sum total), or null when no upload with a known size exists. */
fun aggregateUploadProgress(entries: Map<String, UploadBytes>?): Float? {
    if (entries.isNullOrEmpty()) return null
    var sent = 0L
    var total = 0L
    for (b in entries.values) {
        sent += b.sent.coerceAtMost(b.total)
        total += b.total
    }
    if (total <= 0L) return null
    return (sent.toDouble() / total).toFloat().coerceIn(0f, 1f)
}

/** Progress per chat key, for chats that have a known one. */
fun uploadProgressByChat(uploads: UploadsByChat): Map<String, Float> {
    val out = HashMap<String, Float>(uploads.size)
    for ((key, entries) in uploads) aggregateUploadProgress(entries)?.let { out[key] = it }
    return out
}
