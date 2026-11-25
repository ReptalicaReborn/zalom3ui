package com.zalom3ui.app.call

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import androidx.core.app.Person
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import com.zalom3ui.app.R
import com.zalom3ui.app.data.ApiClient
import com.zalom3ui.app.data.ServerEvent
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.server.LocalServer
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
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/** What the call screen shows. Written by [CallActivity] (ringing) and [CallService] (the rest). */
sealed interface CallState {
    data object Idle : CallState

    data class Ringing(val callId: String, val name: String, val avatar: String? = null) : CallState

    /** Answered on our side; waiting for the server to say the media is up. */
    data class Connecting(val callId: String, val name: String, val avatar: String? = null) : CallState

    /** [since] is epoch millis, for the running timer. */
    data class Connected(val callId: String, val name: String, val since: Long, val avatar: String? = null) : CallState

    data class Ended(val reason: String) : CallState
}

/**
 * Keeps an answered call alive in the background (microphone foreground service, main process): shows the ongoing-call
 * notification, follows the call through the server's events, and owns the [CallMedia] while connected.
 */
class CallService : Service() {
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var events: Job? = null
    private var media: CallMedia? = null
    private var callId = ""
    private var name = ""
    private var avatar: String? = null
    private var ending = false

    private val api: ApiClient get() = ApiClient(LocalServer.url(this), SettingsStore(this).localToken())

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_START -> begin(intent)
            ACTION_HANGUP -> hangUp()
            ACTION_STOP -> finish("Call ended")
            else -> if (callId.isEmpty()) stopSelf()
        }
        return START_NOT_STICKY
    }

    private fun begin(intent: Intent) {
        val newId = intent.getStringExtra(EXTRA_CALL_ID).orEmpty()
        if (newId.isEmpty()) {
            stopSelf()
            return
        }
        callId = newId
        name = intent.getStringExtra(EXTRA_NAME).orEmpty()
        avatar = intent.getStringExtra(EXTRA_AVATAR)
        ending = false
        _state.value = CallState.Connecting(callId, name, avatar)
        try {
            startInForeground()
        } catch (e: Exception) {
            // Android refused a microphone service (permission revoked, app not visible).
            finish("Couldn't keep the call running: ${e.message}")
            return
        }
        events?.cancel()
        events = scope.launch { follow() }
    }

    private suspend fun follow() {
        // The call may already be connected (service restarted, or the event came before we listened).
        runCatching { api.currentCall() }.getOrNull()?.let {
            if (it.callId == callId && it.state == "connected") onConnected(it.connectedAt)
        }
        while (scope.isActive) {
            try {
                api.events().collect { event ->
                    when (event) {
                        is ServerEvent.CallConnected -> if (event.callId == callId) onConnected(event.connectedAt)
                        is ServerEvent.CallEnded -> if (event.callId.isEmpty() || event.callId == callId) finish("Call ended")
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

    private fun onConnected(connectedAt: Long) {
        if (ending || media != null) return
        val since = if (connectedAt > 0) connectedAt else System.currentTimeMillis()
        val m = CallMediaFactory.create(applicationContext)
        m.muted = _muted.value
        m.speakerOn = _speaker.value
        try {
            m.start()
        } catch (e: Exception) {
            m.stop()
            // Hang up too: otherwise the other side stays in a silent call until Zalo times it out.
            scope.launch {
                runCatching { api.hangUpCall() }
                finish("Couldn't start call audio: ${e.message}")
            }
            return
        }
        media = m
        activeMedia = m
        _state.value = CallState.Connected(callId, name, since, avatar)
    }

    private fun hangUp() {
        if (ending) return
        scope.launch {
            // Even if the request fails the user wants out; the server ends the call on its own timeout.
            runCatching { api.hangUpCall() }
            finish("Call ended")
        }
    }

    @Synchronized
    private fun finish(reason: String) {
        if (ending) return
        ending = true
        events?.cancel()
        media?.stop()
        media = null
        activeMedia = null
        _state.value = CallState.Ended(reason)
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun startInForeground() {
        ensureChannel(this)
        val type = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else 0
        ServiceCompat.startForeground(this, NOTIFICATION_ID, buildNotification(), type)
    }

    private fun buildNotification(): android.app.Notification {
        val person = Person.Builder().setName(name.ifBlank { "Zalo call" }).setImportant(true).build()
        val open = PendingIntent.getActivity(
            this,
            0,
            CallActivity.intent(this, callId, "", name, avatar, video = false, answerNow = false),
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
        media?.stop()
        media = null
        scope.cancel()
        if (!ending && _state.value !is CallState.Idle) _state.value = CallState.Ended("Call ended")
        activeMedia = null
        super.onDestroy()
    }

    companion object {
        const val ACTION_START = "com.zalom3ui.app.action.CALL_START"
        const val ACTION_HANGUP = "com.zalom3ui.app.action.CALL_HANGUP"
        /** Stops the service without telling the server (the answer request itself failed). */
        const val ACTION_STOP = "com.zalom3ui.app.action.CALL_STOP"
        const val EXTRA_CALL_ID = "callId"
        const val EXTRA_NAME = "callerName"
        const val EXTRA_AVATAR = "avatar"

        const val CHANNEL_ID = "call_ongoing"
        private const val NOTIFICATION_ID = 201
        private const val RECONNECT_MS = 2_000L

        private val _state = MutableStateFlow<CallState>(CallState.Idle)
        val state: StateFlow<CallState> = _state.asStateFlow()

        private val _muted = MutableStateFlow(false)
        val muted: StateFlow<Boolean> = _muted.asStateFlow()

        private val _speaker = MutableStateFlow(false)
        val speakerOn: StateFlow<Boolean> = _speaker.asStateFlow()

        @Volatile
        private var activeMedia: CallMedia? = null

        /** The call screen is showing an incoming call. Does not overwrite a call already being answered. */
        fun showRinging(callId: String, name: String, avatar: String?) {
            val s = _state.value
            val sameCall = (s is CallState.Connecting && s.callId == callId) || (s is CallState.Connected && s.callId == callId)
            if (!sameCall) _state.value = CallState.Ringing(callId, name, avatar)
        }

        /** Back to nothing, once the screen has shown "Call ended". */
        fun reset() {
            val s = _state.value
            if (s is CallState.Ended || s is CallState.Ringing) _state.value = CallState.Idle
        }

        /** Starts the foreground service for a call being answered. Needs RECORD_AUDIO and the app in the foreground. */
        fun start(context: Context, callId: String, name: String, avatar: String? = null) {
            _muted.value = false
            _speaker.value = false
            _state.value = CallState.Connecting(callId, name, avatar)
            val intent = Intent(context, CallService::class.java)
                .setAction(ACTION_START)
                .putExtra(EXTRA_CALL_ID, callId)
                .putExtra(EXTRA_NAME, name)
                .putExtra(EXTRA_AVATAR, avatar)
            ContextCompat.startForegroundService(context, intent)
        }

        /** Ends the connected call: tells the server, then stops the service. */
        fun hangUp(context: Context) {
            context.startService(Intent(context, CallService::class.java).setAction(ACTION_HANGUP))
        }

        /** Drops the service without a hang-up request, for when answering failed. */
        fun stop(context: Context) {
            context.startService(Intent(context, CallService::class.java).setAction(ACTION_STOP))
        }

        fun setMuted(on: Boolean) {
            _muted.value = on
            activeMedia?.muted = on
        }

        fun setSpeaker(on: Boolean) {
            _speaker.value = on
            activeMedia?.speakerOn = on
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
