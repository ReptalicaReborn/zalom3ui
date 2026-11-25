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

/** One H.264 access unit from the server (Annex B, SPS/PPS in front of a key frame); [timestamp] is 90 kHz. */
class IncomingVideo(val key: Boolean, val timestamp: Long, val accessUnit: ByteArray)

/**
 * WebSocket to the local server's call-audio endpoint (docs/call-audio.md section 3). Binary messages only:
 * server to app `01 | u16 seq | u32 timestamp | opus`, app to server `02 | opus`. Video rides the same socket:
 * `03 | flags | u32 timestamp | AU` in, `04 | flags | AU` out, `05` (the other side wants a key frame) in,
 * `06` (we need a key frame) out.
 *
 * [baseUrl] is the local server URL such as `http://127.0.0.1:41234`. Call [connect] to open the socket.
 */
class CallAudioLink(
    baseUrl: String,
    token: String?,
    private val onAudio: (seq: Int, timestamp: Long, opus: ByteArray) -> Unit,
    private val onClosed: (reason: String?) -> Unit,
    client: OkHttpClient = OkHttpClient(),
    private val onVideo: (key: Boolean, timestamp: Long, accessUnit: ByteArray) -> Unit = { _, _, _ -> },
    private val onKeyFrameWanted: () -> Unit = {},
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

    /**
     * Sends one encoded camera frame. A delta frame is dropped (false) when the socket is already backed up, so a slow
     * link makes the picture stutter instead of lagging further and further behind.
     */
    fun sendVideo(key: Boolean, accessUnit: ByteArray): Boolean {
        val s = socket ?: return false
        if (!key && s.queueSize() > MAX_VIDEO_BACKLOG) return false
        return s.send(encodeVideoOut(key, accessUnit).toByteString())
    }

    /** Asks the other side for a key frame (our decoder is starting or lost a frame). */
    fun sendKeyFrameNeeded(): Boolean = socket?.send(encodeKeyFrameNeeded().toByteString()) ?: false

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
            val msg = bytes.toByteArray()
            when (msg.firstOrNull()?.toInt()) {
                TYPE_AUDIO_IN -> decodeIncoming(msg)?.let { onAudio(it.seq, it.timestamp, it.opus) }
                TYPE_VIDEO_IN -> decodeVideoIn(msg)?.let { onVideo(it.key, it.timestamp, it.accessUnit) }
                TYPE_KEY_WANTED -> onKeyFrameWanted()
            }
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
        const val TYPE_VIDEO_IN = 0x03
        const val TYPE_VIDEO_OUT = 0x04
        const val TYPE_KEY_WANTED = 0x05
        const val TYPE_KEY_NEEDED = 0x06
        private const val HEADER_IN = 7
        private const val HEADER_VIDEO_IN = 6
        private const val FLAG_KEY = 1

        /** Bytes the socket may hold unsent before delta frames are dropped (about a second at our bitrate). */
        private const val MAX_VIDEO_BACKLOG = 96 * 1024L

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

        fun encodeVideoOut(key: Boolean, accessUnit: ByteArray): ByteArray {
            val out = ByteArray(accessUnit.size + 2)
            out[0] = TYPE_VIDEO_OUT.toByte()
            out[1] = (if (key) FLAG_KEY else 0).toByte()
            System.arraycopy(accessUnit, 0, out, 2, accessUnit.size)
            return out
        }

        fun encodeKeyFrameNeeded(): ByteArray = byteArrayOf(TYPE_KEY_NEEDED.toByte())

        /** Parses a `03` message; null for other types or a header without a picture. */
        fun decodeVideoIn(msg: ByteArray): IncomingVideo? {
            if (msg.size <= HEADER_VIDEO_IN || msg[0].toInt() != TYPE_VIDEO_IN) return null
            fun u(i: Int) = msg[i].toLong() and 0xFF
            val ts = (u(2) shl 24) or (u(3) shl 16) or (u(4) shl 8) or u(5)
            return IncomingVideo((msg[1].toInt() and FLAG_KEY) != 0, ts, msg.copyOfRange(HEADER_VIDEO_IN, msg.size))
        }

        fun isKeyFrameWanted(msg: ByteArray): Boolean = msg.isNotEmpty() && msg[0].toInt() == TYPE_KEY_WANTED
    }
}
