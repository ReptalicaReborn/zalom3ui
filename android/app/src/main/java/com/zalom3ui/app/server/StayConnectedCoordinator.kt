package com.zalom3ui.app.server

import android.app.Application
import android.content.ComponentName
import android.content.Context
import android.content.pm.PackageManager
import android.os.PowerManager
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.ProcessLifecycleOwner
import com.zalom3ui.app.call.CallService
import com.zalom3ui.app.data.ApiClient
import com.zalom3ui.app.data.InFlightWork
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.data.StayConnected
import com.zalom3ui.app.data.StaySettings
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Settings → "Stay connected", carried out. Owns when the chat listener of the on-phone core is connected in the two
 * "While open" modes; "Always" leaves everything to [ZaloService] and this object does nothing.
 *
 * - **While open**: [ProcessLifecycleOwner] says when the app comes to the foreground (core started, `listener/resume`)
 *   and leaves it (a grace period, then `listener/pause`). Pausing waits while a call is active, ringing or connecting,
 *   while [CallService] runs, or while an upload is in flight ([BusyCheck]).
 * - **While open + notification bridge**: additionally, a Zalo message notification ([ZaloNotificationListener]) brings
 *   the core online for a minute without any activity: `ensureStarted`, notify window, `listener/resume`; the replay
 *   arrives, [MessageNotifier] posts Zalỏ's own notifications for it, and after [ListenerGate.BRIDGE_IDLE_MS] without a
 *   new Zalo notification the listener is paused again.
 *
 * What the bridge relies on (Android 12+ restricts starting foreground services from the background, and the bridge
 * never does): the work runs in coroutines inside the process of the bound [ZaloNotificationListener], which the system
 * binds for as long as notification access is granted and which is allowed to run when it delivers a notification.
 * No foreground service, activity or alarm is started. A partial wake lock with a timeout keeps the CPU awake for the
 * window. Whether the network is usable in Doze at that moment is up to the system and only a real device can tell.
 *
 * All the decisions are in [ListenerGate] (plain Kotlin, unit tested); this class feeds it and applies them.
 */
object StayConnectedCoordinator {
    private const val WAKE_LOCK_TAG = "zaloo:bridge"
    private const val RESUME_RETRY_MS = 5_000L

    /** Time for the notifier to subscribe to the core's events before the listener connects and the replay arrives. */
    private const val SUBSCRIBE_SETTLE_MS = 400L

    /** The notify window outlasts the bridge's online time by this much, so a late replay still counts. */
    private const val NOTIFY_WINDOW_SLACK_MS = 30_000L

    private val lock = Any()
    private val gate = ListenerGate()
    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.IO)
    private var appContext: Context? = null
    private var mode = StayConnected.ALWAYS

    /** The on-phone core is used and the mode pauses it when the app is closed. */
    private var enabled = false
    private var evalJob: Job? = null
    private var notifierScope: CoroutineScope? = null
    private var wakeLock: PowerManager.WakeLock? = null

    private val lifecycleObserver = LifecycleEventObserver { _, event ->
        when (event) {
            Lifecycle.Event.ON_START -> onForeground()
            Lifecycle.Event.ON_STOP -> onBackground()
            else -> Unit
        }
    }

    /** Call once from `Application.onCreate` (the main thread). */
    fun init(app: Application) {
        appContext = app
        refreshFromSettings(app)
        ProcessLifecycleOwner.get().lifecycle.addObserver(lifecycleObserver)
    }

    /** Saves [stay] and switches to it now: service, listener component, pause or resume. */
    fun applyMode(context: Context, stay: StaySettings) {
        val app = context.applicationContext
        SettingsStore(app).saveStay(stay)
        refreshFromSettings(app)
        if (stay.mode.keepsForegroundService) {
            // Back to today's behavior: the service runs, and the listener is connected.
            LocalServer.start(app)
            scope.launch {
                if (ZaloCore.isStarted) {
                    runCatching {
                        val api = ApiClient.onDevice(app)
                        api.clearNotifyWindow()
                        api.listenerResume()
                    }
                }
            }
        } else if (enabled) {
            // No service in these modes. The process stays; the core keeps running and is paused when the app is closed.
            LocalServer.stop(app)
            StartupLog.write(app, "stay: mode ${stay.mode.key}")
        }
    }

    /** The "Hide Zalo's notifications" switch changed (read when needed; nothing to restart). */
    fun saveHideZaloNotifications(context: Context, hide: Boolean) {
        val store = SettingsStore(context.applicationContext)
        store.saveStay(store.loadStay().copy(hideZaloNotifications = hide))
    }

    /**
     * The "Codes from Zalo's notifications" switch changed: saves it and enables or disables the notification listener
     * component (on when the bridge mode or this switch wants it). Never touches the core or the listener connection.
     */
    fun saveOtpFromZaloNotifications(context: Context, on: Boolean) {
        val app = context.applicationContext
        val store = SettingsStore(app)
        store.saveStay(store.loadStay().copy(otpFromZaloNotifications = on))
        refreshFromSettings(app)
        if (on) ZaloNotificationListener.requestRebind(app)
    }

    /** Reads the saved settings again (the app switched between the phone and a server, or the mode changed). */
    fun refreshFromSettings(context: Context) {
        val app = context.applicationContext
        val store = SettingsStore(app)
        val onDevice = store.load().onDevice
        val saved = store.loadStay()
        val newMode = if (onDevice) saved.mode else StayConnected.ALWAYS
        synchronized(lock) {
            mode = newMode
            enabled = onDevice && newMode.pausesWhenClosed
        }
        // One component, two independent jobs: the bridge (this mode) and the code rescue (its own switch).
        setListenerComponentEnabled(app, ListenerJobs.componentEnabled(onDevice, saved.mode, saved.otpFromZaloNotifications))
        if (enabled) {
            synchronized(lock) {
                // The core, if running, is connected; foreground or not decides what happens next.
                gate.markConnected()
                if (appIsVisible()) gate.onForeground() else gate.onBackground(System.currentTimeMillis())
            }
            schedule()
        } else {
            cancelEverything()
        }
    }

    // ---- Events ----

    private fun onForeground() {
        if (!enabled) return
        synchronized(lock) { gate.onForeground() }
        schedule()
    }

    private fun onBackground() {
        if (!enabled) return
        synchronized(lock) { gate.onBackground(System.currentTimeMillis()) }
        schedule()
    }

    /** [ZaloCore] started the core in this process. */
    fun onCoreStarted() {
        if (!enabled) return
        synchronized(lock) { gate.onCoreStarted(System.currentTimeMillis()) }
        // The loop that started the core (resume) carries on by itself; otherwise start one for the grace period.
        if (evalJob?.isActive != true) schedule()
    }

    /** The official Zalo app posted a message notification ([ZaloNotificationListener]). */
    fun onZaloMessage(context: Context, postTime: Long) {
        if (!enabled || !mode.usesBridge) return
        val app = context.applicationContext
        val now = System.currentTimeMillis()
        synchronized(lock) { gate.onBridgeActivity(now) }
        holdWakeLock(app)
        scope.launch {
            if (!ZaloCore.ensureStarted(app)) return@launch
            // Before the listener connects: the replay is the news. A burst extends the same window.
            val trigger = if (postTime in 1..(now + 60_000L)) postTime else now
            runCatching {
                ApiClient.onDevice(app).setNotifyWindow(trigger, ListenerGate.BRIDGE_IDLE_MS + NOTIFY_WINDOW_SLACK_MS)
            }
            schedule()
        }
    }

    // ---- The loop that carries out the gate's decisions ----

    private fun schedule() {
        synchronized(lock) {
            evalJob?.cancel()
            evalJob = scope.launch { runLoop() }
        }
    }

    private suspend fun runLoop() {
        while (currentCoroutineContext().isActive) {
            val app = appContext ?: return
            val busy = isBusy(app)
            val decision = synchronized(lock) { gate.evaluate(System.currentTimeMillis(), busy) }
            when (decision.action) {
                ListenerGate.Action.RESUME -> if (!resume(app)) {
                    delay(RESUME_RETRY_MS)
                    continue
                }
                ListenerGate.Action.PAUSE -> if (!pause(app)) {
                    delay(RESUME_RETRY_MS)
                    continue
                }
                ListenerGate.Action.NONE -> Unit
            }
            val wantsOnline = synchronized(lock) { gate.wantsOnline(System.currentTimeMillis()) }
            if (wantsOnline) startNotifier(app) else stopNotifier()
            delay(decision.recheckInMs ?: return)
        }
    }

    private suspend fun resume(app: Context): Boolean {
        if (!ZaloCore.ensureStarted(app)) return false
        // Subscribe to the core's events first, so the replayed messages are seen.
        if (startNotifier(app)) delay(SUBSCRIBE_SETTLE_MS)
        return try {
            ApiClient.onDevice(app).listenerResume()
            synchronized(lock) { gate.applied(ListenerGate.Action.RESUME) }
            StartupLog.write(app, "stay: listener resumed")
            true
        } catch (e: Exception) {
            StartupLog.write(app, "stay: resume failed: ${e.message}")
            false
        }
    }

    private suspend fun pause(app: Context): Boolean {
        stopNotifier()
        if (ZaloCore.isStarted) {
            try {
                val api = ApiClient.onDevice(app)
                runCatching { api.clearNotifyWindow() }
                api.listenerPause()
            } catch (e: Exception) {
                StartupLog.write(app, "stay: pause failed: ${e.message}")
                return false
            }
        }
        synchronized(lock) { gate.applied(ListenerGate.Action.PAUSE) }
        releaseWakeLock()
        StartupLog.write(app, "stay: listener paused")
        return true
    }

    private suspend fun isBusy(app: Context): Boolean {
        val serverHasCall = ZaloCore.isStarted && runCatching { ApiClient.onDevice(app).currentCall() != null }.getOrDefault(false)
        return BusyCheck.isBusy(CallService.state.value, CallService.isRunning, InFlightWork.active, serverHasCall)
    }

    // ---- Notifications while online ----

    /** Starts the message/call notifier for the time Zalỏ is online. False if it was already running. */
    private fun startNotifier(app: Context): Boolean = synchronized(lock) {
        if (notifierScope?.isActive == true) return@synchronized false
        val s = CoroutineScope(SupervisorJob() + Dispatchers.IO)
        notifierScope = s
        MessageNotifier(app, onPosted = { _, chatName -> hideZaloNotificationFor(app, chatName) }).start(s)
        true
    }

    private fun stopNotifier() {
        synchronized(lock) {
            notifierScope?.cancel()
            notifierScope = null
        }
    }

    /** Zalỏ posted its notification for [chatName]: take Zalo's own for the same chat away, if that is wanted. */
    private fun hideZaloNotificationFor(app: Context, chatName: String) {
        if (!mode.usesBridge || !SettingsStore(app).loadStay().hideZaloNotifications) return
        ZaloNotificationListener.hideZaloNotificationsFor(chatName)
    }

    // ---- Housekeeping ----

    private fun cancelEverything() {
        synchronized(lock) {
            evalJob?.cancel()
            evalJob = null
            gate.reset()
        }
        stopNotifier()
        releaseWakeLock()
    }

    private fun appIsVisible(): Boolean =
        ProcessLifecycleOwner.get().lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)

    private fun setListenerComponentEnabled(app: Context, enable: Boolean) {
        runCatching {
            app.packageManager.setComponentEnabledSetting(
                ComponentName(app, ZaloNotificationListener::class.java),
                if (enable) PackageManager.COMPONENT_ENABLED_STATE_ENABLED else PackageManager.COMPONENT_ENABLED_STATE_DISABLED,
                PackageManager.DONT_KILL_APP,
            )
        }
    }

    private fun holdWakeLock(app: Context) {
        synchronized(lock) {
            runCatching {
                val wl = wakeLock ?: app.getSystemService(PowerManager::class.java)
                    .newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, WAKE_LOCK_TAG)
                    .apply { setReferenceCounted(false) }
                    .also { wakeLock = it }
                // Times out by itself, so it can never be left held.
                wl.acquire(ListenerGate.BRIDGE_IDLE_MS + NOTIFY_WINDOW_SLACK_MS)
            }
        }
    }

    private fun releaseWakeLock() {
        synchronized(lock) {
            runCatching { wakeLock?.takeIf { it.isHeld }?.release() }
        }
    }
}
