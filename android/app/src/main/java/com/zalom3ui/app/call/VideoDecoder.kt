package com.zalom3ui.app.call

import android.media.MediaCodec
import android.media.MediaFormat
import android.os.SystemClock
import android.view.Surface

/**
 * Decodes the other side's H.264 (Annex B access units from [CallAudioLink]) onto a [Surface] with the hardware decoder.
 *
 * Starts at a key frame (it asks for one with [requestKeyFrame] until it gets it), and asks again whenever the codec
 * fails or frames had to be dropped. All MediaCodec calls happen on one worker thread; [submit] only queues.
 * [onSize] reports the picture size (also after the other side changes resolution) so the screen can keep the aspect.
 */
class VideoDecoder(
    private val requestKeyFrame: () -> Unit,
    private val onSize: (width: Int, height: Int) -> Unit,
) {
    private class Frame(val key: Boolean, val timestamp: Long, val data: ByteArray)

    @Suppress("PLATFORM_CLASS_MAPPED_TO_KOTLIN")
    private val lock = Object()
    private val queue = ArrayDeque<Frame>()
    private var surface: Surface? = null
    private var surfaceChanged = false
    private var waitingForKey = true
    private var running = true
    private var lastAsk = 0L

    // Only the worker touches these.
    private var codec: MediaCodec? = null
    private var timeBase = 0L
    private var lastTs = -1L

    @Volatile private var codecReleased = true

    private val worker = Thread({ run() }, "video-decoder").apply { isDaemon = true; start() }

    /** Queues one access unit; called from the socket's thread. Never blocks. */
    fun submit(key: Boolean, timestamp: Long, accessUnit: ByteArray) {
        synchronized(lock) {
            if (!running || surface == null) return
            if (waitingForKey && !key) {
                askForKeyFrame()
                return
            }
            if (key) waitingForKey = false
            if (queue.size >= MAX_QUEUE) {
                // Behind: throw the backlog away. Delta frames can't be used without it, so wait for the next key frame.
                queue.clear()
                if (!key) {
                    waitingForKey = true
                    askForKeyFrame()
                    return
                }
            }
            queue.addLast(Frame(key, timestamp, accessUnit))
            lock.notifyAll()
        }
    }

    /**
     * Sets where the picture goes; null when the screen's surface is going away. With null this returns only after the
     * decoder let go of the old surface, so the caller may destroy it.
     */
    fun setSurface(s: Surface?) {
        synchronized(lock) {
            surface = s
            surfaceChanged = true
            queue.clear()
            waitingForKey = true
            codecReleased = false
            lock.notifyAll()
            if (s == null) {
                val end = SystemClock.elapsedRealtime() + RELEASE_WAIT_MS
                while (!codecReleased && running) {
                    val left = end - SystemClock.elapsedRealtime()
                    if (left <= 0) break
                    lock.wait(left)
                }
            } else {
                askForKeyFrame(force = true)
            }
        }
    }

    fun release() {
        synchronized(lock) {
            running = false
            lock.notifyAll()
        }
        runCatching { worker.join(RELEASE_WAIT_MS) }
    }

    private fun askForKeyFrame(force: Boolean = false) {
        val now = SystemClock.elapsedRealtime()
        if (!force && now - lastAsk < KEY_FRAME_RETRY_MS) return
        lastAsk = now
        requestKeyFrame()
    }

    private fun run() {
        try {
            while (true) {
                var frame: Frame? = null
                var newSurface: Surface? = null
                var swap = false
                synchronized(lock) {
                    if (!running) return@synchronized
                    if (!surfaceChanged && queue.isEmpty()) lock.wait(IDLE_WAIT_MS)
                    if (surfaceChanged) {
                        surfaceChanged = false
                        newSurface = surface
                        swap = true
                    }
                    frame = queue.removeFirstOrNull()
                }
                if (!running) break
                if (swap) {
                    releaseCodec()
                    synchronized(lock) {
                        codecReleased = true
                        lock.notifyAll()
                    }
                    if (newSurface == null) continue
                }
                frame?.let { feed(it) }
                drain()
            }
        } finally {
            releaseCodec()
            synchronized(lock) {
                codecReleased = true
                lock.notifyAll()
            }
        }
    }

    private fun feed(f: Frame) {
        var c = codec
        if (c == null) {
            if (!f.key) return
            val s = synchronized(lock) { surface } ?: return
            c = try {
                open(s)
            } catch (_: Exception) {
                failed()
                return
            }
            codec = c
            timeBase = 0
            lastTs = -1
        }
        try {
            val i = c.dequeueInputBuffer(INPUT_WAIT_US)
            if (i < 0) {
                // The decoder is not taking frames: drop this one and resynchronise on a key frame.
                if (!f.key) failedSoft()
                return
            }
            val buf = c.getInputBuffer(i) ?: return
            buf.clear()
            if (f.data.size > buf.capacity()) {
                c.queueInputBuffer(i, 0, 0, 0, 0)
                failed()
                return
            }
            buf.put(f.data)
            c.queueInputBuffer(i, 0, f.data.size, ptsUs(f.timestamp), if (f.key) MediaCodec.BUFFER_FLAG_KEY_FRAME else 0)
        } catch (_: Exception) {
            failed()
        }
    }

    /** RTP timestamps are 32-bit and wrap; the codec wants a pts that only grows. */
    private fun ptsUs(ts: Long): Long {
        if (lastTs >= 0 && ts < lastTs && lastTs - ts > 1L shl 31) timeBase += 1L shl 32
        lastTs = ts
        return (timeBase + ts) * 1_000_000L / 90_000L
    }

    private fun open(s: Surface): MediaCodec {
        val c = MediaCodec.createDecoderByType(MIME)
        try {
            val format = MediaFormat.createVideoFormat(MIME, 640, 480)
            // Room for the other side to change resolution without rebuilding the codec.
            format.setInteger(MediaFormat.KEY_MAX_WIDTH, 1280)
            format.setInteger(MediaFormat.KEY_MAX_HEIGHT, 720)
            // Show frames as they arrive (Android 11+ decoders that support it).
            format.setInteger("low-latency", 1)
            c.configure(format, s, null, 0)
            c.start()
        } catch (e: Exception) {
            runCatching { c.release() }
            throw e
        }
        return c
    }

    private fun drain() {
        val c = codec ?: return
        val info = MediaCodec.BufferInfo()
        try {
            while (true) {
                when (val i = c.dequeueOutputBuffer(info, 0)) {
                    MediaCodec.INFO_TRY_AGAIN_LATER -> return
                    MediaCodec.INFO_OUTPUT_FORMAT_CHANGED -> reportSize(c.outputFormat)
                    else -> if (i >= 0) c.releaseOutputBuffer(i, true) else return
                }
            }
        } catch (_: Exception) {
            failed()
        }
    }

    private fun reportSize(f: MediaFormat) {
        fun int(k: String) = if (f.containsKey(k)) f.getInteger(k) else -1
        // The decoder's buffer is padded to a block size; the crop says what is really picture.
        var w = int("crop-right") - int("crop-left") + 1
        var h = int("crop-bottom") - int("crop-top") + 1
        if (int("crop-right") < 0 || w <= 0 || h <= 0) {
            w = int(MediaFormat.KEY_WIDTH)
            h = int(MediaFormat.KEY_HEIGHT)
        }
        if (w > 0 && h > 0) onSize(w, h)
    }

    /** The codec broke: rebuild it at the next key frame, which we ask for. */
    private fun failed() {
        releaseCodec()
        failedSoft()
    }

    private fun failedSoft() {
        synchronized(lock) {
            queue.clear()
            waitingForKey = true
            askForKeyFrame()
        }
    }

    private fun releaseCodec() {
        val c = codec ?: return
        codec = null
        runCatching { c.stop() }
        runCatching { c.release() }
    }

    private companion object {
        const val MIME = "video/avc"
        const val MAX_QUEUE = 8
        const val IDLE_WAIT_MS = 40L
        const val INPUT_WAIT_US = 20_000L
        const val KEY_FRAME_RETRY_MS = 1_000L
        const val RELEASE_WAIT_MS = 500L
    }
}
