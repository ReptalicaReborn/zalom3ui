package com.zalom3ui.app.call

/**
 * One Opus (or test) codec for 20 ms mono frames. The engine only depends on this interface so it can be built and
 * tested without the native library.
 */
interface FrameCodec : AutoCloseable {
    /** Encodes one frame of [pcm] (320 samples at 16 kHz) into a packet. */
    fun encode(pcm: ShortArray): ByteArray

    /** Decodes one packet into a 320-sample frame; null means the packet was lost and asks for concealment. */
    fun decode(packet: ByteArray?): ShortArray
}
