@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package com.zalom3ui.app.ui.call

import android.graphics.Outline
import android.view.Surface
import android.view.SurfaceHolder
import android.view.SurfaceView
import android.view.View
import android.view.ViewOutlineProvider
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.VectorConverter
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.slideOutVertically
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.gestures.detectDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
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
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.VolumeOff
import androidx.compose.material.icons.automirrored.filled.VolumeUp
import androidx.compose.material.icons.filled.CallEnd
import androidx.compose.material.icons.filled.Cameraswitch
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MicOff
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material.icons.filled.VideocamOff
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FloatingToolbarDefaults
import androidx.compose.material3.HorizontalFloatingToolbar
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface as M3Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
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
import kotlinx.coroutines.launch

private val SELF_WIDTH = 96.dp
private val SELF_HEIGHT = 128.dp
private val SELF_MARGIN = 12.dp
private val SELF_CORNER_RADIUS = 16.dp
private val SELF_TOP_INSET = 72.dp
private val SELF_BOTTOM_INSET = 104.dp

/**
 * A connected video call: the other side full screen (their avatar while their camera is off), our own picture in a small
 * rounded corner-snapping window that can be dragged, and an expressive floating toolbar along the bottom.
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
    var chromeVisible by remember { mutableStateOf(true) }
    val showChrome = !showRemote || chromeVisible

    Box(
        Modifier
            .fillMaxSize()
            .background(if (showRemote) Color.Black else scheme.surface)
            .pointerInput(showRemote) {
                detectTapGestures {
                    if (showRemote) chromeVisible = !chromeVisible
                }
            },
    ) {
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

        AnimatedVisibility(
            visible = showChrome,
            modifier = Modifier.align(Alignment.TopCenter).fillMaxWidth(),
            enter = fadeIn(MaterialTheme.motionScheme.fastEffectsSpec()) +
                slideInVertically(MaterialTheme.motionScheme.fastEffectsSpec()) { -it / 2 },
            exit = fadeOut(MaterialTheme.motionScheme.fastEffectsSpec()) +
                slideOutVertically(MaterialTheme.motionScheme.fastEffectsSpec()) { -it / 2 },
        ) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .then(
                        if (showRemote) {
                            Modifier.background(
                                Brush.verticalGradient(listOf(Color.Black.copy(alpha = 0.58f), Color.Transparent)),
                            )
                        } else {
                            Modifier
                        },
                    ),
            ) {
                Column(
                    Modifier
                        .fillMaxWidth()
                        .safeDrawingPadding()
                        .padding(top = 16.dp, bottom = 28.dp, start = 24.dp, end = 24.dp),
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
                        style = MaterialTheme.typography.bodyMedium.copy(fontFeatureSettings = "tnum"),
                        color = if (showRemote) Color.White.copy(alpha = 0.85f) else scheme.onSurfaceVariant,
                        textAlign = TextAlign.Center,
                    )
                }
            }
        }

        if (state.cameraOn) SelfView(chromeVisible = showChrome, onSurface = onPreviewSurface)

        AnimatedVisibility(
            visible = showChrome,
            modifier = Modifier.align(Alignment.BottomCenter).fillMaxWidth(),
            enter = fadeIn(MaterialTheme.motionScheme.fastEffectsSpec()) +
                slideInVertically(MaterialTheme.motionScheme.fastEffectsSpec()) { it / 2 },
            exit = fadeOut(MaterialTheme.motionScheme.fastEffectsSpec()) +
                slideOutVertically(MaterialTheme.motionScheme.fastEffectsSpec()) { it / 2 },
        ) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .then(
                        if (showRemote) {
                            Modifier.background(
                                Brush.verticalGradient(listOf(Color.Transparent, Color.Black.copy(alpha = 0.65f))),
                            )
                        } else {
                            Modifier
                        },
                    ),
                contentAlignment = Alignment.BottomCenter,
            ) {
                Column(
                    Modifier
                        .fillMaxWidth()
                        .safeDrawingPadding()
                        .padding(top = 28.dp, bottom = 20.dp, start = 12.dp, end = 12.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                    verticalArrangement = Arrangement.spacedBy(12.dp),
                ) {
                    state.notice?.let {
                        Text(
                            it,
                            style = MaterialTheme.typography.bodyMedium,
                            color = if (showRemote) Color.White else scheme.error,
                            textAlign = TextAlign.Center,
                        )
                    }
                    HorizontalFloatingToolbar(
                        expanded = true,
                        modifier = if (showRemote) {
                            Modifier.border(1.dp, Color.White.copy(alpha = 0.14f), CircleShape)
                        } else {
                            Modifier
                        },
                        colors = FloatingToolbarDefaults.standardFloatingToolbarColors(
                            toolbarContainerColor = if (showRemote) Color(0xFF14161A).copy(alpha = 0.78f) else scheme.surfaceContainerHigh,
                            toolbarContentColor = if (showRemote) Color.White else scheme.onSurface,
                        ),
                        contentPadding = PaddingValues(horizontal = 10.dp, vertical = 8.dp),
                        shape = CircleShape,
                        expandedShadowElevation = if (showRemote) 0.dp else FloatingToolbarDefaults.ContainerExpandedElevation,
                    ) {
                        Row(
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RoundButton(
                                if (state.muted) Icons.Filled.MicOff else Icons.Filled.Mic,
                                if (state.muted) "Unmute" else "Mute",
                                state.muted,
                                showRemote,
                                onToggleMute,
                            )
                            RoundButton(
                                if (state.cameraOn) Icons.Filled.Videocam else Icons.Filled.VideocamOff,
                                if (state.cameraOn) "Camera off" else "Camera on",
                                !state.cameraOn,
                                showRemote,
                                onToggleCamera,
                            )
                            if (state.cameraOn) {
                                RoundButton(Icons.Filled.Cameraswitch, "Switch camera", false, showRemote, onSwitchCamera)
                            }
                            RoundButton(
                                if (state.speakerOn) Icons.AutoMirrored.Filled.VolumeUp else Icons.AutoMirrored.Filled.VolumeOff,
                                "Speaker",
                                state.speakerOn,
                                showRemote,
                                onToggleSpeaker,
                            )
                            M3Surface(
                                onClick = onHangUp,
                                modifier = Modifier.size(width = 64.dp, height = 48.dp),
                                shape = CircleShape,
                                color = scheme.error,
                                contentColor = scheme.onError,
                            ) {
                                Box(contentAlignment = Alignment.Center) {
                                    Icon(Icons.Filled.CallEnd, contentDescription = "Hang up", modifier = Modifier.size(24.dp))
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/**
 * Snaps a PiP window position to the nearest of the 4 safe corners inside `[0, maxX] × [topInset, maxY - bottomInset]`.
 * Handles overlapping insets or degenerate zero-size bounds safely without throwing or inverting coordinates.
 */
internal fun snapPipCorner(
    x: Float,
    y: Float,
    maxX: Float,
    maxY: Float,
    topInset: Float,
    bottomInset: Float,
): Pair<Float, Float> {
    val safeMaxX = if (maxX.isFinite()) maxX.coerceAtLeast(0f) else 0f
    val safeMaxY = if (maxY.isFinite()) maxY.coerceAtLeast(0f) else 0f
    val safeTop = if (topInset.isFinite()) topInset.coerceIn(0f, safeMaxY) else 0f
    val safeBottomInset = if (bottomInset.isFinite()) bottomInset.coerceAtLeast(0f) else 0f
    val safeBottom = (safeMaxY - safeBottomInset).coerceIn(0f, safeMaxY)
    val topY: Float
    val bottomY: Float
    if (safeTop <= safeBottom) {
        topY = safeTop
        bottomY = safeBottom
    } else {
        val midY = ((safeTop + safeBottom) / 2f).coerceIn(0f, safeMaxY)
        topY = midY
        bottomY = midY
    }
    val clampedX = if (x.isFinite()) x.coerceIn(0f, safeMaxX) else safeMaxX
    val clampedY = if (y.isFinite()) y.coerceIn(0f, safeMaxY) else topY
    val snapX = if (clampedX <= safeMaxX / 2f) 0f else safeMaxX
    val snapY = if (clampedY <= (topY + bottomY) / 2f) topY else bottomY
    return snapX to snapY
}

/** Our own picture, top right to begin with; drag it and it springs cleanly to the nearest safe corner. */
@Composable
private fun SelfView(chromeVisible: Boolean, onSurface: (Surface?) -> Unit) {
    val density = LocalDensity.current
    val scope = rememberCoroutineScope()
    val spatialSpec = MaterialTheme.motionScheme.defaultSpatialSpec<Offset>()
    val pipShape = remember { RoundedCornerShape(SELF_CORNER_RADIUS) }
    BoxWithConstraints(Modifier.fillMaxSize().safeDrawingPadding().padding(SELF_MARGIN)) {
        val maxX = with(density) { (maxWidth - SELF_WIDTH).toPx() }.coerceAtLeast(0f)
        val maxY = with(density) { (maxHeight - SELF_HEIGHT).toPx() }.coerceAtLeast(0f)
        val topInset = if (chromeVisible) with(density) { SELF_TOP_INSET.toPx() } else 0f
        val bottomInset = if (chromeVisible) with(density) { SELF_BOTTOM_INSET.toPx() } else 0f
        // Starts below the header, at the right edge.
        val initial = snapPipCorner(maxX, topInset, maxX, maxY, topInset, bottomInset)
        val pos = remember { Animatable(Offset(initial.first, initial.second), Offset.VectorConverter) }
        var hasLaidOut by remember { mutableStateOf(false) }

        LaunchedEffect(maxX, maxY, topInset, bottomInset) {
            if (maxX <= 0f && maxY <= 0f) return@LaunchedEffect
            if (!hasLaidOut) {
                hasLaidOut = true
                pos.snapTo(Offset(initial.first, initial.second))
            } else {
                val (sx, sy) = snapPipCorner(pos.value.x, pos.value.y, maxX, maxY, topInset, bottomInset)
                pos.animateTo(Offset(sx, sy), animationSpec = spatialSpec)
            }
        }

        Box(
            Modifier
                .offset { IntOffset(pos.value.x.roundToInt(), pos.value.y.roundToInt()) }
                .size(SELF_WIDTH, SELF_HEIGHT)
                .clip(pipShape)
                .pointerInput(maxX, maxY, topInset, bottomInset) {
                    var dragPos = pos.value
                    detectDragGestures(
                        onDragStart = { dragPos = pos.value },
                        onDragEnd = {
                            val (sx, sy) = snapPipCorner(dragPos.x, dragPos.y, maxX, maxY, topInset, bottomInset)
                            scope.launch { pos.animateTo(Offset(sx, sy), animationSpec = spatialSpec) }
                        },
                        onDragCancel = {
                            val (sx, sy) = snapPipCorner(dragPos.x, dragPos.y, maxX, maxY, topInset, bottomInset)
                            scope.launch { pos.animateTo(Offset(sx, sy), animationSpec = spatialSpec) }
                        },
                        onDrag = { change, drag ->
                            change.consume()
                            dragPos = Offset(
                                (dragPos.x + drag.x).coerceIn(0f, maxX),
                                (dragPos.y + drag.y).coerceIn(0f, maxY),
                            )
                            scope.launch { pos.snapTo(dragPos) }
                        },
                    )
                },
        ) {
            VideoSurface(Modifier.fillMaxSize(), overlay = true, onSurface = onSurface)
            Box(Modifier.matchParentSize().border(1.dp, Color.White.copy(alpha = 0.30f), pipShape))
        }
    }
}

/** A SurfaceView whose surface is handed to [onSurface] while it exists (null when it is destroyed). */
@Composable
private fun VideoSurface(modifier: Modifier, overlay: Boolean, onSurface: (Surface?) -> Unit) {
    val current by rememberUpdatedState(onSurface)
    val radiusPx = with(LocalDensity.current) { SELF_CORNER_RADIUS.toPx() }
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
                if (overlay) {
                    setZOrderMediaOverlay(true)
                    outlineProvider = object : ViewOutlineProvider() {
                        override fun getOutline(view: View, outline: Outline) {
                            outline.setRoundRect(0, 0, view.width, view.height, radiusPx)
                        }
                    }
                    clipToOutline = true
                }
                holder.addCallback(callback)
                surfaceView = this
            }
        },
    )
}

@Composable
private fun RoundButton(icon: ImageVector, label: String, highlighted: Boolean, onVideo: Boolean, onClick: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    val corner by animateDpAsState(
        targetValue = if (highlighted) 14.dp else 24.dp,
        animationSpec = MaterialTheme.motionScheme.fastSpatialSpec(),
        label = "videoToggleCorner",
    )
    val container by animateColorAsState(
        targetValue = when {
            onVideo -> if (highlighted) Color.White else Color.White.copy(alpha = 0.20f)
            else -> if (highlighted) scheme.primary else scheme.surfaceContainerHighest
        },
        animationSpec = MaterialTheme.motionScheme.fastEffectsSpec(),
        label = "videoToggleContainer",
    )
    val content by animateColorAsState(
        targetValue = when {
            onVideo -> if (highlighted) Color.Black else Color.White
            else -> if (highlighted) scheme.onPrimary else scheme.onSurface
        },
        animationSpec = MaterialTheme.motionScheme.fastEffectsSpec(),
        label = "videoToggleContent",
    )
    M3Surface(
        onClick = onClick,
        modifier = Modifier.size(48.dp),
        shape = RoundedCornerShape(corner),
        color = container,
        contentColor = content,
    ) {
        Box(contentAlignment = Alignment.Center) {
            Icon(icon, contentDescription = label, modifier = Modifier.size(22.dp))
        }
    }
}

private const val DEFAULT_ASPECT = 3f / 4f

