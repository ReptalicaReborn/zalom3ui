package com.zalom3ui.app.data

import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.suspendCancellableCoroutine
import okhttp3.Call
import okhttp3.Callback
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.MediaType
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.MultipartBody
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody
import okhttp3.RequestBody.Companion.asRequestBody
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.BufferedSink
import okio.source
import java.io.IOException
import java.io.InputStream
import java.util.concurrent.TimeUnit
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

/** REST over OkHttp and events over the server's `/ws`, for a Zalo server reached over the network. */
internal class HttpTransport(serverUrl: String, private val token: String?) : Transport {
    private val baseUrl: HttpUrl = ApiClient.normalizeServerUrl(serverUrl)
        ?: throw IllegalArgumentException("Invalid server URL: $serverUrl")

    /** The full URL of [path] (percent-encoded path and query) below the server address, keeping any path prefix. */
    internal fun url(path: String): HttpUrl = (baseUrl.toString().trimEnd('/') + path).toHttpUrl()

    internal fun eventsRequest(): Request {
        val wsUrl = baseUrl.newBuilder()
            .addPathSegment("ws")
            .build()
        return Request.Builder()
            .url(wsUrl.toString().replaceFirst("http", "ws"))
            .apply { token?.let { header("Authorization", "Bearer $it") } }
            .build()
    }

    internal fun buildRequest(method: String, path: String, body: ApiBody): Request.Builder {
        val builder = Request.Builder().url(url(path))
        when (method) {
            "GET" -> builder.get()
            "DELETE" -> builder.delete()
            else -> builder.method(method, requestBody(path, body))
        }
        return builder
    }

    private fun requestBody(path: String, body: ApiBody): RequestBody = when (body) {
        ApiBody.None -> "".toRequestBody(JSON_MEDIA_TYPE)
        is ApiBody.Bytes -> body.bytes.toRequestBody(body.contentType.toMediaType())
        is ApiBody.Stream -> StreamBody(body.contentType?.toMediaTypeOrNull() ?: OCTET_STREAM, body.length, body.open)
        is ApiBody.VoiceFile -> MultipartBody.Builder()
            .setType(MultipartBody.FORM)
            .addFormDataPart("voice", body.file.name, body.file.asRequestBody(body.contentType.toMediaType()))
            .build()
    }

    override suspend fun execute(method: String, path: String, body: ApiBody, upload: Boolean): TransportResponse =
        suspendCancellableCoroutine { cont ->
            val builder = buildRequest(method, path, body)
            token?.let { builder.header("Authorization", "Bearer $it") }
            val call = (if (upload) UPLOAD_HTTP else http).newCall(builder.build())
            cont.invokeOnCancellation { call.cancel() }
            call.enqueue(object : Callback {
                override fun onFailure(call: Call, e: IOException) {
                    if (cont.isActive) cont.resumeWithException(e)
                }

                override fun onResponse(call: Call, response: Response) {
                    try {
                        response.use { res ->
                            val text = res.body?.string().orEmpty()
                            if (cont.isActive) cont.resume(TransportResponse(res.code, text))
                        }
                    } catch (e: Throwable) {
                        if (cont.isActive) cont.resumeWithException(e)
                    }
                }
            })
        }

    override fun events(): Flow<String> = callbackFlow {
        val socket = http.newWebSocket(eventsRequest(), object : WebSocketListener() {
            override fun onMessage(webSocket: WebSocket, text: String) {
                trySend(text)
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                close()
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                close(if (response?.code == 401) ApiException("Wrong access token", 401) else t)
            }
        })
        awaitClose { socket.close(1000, null) }
    }

    /** Streams a picked photo or file from the content resolver instead of loading it into memory. */
    private class StreamBody(private val type: MediaType, private val length: Long, private val open: () -> InputStream) : RequestBody() {
        override fun contentType(): MediaType = type

        override fun contentLength(): Long = length

        override fun writeTo(sink: BufferedSink) {
            open().source().use { sink.writeAll(it) }
        }
    }

    internal companion object {
        private val JSON_MEDIA_TYPE = "application/json".toMediaType()
        private val OCTET_STREAM = "application/octet-stream".toMediaType()

        // Shared so the connection pool survives client rebuilds after settings changes and is reused by Coil.
        internal val SHARED_HTTP: OkHttpClient = OkHttpClient.Builder()
            .connectTimeout(10, TimeUnit.SECONDS)
            .readTimeout(30, TimeUnit.SECONDS)
            .pingInterval(30, TimeUnit.SECONDS)
            .build()

        private val http: OkHttpClient get() = SHARED_HTTP

        /** Uploads go to Zalo before the server answers: give them minutes, not seconds. */
        private val UPLOAD_HTTP: OkHttpClient = SHARED_HTTP.newBuilder()
            .writeTimeout(5, TimeUnit.MINUTES)
            .readTimeout(5, TimeUnit.MINUTES)
            .build()
    }
}
