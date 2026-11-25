package com.zalom3ui.app.ui.call

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.app.NotificationManagerCompat
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import com.zalom3ui.app.call.CallService
import com.zalom3ui.app.call.CallState
import com.zalom3ui.app.data.ApiClient
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.server.LocalServer
import com.zalom3ui.app.server.MessageNotifier
import com.zalom3ui.app.ui.AppTheme
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.launch

/**
 * Full-screen call UI: ringing (Answer / Decline), connecting, connected (Mute / Speaker / Hang up) and "Call ended".
 * Opens over the lock screen from the incoming-call notification.
 */
class CallActivity : ComponentActivity() {
    private var callId by mutableStateOf("")
    private var callerId by mutableStateOf("")
    private var callerName by mutableStateOf("")
    private var avatar by mutableStateOf<String?>(null)
    private var video by mutableStateOf(false)
    private var micDenied by mutableStateOf(false)
    private var error by mutableStateOf<String?>(null)
    private var answering = false

    private val micPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) answer() else micDenied = true
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O_MR1) {
            setShowWhenLocked(true)
            setTurnScreenOn(true)
        } else {
            @Suppress("DEPRECATION")
            window.addFlags(
                WindowManager.LayoutParams.FLAG_SHOW_WHEN_LOCKED or WindowManager.LayoutParams.FLAG_TURN_SCREEN_ON,
            )
        }
        val appearance = SettingsStore(this).loadAppearance()
        setContent {
            AppTheme(appearance) {
                val service by CallService.state.collectAsStateWithLifecycle()
                val muted by CallService.muted.collectAsStateWithLifecycle()
                val speaker by CallService.speakerOn.collectAsStateWithLifecycle()
                CallScreen(
                    state = CallScreenState(
                        name = nameOf(service),
                        callerId = callerId,
                        avatar = avatarOf(service),
                        phase = phaseOf(service),
                        muted = muted,
                        speakerOn = speaker,
                        micDenied = micDenied,
                        error = error,
                    ),
                    onAnswer = ::answer,
                    onDecline = ::decline,
                    onHangUp = { CallService.hangUp(this@CallActivity) },
                    onToggleMute = { CallService.setMuted(!muted) },
                    onToggleSpeaker = { CallService.setSpeaker(!speaker) },
                )
            }
        }
        handle(intent)
        // Close a moment after the call ends, so "Call ended" is readable.
        lifecycleScope.launch {
            CallService.state.collectLatest { s ->
                if (s is CallState.Ended && error == null) {
                    delay(CLOSE_DELAY_MS)
                    CallService.reset()
                    finish()
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handle(intent)
    }

    private fun nameOf(s: CallState): String = when (s) {
        is CallState.Connecting -> s.name
        is CallState.Connected -> s.name
        is CallState.Ringing -> s.name
        else -> callerName
    }.ifBlank { callerName }

    private fun avatarOf(s: CallState): String? = when (s) {
        is CallState.Connecting -> s.avatar
        is CallState.Connected -> s.avatar
        is CallState.Ringing -> s.avatar
        else -> null
    } ?: avatar

    private fun phaseOf(s: CallState): CallPhase = when (s) {
        is CallState.Connecting -> CallPhase.Connecting
        is CallState.Connected -> CallPhase.Connected(s.since)
        is CallState.Ended -> CallPhase.Ended
        else -> CallPhase.Ringing(video)
    }

    private fun handle(intent: Intent) {
        val id = intent.getStringExtra(EXTRA_CALL_ID).orEmpty()
        if (id.isEmpty()) {
            finish()
            return
        }
        if (id != callId) {
            answering = false
            micDenied = false
            error = null
        }
        callId = id
        callerId = intent.getStringExtra(EXTRA_CALLER_ID).orEmpty()
        callerName = intent.getStringExtra(EXTRA_CALLER_NAME).orEmpty()
        avatar = intent.getStringExtra(EXTRA_AVATAR)
        video = intent.getBooleanExtra(EXTRA_VIDEO, false)
        CallService.showRinging(callId, callerName, avatar)
        if (intent.action == ACTION_ANSWER) answer()
    }

    private fun answer() {
        if (answering || callId.isEmpty()) return
        if (checkSelfPermission(Manifest.permission.RECORD_AUDIO) != PackageManager.PERMISSION_GRANTED) {
            micPermission.launch(Manifest.permission.RECORD_AUDIO)
            return
        }
        val s = CallService.state.value
        if (s is CallState.Connecting || s is CallState.Connected) return
        answering = true
        val id = callId
        val name = callerName
        // The call is answered now, so the ringing notification has done its job.
        NotificationManagerCompat.from(this).cancel(MessageNotifier.callTag(id), MessageNotifier.CALL_NOTIFICATION_ID)
        CallService.start(this, id, name, avatar)
        lifecycleScope.launch {
            try {
                ApiClient(LocalServer.url(this@CallActivity), SettingsStore(this@CallActivity).localToken()).answerCall(id)
            } catch (e: Exception) {
                error = e.message ?: "Couldn't answer the call"
                CallService.stop(this@CallActivity)
                delay(ERROR_DELAY_MS)
                CallService.reset()
                finish()
            }
        }
    }

    private fun decline() {
        val id = callId
        NotificationManagerCompat.from(this).cancel(MessageNotifier.callTag(id), MessageNotifier.CALL_NOTIFICATION_ID)
        val api = ApiClient(LocalServer.url(this), SettingsStore(this).localToken())
        // Outlives the screen: the request should finish even though we close right away.
        CoroutineScope(Dispatchers.IO).launch { runCatching { api.declineCall(id) } }
        CallService.reset()
        finish()
    }

    companion object {
        const val ACTION_RING = "com.zalom3ui.app.action.CALL_SHOW"
        const val ACTION_ANSWER = "com.zalom3ui.app.action.CALL_ANSWER_NOW"
        const val EXTRA_CALL_ID = "callId"
        const val EXTRA_CALLER_ID = "callerId"
        const val EXTRA_CALLER_NAME = "callerName"
        const val EXTRA_AVATAR = "avatar"
        const val EXTRA_VIDEO = "video"
        private const val CLOSE_DELAY_MS = 1_200L
        private const val ERROR_DELAY_MS = 2_500L

        /** [answerNow]: the Answer button of the notification, so the call is picked up as soon as the screen opens. */
        fun intent(
            context: Context,
            callId: String,
            callerId: String,
            callerName: String,
            avatar: String? = null,
            video: Boolean = false,
            answerNow: Boolean = false,
        ): Intent = Intent(context, CallActivity::class.java)
            .setAction(if (answerNow) ACTION_ANSWER else ACTION_RING)
            .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
            .putExtra(EXTRA_CALL_ID, callId)
            .putExtra(EXTRA_CALLER_ID, callerId)
            .putExtra(EXTRA_CALLER_NAME, callerName)
            .putExtra(EXTRA_AVATAR, avatar)
            .putExtra(EXTRA_VIDEO, video)
    }
}
