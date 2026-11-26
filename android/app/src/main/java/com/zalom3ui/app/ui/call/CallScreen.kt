@file:OptIn(ExperimentalMaterial3ExpressiveApi::class)

package com.zalom3ui.app.ui.call

import android.view.Surface
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.FastOutSlowInEasing
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.StartOffset
import androidx.compose.animation.core.StartOffsetType
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.collectIsPressedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.VolumeOff
import androidx.compose.material.icons.automirrored.filled.VolumeUp
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.CallEnd
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MicOff
import androidx.compose.material.icons.filled.Videocam
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface as M3Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.lerp
import androidx.compose.ui.graphics.luminance
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.zalom3ui.app.ui.Avatar
import com.zalom3ui.app.ui.LocalUltraExpressive
import com.zalom3ui.app.ui.avatarColors
import kotlinx.coroutines.delay

/** Where the call is, as far as the screen cares. */
sealed interface CallPhase {
    data class Ringing(val video: Boolean) : CallPhase

    data object Connecting : CallPhase

    /** A call we placed, not picked up yet: "Calling…", or "Ringing…" once their phone rings. */
    data class Calling(val ringing: Boolean) : CallPhase

    /** [since] is epoch millis. */
    data class Connected(val since: Long) : CallPhase

    data object Ended : CallPhase
}

/** Everything [CallScreen] draws, so it can be previewed or tested without a service. */
data class CallScreenState(
    val name: String,
    val callerId: String,
    val avatar: String?,
    val phase: CallPhase,
    val muted: Boolean = false,
    val speakerOn: Boolean = false,
    /** RECORD_AUDIO was refused: explain, and leave only Decline. */
    val micDenied: Boolean = false,
    val error: String? = null,
    /** A video call: the screen shows pictures instead of just the avatar. */
    val video: Boolean = false,
    val cameraOn: Boolean = false,
    /** The other side's camera is on, so their picture fills the screen. */
    val peerCameraOn: Boolean = true,
    /** Width / height of the other side's decoded picture, once known. */
    val remoteAspect: Float? = null,
    /** Something to tell about the camera (denied, couldn't start). */
    val notice: String? = null,
)

private val AnswerGreen = Color(0xFF2E9E5B)
private val DeepCharcoal = Color(0xFF0E1014)

@Composable
fun CallScreen(
    state: CallScreenState,
    onAnswer: () -> Unit,
    onDecline: () -> Unit,
    onHangUp: () -> Unit,
    onToggleMute: () -> Unit,
    onToggleSpeaker: () -> Unit,
    onAnswerWithVideo: () -> Unit = {},
    onToggleCamera: () -> Unit = {},
    onSwitchCamera: () -> Unit = {},
    onRemoteSurface: (Surface?) -> Unit = {},
    onPreviewSurface: (Surface?) -> Unit = {},
) {
    if (state.video && (state.phase == CallPhase.Connecting || state.phase is CallPhase.Connected)) {
        VideoCallLayout(
            state, onHangUp, onToggleMute, onToggleSpeaker, onToggleCamera, onSwitchCamera, onRemoteSurface, onPreviewSurface,
        )
        return
    }
    val scheme = MaterialTheme.colorScheme
    val avatarKey = state.callerId.ifEmpty { state.name }
    val accent = remember(avatarKey) { avatarColors(avatarKey) }
    val isDark = scheme.surface.luminance() < 0.5f
    val pulsing = state.phase is CallPhase.Ringing || state.phase == CallPhase.Connecting

    Box(
        Modifier
            .fillMaxSize()
            .drawBehind {
                val verticalColors = if (isDark) {
                    listOf(
                        lerp(scheme.surface, accent.content, 0.28f),
                        lerp(scheme.surface, DeepCharcoal, 0.30f),
                        lerp(scheme.surface, DeepCharcoal, 0.55f),
                    )
                } else {
                    listOf(
                        lerp(scheme.surface, accent.container, 0.58f),
                        lerp(scheme.surface, accent.container, 0.20f),
                        scheme.surface,
                    )
                }
                drawRect(Brush.verticalGradient(verticalColors))
                val glowColors = if (isDark) {
                    listOf(
                        accent.content.copy(alpha = 0.30f),
                        accent.container.copy(alpha = 0.08f),
                        Color.Transparent,
                    )
                } else {
                    listOf(
                        accent.container.copy(alpha = 0.68f),
                        accent.content.copy(alpha = 0.06f),
                        Color.Transparent,
                    )
                }
                drawRect(
                    Brush.radialGradient(
                        colors = glowColors,
                        center = Offset(size.width * 0.5f, size.height * 0.27f),
                        radius = size.maxDimension * 0.62f,
                    ),
                )
            }
            .safeDrawingPadding(),
    ) {
        Column(
            Modifier.fillMaxSize().padding(horizontal = 24.dp, vertical = 32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Spacer(Modifier.height(48.dp))
            PulsingAvatar(
                name = state.name,
                url = state.avatar.orEmpty(),
                key = avatarKey,
                ringColor = if (isDark) accent.container else accent.content,
                pulsing = pulsing,
            )
            Spacer(Modifier.height(24.dp))
            Text(
                state.name.ifBlank { "Unknown" },
                style = MaterialTheme.typography.headlineMedium,
                color = scheme.onSurface,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                callStatusText(state),
                style = MaterialTheme.typography.titleMedium.copy(fontFeatureSettings = "tnum"),
                color = scheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            if (state.micDenied) {
                Spacer(Modifier.height(16.dp))
                Text(
                    "Zalỏ needs the microphone to take calls. Allow it in the phone's app settings, or decline the call.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = scheme.error,
                    textAlign = TextAlign.Center,
                )
            }
            state.notice?.let {
                Spacer(Modifier.height(16.dp))
                Text(it, style = MaterialTheme.typography.bodyMedium, color = scheme.error, textAlign = TextAlign.Center)
            }
            Box(Modifier.weight(1f))
            when (state.phase) {
                is CallPhase.Ringing -> RingingButtons(state, onAnswer, onAnswerWithVideo, onDecline)
                CallPhase.Connecting, is CallPhase.Connected -> ConnectedButtons(state, onHangUp, onToggleMute, onToggleSpeaker)
                is CallPhase.Calling -> LabeledButton(Icons.Filled.CallEnd, "Cancel", MaterialTheme.colorScheme.error, MaterialTheme.colorScheme.onError, onHangUp)
                CallPhase.Ended -> Unit
            }
        }
    }
}

/** Caller avatar with two staggered expanding/fading halo rings while ringing or connecting. */
@Composable
private fun PulsingAvatar(name: String, url: String, key: String, ringColor: Color, pulsing: Boolean) {
    val haloModifier = if (pulsing) {
        val transition = rememberInfiniteTransition(label = "callHalo")
        val ring1 by transition.animateFloat(
            initialValue = 0f,
            targetValue = 1f,
            animationSpec = infiniteRepeatable(
                animation = tween(durationMillis = 2000, easing = FastOutSlowInEasing),
                repeatMode = RepeatMode.Restart,
            ),
            label = "haloRing1",
        )
        val ring2 by transition.animateFloat(
            initialValue = 0f,
            targetValue = 1f,
            animationSpec = infiniteRepeatable(
                animation = tween(durationMillis = 2000, easing = FastOutSlowInEasing),
                repeatMode = RepeatMode.Restart,
                initialStartOffset = StartOffset(1000, StartOffsetType.FastForward),
            ),
            label = "haloRing2",
        )
        Modifier.drawBehind {
            val baseRadius = size.minDimension / 2f
            val maxSpread = 38.dp.toPx()
            val strokePx = 2.dp.toPx()
            for (progress in floatArrayOf(ring1, ring2)) {
                val radius = baseRadius + maxSpread * progress
                val alpha = (1f - progress) * 0.42f
                drawCircle(color = ringColor.copy(alpha = alpha * 0.28f), radius = radius)
                drawCircle(color = ringColor.copy(alpha = alpha), radius = radius, style = Stroke(width = strokePx))
            }
        }
    } else {
        Modifier
    }
    Box(Modifier.size(128.dp).then(haloModifier), contentAlignment = Alignment.Center) {
        Avatar(name = name, url = url, key = key, isGroup = false, size = 128.dp)
    }
}

@Composable
internal fun callStatusText(state: CallScreenState): String {
    state.error?.let { return it }
    return when (val phase = state.phase) {
        is CallPhase.Ringing -> if (phase.video) "Incoming video call" else "Incoming voice call"
        CallPhase.Connecting -> "Connecting…"
        is CallPhase.Calling -> if (phase.ringing) "Ringing…" else "Calling…"
        is CallPhase.Connected -> elapsedText(phase.since)
        CallPhase.Ended -> "Call ended"
    }
}

/** A running mm:ss (h:mm:ss past an hour) that ticks once a second. */
@Composable
private fun elapsedText(since: Long): String {
    val now by produceState(System.currentTimeMillis(), since) {
        while (true) {
            value = System.currentTimeMillis()
            delay(1_000)
        }
    }
    return formatElapsed(now - since)
}

fun formatElapsed(millis: Long): String {
    val total = (millis / 1000).coerceAtLeast(0)
    val h = total / 3600
    val m = total % 3600 / 60
    val s = total % 60
    return if (h > 0) "%d:%02d:%02d".format(h, m, s) else "%02d:%02d".format(m, s)
}

@Composable
private fun RingingButtons(state: CallScreenState, onAnswer: () -> Unit, onAnswerWithVideo: () -> Unit, onDecline: () -> Unit) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly) {
        LabeledButton(
            Icons.Filled.CallEnd,
            "Decline",
            MaterialTheme.colorScheme.error,
            MaterialTheme.colorScheme.onError,
            onDecline,
        )
        if (!state.micDenied) LabeledButton(Icons.Filled.Call, "Answer", AnswerGreen, Color.White, onAnswer, emphasize = true)
        // Voice only stays one tap; the camera is something to choose, not a surprise.
        if (!state.micDenied && (state.phase as? CallPhase.Ringing)?.video == true) {
            LabeledButton(Icons.Filled.Videocam, "Answer with video", AnswerGreen, Color.White, onAnswerWithVideo, emphasize = true)
        }
    }
}

@Composable
private fun ConnectedButtons(state: CallScreenState, onHangUp: () -> Unit, onToggleMute: () -> Unit, onToggleSpeaker: () -> Unit) {
    Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(28.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly) {
            ToggleButton(
                if (state.muted) Icons.Filled.MicOff else Icons.Filled.Mic,
                if (state.muted) "Unmute" else "Mute",
                state.muted,
                onToggleMute,
            )
            ToggleButton(
                if (state.speakerOn) Icons.AutoMirrored.Filled.VolumeUp else Icons.AutoMirrored.Filled.VolumeOff,
                "Speaker",
                state.speakerOn,
                onToggleSpeaker,
            )
        }
        HangUpButton(onHangUp)
    }
}

@Composable
private fun LabeledButton(
    icon: ImageVector,
    label: String,
    container: Color,
    content: Color,
    onClick: () -> Unit,
    emphasize: Boolean = false,
) {
    val interactionSource = remember { MutableInteractionSource() }
    val pressed by interactionSource.collectIsPressedAsState()
    val pressScale by animateFloatAsState(
        targetValue = if (pressed) 0.91f else 1f,
        animationSpec = MaterialTheme.motionScheme.fastSpatialSpec(),
        label = "ringButtonScale",
    )
    val elevation by animateDpAsState(
        targetValue = if (pressed) 2.dp else 6.dp,
        animationSpec = MaterialTheme.motionScheme.fastEffectsSpec(),
        label = "ringButtonElevation",
    )
    val idleScale = if (emphasize) {
        val pulse by rememberInfiniteTransition(label = "answerPulse").animateFloat(
            initialValue = 1f,
            targetValue = 1.045f,
            animationSpec = infiniteRepeatable(
                animation = tween(durationMillis = 950, easing = FastOutSlowInEasing),
                repeatMode = RepeatMode.Reverse,
            ),
            label = "answerPulseScale",
        )
        pulse
    } else {
        1f
    }
    val ultra = LocalUltraExpressive.current
    val combinedScale = pressScale * idleScale
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        M3Surface(
            onClick = onClick,
            modifier = Modifier
                .size(if (ultra) 112.dp else 72.dp)
                .graphicsLayer {
                    scaleX = combinedScale
                    scaleY = combinedScale
                },
            shape = CircleShape,
            color = container,
            contentColor = content,
            shadowElevation = elevation,
            interactionSource = interactionSource,
        ) {
            Box(contentAlignment = Alignment.Center) {
                Icon(icon, contentDescription = label, modifier = Modifier.size(if (ultra) 54.dp else 32.dp))
            }
        }
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 8.dp),
        )
    }
}

@Composable
private fun HangUpButton(onClick: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    val ultra = LocalUltraExpressive.current
    val interactionSource = remember { MutableInteractionSource() }
    val pressed by interactionSource.collectIsPressedAsState()
    val scale by animateFloatAsState(
        targetValue = if (pressed) 0.93f else 1f,
        animationSpec = MaterialTheme.motionScheme.fastSpatialSpec(),
        label = "hangUpScale",
    )
    val elevation by animateDpAsState(
        targetValue = if (pressed) 2.dp else 6.dp,
        animationSpec = MaterialTheme.motionScheme.fastEffectsSpec(),
        label = "hangUpElevation",
    )
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        M3Surface(
            onClick = onClick,
            modifier = Modifier
                .size(width = if (ultra) 156.dp else 88.dp, height = if (ultra) 104.dp else 64.dp)
                .graphicsLayer {
                    scaleX = scale
                    scaleY = scale
                },
            shape = CircleShape,
            color = scheme.error,
            contentColor = scheme.onError,
            shadowElevation = elevation,
            interactionSource = interactionSource,
        ) {
            Box(contentAlignment = Alignment.Center) {
                Icon(Icons.Filled.CallEnd, contentDescription = "Hang up", modifier = Modifier.size(if (ultra) 54.dp else 32.dp))
            }
        }
        Text(
            text = "Hang up",
            style = MaterialTheme.typography.labelMedium,
            color = scheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 8.dp),
        )
    }
}

@Composable
private fun ToggleButton(icon: ImageVector, label: String, on: Boolean, onClick: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    val corner by animateDpAsState(
        targetValue = if (on) 18.dp else 32.dp,
        animationSpec = MaterialTheme.motionScheme.fastSpatialSpec(),
        label = "toggleCorner",
    )
    val container by animateColorAsState(
        targetValue = if (on) scheme.primary else scheme.surfaceContainerHigh,
        animationSpec = MaterialTheme.motionScheme.fastEffectsSpec(),
        label = "toggleContainer",
    )
    val content by animateColorAsState(
        targetValue = if (on) scheme.onPrimary else scheme.onSurface,
        animationSpec = MaterialTheme.motionScheme.fastEffectsSpec(),
        label = "toggleContent",
    )
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        M3Surface(
            onClick = onClick,
            modifier = Modifier.size(64.dp),
            shape = RoundedCornerShape(corner),
            color = container,
            contentColor = content,
        ) {
            Box(contentAlignment = Alignment.Center) {
                Icon(icon, contentDescription = label, modifier = Modifier.size(28.dp))
            }
        }
        Text(
            text = label,
            style = MaterialTheme.typography.labelMedium,
            color = scheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(top = 8.dp),
        )
    }
}
