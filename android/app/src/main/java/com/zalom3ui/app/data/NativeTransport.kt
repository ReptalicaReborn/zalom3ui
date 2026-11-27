package com.zalom3ui.app.data

import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.awaitCancellation
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.io.IOException
import java.net.URLEncoder
import java.util.UUID

/**
 * The native calls into the Zalo core (libzalocore.so, see `ZaloCore`). All of them block; call them from background
 * threads. Pull model: the core never calls back into the JVM.
 */
internal interface CoreNatives {
    /**
     * Runs one request through the core's router. Returns `u16 BE status | u16 BE content-type length | content type
     * (UTF-8) | body`; status 0 means the call itself failed and the body is the message.
     */
    fun request(method: String, pathAndQuery: String, contentType: String, body: ByteArray?): ByteArray

    /** Like [request], with the body streamed from [filePath] (attachments and voice go up to 100 MB). */
    fun requestFile(method: String, pathAndQuery: String, contentType: String, filePath: String): ByteArray

    /** Opens an event subscription; 0 when the core is not running. */
    fun eventsOpen(): Long

    /** The next event's JSON text; null after [timeoutMs] with none; the empty string once the subscription is closed. */
    fun eventsNext(handle: Long, timeoutMs: Long): String?

    fun eventsClose(handle: Long)

    /** Opens the call-audio link. A handle above 0, [AUDIO_NO_ACTIVE_CALL] when no call is in progress, or another negative number. */
    fun audioOpen(): Long

    /** Sends one app-to-server frame (`0x02` audio, `0x04` video, `0x06` key-frame request); false when closed. */
    fun audioSend(handle: Long, data: ByteArray): Boolean

    /** The next server-to-app frame; null after [timeoutMs] with none; an empty array once the link is closed. */
    fun audioNext(handle: Long, timeoutMs: Long): ByteArray?

    /** `"<code> <reason>"` such as `"1000 call ended"` once the link has closed, else null. */
    fun audioCloseReason(handle: Long): String?

    fun audioClose(handle: Long)

    companion object {
        /** What [audioOpen] returns without a call in progress: the code the WebSocket path closes with (1008). */
        const val AUDIO_NO_ACTIVE_CALL = -1008L
    }
}

/**
 * Talks to the Zalo core running in this process: the same REST requests and event stream as [HttpTransport], without
 * a socket, port or token. [ensureStarted] runs before every call, so the core is up whoever asks first.
 *
 * [tempDir] holds the copy of a picked file while it uploads (a content URI can only be read as a stream, the core
 * reads a file).
 */
internal class NativeTransport(
    private val natives: CoreNatives,
    private val tempDir: File,
    private val ensureStarted: () -> Boolean = { true },
) : Transport {
    override suspend fun execute(method: String, path: String, body: ApiBody, upload: Boolean): TransportResponse =
        withContext(Dispatchers.IO) {
            if (!ensureStarted()) throw IOException(CORE_NOT_STARTED)
            val raw = when (body) {
                ApiBody.None -> natives.request(method, path, defaultContentType(method), null)
                is ApiBody.Bytes -> natives.request(method, path, body.contentType, body.bytes)
                is ApiBody.Stream -> withTempCopy(body) { file ->
                    natives.requestFile(method, path, body.contentType ?: OCTET_STREAM, file.absolutePath)
                }
                is ApiBody.VoiceFile ->
                    natives.requestFile(method, voicePath(path, body.file), body.contentType, body.file.absolutePath)
            }
            decodeResponse(raw)
        }

    private fun defaultContentType(method: String) = if (method == "GET" || method == "DELETE") "" else JSON

    private inline fun <T> withTempCopy(body: ApiBody.Stream, block: (File) -> T): T {
        tempDir.mkdirs()
        val file = File(tempDir, "upload-${UUID.randomUUID()}.tmp")
        try {
            body.open().use { input -> file.outputStream().use { out -> input.copyTo(out) } }
            return block(file)
        } finally {
            file.delete()
        }
    }

    override fun events(): Flow<String> = flow {
        if (!ensureStarted()) throw IOException(CORE_NOT_STARTED)
        val handle = natives.eventsOpen()
        if (handle <= 0L) throw IOException(CORE_NOT_STARTED)
        try {
            coroutineScope {
                // Closing the handle wakes the waiting poll at once, so cancelling does not wait out the timeout.
                val watcher = launch(Dispatchers.Unconfined) {
                    try {
                        awaitCancellation()
                    } finally {
                        natives.eventsClose(handle)
                    }
                }
                try {
                    while (true) {
                        val text = withContext(Dispatchers.IO) { natives.eventsNext(handle, EVENT_POLL_MS) } ?: continue
                        if (text.isEmpty()) break // the subscription closed (core stopped or restarted)
                        emit(text)
                    }
                } finally {
                    watcher.cancel()
                }
            }
        } finally {
            natives.eventsClose(handle)
        }
    }

    internal companion object {
        const val CORE_NOT_STARTED = "The Zalo core is not running"
        private const val JSON = "application/json"
        private const val OCTET_STREAM = "application/octet-stream"
        private const val EVENT_POLL_MS = 60_000L

        /** A voice file goes up bare (no multipart); its name rides in the query. */
        internal fun voicePath(path: String, file: File): String {
            val name = URLEncoder.encode(file.name, "UTF-8").replace("+", "%20")
            return path + (if ('?' in path) "&" else "?") + "name=" + name
        }

        /** Decodes `u16 BE status | u16 BE content-type length | content type | body`; throws when status is 0 (the call failed). */
        internal fun decodeResponse(raw: ByteArray): TransportResponse {
            if (raw.size < 4) throw IOException("Malformed answer from the Zalo core")
            val status = ((raw[0].toInt() and 0xFF) shl 8) or (raw[1].toInt() and 0xFF)
            val ctLength = ((raw[2].toInt() and 0xFF) shl 8) or (raw[3].toInt() and 0xFF)
            if (raw.size < 4 + ctLength) throw IOException("Malformed answer from the Zalo core")
            val body = String(raw, 4 + ctLength, raw.size - 4 - ctLength, Charsets.UTF_8)
            if (status == 0) throw IOException(body.ifBlank { "The Zalo core failed" })
            return TransportResponse(status, body)
        }
    }
}
