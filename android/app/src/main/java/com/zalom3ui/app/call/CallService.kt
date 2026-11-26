package com.zalom3ui.app.call

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.view.Surface
import android.os.IBinder
import android.os.PowerManager
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import com.zalom3ui.app.R
import com.zalom3ui.app.data.ApiClient
import com.zalom3ui.app.data.ServerEvent
import com.zalom3ui.app.ui.call.CallActivity
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.drop
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/** What the call screen shows. Written by [CallActivity] (ringing) and [CallService] (the rest). */
sealed interface CallState {
    data object Idle : CallState

    data class Ringing(val callId: String, val name: String, val avatar: String? = null) : CallState

    /** A call we placed, before the other side picks up. [ringing]: their phone rings (else we're still setting it up). */
    data class Calling(val callId: String, val name: String, val avatar: String? = null, val ringing: Boolean = false) : CallState

    /** Answered on our side; waiting for the server to say the media is up. */
    data class Connecting(val callId: String, val name: String, val avatar: String? = null) : CallState

    /** [since] is epoch millis, for the running timer. */
    data class Connected(val callId: String, val name: String, val since: Long, val avatar: String? = null) : CallState

    data class Ended(val reason: String) : CallState
}

/**
 * Keeps an answered call alive in the background (microphone foreground service, plus camera while it is on, main process): shows the ongoing-call
 * notification, follows the call through the server's events, and owns the [CallMedia] while connected.
 */
class CallService : Service() {
    /** Coordinates call lifecycle, media attachment, and companion surface/control updates without lock-ordering cycles. */
    private val stateLock: Any = mediaLock
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var events: Job? = null
    private var muteSync: Job? = null
    private var media: CallMedia? = null
    private var wakeLock: PowerManager.WakeLock? = null
    private var callId = ""
    private var name = ""
    private var avatar: String? = null
    private var ending = false

    /** The call was placed from here (so the server's reasons for ending it are worth showing). */
    private var outgoing = false
    private var callGen = 0L
    private var cameraGen = 0L

    /** Camera wanted before the media was up (answered with video); started once connected. */
    private var cameraWanted = false
    private val main = Handler(Looper.getMainLooper())

    private val api: ApiClient get() = ApiClient.onDevice(this)

    override fun onCreate() {
        super.onCreate()
        synchronized(stateLock) {
            activeInstance = this
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> begin(intent)
            ACTION_HANGUP -> hangUp()
            ACTION_STOP -> finish(intent.getStringExtra(EXTRA_REASON).orEmpty().ifBlank { "Call ended" })
            ACTION_CAMERA -> setCamera(intent.getBooleanExtra(EXTRA_ON, false))
            else -> {
                val noActiveCall = synchronized(stateLock) { callId.isEmpty() || ending }
                if (noActiveCall) satisfyForegroundThenStop()
            }
        }
        return START_NOT_STICKY
    }

    /**
     * Satisfies Android 8+'s startForegroundService() contract before stopping when invoked with an empty/null intent.
     */
    private fun satisfyForegroundThenStop() {
        runCatching { startInForeground(camera = false) }
        runCatching { ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE) }
        stopSelf()
    }

    private fun begin(intent: Intent) {
        val peerId = intent.getStringExtra(EXTRA_PEER_ID).orEmpty()
        val placing = peerId.isNotEmpty()
        // A call we place has no id until the server answers `calls/start`.
        val newId = if (placing) PENDING_CALL_ID else intent.getStringExtra(EXTRA_CALL_ID).orEmpty()
        if (newId.isEmpty()) {
            val noActiveCall = synchronized(stateLock) { callId.isEmpty() || ending }
            if (noActiveCall) satisfyForegroundThenStop()
            return
        }
        val newName = intent.getStringExtra(EXTRA_NAME).orEmpty()
        val newAvatar = intent.getStringExtra(EXTRA_AVATAR)
        // Only ask for the camera type when we may use it: Android 14 refuses the service otherwise.
        val wantCamera = intent.getBooleanExtra(EXTRA_CAMERA, false) && hasCameraPermission()

        val oldEvents: Job?
        val oldMuteSync: Job?
        val oldMedia: CallMedia?
        val oldWakeLock: PowerManager.WakeLock?
        val gen: Long
        synchronized(stateLock) {
            oldEvents = events
            events = null
            oldMuteSync = muteSync
            muteSync = null
            oldMedia = media
            media = null
            if (activeMedia === oldMedia) activeMedia = null
            oldWakeLock = wakeLock
            wakeLock = null

            callGen++
            cameraGen++
            gen = callGen
            callId = newId
            outgoing = placing
            name = newName
            avatar = newAvatar
            ending = false
            cameraWanted = wantCamera
            activeInstance = this
        }

        oldEvents?.cancel()
        oldMuteSync?.cancel()
        runCatching { oldMedia?.stop() }
        releaseWakeLock(oldWakeLock)

        _state.value = if (placing) CallState.Calling(newId, newName, newAvatar) else CallState.Connecting(newId, newName, newAvatar)
        try {
            startInForeground(wantCamera)
        } catch (e: Exception) {
            // Android refused a microphone service (permission revoked, app not visible).
            finish("Couldn't keep the call running: ${e.message}", expectedGen = gen)
            return
        }
        acquireWakeLock(gen)

        val wantVideo = intent.getBooleanExtra(EXTRA_VIDEO, false)
        val newEvents = scope.launch { if (placing) placeCall(peerId, wantVideo, gen) else follow(newId, gen) }
        // The other side's app shows when we mute, as with the official apps.
        val newMuteSync = scope.launch { _muted.drop(1).collect { runCatching { api.setCallMuted(it) } } }
        val stale = synchronized(stateLock) {
            if (ending || gen != callGen) {
                true
            } else {
                events = newEvents
                muteSync = newMuteSync
                false
            }
        }
        if (stale) {
            newEvents.cancel()
            newMuteSync.cancel()
        }
    }

    private fun acquireWakeLock(gen: Long) {
        val wl = runCatching {
            val pm = getSystemService(PowerManager::class.java)
            pm?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "zalom3ui:call")?.apply {
                setReferenceCounted(false)
                acquire(WAKE_LOCK_TIMEOUT_MS)
            }
        }.getOrNull() ?: return
        val stale = synchronized(stateLock) {
            if (ending || gen != callGen) {
                true
            } else {
                wakeLock = wl
                false
            }
        }
        if (stale) releaseWakeLock(wl)
    }

    private fun releaseWakeLock(wl: PowerManager.WakeLock?) {
        if (wl == null) return
        runCatching { if (wl.isHeld) wl.release() }
    }

    /** Posts `calls/start`, then follows the call the server made. */
    private suspend fun placeCall(peerId: String, video: Boolean, gen: Long) {
        val started = try {
            api.startCall(peerId, video)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            finish(e.message ?: "Couldn't place the call", expectedGen = gen)
            return
        }
        val adopted = synchronized(stateLock) {
            if (ending || gen != callGen) {
                false
            } else {
                callId = started.callId
                true
            }
        }
        if (!adopted) {
            // Cancelled while the request was in flight: don't leave the call ringing.
            runCatching { api.hangUpCall() }
            return
        }
        follow(started.callId, gen, placed = true)
    }

    /** Shows "Ringing…" once the other phone rings. */
    private fun onRinging(gen: Long) {
        val (id, n, a) = synchronized(stateLock) {
            if (ending || gen != callGen) return
            Triple(callId, name, avatar)
        }
        if (_state.value is CallState.Calling) _state.value = CallState.Calling(id, n, a, ringing = true)
    }

    private suspend fun follow(targetCallId: String, gen: Long, placed: Boolean = false) {
        // The call may already be connected (service restarted, or the event came before we listened).
        val snapshot = runCatching { api.currentCall() }
        snapshot.getOrNull()?.let {
            if (it.callId == targetCallId && it.state == "connected") onConnected(it.connectedAt, gen)
            if (it.callId == targetCallId && it.state == "ringing") onRinging(gen)
        }
        // A call we placed that is already gone (ended before we listened).
        if (placed && snapshot.isSuccess && snapshot.getOrNull() == null) {
            finish("Call ended", expectedGen = gen)
            return
        }
        while (scope.isActive) {
            if (synchronized(stateLock) { ending || gen != callGen }) return
            try {
                api.events().collect { event ->
                    if (synchronized(stateLock) { ending || gen != callGen }) return@collect
                    when (event) {
                        is ServerEvent.CallConnected -> if (event.callId == targetCallId) onConnected(event.connectedAt, gen)
                        is ServerEvent.CallRinging -> if (event.callId == targetCallId) onRinging(gen)
                        is ServerEvent.CallPeer -> if (event.callId.isEmpty() || event.callId == targetCallId) _peerCamera.value = event.cameraOn
                        is ServerEvent.CallEnded -> if (event.callId.isEmpty() || event.callId == targetCallId) {
                            val placedHere = synchronized(stateLock) { outgoing }
                            finish(if (placedHere) endReason(event.act) else "Call ended", expectedGen = gen)
                        }
                        else -> Unit
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // The socket dropped; the call itself may still be up, so keep listening.
            }
            delay(RECONNECT_MS)
        }
    }

    private fun onConnected(connectedAt: Long, gen: Long) {
        val shouldStart = synchronized(stateLock) {
            !ending && media == null && gen == callGen
        }
        if (!shouldStart) return

        val since = if (connectedAt > 0) connectedAt else System.currentTimeMillis()
        val m = CallMediaFactory.create(applicationContext)
        m.muted = _muted.value
        m.speakerOn = _speaker.value
        try {
            m.start()
        } catch (e: Exception) {
            runCatching { m.stop() }
            // Hang up too: otherwise the other side stays in a silent call until Zalo times it out.
            scope.launch {
                val stillCurrent = synchronized(stateLock) { !ending && gen == callGen }
                if (stillCurrent) {
                    runCatching { api.hangUpCall() }
                    finish("Couldn't start call audio: ${e.message}", expectedGen = gen)
                }
            }
            return
        }

        val listener = createVideoListener(gen)
        val attachedId: String
        val attachedName: String
        val attachedAvatar: String?
        val wantCam: Boolean
        val camGenAtAttach: Long
        val aborted: Boolean
        synchronized(stateLock) {
            if (ending || media != null || gen != callGen) {
                aborted = true
                attachedId = ""
                attachedName = ""
                attachedAvatar = null
                wantCam = false
                camGenAtAttach = 0L
            } else {
                aborted = false
                m.muted = _muted.value
                m.speakerOn = _speaker.value
                m.setVideoListener(listener)
                remoteSurface?.let { m.setRemoteSurface(it) }
                previewSurface?.let { m.setPreviewSurface(it) }
                media = m
                activeMedia = m
                attachedId = callId
                attachedName = name
                attachedAvatar = avatar
                wantCam = cameraWanted
                camGenAtAttach = cameraGen
            }
        }
        if (aborted) {
            runCatching { m.stop() }
            return
        }
        _state.value = CallState.Connected(attachedId, attachedName, since, attachedAvatar)
        if (wantCam) {
            main.post {
                val stillWanted = synchronized(stateLock) {
                    !ending && gen == callGen && camGenAtAttach == cameraGen && cameraWanted
                }
                if (stillWanted) setCamera(true, expectedGen = gen)
            }
        }
    }

    private fun createVideoListener(gen: Long) = object : VideoListener {
        override fun onRemoteVideoSize(width: Int, height: Int) {
            if (width <= 0 || height <= 0) return
            val active = synchronized(stateLock) { !ending && gen == callGen }
            if (active) _remoteSize.value = width to height
        }

        override fun onCameraStopped(message: String) {
            val camGen = synchronized(stateLock) { cameraGen }
            main.post { cameraFailed(message, expectedCallGen = gen, expectedCameraGen = camGen) }
        }
    }

    private fun hasCameraPermission() =
        checkSelfPermission(Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED

    /** Starts or stops sending our camera and tells the other side. Runs on the main thread (the service's). */
    private fun setCamera(on: Boolean, expectedGen: Long? = null) {
        val m: CallMedia?
        val gen: Long
        synchronized(stateLock) {
            if (ending) return
            if (expectedGen != null && expectedGen != callGen) return
            gen = callGen
            if (on) {
                if (!hasCameraPermission()) {
                    cameraWanted = false
                    cameraGen++
                    _cameraOn.value = false
                    _notice.value = "Allow the camera to send video"
                    return
                }
                m = media
                if (m == null) {
                    // Not connected yet: connecting starts the camera (cameraWanted).
                    cameraWanted = true
                    cameraGen++
                    _cameraOn.value = true
                    return
                }
                cameraWanted = true
                cameraGen++
            } else {
                cameraWanted = false
                cameraGen++
                m = media
            }
        }
        if (on) {
            if (m == null || !startCamera(m, gen)) return
        } else {
            runCatching { m?.setCameraOn(false) }
            // Back to a microphone-only service; Android 14 wants the type to match what is running.
            runCatching { startInForeground(false) }
        }
        val stillActive = synchronized(stateLock) { !ending && gen == callGen }
        if (!stillActive) return
        _cameraOn.value = on
        scope.launch { runCatching { api.setCamera(on) } }
    }

    private fun startCamera(m: CallMedia, gen: Long): Boolean {
        try {
            // The camera type goes on before the camera opens, and only while we are allowed to use it.
            startInForeground(true)
            m.setCameraOn(true)
        } catch (e: Exception) {
            runCatching { m.setCameraOn(false) }
            runCatching { startInForeground(false) }
            synchronized(stateLock) {
                if (gen == callGen) {
                    cameraWanted = false
                    cameraGen++
                }
            }
            _cameraOn.value = false
            _notice.value = "Couldn't start the camera: ${e.message}"
            return false
        }
        _notice.value = null
        return true
    }

    private fun cameraFailed(message: String, expectedCallGen: Long, expectedCameraGen: Long) {
        val shouldHandle = synchronized(stateLock) {
            if (ending || expectedCallGen != callGen || expectedCameraGen != cameraGen || !_cameraOn.value) {
                false
            } else {
                cameraWanted = false
                cameraGen++
                true
            }
        }
        if (!shouldHandle) return
        _cameraOn.value = false
        _notice.value = message
        runCatching { startInForeground(false) }
        scope.launch { runCatching { api.setCamera(false) } }
    }

    private fun hangUp() {
        val gen = synchronized(stateLock) {
            if (ending) return
            callGen
        }
        scope.launch {
            // Even if the request fails the user wants out; the server ends the call on its own timeout.
            runCatching { api.hangUpCall() }
            finish("Call ended", expectedGen = gen)
        }
    }

    private fun finish(reason: String, expectedGen: Long? = null) {
        val eventsToCancel: Job?
        val muteSyncToCancel: Job?
        val mediaToStop: CallMedia?
        val wakeLockToRelease: PowerManager.WakeLock?
        synchronized(stateLock) {
            if (ending) return
            if (expectedGen != null && expectedGen != callGen) return
            ending = true
            callGen++
            cameraGen++
            cameraWanted = false
            eventsToCancel = events
            events = null
            muteSyncToCancel = muteSync
            muteSync = null
            mediaToStop = media
            media = null
            if (activeMedia === mediaToStop || activeInstance === this) activeMedia = null
            wakeLockToRelease = wakeLock
            wakeLock = null
            if (activeInstance === this) activeInstance = null
        }
        eventsToCancel?.cancel()
        muteSyncToCancel?.cancel()
        runCatching { mediaToStop?.stop() }
        releaseWakeLock(wakeLockToRelease)
        _cameraOn.value = false
        _state.value = CallState.Ended(reason)
        runCatching { ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE) }
        stopSelf()
    }

    /** (Re)declares the foreground service type: the microphone, plus the camera while it is on. */
    private fun startInForeground(camera: Boolean) {
        ensureChannel(this)
        var type = 0
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            type = ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
            if (camera) type = type or ServiceInfo.FOREGROUND_SERVICE_TYPE_CAMERA
        }
        ServiceCompat.startForeground(this, NOTIFICATION_ID, buildNotification(), type)
    }

    private fun buildNotification(): android.app.Notification {
        val currentId: String
        val currentName: String
        val currentAvatar: String?
        synchronized(stateLock) {
            currentId = callId
            currentName = name
            currentAvatar = avatar
        }
        val person = Person.Builder().setName(currentName.ifBlank { "Zalo call" }).setImportant(true).build()
        val open = PendingIntent.getActivity(
            this,
            0,
            CallActivity.intent(this, currentId, "", currentName, currentAvatar, video = _videoCall.value, answerNow = false),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val hangUp = PendingIntent.getService(
            this,
            1,
            Intent(this, CallService::class.java).setAction(ACTION_HANGUP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setStyle(NotificationCompat.CallStyle.forOngoingCall(person, hangUp))
            .setContentIntent(open)
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setOngoing(true)
            .setUsesChronometer(true)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .build()
    }

    override fun onDestroy() {
        // Normally finish() ran already; this covers the system killing the service.
        val eventsToCancel: Job?
        val muteSyncToCancel: Job?
        val mediaToStop: CallMedia?
        val wakeLockToRelease: PowerManager.WakeLock?
        val wasEnding: Boolean
        synchronized(stateLock) {
            wasEnding = ending
            ending = true
            callGen++
            cameraGen++
            cameraWanted = false
            eventsToCancel = events
            events = null
            muteSyncToCancel = muteSync
            muteSync = null
            mediaToStop = media
            media = null
            if (activeMedia === mediaToStop || activeInstance === this) activeMedia = null
            wakeLockToRelease = wakeLock
            wakeLock = null
            if (activeInstance === this) activeInstance = null
        }
        eventsToCancel?.cancel()
        muteSyncToCancel?.cancel()
        runCatching { mediaToStop?.stop() }
        releaseWakeLock(wakeLockToRelease)
        scope.cancel()
        _cameraOn.value = false
        if (!wasEnding && _state.value !is CallState.Idle) _state.value = CallState.Ended("Call ended")
        super.onDestroy()
    }

    companion object {
        const val ACTION_START = "com.zalom3ui.app.action.CALL_START"
        const val ACTION_HANGUP = "com.zalom3ui.app.action.CALL_HANGUP"
        /** Stops the service without telling the server (the answer request itself failed). */
        const val ACTION_STOP = "com.zalom3ui.app.action.CALL_STOP"
        const val EXTRA_CALL_ID = "callId"
        /** The person to call (a call we place). */
        const val EXTRA_PEER_ID = "peerId"
        /** Stands in for the call id until the server has made the call. */
        private const val PENDING_CALL_ID = "outgoing"
        const val EXTRA_NAME = "callerName"
        const val EXTRA_AVATAR = "avatar"
        const val EXTRA_VIDEO = "video"
        const val EXTRA_CAMERA = "camera"
        const val EXTRA_ON = "on"
        private const val EXTRA_REASON = "reason"
        const val ACTION_CAMERA = "com.zalom3ui.app.action.CALL_CAMERA"

        const val CHANNEL_ID = "call_ongoing"
        private const val NOTIFICATION_ID = 201
        private const val RECONNECT_MS = 2_000L
        private const val WAKE_LOCK_TIMEOUT_MS = 2 * 60 * 60 * 1_000L

        /** Shared lock coordinating CallService lifecycle with companion surface/media helpers. */
        private val mediaLock = Any()

        private val _state = MutableStateFlow<CallState>(CallState.Idle)
        val state: StateFlow<CallState> = _state.asStateFlow()

        private val _muted = MutableStateFlow(false)
        val muted: StateFlow<Boolean> = _muted.asStateFlow()

        private val _speaker = MutableStateFlow(false)
        val speakerOn: StateFlow<Boolean> = _speaker.asStateFlow()

        /** The call is a video call (shows the video screen). */
        private val _videoCall = MutableStateFlow(false)
        val videoCall: StateFlow<Boolean> = _videoCall.asStateFlow()

        /** Our camera is on (or wanted, until the call connects). */
        private val _cameraOn = MutableStateFlow(false)
        val cameraOn: StateFlow<Boolean> = _cameraOn.asStateFlow()

        /** The other side's camera is on; starts as true for a video call, then follows the server's `peer` events. */
        private val _peerCamera = MutableStateFlow(false)
        val peerCameraOn: StateFlow<Boolean> = _peerCamera.asStateFlow()

        /** Size of the other side's picture once the first frame is decoded, for the aspect ratio. */
        private val _remoteSize = MutableStateFlow<Pair<Int, Int>?>(null)
        val remoteSize: StateFlow<Pair<Int, Int>?> = _remoteSize.asStateFlow()

        /** Something the screen should say about the camera ("Couldn't start the camera: ..."). */
        private val _notice = MutableStateFlow<String?>(null)
        val notice: StateFlow<String?> = _notice.asStateFlow()

        private var activeInstance: CallService? = null
        private var activeMedia: CallMedia? = null

        // The screen's surfaces outlive the media (they exist while connecting), so they are kept here.
        private var remoteSurface: Surface? = null
        private var previewSurface: Surface? = null

        /** The surface the other side's video is drawn on; null when it is destroyed. */
        fun setRemoteSurface(surface: Surface?) {
            synchronized(mediaLock) {
                remoteSurface = surface
                activeMedia?.setRemoteSurface(surface)
            }
        }

        /** The surface of our small self-view; null when it is destroyed. */
        fun setPreviewSurface(surface: Surface?) {
            synchronized(mediaLock) {
                previewSurface = surface
                activeMedia?.setPreviewSurface(surface)
            }
        }

        /**
         * Turns our camera on or off. The caller must hold the CAMERA permission and be visible when turning it on
         * (the service type changes to include the camera).
         */
        fun setCamera(context: Context, on: Boolean) {
            _cameraOn.value = on
            val instance = synchronized(mediaLock) { activeInstance }
            if (instance != null) {
                instance.main.post { instance.setCamera(on) }
                return
            }
            val intent = Intent(context, CallService::class.java).setAction(ACTION_CAMERA).putExtra(EXTRA_ON, on)
            runCatching { context.startService(intent) }
        }

        fun switchCamera() {
            synchronized(mediaLock) {
                activeMedia?.switchCamera()
            }
        }

        fun clearNotice() {
            _notice.value = null
        }

        /** What the screen says when a call we placed ends: the server's `act`. */
        fun endReason(act: String): String = when (act) {
            "declined" -> "Declined"
            "busy" -> "Busy"
            "no_answer" -> "No answer"
            "unreachable" -> "Couldn't reach them"
            "failed" -> "Call failed"
            else -> "Call ended"
        }

        /** The call screen is showing an incoming call. Does not overwrite a call already being answered or placed. */
        fun showRinging(callId: String, name: String, avatar: String?) {
            val s = _state.value
            if (s is CallState.Calling) return
            val sameCall = (s is CallState.Connecting && s.callId == callId) || (s is CallState.Connected && s.callId == callId)
            if (!sameCall) _state.value = CallState.Ringing(callId, name, avatar)
        }

        /** Back to nothing, once the screen has shown "Call ended". */
        fun reset() {
            val s = _state.value
            if (s is CallState.Ended || s is CallState.Ringing) _state.value = CallState.Idle
        }

        /**
         * Starts the foreground service for a call being answered. Needs RECORD_AUDIO and the app in the foreground.
         * [video]: a video call (speaker on, video screen); [camera]: also send our camera (CAMERA already granted).
         */
        fun start(
            context: Context,
            callId: String,
            name: String,
            avatar: String? = null,
            video: Boolean = false,
            camera: Boolean = false,
        ) {
            _muted.value = false
            _speaker.value = video
            _videoCall.value = video
            _cameraOn.value = camera
            _peerCamera.value = video
            _remoteSize.value = null
            _notice.value = null
            _state.value = CallState.Connecting(callId, name, avatar)
            val intent = Intent(context, CallService::class.java)
                .setAction(ACTION_START)
                .putExtra(EXTRA_CALL_ID, callId)
                .putExtra(EXTRA_NAME, name)
                .putExtra(EXTRA_AVATAR, avatar)
                .putExtra(EXTRA_VIDEO, video)
                .putExtra(EXTRA_CAMERA, camera)
            try {
                ContextCompat.startForegroundService(context, intent)
            } catch (e: Exception) {
                _cameraOn.value = false
                _state.value = CallState.Ended(e.message ?: "Couldn't keep the call running")
                throw e
            }
        }

        /**
         * Places a call to [peerId]: starts the foreground service, which asks the server to call and follows it.
         * Needs RECORD_AUDIO and the app in the foreground. [camera]: a video call that starts with our camera on
         * (CAMERA already granted), once the other side picks up.
         */
        fun startOutgoing(
            context: Context,
            peerId: String,
            name: String,
            avatar: String? = null,
            video: Boolean = false,
            camera: Boolean = false,
        ) {
            _muted.value = false
            _speaker.value = video
            _videoCall.value = video
            _cameraOn.value = camera
            _peerCamera.value = video
            _remoteSize.value = null
            _notice.value = null
            _state.value = CallState.Calling(PENDING_CALL_ID, name, avatar)
            val intent = Intent(context, CallService::class.java)
                .setAction(ACTION_START)
                .putExtra(EXTRA_PEER_ID, peerId)
                .putExtra(EXTRA_NAME, name)
                .putExtra(EXTRA_AVATAR, avatar)
                .putExtra(EXTRA_VIDEO, video)
                .putExtra(EXTRA_CAMERA, camera)
            try {
                ContextCompat.startForegroundService(context, intent)
            } catch (e: Exception) {
                _cameraOn.value = false
                _state.value = CallState.Ended(e.message ?: "Couldn't place the call")
                throw e
            }
        }

        /** Ends the call in progress: hangs up when connected, cancels while it rings; tells the server, then stops the service. */
        fun hangUp(context: Context) {
            val instance = synchronized(mediaLock) { activeInstance }
            if (instance != null) {
                instance.main.post { instance.hangUp() }
                return
            }
            runCatching { context.startService(Intent(context, CallService::class.java).setAction(ACTION_HANGUP)) }
        }

        /** Drops the service without a hang-up request, for when answering failed. */
        fun stop(context: Context, reason: String = "Call ended") {
            val instance = synchronized(mediaLock) { activeInstance }
            if (instance != null) {
                instance.main.post { instance.finish(reason) }
                return
            }
            val intent = Intent(context, CallService::class.java)
                .setAction(ACTION_STOP)
                .putExtra(EXTRA_REASON, reason)
            val started = runCatching { context.startService(intent) }.isSuccess
            if (!started) {
                runCatching { context.stopService(Intent(context, CallService::class.java)) }
                _cameraOn.value = false
                _state.value = CallState.Ended(reason)
            }
        }

        fun setMuted(on: Boolean) {
            synchronized(mediaLock) {
                _muted.value = on
                activeMedia?.muted = on
            }
        }

        fun setSpeaker(on: Boolean) {
            synchronized(mediaLock) {
                _speaker.value = on
                activeMedia?.speakerOn = on
            }
        }

        private fun ensureChannel(context: Context) {
            context.getSystemService(NotificationManager::class.java).createNotificationChannel(
                NotificationChannel(CHANNEL_ID, "Ongoing call", NotificationManager.IMPORTANCE_LOW).apply {
                    description = "Shown while a call is connected"
                },
            )
        }
    }
}
