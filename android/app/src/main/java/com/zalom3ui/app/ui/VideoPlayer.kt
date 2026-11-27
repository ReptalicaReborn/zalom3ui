package com.zalom3ui.app.ui

import android.media.AudioAttributes
import android.media.MediaPlayer
import android.net.Uri
import android.os.Handler
import android.os.HandlerThread
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.Replay
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material3.Button
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LinearWavyProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.LocalLifecycleOwner
import coil.compose.AsyncImage
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.delay

/**
 * Owns one platform [MediaPlayer]. Every player call, callback and position poll runs on a private
 * [HandlerThread] (same reasoning as [VoicePlayback]: binder IPC must not block the UI thread).
 * Observable state is Compose snapshot state, which is safe to write from the worker thread.
 */
internal class VideoController(private val url: String) {
    private val thread = HandlerThread("video-playback").apply { start() }
    private val handler = Handler(thread.looper)

    @Volatile private var released = false

    // Worker-thread-only.
    private var player: MediaPlayer? = null
    private var prepared = false
    private var wantPlay = true
    private var surface: android.view.Surface? = null

    var isPlaying by mutableStateOf(false); private set
    var ready by mutableStateOf(false); private set
    var buffering by mutableStateOf(false); private set
    var firstFrame by mutableStateOf(false); private set
    var completed by mutableStateOf(false); private set
    var error by mutableStateOf(false); private set
    var positionMs by mutableIntStateOf(0); private set
    var durationMs by mutableIntStateOf(0); private set
    var videoWidth by mutableIntStateOf(0); private set
    var videoHeight by mutableIntStateOf(0); private set

    private val poll = object : Runnable {
        override fun run() {
            val p = player ?: return
            if (released || !prepared) return
            val playing = runCatching { p.isPlaying }.getOrDefault(false)
            if (playing) {
                runCatching { p.currentPosition }.getOrNull()?.let { positionMs = it.coerceAtLeast(0) }
            }
            isPlaying = playing
            if (playing) handler.postDelayed(this, POLL_MS)
        }
    }

    fun setSurface(s: android.view.Surface?) {
        handler.post {
            if (released) return@post
            surface = s
            player?.let { runCatching { it.setSurface(s) } }
        }
    }

    /**
     * Detaches the surface before the SurfaceView destroys it, waiting (briefly) for the worker: the player must
     * not draw into a surface after `surfaceDestroyed` returns.
     */
    fun detachSurfaceBlocking() {
        if (released) return
        val done = CountDownLatch(1)
        val posted = handler.post {
            surface = null
            player?.let { runCatching { it.setSurface(null) } }
            done.countDown()
        }
        if (posted) runCatching { done.await(DETACH_WAIT_MS, TimeUnit.MILLISECONDS) }
    }

    fun start() {
        handler.post {
            if (released) return@post
            val uri = runCatching { Uri.parse(url.trim()) }.getOrNull()
            val scheme = uri?.scheme?.lowercase()
            if (uri == null || (scheme != "http" && scheme != "https") || uri.host.isNullOrBlank()) {
                error = true
                return@post
            }
            val p = MediaPlayer()
            player = p
            try {
                p.setAudioAttributes(
                    AudioAttributes.Builder()
                        .setUsage(AudioAttributes.USAGE_MEDIA)
                        .setContentType(AudioAttributes.CONTENT_TYPE_MOVIE)
                        .build(),
                )
                p.setDataSource(uri.toString())
                surface?.let { p.setSurface(it) }
                p.setOnVideoSizeChangedListener { mp, w, h ->
                    if (player === mp && !released) {
                        videoWidth = w
                        videoHeight = h
                    }
                }
                p.setOnInfoListener { mp, what, _ ->
                    if (player === mp && !released) {
                        when (what) {
                            MediaPlayer.MEDIA_INFO_BUFFERING_START -> buffering = true
                            MediaPlayer.MEDIA_INFO_BUFFERING_END -> buffering = false
                            MediaPlayer.MEDIA_INFO_VIDEO_RENDERING_START -> firstFrame = true
                        }
                    }
                    true
                }
                p.setOnPreparedListener { mp ->
                    if (player === mp && !released) {
                        prepared = true
                        ready = true
                        durationMs = runCatching { mp.duration }.getOrDefault(0).coerceAtLeast(0)
                        if (mp.videoWidth > 0 && mp.videoHeight > 0) {
                            videoWidth = mp.videoWidth
                            videoHeight = mp.videoHeight
                        }
                        if (wantPlay) {
                            runCatching { mp.start() }
                            isPlaying = true
                            handler.removeCallbacks(poll)
                            handler.postDelayed(poll, POLL_MS)
                        }
                    }
                }
                p.setOnCompletionListener { mp ->
                    if (player === mp && !released) {
                        handler.removeCallbacks(poll)
                        completed = true
                        isPlaying = false
                        positionMs = durationMs
                    }
                }
                p.setOnErrorListener { mp, _, _ ->
                    if (player === mp && !released) {
                        handler.removeCallbacks(poll)
                        error = true
                        isPlaying = false
                        buffering = false
                    }
                    true
                }
                p.prepareAsync()
            } catch (_: Exception) {
                error = true
            }
        }
    }

    fun togglePlay() {
        handler.post {
            val p = player
            if (released || p == null || error) return@post
            if (!prepared) {
                wantPlay = !wantPlay
                return@post
            }
            if (runCatching { p.isPlaying }.getOrDefault(false)) {
                wantPlay = false
                runCatching { p.pause() }
                isPlaying = false
                handler.removeCallbacks(poll)
            } else {
                wantPlay = true
                if (completed) {
                    runCatching { p.seekTo(0L, MediaPlayer.SEEK_PREVIOUS_SYNC) }
                    positionMs = 0
                    completed = false
                }
                runCatching { p.start() }
                isPlaying = true
                handler.removeCallbacks(poll)
                handler.postDelayed(poll, POLL_MS)
            }
        }
    }

    fun pause() {
        handler.post {
            val p = player
            if (released) return@post
            wantPlay = false
            if (p != null && prepared) {
                runCatching { p.pause() }
                isPlaying = false
                handler.removeCallbacks(poll)
            }
        }
    }

    fun seekTo(ms: Int) {
        handler.post {
            val p = player
            if (released || p == null || !prepared) return@post
            val target = ms.coerceIn(0, durationMs.coerceAtLeast(0))
            runCatching { p.seekTo(target.toLong(), MediaPlayer.SEEK_PREVIOUS_SYNC) }
            positionMs = target
            if (completed && target < durationMs) completed = false
        }
    }

    fun release() {
        released = true
        handler.post {
            handler.removeCallbacks(poll)
            player?.let { runCatching { it.release() } }
            player = null
            prepared = false
            surface = null
            thread.quitSafely()
        }
    }

    private companion object {
        const val POLL_MS = 250L
        const val DETACH_WAIT_MS = 500L
    }
}

private const val CONTROLS_HIDE_MS = 3000L

/** Full-screen in-app player for a remote video, using the platform [MediaPlayer]. */
@Composable
fun VideoPlayerDialog(url: String, thumb: String?, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val controller = remember(url) { VideoController(url) }
    DisposableEffect(controller) {
        controller.start()
        onDispose { controller.release() }
    }
    val lifecycleOwner = LocalLifecycleOwner.current
    DisposableEffect(lifecycleOwner, controller) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_STOP) controller.pause()
        }
        lifecycleOwner.lifecycle.addObserver(observer)
        onDispose { lifecycleOwner.lifecycle.removeObserver(observer) }
    }

    var controlsVisible by remember { mutableStateOf(true) }
    var interaction by remember { mutableIntStateOf(0) }
    var dragging by remember { mutableStateOf(false) }
    var dragValue by remember { mutableStateOf(0f) }

    val isPlaying = controller.isPlaying
    LaunchedEffect(controlsVisible, isPlaying, dragging, interaction) {
        if (controlsVisible && isPlaying && !dragging) {
            delay(CONTROLS_HIDE_MS)
            controlsVisible = false
        }
    }

    val hasSize = controller.videoWidth > 0 && controller.videoHeight > 0
    val showSpinner = !controller.error && (!controller.ready || controller.buffering)
    val showControls = controlsVisible || controller.error

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false),
    ) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .background(Color.Black)
                .pointerInput(Unit) {
                    detectTapGestures {
                        controlsVisible = !controlsVisible
                        interaction++
                    }
                },
        ) {
            AndroidView(
                factory = { ctx ->
                    SurfaceView(ctx).apply {
                        holder.addCallback(object : SurfaceHolder.Callback {
                            override fun surfaceCreated(holder: SurfaceHolder) {
                                controller.setSurface(holder.surface)
                            }

                            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) = Unit

                            override fun surfaceDestroyed(holder: SurfaceHolder) {
                                controller.detachSurfaceBlocking()
                            }
                        })
                    }
                },
                update = { it.keepScreenOn = isPlaying },
                modifier = Modifier
                    .align(Alignment.Center)
                    .then(
                        if (hasSize) {
                            Modifier.aspectRatio(controller.videoWidth.toFloat() / controller.videoHeight)
                        } else {
                            Modifier.fillMaxSize()
                        },
                    ),
            )

            if (!controller.firstFrame && !controller.error && thumb != null) {
                AsyncImage(
                    model = thumb,
                    contentDescription = null,
                    contentScale = ContentScale.Fit,
                    modifier = Modifier.fillMaxSize(),
                )
            }

            if (showSpinner) {
                CircularProgressIndicator(color = Color.White, modifier = Modifier.align(Alignment.Center).size(48.dp))
            }

            if (controller.error) {
                Column(
                    modifier = Modifier.align(Alignment.Center).padding(32.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    Text(
                        "Can't play this video",
                        color = Color.White,
                        style = MaterialTheme.typography.titleMedium,
                        textAlign = TextAlign.Center,
                    )
                    Button(onClick = { openExternally(context, url, "video/*") }) {
                        Text("Open in another app")
                    }
                }
            }

            AnimatedVisibility(
                visible = showControls,
                enter = fadeIn(),
                exit = fadeOut(),
                modifier = Modifier.fillMaxSize(),
            ) {
                Box(Modifier.fillMaxSize()) {
                    Row(
                        modifier = Modifier
                            .align(Alignment.TopStart)
                            .fillMaxWidth()
                            .statusBarsPadding()
                            .padding(horizontal = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        IconButton(onClick = onDismiss) {
                            Icon(Icons.Outlined.Close, contentDescription = "Close", tint = Color.White)
                        }
                        Box(Modifier.weight(1f))
                        IconButton(onClick = { openExternally(context, url, "video/*") }) {
                            Icon(
                                Icons.AutoMirrored.Outlined.OpenInNew,
                                contentDescription = "Open in another app",
                                tint = Color.White,
                            )
                        }
                    }

                    if (!controller.error && controller.ready && !controller.buffering) {
                        Surface(
                            shape = CircleShape,
                            color = Color.Black.copy(alpha = 0.55f),
                            contentColor = Color.White,
                            modifier = Modifier.align(Alignment.Center),
                        ) {
                            IconButton(
                                onClick = {
                                    controller.togglePlay()
                                    interaction++
                                },
                                modifier = Modifier.size(64.dp),
                            ) {
                                Icon(
                                    imageVector = when {
                                        controller.completed -> Icons.Filled.Replay
                                        isPlaying -> Icons.Filled.Pause
                                        else -> Icons.Filled.PlayArrow
                                    },
                                    contentDescription = when {
                                        controller.completed -> "Replay"
                                        isPlaying -> "Pause"
                                        else -> "Play"
                                    },
                                    modifier = Modifier.size(36.dp),
                                )
                            }
                        }
                    }

                    if (!controller.error) {
                        val total = controller.durationMs.coerceAtLeast(0)
                        val shownPos = if (dragging) dragValue else controller.positionMs.toFloat()
                        Row(
                            modifier = Modifier
                                .align(Alignment.BottomStart)
                                .fillMaxWidth()
                                .navigationBarsPadding()
                                .padding(horizontal = 16.dp, vertical = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(formatDuration(shownPos.toLong()), color = Color.White)
                            WavySeekBar(
                                value = shownPos.coerceIn(0f, total.coerceAtLeast(1).toFloat()),
                                valueRange = 0f..total.coerceAtLeast(1).toFloat(),
                                onValueChange = {
                                    dragging = true
                                    dragValue = it
                                },
                                onValueChangeFinished = {
                                    controller.seekTo(dragValue.toInt())
                                    dragging = false
                                },
                                enabled = controller.ready,
                                waving = isPlaying && !controller.buffering && !controller.completed,
                                modifier = Modifier.weight(1f).padding(horizontal = 8.dp),
                            )
                            Text(formatDuration(total.toLong()), color = Color.White)
                        }
                    }
                }
            }
        }
    }
}

/**
 * Material 3 Expressive seek bar: a standard [Slider] (drag, tap-to-seek, accessibility, 44dp touch target, vertical pill
 * thumb with the M3 gap) whose active track is a wavy progress line. The wave travels while [waving] and flattens
 * (amplitude animates to 0, travel stops) when paused, buffering or finished; the inactive track is flat.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun WavySeekBar(
    value: Float,
    valueRange: ClosedFloatingPointRange<Float>,
    onValueChange: (Float) -> Unit,
    onValueChangeFinished: () -> Unit,
    enabled: Boolean,
    waving: Boolean,
    modifier: Modifier = Modifier,
) {
    val interactionSource = remember { MutableInteractionSource() }
    val colors = SliderDefaults.colors(
        thumbColor = Color.White,
        disabledThumbColor = Color.White.copy(alpha = 0.38f),
    )
    Slider(
        value = value,
        onValueChange = onValueChange,
        modifier = modifier,
        enabled = enabled,
        onValueChangeFinished = onValueChangeFinished,
        colors = colors,
        interactionSource = interactionSource,
        valueRange = valueRange,
        thumb = {
            SliderDefaults.Thumb(interactionSource = interactionSource, colors = colors, enabled = enabled)
        },
        track = { sliderState ->
            val span = sliderState.valueRange.endInclusive - sliderState.valueRange.start
            LinearWavyProgressIndicator(
                progress = {
                    if (span <= 0f) 0f
                    else ((sliderState.value - sliderState.valueRange.start) / span).coerceIn(0f, 1f)
                },
                modifier = Modifier.fillMaxWidth(),
                color = if (enabled) Color.White else Color.White.copy(alpha = 0.38f),
                trackColor = Color.White.copy(alpha = 0.3f),
                // Room for the 4dp thumb plus the M3 6dp gap on each side.
                gapSize = 16.dp,
                stopSize = 0.dp,
                amplitude = { if (waving) 1f else 0f },
                wavelength = 32.dp,
                waveSpeed = if (waving) 24.dp else 0.dp,
            )
        },
    )
}
