package com.zalom3ui.app.ui

import android.content.Context
import android.media.MediaRecorder
import android.os.Build
import java.io.File
import java.util.Locale
import kotlin.math.sqrt
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

private const val MAX_AMPLITUDE = 32767f
private const val MIN_AMPLITUDE_BAR = 0.08f
private const val SAMPLE_INTERVAL_MS = 80L
private const val MAX_WAVEFORM_BARS = 40
private const val MIN_RECORDING_MS = 350L

/**
 * Maps [MediaRecorder.getMaxAmplitude] (`0..32767`) with a gentle square-root curve into `0.08f..1f`
 * so quiet speech still animates smoothly while loud peaks reach `1f`.
 */
internal fun normalizeAmplitude(raw: Int): Float {
    val linear = raw.coerceIn(0, 32767) / MAX_AMPLITUDE
    val curved = sqrt(linear.toDouble()).toFloat()
    return (MIN_AMPLITUDE_BAR + curved * (1f - MIN_AMPLITUDE_BAR)).coerceIn(MIN_AMPLITUDE_BAR, 1f)
}

/** Formats an active recording's elapsed milliseconds as `m:ss` (e.g. `0:00`, `0:14`, `1:05`). */
internal fun formatRecordingDuration(ms: Long): String {
    val totalSeconds = ms.coerceAtLeast(0L) / 1000L
    val minutes = totalSeconds / 60L
    val seconds = totalSeconds % 60L
    return String.format(Locale.US, "%d:%02d", minutes, seconds)
}

/**
 * Wraps Android's [MediaRecorder] for hold-to-record voice messages in AAC/M4A format,
 * exposing live recording state, elapsed duration, and a rolling waveform amplitude window.
 */
class VoiceRecorder(context: Context) {
    private val appContext: Context = context.applicationContext ?: context

    @Volatile
    private var recorder: MediaRecorder? = null
    private var outputFile: File? = null
    private var startedAtNanos: Long = 0L
    private var samplerJob: Job? = null

    private val _isRecording = MutableStateFlow(false)
    val isRecording: StateFlow<Boolean> = _isRecording.asStateFlow()

    private val _elapsedMs = MutableStateFlow(0L)
    val elapsedMs: StateFlow<Long> = _elapsedMs.asStateFlow()

    private val _amplitudes = MutableStateFlow<List<Float>>(emptyList())
    val amplitudes: StateFlow<List<Float>> = _amplitudes.asStateFlow()

    /**
     * Stops any previous session, prepares and starts recording into a temp `.m4a` file in cache,
     * and launches the 80ms ticker/amplitude sampler on [scope]. Returns `true` on success.
     */
    @Synchronized
    fun start(scope: CoroutineScope): Boolean {
        cancel()

        val voiceDir = File(appContext.cacheDir, "voice").apply { mkdirs() }
        val targetFile = File(voiceDir, "voice_${System.currentTimeMillis()}.m4a")
        val mr = createMediaRecorder(appContext)

        try {
            mr.setAudioSource(MediaRecorder.AudioSource.MIC)
            mr.setOutputFormat(MediaRecorder.OutputFormat.MPEG_4)
            mr.setAudioEncoder(MediaRecorder.AudioEncoder.AAC)
            mr.setAudioSamplingRate(44100)
            mr.setAudioEncodingBitRate(64_000)
            mr.setAudioChannels(1)
            mr.setOutputFile(targetFile.absolutePath)
            mr.prepare()
            mr.start()
        } catch (_: Exception) {
            runCatching { mr.reset() }
            runCatching { mr.release() }
            runCatching { targetFile.delete() }
            resetState()
            return false
        }

        val startNanos = System.nanoTime()
        recorder = mr
        outputFile = targetFile
        startedAtNanos = startNanos
        _isRecording.value = true
        _elapsedMs.value = 0L
        _amplitudes.value = emptyList()

        samplerJob = scope.launch {
            while (isActive && _isRecording.value) {
                delay(SAMPLE_INTERVAL_MS)
                val active = recorder ?: break
                val elapsed = ((System.nanoTime() - startNanos) / 1_000_000L).coerceAtLeast(0L)
                _elapsedMs.value = elapsed
                val rawAmp = runCatching { active.maxAmplitude }.getOrDefault(0)
                val bar = normalizeAmplitude(rawAmp)
                val updated = _amplitudes.value + bar
                _amplitudes.value = if (updated.size > MAX_WAVEFORM_BARS) {
                    updated.takeLast(MAX_WAVEFORM_BARS)
                } else {
                    updated
                }
            }
        }
        return true
    }

    /**
     * Stops the active recording safely. Deletes the temp file and returns `null` if the recording
     * lasted less than 350ms or produced an empty/missing file; otherwise returns the recorded [File].
     */
    @Synchronized
    fun stop(): File? {
        samplerJob?.cancel()
        samplerJob = null

        val elapsed = if (startedAtNanos > 0L) {
            ((System.nanoTime() - startedAtNanos) / 1_000_000L).coerceAtLeast(0L)
        } else {
            0L
        }
        val mr = recorder
        val file = outputFile
        recorder = null
        outputFile = null
        startedAtNanos = 0L
        resetState()

        var stoppedCleanly = false
        if (mr != null) {
            stoppedCleanly = runCatching {
                mr.stop()
                true
            }.getOrDefault(false)
            runCatching { mr.release() }
        }

        if (!stoppedCleanly || elapsed < MIN_RECORDING_MS || file == null || !file.exists() || file.length() == 0L) {
            file?.let { runCatching { it.delete() } }
            return null
        }
        return file
    }

    /** Stops any active recording safely and deletes the temporary audio file. */
    @Synchronized
    fun cancel() {
        samplerJob?.cancel()
        samplerJob = null

        val mr = recorder
        val file = outputFile
        recorder = null
        outputFile = null
        startedAtNanos = 0L
        resetState()

        if (mr != null) {
            runCatching { mr.stop() }
            runCatching { mr.release() }
        }
        file?.let { runCatching { it.delete() } }
    }

    private fun resetState() {
        _isRecording.value = false
        _elapsedMs.value = 0L
        _amplitudes.value = emptyList()
    }

    private fun createMediaRecorder(context: Context): MediaRecorder =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
            MediaRecorder(context)
        } else {
            @Suppress("DEPRECATION")
            MediaRecorder()
        }
}
