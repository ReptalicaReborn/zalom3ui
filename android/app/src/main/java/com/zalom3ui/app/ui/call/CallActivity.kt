package com.zalom3ui.app.ui.call

import android.Manifest
import android.app.KeyguardManager
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
import com.zalom3ui.app.data.ServerEvent
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.server.LocalServer
import com.zalom3ui.app.server.MessageNotifier
import com.zalom3ui.app.ui.AppTheme
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.collectLatest
import kotlinx.coroutines.flow.first
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

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
    private var cameraDenied by mutableStateOf(false)

    /** This screen was opened to place a call (not to answer one). */
    private var outgoing by mutableStateOf(false)
    private var placeWithVideo = false

    /** Placing a call asks for the microphone and, for video, the camera; a refused camera still places a voice-only start. */
    private val placePermissions = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { _ ->
        if (!hasPermission(Manifest.permission.RECORD_AUDIO)) {
            error = "Allow the microphone in the phone's app settings to make calls"
        } else {
            if (placeWithVideo && !hasPermission(Manifest.permission.CAMERA)) cameraDenied = true
            placeCall()
        }
    }

    /** Answering asks for the microphone and, with video, the camera in one go; a refused camera still answers by voice. */
    private var answerWithVideo = false

    private val answerPermissions = registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) { granted ->
        if (granted[Manifest.permission.RECORD_AUDIO] == false || !hasPermission(Manifest.permission.RECORD_AUDIO)) {
            micDenied = true
        } else {
            if (answerWithVideo && !hasPermission(Manifest.permission.CAMERA)) cameraDenied = true
            answer(answerWithVideo)
        }
    }

    private val cameraPermission = registerForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) CallService.setCamera(this, true) else cameraDenied = true
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        answerWithVideo = savedInstanceState?.getBoolean(STATE_ANSWER_WITH_VIDEO, false) ?: false
        placeWithVideo = savedInstanceState?.getBoolean(STATE_PLACE_WITH_VIDEO, false) ?: false
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
                val videoCall by CallService.videoCall.collectAsStateWithLifecycle()
                val cameraOn by CallService.cameraOn.collectAsStateWithLifecycle()
                val peerCamera by CallService.peerCameraOn.collectAsStateWithLifecycle()
                val remoteSize by CallService.remoteSize.collectAsStateWithLifecycle()
                val notice by CallService.notice.collectAsStateWithLifecycle()
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
                        // Before the answer the call's own flag decides; afterwards the service's.
                        video = if (service is CallState.Ringing) video else videoCall,
                        cameraOn = cameraOn,
                        peerCameraOn = peerCamera,
                        remoteAspect = remoteSize?.let { (w, h) -> if (w > 0 && h > 0) w.toFloat() / h else null },
                        notice = notice ?: if (cameraDenied) "Allow the camera in the phone's app settings to send video" else null,
                    ),
                    onAnswer = { answer(false) },
                    onAnswerWithVideo = { answer(true) },
                    onToggleCamera = { toggleCamera(cameraOn) },
                    onSwitchCamera = CallService::switchCamera,
                    onRemoteSurface = CallService::setRemoteSurface,
                    onPreviewSurface = CallService::setPreviewSurface,
                    onDecline = ::decline,
                    onHangUp = {
                        // Before the call service exists there is nothing to cancel.
                        if (outgoing && CallService.state.value is CallState.Idle) finish() else CallService.hangUp(this@CallActivity)
                    },
                    onToggleMute = { CallService.setMuted(!muted) },
                    onToggleSpeaker = { CallService.setSpeaker(!speaker) },
                )
            }
        }
        handle(intent, isFreshStart = savedInstanceState == null)
        // Keep the display awake while a call is active, watch for remote hang-ups while ringing,
        // and close a moment after the call ends so "Call ended" (or an error reason) is readable.
        lifecycleScope.launch {
            CallService.state.collectLatest { s ->
                updateKeepScreenOn(s)
                when (s) {
                    is CallState.Ringing -> watchRingingCall(s.callId)
                    is CallState.Ended -> {
                        val hasError = error != null || s.reason != "Call ended"
                        if (s.reason != "Call ended" && error == null) error = s.reason
                        delay(if (hasError) ERROR_DELAY_MS else CLOSE_DELAY_MS)
                        CallService.reset()
                        finish()
                    }
                    else -> Unit
                }
            }
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putBoolean(STATE_ANSWER_WITH_VIDEO, answerWithVideo)
        outState.putBoolean(STATE_PLACE_WITH_VIDEO, placeWithVideo)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        handle(intent, isFreshStart = true)
    }

    private fun updateKeepScreenOn(state: CallState) {
        val keepOn = state is CallState.Ringing || state is CallState.Calling || state is CallState.Connecting || state is CallState.Connected
        if (keepOn) {
            window.addFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        } else {
            window.clearFlags(WindowManager.LayoutParams.FLAG_KEEP_SCREEN_ON)
        }
    }

    private suspend fun watchRingingCall(ringingCallId: String) {
        if (ringingCallId.isEmpty()) return
        val api = ApiClient.onDevice(this)
        val ended = withTimeoutOrNull(RINGING_TIMEOUT_MS) {
            while (isActive) {
                val current = runCatching { api.currentCall() }.getOrNull()
                if (current != null && (current.callId != ringingCallId || current.state != "ringing")) {
                    return@withTimeoutOrNull true
                }
                try {
                    api.events().first { event ->
                        when (event) {
                            is ServerEvent.CallEnded -> event.callId.isEmpty() || event.callId == ringingCallId
                            is ServerEvent.CallConnected -> event.callId.isEmpty() || event.callId == ringingCallId
                            else -> false
                        }
                    }
                    return@withTimeoutOrNull true
                } catch (e: CancellationException) {
                    throw e
                } catch (_: NoSuchElementException) {
                    // Socket closed cleanly; reconnect and re-check currentCall().
                } catch (_: Exception) {
                    // Socket dropped; reconnect and re-check currentCall().
                }
                delay(RINGING_RECONNECT_MS)
            }
            false
        }
        if (ended != false) dismissRinging(ringingCallId)
    }

    private fun dismissRinging(ringingCallId: String) {
        val current = CallService.state.value
        if (answering || current !is CallState.Ringing || current.callId != ringingCallId) return
        NotificationManagerCompat.from(this).cancel(MessageNotifier.callTag(ringingCallId), MessageNotifier.CALL_NOTIFICATION_ID)
        CallService.reset()
        finish()
    }

    private fun nameOf(s: CallState): String = when (s) {
        is CallState.Calling -> s.name
        is CallState.Connecting -> s.name
        is CallState.Connected -> s.name
        is CallState.Ringing -> s.name
        else -> callerName
    }.ifBlank { callerName }

    private fun avatarOf(s: CallState): String? = when (s) {
        is CallState.Calling -> s.avatar
        is CallState.Connecting -> s.avatar
        is CallState.Connected -> s.avatar
        is CallState.Ringing -> s.avatar
        else -> null
    } ?: avatar

    private fun phaseOf(s: CallState): CallPhase = when (s) {
        is CallState.Calling -> CallPhase.Calling(s.ringing)
        is CallState.Connecting -> CallPhase.Connecting
        is CallState.Connected -> CallPhase.Connected(s.since)
        is CallState.Ended -> CallPhase.Ended
        else -> if (outgoing) CallPhase.Calling(false) else CallPhase.Ringing(video)
    }

    /** Opened to call someone: ask for the permissions, then start the call. A call already running is just shown. */
    private fun handleOutgoing(intent: Intent, isFreshStart: Boolean) {
        outgoing = true
        callId = PENDING_ID
        callerId = intent.getStringExtra(EXTRA_CALLER_ID).orEmpty()
        callerName = intent.getStringExtra(EXTRA_CALLER_NAME).orEmpty()
        avatar = intent.getStringExtra(EXTRA_AVATAR)
        video = intent.getBooleanExtra(EXTRA_VIDEO, false)
        if (!isFreshStart || callerId.isEmpty()) return
        // A finished call's "Ended" would close this screen again.
        if (CallService.state.value is CallState.Ended) CallService.reset()
        val s = CallService.state.value
        if (s is CallState.Calling || s is CallState.Connecting || s is CallState.Connected) return
        placeWithVideo = video
        val missing = buildList {
            if (!hasPermission(Manifest.permission.RECORD_AUDIO)) add(Manifest.permission.RECORD_AUDIO)
            if (video && !hasPermission(Manifest.permission.CAMERA)) add(Manifest.permission.CAMERA)
        }
        if (missing.isEmpty()) placeCall() else placePermissions.launch(missing.toTypedArray())
    }

    private fun placeCall() {
        val s = CallService.state.value
        if (s is CallState.Calling || s is CallState.Connecting || s is CallState.Connected) return
        try {
            // A video call starts with the camera on, if we may use it; the service switches it on once they pick up.
            CallService.startOutgoing(
                this,
                callerId,
                callerName,
                avatar,
                video = placeWithVideo,
                camera = placeWithVideo && hasPermission(Manifest.permission.CAMERA),
            )
        } catch (e: Exception) {
            error = e.message ?: "Couldn't place the call"
        }
    }

    private fun handle(intent: Intent, isFreshStart: Boolean) {
        if (intent.action == ACTION_OUTGOING) {
            handleOutgoing(intent, isFreshStart)
            return
        }
        outgoing = false
        val id = intent.getStringExtra(EXTRA_CALL_ID).orEmpty()
        if (id.isEmpty()) {
            finish()
            return
        }
        if (id != callId) {
            answering = false
            micDenied = false
            cameraDenied = false
            error = null
        }
        callId = id
        callerId = intent.getStringExtra(EXTRA_CALLER_ID).orEmpty()
        callerName = intent.getStringExtra(EXTRA_CALLER_NAME).orEmpty()
        avatar = intent.getStringExtra(EXTRA_AVATAR)
        video = intent.getBooleanExtra(EXTRA_VIDEO, false)
        if (isFreshStart || CallService.state.value is CallState.Idle) {
            CallService.showRinging(callId, callerName, avatar)
        }
        // The notification's Answer is voice only, even for a video call; "Answer with video" is on the screen.
        if (isFreshStart && intent.action == ACTION_ANSWER) answer(false)
    }

    private fun hasPermission(permission: String) = checkSelfPermission(permission) == PackageManager.PERMISSION_GRANTED

    /** Dismisses the lock screen before showing a system permission prompt so the dialog isn't hidden behind it. */
    private fun runAfterUnlock(action: () -> Unit) {
        val km = getSystemService(KeyguardManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O && km != null && km.isKeyguardLocked) {
            km.requestDismissKeyguard(
                this,
                object : KeyguardManager.KeyguardDismissCallback() {
                    override fun onDismissSucceeded() {
                        if (!isFinishing && !isDestroyed) action()
                    }

                    override fun onDismissCancelled() = Unit

                    override fun onDismissError() {
                        if (!isFinishing && !isDestroyed) action()
                    }
                },
            )
        } else {
            action()
        }
    }

    /** Camera button: on needs the permission (asked now, the first time), off just stops. */
    private fun toggleCamera(on: Boolean) {
        when {
            on -> CallService.setCamera(this, false)
            hasPermission(Manifest.permission.CAMERA) -> {
                cameraDenied = false
                CallService.setCamera(this, true)
            }
            else -> runAfterUnlock { cameraPermission.launch(Manifest.permission.CAMERA) }
        }
    }

    private fun answer(withVideo: Boolean) {
        if (answering || callId.isEmpty()) return
        val missing = buildList {
            if (!hasPermission(Manifest.permission.RECORD_AUDIO)) add(Manifest.permission.RECORD_AUDIO)
            if (withVideo && !hasPermission(Manifest.permission.CAMERA)) add(Manifest.permission.CAMERA)
        }
        if (missing.isNotEmpty()) {
            answerWithVideo = withVideo
            runAfterUnlock { answerPermissions.launch(missing.toTypedArray()) }
            return
        }
        val s = CallService.state.value
        if (s is CallState.Connecting || s is CallState.Connected || s is CallState.Ended) return
        answering = true
        val id = callId
        val name = callerName
        val appContext = applicationContext
        // The call is answered now, so the ringing notification has done its job.
        NotificationManagerCompat.from(this).cancel(MessageNotifier.callTag(id), MessageNotifier.CALL_NOTIFICATION_ID)
        // A video call starts on the speaker; the camera only if asked for and allowed.
        try {
            CallService.start(this, id, name, avatar, video = video, camera = withVideo && hasPermission(Manifest.permission.CAMERA))
        } catch (e: Exception) {
            answering = false
            error = e.message ?: "Couldn't answer the call"
            return
        }
        // Outlives Activity rotation so answering completes instead of aborting mid-flight.
        callActionScope.launch {
            try {
                ApiClient.onDevice(appContext).answerCall(id)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                CallService.stop(appContext, e.message ?: "Couldn't answer the call")
            }
        }
    }

    private fun decline() {
        val id = callId
        val appContext = applicationContext
        NotificationManagerCompat.from(this).cancel(MessageNotifier.callTag(id), MessageNotifier.CALL_NOTIFICATION_ID)
        val api = ApiClient.onDevice(appContext)
        // Outlives the screen: the request should finish even though we close right away.
        callActionScope.launch { runCatching { api.declineCall(id) } }
        CallService.reset()
        finish()
    }

    companion object {
        const val ACTION_RING = "com.zalom3ui.app.action.CALL_SHOW"
        const val ACTION_ANSWER = "com.zalom3ui.app.action.CALL_ANSWER_NOW"
        /** Place a call to [EXTRA_CALLER_ID]. */
        const val ACTION_OUTGOING = "com.zalom3ui.app.action.CALL_PLACE"
        private const val PENDING_ID = "outgoing"
        private const val STATE_PLACE_WITH_VIDEO = "placeWithVideo"
        const val EXTRA_CALL_ID = "callId"
        const val EXTRA_CALLER_ID = "callerId"
        const val EXTRA_CALLER_NAME = "callerName"
        const val EXTRA_AVATAR = "avatar"
        const val EXTRA_VIDEO = "video"
        private const val STATE_ANSWER_WITH_VIDEO = "answerWithVideo"
        private const val CLOSE_DELAY_MS = 1_200L
        private const val ERROR_DELAY_MS = 2_500L
        private const val RINGING_RECONNECT_MS = 2_000L
        private const val RINGING_TIMEOUT_MS = 75_000L

        /** Application-scoped coroutine scope so answer/decline requests survive Activity rotation or finish. */
        private val callActionScope = CoroutineScope(SupervisorJob() + Dispatchers.IO)

        /** Opens the call screen to call [peerId] (a 1:1 chat's person); it asks for the permissions and places the call. */
        fun outgoingIntent(context: Context, peerId: String, name: String, avatar: String? = null, video: Boolean = false): Intent =
            Intent(context, CallActivity::class.java)
                .setAction(ACTION_OUTGOING)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_SINGLE_TOP)
                .putExtra(EXTRA_CALLER_ID, peerId)
                .putExtra(EXTRA_CALLER_NAME, name)
                .putExtra(EXTRA_AVATAR, avatar)
                .putExtra(EXTRA_VIDEO, video)

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
