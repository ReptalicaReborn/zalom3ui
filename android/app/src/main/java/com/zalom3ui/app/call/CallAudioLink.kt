package com.zalom3ui.app.call

import android.content.Context
import com.zalom3ui.app.data.ApiClient
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.server.ZaloCore
import okhttp3.Request
import java.util.concurrent.atomic.AtomicBoolean

/** One audio packet from the server: RTP sequence, RTP timestamp and the Opus payload. */
class IncomingAudio(val seq: Int, val timestamp: Long, val opus: ByteArray)

/** One H.264 access unit from the server (Annex B, SPS/PPS in front of a key frame); [timestamp] is 90 kHz. */
class IncomingVideo(val key: Boolean, val timestamp: Long, val accessUnit: ByteArray)

/**
 * The call's audio and video link (docs/call-audio.md section 3). Binary messages only: server to app
 * `01 | u16 seq | u32 timestamp | opus`, app to server `02 | opus`. Video rides the same link:
 * `03 | flags | u32 timestamp | AU` in, `04 | flags | AU` out, `05` (the other side wants a key frame) in,
 * `06` (we need a key frame) out.
 *
 * The messages travel over an [AudioTransport]: a WebSocket to a Zalo server ([WebSocketAudioTransport]), or a
 * handle into the core running in this process ([NativeAudioTransport]). Framing, backlog limits and close handling
 * are the same for both. Call [connect] to open the link.
 */
class CallAudioLink internal constructor(
    private val transport: AudioTransport,
    private val onAudio: (seq: Int, timestamp: Long, opus: ByteArray) -> Unit,
    private val onClosed: (reason: String?) -> Unit,
    private val onVideo: (key: Boolean, timestamp: Long, accessUnit: ByteArray) -> Unit = { _, _, _ -> },
    private val onKeyFrameWanted: () -> Unit = {},
) {
    private val closed = AtomicBoolean(false)
    private val finished = AtomicBoolean(false)

    @Synchronized
    fun connect() {
        if (closed.get()) return
        transport.connect(Listener())
    }

    /** Sends one 20 ms Opus frame; false if the link is not open or its queue is full. */
    fun send(opus: ByteArray): Boolean = transport.send(encodeOutgoing(opus))

    /**
     * Sends one encoded camera frame. A delta frame is dropped (false) when the link is already backed up, so a slow
     * link makes the picture stutter instead of lagging further and further behind. Key frames are also capped well
     * below OkHttp's 16 MiB disconnect threshold.
     */
    fun sendVideo(key: Boolean, accessUnit: ByteArray): Boolean {
        if (!shouldSendVideo(key, transport.queueSize())) return false
        return transport.send(encodeVideoOut(key, accessUnit))
    }

    /** Asks the other side for a key frame (our decoder is starting or lost a frame). */
    fun sendKeyFrameNeeded(): Boolean = transport.send(encodeKeyFrameNeeded())

    fun close() {
        closed.set(true)
        transport.close()
    }

    private inner class Listener : AudioTransport.Listener {
        override fun onMessage(msg: ByteArray) {
            when (msg.firstOrNull()?.toInt()) {
                TYPE_AUDIO_IN -> decodeIncoming(msg)?.let { onAudio(it.seq, it.timestamp, it.opus) }
                TYPE_VIDEO_IN -> decodeVideoIn(msg)?.let { onVideo(it.key, it.timestamp, it.accessUnit) }
                TYPE_KEY_WANTED -> onKeyFrameWanted()
            }
        }

        override fun onClosed(reason: String?) {
            if (!finished.compareAndSet(false, true)) return
            this@CallAudioLink.onClosed(reason)
        }
    }

    companion object {
        const val TYPE_AUDIO_IN = 0x01
        const val TYPE_AUDIO_OUT = 0x02
        const val TYPE_VIDEO_IN = 0x03
        const val TYPE_VIDEO_OUT = 0x04
        const val TYPE_KEY_WANTED = 0x05
        const val TYPE_KEY_NEEDED = 0x06
        internal const val MAX_AUDIO_PAYLOAD = 4096
        private const val HEADER_IN = 7
        private const val HEADER_VIDEO_IN = 6
        private const val FLAG_KEY = 1

        /** Bytes the socket may hold unsent before delta frames are dropped (about a second at our bitrate). */
        internal const val MAX_VIDEO_BACKLOG = 96 * 1024L

        /** Upper cap on queued bytes for key frames too, well below OkHttp's 16 MiB disconnect limit. */
        internal const val MAX_KEYFRAME_BACKLOG = 4 * 1024 * 1024L

        /**
         * The link the settings ask for: into the core on this phone, or over a WebSocket to the configured server.
         * Null while a server is chosen but no address is set.
         */
        fun forSettings(
            context: Context,
            onAudio: (seq: Int, timestamp: Long, opus: ByteArray) -> Unit,
            onClosed: (reason: String?) -> Unit,
            onVideo: (key: Boolean, timestamp: Long, accessUnit: ByteArray) -> Unit = { _, _, _ -> },
            onKeyFrameWanted: () -> Unit = {},
        ): CallAudioLink? {
            val app = context.applicationContext ?: context
            val settings = SettingsStore(app).load()
            val transport = when {
                settings.onDevice -> NativeAudioTransport(ZaloCore) { ZaloCore.ensureStarted(app) }
                ApiClient.normalizeServerUrl(settings.serverUrl) != null ->
                    WebSocketAudioTransport(settings.serverUrl, settings.token.ifBlank { null })
                else -> return null
            }
            return CallAudioLink(transport, onAudio, onClosed, onVideo, onKeyFrameWanted)
        }

        internal fun shouldSendVideo(key: Boolean, queueSize: Long): Boolean =
            if (key) queueSize <= MAX_KEYFRAME_BACKLOG else queueSize <= MAX_VIDEO_BACKLOG

        fun buildUrl(baseUrl: String): String {
            val ws = baseUrl.trimEnd('/')
                .replaceFirst("https://", "wss://")
                .replaceFirst("http://", "ws://")
            return "$ws/ws/call-audio"
        }

        fun buildRequest(baseUrl: String, token: String?): Request =
            Request.Builder()
                .url(buildUrl(baseUrl))
                .apply {
                    if (!token.isNullOrEmpty()) header("Authorization", "Bearer $token")
                }
                .build()

        fun encodeOutgoing(opus: ByteArray): ByteArray {
            val out = ByteArray(opus.size + 1)
            out[0] = TYPE_AUDIO_OUT.toByte()
            System.arraycopy(opus, 0, out, 1, opus.size)
            return out
        }

        /** Parses a server message; null for unknown types, oversized payloads, or messages too short to hold a header and payload. */
        fun decodeIncoming(msg: ByteArray): IncomingAudio? {
            if (msg.size <= HEADER_IN || msg[0].toInt() != TYPE_AUDIO_IN) return null
            if (msg.size - HEADER_IN > MAX_AUDIO_PAYLOAD) return null
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
