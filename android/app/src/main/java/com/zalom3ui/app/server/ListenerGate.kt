package com.zalom3ui.app.server

import com.zalom3ui.app.call.CallState

/**
 * Decides when the chat listener should be connected in the "While open" modes. Plain Kotlin with the clock passed in, so
 * it is tested on the JVM; [StayConnectedCoordinator] feeds it the app's lifecycle and Zalo notifications and carries out
 * its decisions.
 *
 * Online is wanted while the app is in the foreground, for [graceMs] after it leaves the foreground (the user often comes
 * straight back), and for [bridgeIdleMs] after the latest Zalo notification (the notification bridge). Pausing is never
 * done while [evaluate] is told the app is busy (a call, an upload); it is postponed and checked again.
 */
class ListenerGate(
    private val graceMs: Long = FOREGROUND_GRACE_MS,
    private val bridgeIdleMs: Long = BRIDGE_IDLE_MS,
) {
    enum class Action { NONE, RESUME, PAUSE }

    /** What to do now, and when to ask again (null: only when something happens). */
    data class Decision(val action: Action, val recheckInMs: Long?)

    private var foreground = false
    private var leftAt: Long? = null
    private var bridgeUntil = Long.MIN_VALUE

    /** The listener may be connected: we resumed it, or the core was started (which connects on its own). */
    var connected = false
        private set

    fun onForeground() {
        foreground = true
        leftAt = null
    }

    fun onBackground(now: Long) {
        foreground = false
        leftAt = now
    }

    /** A Zalo message notification: online for [bridgeIdleMs] from now. A burst only extends the window. */
    fun onBridgeActivity(now: Long) {
        bridgeUntil = maxOf(bridgeUntil, now + bridgeIdleMs)
    }

    /**
     * The core started in this process (it connects by itself once logged in). If nothing wants it online, the grace
     * period starts now, so a start from a receiver or the bridge is paused again after a minute.
     */
    fun onCoreStarted(now: Long) {
        connected = true
        if (!foreground && leftAt == null && now >= bridgeUntil) leftAt = now
    }

    /** The core is considered connected (e.g. when switching modes while it runs). */
    fun markConnected() {
        connected = true
    }

    /** Forget everything (mode switched to "Always", or away from the on-phone core). */
    fun reset() {
        foreground = false
        leftAt = null
        bridgeUntil = Long.MIN_VALUE
        connected = false
    }

    fun wantsOnline(now: Long): Boolean =
        foreground || now < bridgeUntil || leftAt?.let { now < it + graceMs } == true

    /** Tell the gate a decision was carried out. A failed [Action.RESUME] is simply not applied, so it is asked again. */
    fun applied(action: Action) {
        when (action) {
            Action.RESUME -> connected = true
            Action.PAUSE -> connected = false
            Action.NONE -> Unit
        }
    }

    fun evaluate(now: Long, busy: Boolean): Decision {
        if (wantsOnline(now)) {
            return Decision(if (connected) Action.NONE else Action.RESUME, expiresIn(now))
        }
        if (!connected) return Decision(Action.NONE, null)
        // Never pause under a call or an upload: wait, and look again.
        return if (busy) Decision(Action.NONE, BUSY_RECHECK_MS) else Decision(Action.PAUSE, null)
    }

    /** Milliseconds until the current reason to be online runs out (null while in the foreground). */
    private fun expiresIn(now: Long): Long? {
        if (foreground) return null
        val ends = listOfNotNull(
            bridgeUntil.takeIf { it > now },
            leftAt?.let { it + graceMs }?.takeIf { it > now },
        )
        return ends.maxOrNull()?.minus(now)
    }

    companion object {
        /** How long the listener stays connected after the app leaves the foreground. */
        const val FOREGROUND_GRACE_MS = 60_000L

        /** How long the listener stays connected after the latest Zalo notification. */
        const val BRIDGE_IDLE_MS = 60_000L

        /** While busy, how often to see whether it is over. */
        const val BUSY_RECHECK_MS = 5_000L
    }
}

/** What counts as "busy" for [ListenerGate.evaluate]: pausing then waits. */
object BusyCheck {
    /** A call is ringing, being placed, connecting or connected. */
    fun callInProgress(state: CallState): Boolean = when (state) {
        is CallState.Ringing, is CallState.Calling, is CallState.Connecting, is CallState.Connected -> true
        else -> false
    }

    fun isBusy(
        callState: CallState,
        callServiceRunning: Boolean,
        uploadsInFlight: Int,
        serverHasCall: Boolean,
    ): Boolean = callInProgress(callState) || callServiceRunning || uploadsInFlight > 0 || serverHasCall
}
