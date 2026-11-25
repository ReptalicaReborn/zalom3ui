package com.zalom3ui.app.call

// libopus through JNI (libzaloopus.so). Calls use 16 kHz mono with 20 ms frames.

private object OpusLib {
    init {
        System.loadLibrary("zaloopus")
    }

    // Touching this runs the one-time library load.
    fun load() = Unit
}

class OpusEncoder(sampleRate: Int = 16000, private val channels: Int = 1, bitrate: Int = 20_000) : AutoCloseable {
    private val frameSize = sampleRate / 50
    private var handle: Long

    init {
        OpusLib.load()
        handle = nativeCreate(sampleRate, channels, bitrate)
    }

    /** One 20 ms frame: sampleRate / 50 * channels samples. Returns the Opus packet. */
    @Synchronized
    fun encode(pcm: ShortArray): ByteArray {
        check(handle != 0L) { "OpusEncoder is closed" }
        require(pcm.size == frameSize * channels) { "expected ${frameSize * channels} samples, got ${pcm.size}" }
        return nativeEncode(handle, pcm, frameSize)
    }

    @Synchronized
    override fun close() {
        if (handle != 0L) {
            nativeDestroy(handle)
            handle = 0L
        }
    }

    private external fun nativeCreate(rate: Int, channels: Int, bitrate: Int): Long
    private external fun nativeEncode(handle: Long, pcm: ShortArray, frameSize: Int): ByteArray
    private external fun nativeDestroy(handle: Long)
}

class OpusDecoder(sampleRate: Int = 16000, private val channels: Int = 1) : AutoCloseable {
    private val frameSize = sampleRate / 50
    private var handle: Long

    init {
        OpusLib.load()
        handle = nativeCreate(sampleRate, channels)
    }

    /** Decodes one packet into one 20 ms frame; null = the packet was lost (packet-loss concealment). */
    @Synchronized
    fun decode(packet: ByteArray?): ShortArray {
        check(handle != 0L) { "OpusDecoder is closed" }
        return nativeDecode(handle, packet, frameSize, channels)
    }

    @Synchronized
    override fun close() {
        if (handle != 0L) {
            nativeDestroy(handle)
            handle = 0L
        }
    }

    private external fun nativeCreate(rate: Int, channels: Int): Long
    private external fun nativeDecode(handle: Long, packet: ByteArray?, frameSize: Int, channels: Int): ShortArray
    private external fun nativeDestroy(handle: Long)
}
