package com.zalom3ui.app.ui.call

import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.VolumeOff
import androidx.compose.material.icons.automirrored.filled.VolumeUp
import androidx.compose.material.icons.filled.CallEnd
import androidx.compose.material.icons.filled.Cameraswitch
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MicOff
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material.icons.filled.VideocamOff
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.viewinterop.AndroidView
import com.zalom3ui.app.ui.Avatar
import kotlin.math.roundToInt

private val SELF_WIDTH = 96.dp
private val SELF_HEIGHT = 128.dp
private val SELF_MARGIN = 12.dp

/**
 * A connected video call: the other side full screen (their avatar while their camera is off), our own picture in a small
 * window that can be dragged, and the call buttons along the bottom.
 */
@Composable
internal fun VideoCallLayout(
    state: CallScreenState,
    onHangUp: () -> Unit,
    onToggleMute: () -> Unit,
    onToggleSpeaker: () -> Unit,
    onToggleCamera: () -> Unit,
    onSwitchCamera: () -> Unit,
    onRemoteSurface: (Surface?) -> Unit,
    onPreviewSurface: (Surface?) -> Unit,
) {
    val showRemote = state.peerCameraOn
    val scheme = MaterialTheme.colorScheme
    val onVideo = if (showRemote) Color.White else scheme.onSurface
    Box(Modifier.fillMaxSize().background(if (showRemote) Color.Black else scheme.surface)) {
        if (showRemote) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                VideoSurface(Modifier.aspectRatio(state.remoteAspect ?: DEFAULT_ASPECT), overlay = false, onSurface = onRemoteSurface)
            }
        } else {
            Column(Modifier.fillMaxSize(), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.Center) {
                Avatar(name = state.name, url = state.avatar.orEmpty(), key = state.callerId.ifEmpty { state.name }, isGroup = false, size = 128.dp)
                Spacer(Modifier.height(12.dp))
                Text("Camera off", style = MaterialTheme.typography.bodyMedium, color = scheme.onSurfaceVariant)
            }
        }

        Column(
            Modifier.align(Alignment.TopCenter).fillMaxWidth().safeDrawingPadding().padding(top = 16.dp, start = 24.dp, end = 24.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(
                state.name.ifBlank { "Unknown" },
                style = MaterialTheme.typography.titleLarge,
                color = onVideo,
                textAlign = TextAlign.Center,
            )
            Text(
                callStatusText(state),
                style = MaterialTheme.typography.bodyMedium,
                color = if (showRemote) Color.White.copy(alpha = 0.85f) else scheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
        }

        if (state.cameraOn) SelfView(onPreviewSurface)

        Column(
            Modifier.align(Alignment.BottomCenter).fillMaxWidth().safeDrawingPadding().padding(bottom = 24.dp, start = 12.dp, end = 12.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            state.notice?.let {
                Text(it, style = MaterialTheme.typography.bodyMedium, color = if (showRemote) Color.White else scheme.error, textAlign = TextAlign.Center)
            }
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly, verticalAlignment = Alignment.CenterVertically) {
                RoundButton(if (state.muted) Icons.Filled.MicOff else Icons.Filled.Mic, if (state.muted) "Unmute" else "Mute", state.muted, showRemote, onToggleMute)
                RoundButton(
                    if (state.cameraOn) Icons.Filled.Videocam else Icons.Filled.VideocamOff,
                    if (state.cameraOn) "Camera off" else "Camera on",
                    !state.cameraOn,
                    showRemote,
                    onToggleCamera,
                )
                if (state.cameraOn) RoundButton(Icons.Filled.Cameraswitch, "Switch camera", false, showRemote, onSwitchCamera)
                RoundButton(
                    if (state.speakerOn) Icons.AutoMirrored.Filled.VolumeUp else Icons.AutoMirrored.Filled.VolumeOff,
                    "Speaker",
                    state.speakerOn,
                    showRemote,
                    onToggleSpeaker,
                )
                IconButton(
                    onClick = onHangUp,
                    modifier = Modifier.size(56.dp),
                    colors = IconButtonDefaults.iconButtonColors(containerColor = scheme.error, contentColor = scheme.onError),
                ) {
                    Icon(Icons.Filled.CallEnd, contentDescription = "Hang up", modifier = Modifier.size(28.dp))
                }
            }
        }
    }
}

/** Our own picture, top right to begin with; drag it out of the way. Stays inside the screen. */
@Composable
private fun SelfView(onSurface: (Surface?) -> Unit) {
    val density = LocalDensity.current
    BoxWithConstraints(Modifier.fillMaxSize().safeDrawingPadding().padding(SELF_MARGIN)) {
        val maxX = with(density) { (maxWidth - SELF_WIDTH).toPx() }.coerceAtLeast(0f)
        val maxY = with(density) { (maxHeight - SELF_HEIGHT).toPx() }.coerceAtLeast(0f)
        // Starts below the name, at the right edge.
        var pos by remember { mutableStateOf(Offset(Float.MAX_VALUE, with(density) { 56.dp.toPx() })) }
        val x = pos.x.coerceIn(0f, maxX)
        val y = pos.y.coerceIn(0f, maxY)
        VideoSurface(
            Modifier
                .offset { IntOffset(x.roundToInt(), y.roundToInt()) }
                .size(SELF_WIDTH, SELF_HEIGHT)
                .pointerInput(maxX, maxY) {
                    detectDragGestures { change, drag ->
                        change.consume()
                        pos = Offset((pos.x.coerceIn(0f, maxX) + drag.x).coerceIn(0f, maxX), (pos.y.coerceIn(0f, maxY) + drag.y).coerceIn(0f, maxY))
                    }
                },
            overlay = true,
            onSurface = onSurface,
        )
    }
}

/** A SurfaceView whose surface is handed to [onSurface] while it exists (null when it is destroyed). */
@Composable
private fun VideoSurface(modifier: Modifier, overlay: Boolean, onSurface: (Surface?) -> Unit) {
    val current by rememberUpdatedState(onSurface)
    var surfaceView by remember { mutableStateOf<SurfaceView?>(null) }
    val callback = remember {
        object : SurfaceHolder.Callback {
            override fun surfaceCreated(holder: SurfaceHolder) = current(holder.surface)

            override fun surfaceChanged(holder: SurfaceHolder, format: Int, width: Int, height: Int) = Unit

            override fun surfaceDestroyed(holder: SurfaceHolder) = current(null)
        }
    }
    DisposableEffect(Unit) {
        onDispose {
            surfaceView?.holder?.removeCallback(callback)
            current(null)
        }
    }
    AndroidView(
        modifier = modifier,
        factory = { context ->
            SurfaceView(context).apply {
                // Above the other side's picture, which is a SurfaceView too.
                if (overlay) setZOrderMediaOverlay(true)
                holder.addCallback(callback)
                surfaceView = this
            }
        },
    )
}

@Composable
private fun RoundButton(icon: ImageVector, label: String, highlighted: Boolean, onVideo: Boolean, onClick: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    val container = when {
        onVideo -> if (highlighted) Color.White else Color.White.copy(alpha = 0.22f)
        else -> if (highlighted) scheme.primary else scheme.surfaceVariant
    }
    val content = when {
        onVideo -> if (highlighted) Color.Black else Color.White
        else -> if (highlighted) scheme.onPrimary else scheme.onSurfaceVariant
    }
    IconButton(onClick = onClick, modifier = Modifier.size(56.dp).background(container, CircleShape), colors = IconButtonDefaults.iconButtonColors(contentColor = content)) {
        Icon(icon, contentDescription = label, modifier = Modifier.size(26.dp))
    }
}

private const val DEFAULT_ASPECT = 3f / 4f
