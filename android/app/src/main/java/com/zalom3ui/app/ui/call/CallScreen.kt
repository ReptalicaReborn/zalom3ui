package com.zalom3ui.app.ui.call

import androidx.compose.foundation.background
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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Call
import androidx.compose.material.icons.filled.CallEnd
import androidx.compose.material.icons.filled.Mic
import androidx.compose.material.icons.filled.MicOff
import androidx.compose.material.icons.automirrored.filled.VolumeOff
import androidx.compose.material.icons.automirrored.filled.VolumeUp
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.produceState
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import com.zalom3ui.app.ui.Avatar
import kotlinx.coroutines.delay

/** Where the call is, as far as the screen cares. */
sealed interface CallPhase {
    data class Ringing(val video: Boolean) : CallPhase

    data object Connecting : CallPhase

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
)

private val AnswerGreen = Color(0xFF2E9E5B)

@Composable
fun CallScreen(
    state: CallScreenState,
    onAnswer: () -> Unit,
    onDecline: () -> Unit,
    onHangUp: () -> Unit,
    onToggleMute: () -> Unit,
    onToggleSpeaker: () -> Unit,
) {
    Box(Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surface).safeDrawingPadding()) {
        Column(
            Modifier.fillMaxSize().padding(horizontal = 24.dp, vertical = 32.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Spacer(Modifier.height(48.dp))
            Avatar(name = state.name, url = state.avatar.orEmpty(), key = state.callerId.ifEmpty { state.name }, isGroup = false, size = 128.dp)
            Spacer(Modifier.height(24.dp))
            Text(
                state.name.ifBlank { "Unknown" },
                style = MaterialTheme.typography.headlineMedium,
                color = MaterialTheme.colorScheme.onSurface,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                statusText(state),
                style = MaterialTheme.typography.titleMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            if (state.micDenied) {
                Spacer(Modifier.height(16.dp))
                Text(
                    "Zalỏ needs the microphone to take calls. Allow it in the phone's app settings, or decline the call.",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.error,
                    textAlign = TextAlign.Center,
                )
            }
            Box(Modifier.weight(1f))
            when (state.phase) {
                is CallPhase.Ringing -> RingingButtons(state, onAnswer, onDecline)
                CallPhase.Connecting, is CallPhase.Connected -> ConnectedButtons(state, onHangUp, onToggleMute, onToggleSpeaker)
                CallPhase.Ended -> Unit
            }
        }
    }
}

@Composable
private fun statusText(state: CallScreenState): String {
    state.error?.let { return it }
    return when (val phase = state.phase) {
        is CallPhase.Ringing -> if (phase.video) "Incoming video call (audio only)" else "Incoming voice call"
        CallPhase.Connecting -> "Connecting…"
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
private fun RingingButtons(state: CallScreenState, onAnswer: () -> Unit, onDecline: () -> Unit) {
    Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceEvenly) {
        LabeledButton(
            Icons.Filled.CallEnd,
            "Decline",
            MaterialTheme.colorScheme.error,
            MaterialTheme.colorScheme.onError,
            onDecline,
        )
        if (!state.micDenied) LabeledButton(Icons.Filled.Call, "Answer", AnswerGreen, Color.White, onAnswer)
    }
}

@Composable
private fun ConnectedButtons(state: CallScreenState, onHangUp: () -> Unit, onToggleMute: () -> Unit, onToggleSpeaker: () -> Unit) {
    Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(32.dp)) {
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
        LabeledButton(Icons.Filled.CallEnd, "Hang up", MaterialTheme.colorScheme.error, MaterialTheme.colorScheme.onError, onHangUp)
    }
}

@Composable
private fun LabeledButton(icon: ImageVector, label: String, container: Color, content: Color, onClick: () -> Unit) {
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        IconButton(
            onClick = onClick,
            modifier = Modifier.size(72.dp),
            colors = IconButtonDefaults.iconButtonColors(containerColor = container, contentColor = content),
        ) {
            Icon(icon, contentDescription = label, modifier = Modifier.size(32.dp))
        }
        TextButton(onClick = onClick) { Text(label, color = MaterialTheme.colorScheme.onSurfaceVariant) }
    }
}

@Composable
private fun ToggleButton(icon: ImageVector, label: String, on: Boolean, onClick: () -> Unit) {
    val scheme = MaterialTheme.colorScheme
    Column(horizontalAlignment = Alignment.CenterHorizontally) {
        IconButton(
            onClick = onClick,
            modifier = Modifier.size(64.dp).background(if (on) scheme.primary else scheme.surfaceVariant, CircleShape),
            colors = IconButtonDefaults.iconButtonColors(contentColor = if (on) scheme.onPrimary else scheme.onSurfaceVariant),
        ) {
            Icon(icon, contentDescription = label, modifier = Modifier.size(28.dp))
        }
        TextButton(onClick = onClick) { Text(label, color = scheme.onSurfaceVariant) }
    }
}
