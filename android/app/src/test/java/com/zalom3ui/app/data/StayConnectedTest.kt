package com.zalom3ui.app.data

import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class StayConnectedTest {
    @Test
    fun always_is_todays_behavior_and_the_default() {
        assertEquals(StayConnected.ALWAYS, StayConnected.NEW_INSTALL_DEFAULT)
        assertEquals(StayConnected.ALWAYS, StayConnected.UPGRADE_DEFAULT)
        assertEquals(StayConnected.ALWAYS, StayConnected.initialFor(isUpgrade = true))
        assertEquals(StayConnected.ALWAYS, StayConnected.initialFor(isUpgrade = false))
        assertEquals(StayConnected.ALWAYS, StaySettings().mode)
        assertTrue(StaySettings().hideZaloNotifications)
    }

    @Test
    fun only_always_keeps_the_foreground_service() {
        assertTrue(StayConnected.ALWAYS.keepsForegroundService)
        assertFalse(StayConnected.ALWAYS.pausesWhenClosed)
        assertFalse(StayConnected.ALWAYS.usesBridge)
        for (mode in listOf(StayConnected.WHILE_OPEN, StayConnected.WHILE_OPEN_BRIDGE)) {
            assertFalse(mode.keepsForegroundService)
            assertTrue(mode.pausesWhenClosed)
        }
        assertFalse(StayConnected.WHILE_OPEN.usesBridge)
        assertTrue(StayConnected.WHILE_OPEN_BRIDGE.usesBridge)
    }

    @Test
    fun keys_round_trip_and_unknown_keys_are_null() {
        for (mode in StayConnected.entries) assertEquals(mode, StayConnected.fromKey(mode.key))
        assertNull(StayConnected.fromKey("sometimes"))
        assertNull(StayConnected.fromKey(null))
        assertEquals(StayConnected.entries.size, StayConnected.entries.map { it.key }.toSet().size)
    }

    @Test
    fun the_descriptions_say_the_trade_offs_in_plain_words() {
        val whileOpen = StayConnected.WHILE_OPEN.summary
        assertTrue("no notifications or calls while closed", "No notifications or incoming calls while it is closed" in whileOpen)
        assertTrue("history arrives when opened", "catches up when you open it" in whileOpen)
        assertTrue("web stays usable", "Zalo Web or PC stays usable" in whileOpen)
        val bridge = StayConnected.WHILE_OPEN_BRIDGE.summary
        assertTrue("needs access", "notification access" in bridge)
        assertTrue("kicks web", "sign out Zalo Web or PC" in bridge)
    }

    @Test
    fun the_listener_pause_states_are_logged_in_and_not_an_error() {
        val paused = AppJson.decodeFromString(SessionStatus.serializer(), """{"state":"paused"}""")
        assertTrue(paused.isLoggedIn)
        assertTrue(paused.isPaused)
        assertNull(paused.error)
        assertTrue(AppJson.decodeFromString(SessionStatus.serializer(), """{"state":"ready"}""").isLoggedIn)
        assertTrue(AppJson.decodeFromString(SessionStatus.serializer(), """{"state":"disconnected"}""").isLoggedIn)
        assertFalse(AppJson.decodeFromString(SessionStatus.serializer(), """{"state":"logged_out"}""").isLoggedIn)
        // A state this app has never heard of must not crash, and is not "logged in".
        val unknown = AppJson.decodeFromString(SessionStatus.serializer(), """{"state":"hibernating","extra":1}""")
        assertFalse(unknown.isLoggedIn)
        assertFalse(unknown.isPaused)
    }

    @Test
    fun in_flight_work_is_counted_for_its_whole_duration_even_on_failure() = runBlocking {
        assertEquals(0, InFlightWork.active)
        var during = -1
        InFlightWork.track { during = InFlightWork.active }
        assertEquals(1, during)
        assertEquals(0, InFlightWork.active)

        runCatching { InFlightWork.track { error("upload failed") } }
        assertEquals(0, InFlightWork.active)

        InFlightWork.begin()
        InFlightWork.begin()
        assertEquals(2, InFlightWork.active)
        InFlightWork.end()
        InFlightWork.end()
        InFlightWork.end() // too many ends never go below zero
        assertEquals(0, InFlightWork.active)
    }
}
