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
    private var surfaceGen = 0L
    private var handledGen = 0L
    private var releasedGen = 0L
    private var waitingForKey = true
    private var running = true
    private var lastAsk = 0L

    // Only the worker touches these.
    private var codec: MediaCodec? = null
    private var timeBase = 0L
    private var lastTs = -1L

    private val worker = Thread({ run() }, "video-decoder").apply { isDaemon = true; start() }

    /** Queues one access unit; called from the socket's thread. Never blocks. */
    fun submit(key: Boolean, timestamp: Long, accessUnit: ByteArray) {
        var ask = false
        synchronized(lock) {
            if (!running || surface == null) return
            if (waitingForKey && !key) {
                ask = shouldAskForKeyFrameLocked()
            } else {
                if (key) waitingForKey = false
                if (queue.size >= MAX_QUEUE) {
                    // Behind: throw the backlog away. Delta frames can't be used without it, so wait for the next key frame.
                    queue.clear()
                    if (!key) {
                        waitingForKey = true
                        ask = shouldAskForKeyFrameLocked()
                    } else {
                        queue.addLast(Frame(key, timestamp, accessUnit))
                        lock.notifyAll()
                    }
                } else {
                    queue.addLast(Frame(key, timestamp, accessUnit))
                    lock.notifyAll()
                }
            }
        }
        if (ask) requestKeyFrame()
    }

    /**
     * Sets where the picture goes; null when the screen's surface is going away. With null this returns only after the
     * decoder let go of the old surface, so the caller may destroy it.
     */
    fun setSurface(s: Surface?) {
        var ask = false
        synchronized(lock) {
            surface = s
            val gen = ++surfaceGen
            queue.clear()
            waitingForKey = true
            lock.notifyAll()
            if (s == null) {
                val end = SystemClock.elapsedRealtime() + RELEASE_WAIT_MS
                while (releasedGen < gen && running) {
                    val left = end - SystemClock.elapsedRealtime()
                    if (left <= 0) break
                    try {
                        lock.wait(left)
                    } catch (_: InterruptedException) {
                        Thread.currentThread().interrupt()
                        break
                    }
                }
            } else {
                ask = shouldAskForKeyFrameLocked(force = true)
            }
        }
        if (ask) requestKeyFrame()
    }

    fun release() {
        synchronized(lock) {
            running = false
            lock.notifyAll()
        }
        try {
            worker.join(RELEASE_WAIT_MS)
        } catch (_: InterruptedException) {
            Thread.currentThread().interrupt()
        }
    }

    // Caller must hold lock.
    private fun shouldAskForKeyFrameLocked(force: Boolean = false): Boolean {
        val now = SystemClock.elapsedRealtime()
        if (!force && now - lastAsk < KEY_FRAME_RETRY_MS) return false
        lastAsk = now
        return true
    }

    private fun run() {
        try {
            while (true) {
                var frame: Frame? = null
                var newSurface: Surface? = null
                var swapGen = -1L
                var interrupted = false
                synchronized(lock) {
                    if (!running) return@synchronized
                    if (surfaceGen == handledGen && queue.isEmpty()) {
                        try {
                            if (codec == null) lock.wait() else lock.wait(IDLE_WAIT_MS)
                        } catch (_: InterruptedException) {
                            Thread.currentThread().interrupt()
                            interrupted = true
                            return@synchronized
                        }
                    }
                    if (!running) return@synchronized
                    if (surfaceGen != handledGen) {
                        swapGen = surfaceGen
                        newSurface = surface
                    }
                    if (swapGen < 0 || newSurface != null) {
                        frame = queue.removeFirstOrNull()
                    }
                }
                if (!running || interrupted) break
                if (swapGen >= 0) {
                    releaseCodec()
                    synchronized(lock) {
                        handledGen = swapGen
                        releasedGen = swapGen
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
                handledGen = surfaceGen
                releasedGen = surfaceGen
                lock.notifyAll()
            }
        }
    }

    private fun feed(f: Frame) {
        var c = codec
        if (c == null) {
            if (!f.key) return
            val targetSurface: Surface
            val targetGen: Long
            synchronized(lock) {
                if (!running || surfaceGen != handledGen) return
                val s = surface ?: return
                targetSurface = s
                targetGen = surfaceGen
            }
            if (!targetSurface.isValid) return
            c = try {
                open(targetSurface)
            } catch (_: Exception) {
                failed()
                return
            }
            val stillValid = synchronized(lock) {
                running && surface === targetSurface && surfaceGen == targetGen
            }
            if (!stillValid || !targetSurface.isValid) {
                runCatching { c.stop() }
                runCatching { c.release() }
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
                failedSoft()
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
        val formats = listOf(
            MediaFormat.createVideoFormat(MIME, 640, 480).apply {
                // Room for the other side to change resolution without rebuilding the codec.
                setInteger(MediaFormat.KEY_MAX_WIDTH, 1280)
                setInteger(MediaFormat.KEY_MAX_HEIGHT, 720)
                // Show frames as they arrive (Android 11+ decoders that support it).
                setInteger("low-latency", 1)
            },
            MediaFormat.createVideoFormat(MIME, 640, 480).apply {
                setInteger(MediaFormat.KEY_MAX_WIDTH, 1280)
                setInteger(MediaFormat.KEY_MAX_HEIGHT, 720)
            },
            MediaFormat.createVideoFormat(MIME, 640, 480),
        )
        var lastError: Exception? = null
        for (format in formats) {
            val c = MediaCodec.createDecoderByType(MIME)
            try {
                c.configure(format, s, null, 0)
                c.start()
                return c
            } catch (e: Exception) {
                runCatching { c.release() }
                lastError = e
            }
        }
        throw lastError!!
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
        val left = int("crop-left")
        val top = int("crop-top")
        val right = int("crop-right")
        val bottom = int("crop-bottom")
        var w = right - left + 1
        var h = bottom - top + 1
        if (left < 0 || top < 0 || right < 0 || bottom < 0 || w <= 0 || h <= 0) {
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
        val ask = synchronized(lock) {
            queue.clear()
            waitingForKey = true
            shouldAskForKeyFrameLocked()
        }
        if (ask) requestKeyFrame()
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
