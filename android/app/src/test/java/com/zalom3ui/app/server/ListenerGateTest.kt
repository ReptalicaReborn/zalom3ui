package com.zalom3ui.app.server

import com.zalom3ui.app.call.CallState
import com.zalom3ui.app.server.ListenerGate.Action
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class ListenerGateTest {
    private val grace = ListenerGate.FOREGROUND_GRACE_MS
    private val bridge = ListenerGate.BRIDGE_IDLE_MS

    private fun ListenerGate.decide(now: Long, busy: Boolean = false) = evaluate(now, busy)

    @Test
    fun grace_and_bridge_windows_are_a_minute() {
        assertEquals(60_000L, grace)
        assertEquals(60_000L, bridge)
    }

    @Test
    fun nothing_happens_before_anything_wants_online() {
        val gate = ListenerGate()
        assertEquals(Action.NONE, gate.decide(0).action)
        assertNull(gate.decide(0).recheckInMs)
    }

    @Test
    fun coming_to_the_foreground_resumes_once() {
        val gate = ListenerGate()
        gate.onForeground()
        val first = gate.decide(1_000)
        assertEquals(Action.RESUME, first.action)
        assertNull("stays online until the app leaves the foreground", first.recheckInMs)
        gate.applied(Action.RESUME)
        assertEquals(Action.NONE, gate.decide(2_000).action)
    }

    @Test
    fun a_failed_resume_is_asked_again() {
        val gate = ListenerGate()
        gate.onForeground()
        assertEquals(Action.RESUME, gate.decide(0).action)
        // The call failed, so it was never applied.
        assertEquals(Action.RESUME, gate.decide(5_000).action)
    }

    @Test
    fun leaving_the_foreground_pauses_after_the_grace_period_not_before() {
        val gate = ListenerGate()
        gate.onForeground()
        gate.applied(gate.decide(0).action)
        gate.onBackground(10_000)

        val during = gate.decide(10_000 + grace - 1)
        assertEquals(Action.NONE, during.action)
        assertEquals(1L, during.recheckInMs)

        val after = gate.decide(10_000 + grace)
        assertEquals(Action.PAUSE, after.action)
        gate.applied(Action.PAUSE)
        assertFalse(gate.connected)
        assertEquals(Action.NONE, gate.decide(10_000 + grace + 1).action)
    }

    @Test
    fun coming_back_within_the_grace_period_cancels_the_pause() {
        val gate = ListenerGate()
        gate.onForeground()
        gate.applied(gate.decide(0).action)
        gate.onBackground(1_000)
        gate.onForeground()
        assertEquals(Action.NONE, gate.decide(1_000 + grace + 5_000).action)
        assertTrue(gate.connected)
    }

    @Test
    fun coming_back_after_a_pause_resumes() {
        val gate = ListenerGate()
        gate.onForeground()
        gate.applied(gate.decide(0).action)
        gate.onBackground(0)
        gate.applied(gate.decide(grace).action)
        gate.onForeground()
        assertEquals(Action.RESUME, gate.decide(grace + 10).action)
    }

    @Test
    fun never_pauses_while_busy_and_pauses_once_it_is_over() {
        val gate = ListenerGate()
        gate.onForeground()
        gate.applied(gate.decide(0).action)
        gate.onBackground(0)

        val busy = gate.decide(grace + 1_000, busy = true)
        assertEquals(Action.NONE, busy.action)
        assertEquals(ListenerGate.BUSY_RECHECK_MS, busy.recheckInMs)
        assertTrue(gate.connected)

        assertEquals(Action.NONE, gate.decide(grace + 10_000, busy = true).action)
        assertEquals(Action.PAUSE, gate.decide(grace + 20_000, busy = false).action)
    }

    @Test
    fun a_notification_bridge_burst_resumes_once_and_pauses_a_minute_after_the_last_one() {
        val gate = ListenerGate()
        gate.onBridgeActivity(0)
        val first = gate.decide(0)
        assertEquals(Action.RESUME, first.action)
        assertEquals(bridge, first.recheckInMs)
        gate.applied(Action.RESUME)

        // Two more Zalo notifications while online only extend the window.
        gate.onBridgeActivity(30_000)
        gate.onBridgeActivity(50_000)
        val mid = gate.decide(50_000)
        assertEquals(Action.NONE, mid.action)
        assertEquals(bridge, mid.recheckInMs)

        // The first minute is over, but the window was extended: still online.
        assertEquals(Action.NONE, gate.decide(70_000).action)
        // A minute after the last notification: pause, once.
        assertEquals(Action.NONE, gate.decide(50_000 + bridge - 1).action)
        assertEquals(Action.PAUSE, gate.decide(50_000 + bridge).action)
    }

    @Test
    fun an_earlier_bridge_time_never_shortens_the_window() {
        val gate = ListenerGate()
        gate.onBridgeActivity(100_000)
        gate.onBridgeActivity(10_000)
        assertTrue(gate.wantsOnline(100_000 + bridge - 1))
        assertFalse(gate.wantsOnline(100_000 + bridge))
    }

    @Test
    fun the_bridge_does_not_pause_an_open_app() {
        val gate = ListenerGate()
        gate.onForeground()
        gate.onBridgeActivity(0)
        gate.applied(gate.decide(0).action)
        assertEquals(Action.NONE, gate.decide(10 * bridge).action)
        // Leaving later: the grace period counts from leaving, not from the bridge.
        gate.onBackground(10 * bridge)
        assertEquals(Action.NONE, gate.decide(10 * bridge + grace - 1).action)
        assertEquals(Action.PAUSE, gate.decide(10 * bridge + grace).action)
    }

    @Test
    fun the_bridge_keeps_a_recently_closed_app_online_until_the_later_of_the_two() {
        val gate = ListenerGate()
        gate.onForeground()
        gate.applied(gate.decide(0).action)
        gate.onBackground(0)
        gate.onBridgeActivity(30_000) // online until 90 s, past the 60 s grace
        assertEquals(Action.NONE, gate.decide(grace + 1).action)
        assertEquals(30_000L + bridge - (grace + 1), gate.decide(grace + 1).recheckInMs)
        assertEquals(Action.PAUSE, gate.decide(30_000 + bridge).action)
    }

    @Test
    fun a_core_started_by_something_other_than_the_screen_is_paused_after_the_grace_period() {
        val gate = ListenerGate()
        gate.onCoreStarted(5_000)
        assertTrue(gate.connected)
        val soon = gate.decide(5_001)
        assertEquals(Action.NONE, soon.action)
        assertEquals(grace - 1, soon.recheckInMs)
        assertEquals(Action.PAUSE, gate.decide(5_000 + grace).action)
    }

    @Test
    fun a_core_started_while_the_app_is_open_stays_online() {
        val gate = ListenerGate()
        gate.onForeground()
        gate.onCoreStarted(1_000)
        assertEquals(Action.NONE, gate.decide(10 * grace).action)
    }

    @Test
    fun reset_forgets_everything() {
        val gate = ListenerGate()
        gate.onForeground()
        gate.onBridgeActivity(0)
        gate.onCoreStarted(0)
        gate.reset()
        assertFalse(gate.connected)
        assertFalse(gate.wantsOnline(0))
        assertEquals(Action.NONE, gate.decide(0).action)
    }

    @Test
    fun busy_means_a_call_a_call_service_an_upload_or_a_call_on_the_server() {
        val idle = CallState.Idle
        assertFalse(BusyCheck.isBusy(idle, callServiceRunning = false, uploadsInFlight = 0, serverHasCall = false))
        assertTrue(BusyCheck.isBusy(idle, callServiceRunning = true, uploadsInFlight = 0, serverHasCall = false))
        assertTrue(BusyCheck.isBusy(idle, callServiceRunning = false, uploadsInFlight = 1, serverHasCall = false))
        assertTrue(BusyCheck.isBusy(idle, callServiceRunning = false, uploadsInFlight = 0, serverHasCall = true))
        for (state in listOf(
            CallState.Ringing("c", "A"),
            CallState.Calling("c", "A"),
            CallState.Connecting("c", "A"),
            CallState.Connected("c", "A", since = 1L),
        )) {
            assertTrue("$state", BusyCheck.callInProgress(state))
        }
        assertFalse(BusyCheck.callInProgress(CallState.Idle))
        assertFalse(BusyCheck.callInProgress(CallState.Ended("done")))
    }
}
