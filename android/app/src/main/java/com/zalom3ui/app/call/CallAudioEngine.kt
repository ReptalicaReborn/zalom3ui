package com.zalom3ui.app.call

import android.annotation.SuppressLint
import android.content.Context
import android.media.AudioAttributes
import android.media.AudioDeviceInfo
import android.media.AudioFocusRequest
import android.media.AudioFormat
import android.media.AudioManager
import android.media.AudioRecord
import android.media.AudioTrack
import android.media.MediaRecorder
import android.media.audiofx.AcousticEchoCanceler
import android.media.audiofx.AudioEffect
import android.media.audiofx.AutomaticGainControl
import android.media.audiofx.NoiseSuppressor
import android.os.Build
import android.os.Process
import java.util.concurrent.atomic.AtomicBoolean
import java.util.concurrent.atomic.AtomicLong

/**
 * Microphone to [send] and [onPacket] to the speaker, 16 kHz mono, 20 ms frames.
 *
 * The capture thread reads one frame, encodes it and calls [send]; the playback thread pops the jitter buffer,
 * decodes (null means concealment) and does a blocking write, which is what paces it at 20 ms.
 *
 * Requires RECORD_AUDIO to be granted already: the caller (the call service) checks and asks for it. [start] throws
 * [SecurityException] otherwise. [codec] is closed by [stop].
 */
@SuppressLint("MissingPermission")
class CallAudioEngine(
    context: Context,
    private val codec: FrameCodec,
    private val send: (ByteArray) -> Unit,
) {
    /** Counters for debugging a call: frames sent to the server, frames played, frames the decoder had to conceal. */
    data class Stats(val framesSent: Long, val framesPlayed: Long, val framesConcealed: Long)

    @Volatile var muted: Boolean = false

    @Volatile var speakerOn: Boolean = false
        set(value) {
            field = value
            if (running.get()) applySpeaker()
        }

    private val audioManager = context.applicationContext.getSystemService(Context.AUDIO_SERVICE) as AudioManager
    private val jitter = JitterBuffer()
    private val running = AtomicBoolean(false)
    private val stopped = AtomicBoolean(false)
    private val sent = AtomicLong()
    private val played = AtomicLong()
    private val concealed = AtomicLong()

    private var previousMode = AudioManager.MODE_NORMAL
    private var focusRequest: AudioFocusRequest? = null
    private var record: AudioRecord? = null
    private var track: AudioTrack? = null
    private var effects: List<AudioEffect> = emptyList()
    private var threads: List<Thread> = emptyList()

    fun stats() = Stats(sent.get(), played.get(), concealed.get())

    @Synchronized
    fun start() {
        if (stopped.get() || running.get()) return
        previousMode = audioManager.mode
        audioManager.mode = AudioManager.MODE_IN_COMMUNICATION
        focusRequest = AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN_TRANSIENT)
            .setAudioAttributes(voiceAttributes())
            .setOnAudioFocusChangeListener { }
            .build()
            .also { audioManager.requestAudioFocus(it) }
        try {
            val rec = buildRecord()
            record = rec
            effects = attachEffects(rec.audioSessionId)
            track = buildTrack()
        } catch (t: Throwable) {
            running.set(true) // let stop() do the full cleanup
            stop()
            throw t
        }
        running.set(true)
        applySpeaker()
        record!!.startRecording()
        track!!.play()
        threads = listOf(
            Thread(::captureLoop, "call-capture"),
            Thread(::playbackLoop, "call-playback"),
        ).onEach { it.start() }
    }

    /** Safe to call twice and from any thread. */
    fun stop() {
        if (!stopped.compareAndSet(false, true)) return
        running.set(false)
        val self = Thread.currentThread()
        // Let the loops finish their current frame before the AudioRecord/AudioTrack under them are released.
        synchronized(this) {
            for (t in threads) if (t !== self) runCatching { t.join(500) }
            threads = emptyList()
            runCatching { record?.stop() }
            runCatching { record?.release() }
            record = null
            for (e in effects) runCatching { e.release() }
            effects = emptyList()
            runCatching { track?.stop() }
            runCatching { track?.release() }
            track = null
            runCatching { codec.close() }
            clearSpeaker()
            focusRequest?.let { runCatching { audioManager.abandonAudioFocusRequest(it) } }
            focusRequest = null
            audioManager.mode = previousMode
        }
    }

    /** From the network thread. [timestamp] is unused: playout is paced by the local clock. */
    @Suppress("UNUSED_PARAMETER")
    fun onPacket(seq: Int, timestamp: Long, opus: ByteArray) {
        if (running.get()) jitter.push(seq, opus)
    }

    private fun captureLoop() {
        Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
        val rec = record ?: return
        val silence = ShortArray(FRAME)
        val pcm = ShortArray(FRAME)
        while (running.get()) {
            var got = 0
            while (got < FRAME && running.get()) {
                val n = rec.read(pcm, got, FRAME - got)
                if (n <= 0) return // error or released
                got += n
            }
            if (got < FRAME) return
            try {
                send(codec.encode(if (muted) silence else pcm))
                sent.incrementAndGet()
            } catch (_: Exception) {
                // One bad frame must not end the call.
            }
        }
    }

    private fun playbackLoop() {
        Process.setThreadPriority(Process.THREAD_PRIORITY_URGENT_AUDIO)
        val trk = track ?: return
        val silence = ShortArray(FRAME)
        while (running.get()) {
            val frame = try {
                if (jitter.isPlaying) {
                    val packet = jitter.pop()
                    if (packet == null) concealed.incrementAndGet()
                    played.incrementAndGet()
                    codec.decode(packet)
                } else {
                    silence
                }
            } catch (_: Exception) {
                silence
            }
            if (trk.write(frame, 0, frame.size) < 0) return
        }
    }

    private fun buildRecord(): AudioRecord {
        val min = AudioRecord.getMinBufferSize(RATE, AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT)
        val bytes = maxOf(min, FRAME * 2 * 8)
        val rec = AudioRecord(
            MediaRecorder.AudioSource.VOICE_COMMUNICATION, RATE,
            AudioFormat.CHANNEL_IN_MONO, AudioFormat.ENCODING_PCM_16BIT, bytes,
        )
        if (rec.state != AudioRecord.STATE_INITIALIZED) {
            rec.release()
            throw IllegalStateException("AudioRecord failed to initialise")
        }
        return rec
    }

    private fun buildTrack(): AudioTrack {
        val min = AudioTrack.getMinBufferSize(RATE, AudioFormat.CHANNEL_OUT_MONO, AudioFormat.ENCODING_PCM_16BIT)
        return AudioTrack.Builder()
            .setAudioAttributes(voiceAttributes())
            .setAudioFormat(
                AudioFormat.Builder()
                    .setSampleRate(RATE)
                    .setChannelMask(AudioFormat.CHANNEL_OUT_MONO)
                    .setEncoding(AudioFormat.ENCODING_PCM_16BIT)
                    .build(),
            )
            .setTransferMode(AudioTrack.MODE_STREAM)
            .setBufferSizeInBytes(maxOf(min, FRAME * 2 * 4))
            .build()
    }

    private fun attachEffects(session: Int): List<AudioEffect> {
        val list = ArrayList<AudioEffect>()
        if (AcousticEchoCanceler.isAvailable()) AcousticEchoCanceler.create(session)?.let { list += it }
        if (NoiseSuppressor.isAvailable()) NoiseSuppressor.create(session)?.let { list += it }
        if (AutomaticGainControl.isAvailable()) AutomaticGainControl.create(session)?.let { list += it }
        for (e in list) runCatching { e.enabled = true }
        return list
    }

    private fun voiceAttributes() = AudioAttributes.Builder()
        .setUsage(AudioAttributes.USAGE_VOICE_COMMUNICATION)
        .setContentType(AudioAttributes.CONTENT_TYPE_SPEECH)
        .build()

    private fun applySpeaker() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            if (speakerOn) {
                val speaker = audioManager.availableCommunicationDevices
                    .firstOrNull { it.type == AudioDeviceInfo.TYPE_BUILTIN_SPEAKER }
                if (speaker != null) audioManager.setCommunicationDevice(speaker)
            } else {
                audioManager.clearCommunicationDevice()
            }
        } else {
            @Suppress("DEPRECATION")
            audioManager.isSpeakerphoneOn = speakerOn
        }
    }

    private fun clearSpeaker() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            audioManager.clearCommunicationDevice()
        } else {
            @Suppress("DEPRECATION")
            audioManager.isSpeakerphoneOn = false
        }
    }

    private companion object {
        const val RATE = 16000
        const val FRAME = 320
    }
}
