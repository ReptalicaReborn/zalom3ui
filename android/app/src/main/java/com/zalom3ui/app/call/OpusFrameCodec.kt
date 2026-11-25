package com.zalom3ui.app.call

/**
 * The call's [FrameCodec]: Opus 16 kHz mono, 20 ms frames, ~20 kbit/s (docs/call-audio.md). The encoder and decoder are
 * separate native objects with their own locks, so the capture thread (encode) and the playback thread (decode) never
 * wait on each other.
 */
class OpusFrameCodec(sampleRate: Int = 16000, bitrate: Int = 20_000) : FrameCodec {
    private val encoder = OpusEncoder(sampleRate, 1, bitrate)
    private val decoder = OpusDecoder(sampleRate, 1)

    override fun encode(pcm: ShortArray): ByteArray = encoder.encode(pcm)

    override fun decode(packet: ByteArray?): ShortArray = decoder.decode(packet)

    override fun close() {
        encoder.close()
        decoder.close()
    }
}
