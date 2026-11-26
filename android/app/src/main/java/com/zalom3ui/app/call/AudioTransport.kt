package com.zalom3ui.app.call

import com.zalom3ui.app.data.CoreNatives
import okhttp3.OkHttpClient
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.ByteString
import okio.ByteString.Companion.toByteString
import java.util.concurrent.TimeUnit

/**
 * The pipe under [CallAudioLink]: binary messages both ways, and a close with an optional reason. One connection per
 * transport; [connect] is called once.
 */
internal interface AudioTransport {
    interface Listener {
        /** One binary message from the server. May run on any thread. */
        fun onMessage(msg: ByteArray)

        /** The link ended: closed by either side, replaced by a newer client, no call in progress, or failed. Once. */
        fun onClosed(reason: String?)
    }

    fun connect(listener: Listener)

    /** Sends one binary message; false if the link is not open or its queue is full. */
    fun send(msg: ByteArray): Boolean

    /** Bytes accepted but not yet sent, so a backed-up link can drop video frames; 0 when the link has no queue. */
    fun queueSize(): Long

    fun close()
}

/** The call-audio WebSocket (`/ws/call-audio`) of a Zalo server reached over the network. */
internal class WebSocketAudioTransport(
    baseUrl: String,
    token: String?,
    client: OkHttpClient = sharedClient,
) : AudioTransport {
    private val request = CallAudioLink.buildRequest(baseUrl, token)

    // No read timeout: the socket is silent whenever the other side is.
    private val http: OkHttpClient = if (client === sharedClient) {
        sharedClient
    } else {
        client.newBuilder().readTimeout(0, TimeUnit.MILLISECONDS).build()
    }

    @Volatile private var socket: WebSocket? = null
    private var closed = false

    @Synchronized
    override fun connect(listener: AudioTransport.Listener) {
        if (socket != null || closed) return
        socket = http.newWebSocket(request, object : WebSocketListener() {
            override fun onMessage(webSocket: WebSocket, bytes: ByteString) = listener.onMessage(bytes.toByteArray())

            // Text messages are ignored: the audio socket only carries binary frames.

            override fun onClosing(webSocket: WebSocket, code: Int, reason: String) {
                webSocket.close(code, null)
                end(reason.ifEmpty { null })
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) = end(reason.ifEmpty { null })

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) = end(t.message)

            private fun end(reason: String?) {
                socket = null
                listener.onClosed(reason)
            }
        })
    }

    override fun send(msg: ByteArray): Boolean = socket?.send(msg.toByteString()) ?: false

    override fun queueSize(): Long = socket?.queueSize() ?: 0L

    override fun close() {
        val s = synchronized(this) {
            closed = true
            val current = socket
            socket = null
            current
        }
        if (s != null && !s.close(1000, null)) {
            s.cancel()
        }
    }

    private companion object {
        val sharedClient: OkHttpClient by lazy {
            OkHttpClient.Builder()
                .readTimeout(0, TimeUnit.MILLISECONDS)
                .pingInterval(20, TimeUnit.SECONDS)
                .build()
        }
    }
}

/**
 * A handle into the core running in this process. A thread of its own pulls messages out of the core (the core never
 * calls into the JVM); sends go straight in. Closes the way the WebSocket does: "No active call" without a call,
 * "replaced" when a newer client took over, "call ended" when the call ends.
 */
internal class NativeAudioTransport(
    private val natives: CoreNatives,
    private val ensureStarted: () -> Boolean = { true },
) : AudioTransport {
    @Volatile private var handle = 0L
    @Volatile private var closed = false
    private var started = false

    @Synchronized
    override fun connect(listener: AudioTransport.Listener) {
        if (started || closed) return
        started = true
        Thread({ pump(listener) }, "zalo-call-audio").apply {
            isDaemon = true
            start()
        }
    }

    private fun pump(listener: AudioTransport.Listener) {
        if (!ensureStarted()) {
            listener.onClosed("The Zalo core is not running")
            return
        }
        val h = natives.audioOpen()
        if (h <= 0L) {
            listener.onClosed(if (h == CoreNatives.AUDIO_NO_ACTIVE_CALL) NO_ACTIVE_CALL else "The Zalo core is not running")
            return
        }
        handle = h
        var reason: String? = null
        try {
            while (!closed) {
                val frame = natives.audioNext(h, POLL_MS) ?: continue
                if (frame.isEmpty()) break // closed: read the reason below
                listener.onMessage(frame)
            }
            reason = closeReasonText(natives.audioCloseReason(h))
        } finally {
            handle = 0L
            natives.audioClose(h)
            listener.onClosed(reason)
        }
    }

    override fun send(msg: ByteArray): Boolean {
        val h = handle
        return h > 0L && !closed && natives.audioSend(h, msg)
    }

    // Frames go straight into the core's channel; there is no socket queue to measure.
    override fun queueSize(): Long = 0L

    override fun close() {
        closed = true
        // Wakes the waiting audioNext at once; the pump then reports the close.
        val h = handle
        if (h > 0L) natives.audioClose(h)
    }

    internal companion object {
        const val NO_ACTIVE_CALL = "No active call"
        private const val POLL_MS = 500L

        /** The reason part of `"<code> <reason>"`, or null when there is none. */
        internal fun closeReasonText(info: String?): String? = info?.substringAfter(' ', "")?.ifEmpty { null }
    }
}
