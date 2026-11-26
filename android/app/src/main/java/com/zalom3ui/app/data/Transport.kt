package com.zalom3ui.app.data

import kotlinx.coroutines.flow.Flow
import java.io.File
import java.io.InputStream

/** What a request carries. [ApiClient] builds these; each [Transport] turns them into its own wire form. */
internal sealed interface ApiBody {
    /** No body (GET, DELETE, bodiless POST). */
    data object None : ApiBody

    /** A small in-memory body, JSON in practice. */
    class Bytes(val contentType: String, val bytes: ByteArray) : ApiBody

    /** A picked photo or file, read once from [open] while it is sent; never loaded whole into memory. */
    class Stream(val contentType: String?, val length: Long, val open: () -> InputStream) : ApiBody

    /** A recorded voice message: the server takes it as multipart field `"voice"`, or as the bare file with `?name=`. */
    class VoiceFile(val file: File, val contentType: String) : ApiBody
}

/** The server's answer: status code and body text. Failing to get an answer at all throws an [java.io.IOException]. */
internal class TransportResponse(val code: Int, val body: String)

/**
 * How [ApiClient] reaches the Zalo core: [HttpTransport] to a server over the network (the "A server" mode),
 * [NativeTransport] to the core running in this process. Both speak the same REST API and event stream; the client
 * above them does the JSON, the error mapping and the models.
 */
internal interface Transport {
    /**
     * Sends one request. [path] is the percent-encoded path and query below the server root, such as
     * `/api/threads?limit=5`. [upload] asks for the long timeouts a file going to Zalo needs.
     */
    suspend fun execute(method: String, path: String, body: ApiBody = ApiBody.None, upload: Boolean = false): TransportResponse

    /**
     * The server's live events as JSON text, the first being the current status. Completes when the stream closes and
     * throws when it fails; callers reconnect.
     */
    fun events(): Flow<String>
}
