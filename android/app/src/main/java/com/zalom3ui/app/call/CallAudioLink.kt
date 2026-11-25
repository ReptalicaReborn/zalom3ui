package com.zalom3ui.app.call

import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString
import java.util.concurrent.TimeUnit

/** One audio packet from the server: RTP sequence, RTP timestamp and the Opus payload. */
class IncomingAudio(val seq: Int, val timestamp: Long, val opus: ByteArray)

/**
 * WebSocket to the local server's call-audio endpoint (docs/call-audio.md section 3). Binary messages only:
 * server to app `01 | u16 seq | u32 timestamp | opus`, app to server `02 | opus`.
 *
 * [baseUrl] is the local server URL such as `http://127.0.0.1:41234`. Call [connect] to open the socket.
 */
class CallAudioLink(
    baseUrl: String,
    token: String?,
    private val onAudio: (seq: Int, timestamp: Long, opus: ByteArray) -> Unit,
    private val onClosed: (reason: String?) -> Unit,
    client: OkHttpClient = OkHttpClient(),
) {
    private val url = buildUrl(baseUrl, token)

    // No read timeout: the socket is silent whenever the other side is.
    private val http = client.newBuilder().readTimeout(0, TimeUnit.MILLISECONDS).build()

    @Volatile private var socket: WebSocket? = null
    @Volatile private var finished = false

    fun connect() {
        if (socket != null) return
        socket = http.newWebSocket(Request.Builder().url(url).build(), Listener())
    }

    /** Sends one 20 ms Opus frame; false if the socket is not open or its queue is full. */
    fun send(opus: ByteArray): Boolean = socket?.send(encodeOutgoing(opus).toByteString()) ?: false

    fun close() {
        socket?.close(1000, null)
        socket = null
    }

    private fun finish(reason: String?) {
        if (finished) return
        finished = true
        socket = null
        onClosed(reason)
    }

    private inner class Listener : WebSocketListener() {
        override fun onMessage(webSocket: WebSocket, bytes: ByteString) {
            val m = decodeIncoming(bytes.toByteArray()) ?: return
            onAudio(m.seq, m.timestamp, m.opus)
        }

        // Text messages are ignored: the audio socket only carries binary frames.

        override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
            webSocket.close(code, null)
            finish(reason.ifEmpty { null })
        }

        override fun onClosed(webSocket: WebSocket, code: Int, reason: String) = finish(reason.ifEmpty { null })

        override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) = finish(t.message)
    }

    companion object {
        const val TYPE_AUDIO_IN = 0x01
        const val TYPE_AUDIO_OUT = 0x02
        private const val HEADER_IN = 7

        fun buildUrl(baseUrl: String, token: String?): String {
            val ws = baseUrl.trimEnd('/')
                .replaceFirst("https://", "wss://")
                .replaceFirst("http://", "ws://")
            val q = if (token.isNullOrEmpty()) "" else "?token=" + java.net.URLEncoder.encode(token, "UTF-8")
            return "$ws/ws/call-audio$q"
        }

        fun encodeOutgoing(opus: ByteArray): ByteArray {
            val out = ByteArray(opus.size + 1)
            out[0] = TYPE_AUDIO_OUT.toByte()
            System.arraycopy(opus, 0, out, 1, opus.size)
            return out
        }

        /** Parses a server message; null for unknown types or messages too short to hold a header and payload. */
        fun decodeIncoming(msg: ByteArray): IncomingAudio? {
            if (msg.size <= HEADER_IN || msg[0].toInt() != TYPE_AUDIO_IN) return null
            fun u(i: Int) = msg[i].toLong() and 0xFF
            val seq = ((u(1) shl 8) or u(2)).toInt()
            val ts = (u(3) shl 24) or (u(4) shl 16) or (u(5) shl 8) or u(6)
            return IncomingAudio(seq, ts, msg.copyOfRange(HEADER_IN, msg.size))
        }
    }
}
