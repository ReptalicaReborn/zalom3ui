package com.zalom3ui.app.security

import com.zalom3ui.app.data.AppLockTimeout
import com.zalom3ui.app.data.ThreadRef
import com.zalom3ui.app.server.ConversationShortcuts
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class AppLockTest {
    @Test
    fun disabled_never_locks() {
        assertFalse(AppLockPolicy.shouldLock(enabled = false, lastBackgroundMs = 0L, nowMs = 1_000L, timeoutMs = 0L))
        assertFalse(AppLockPolicy.shouldLock(enabled = false, lastBackgroundMs = 1_000L, nowMs = 999_999L, timeoutMs = 60_000L))
        assertFalse(AppLockPolicy.shouldLock(enabled = false, lastBackgroundMs = 5_000L, nowMs = 1_000L, timeoutMs = 60_000L))
    }

    @Test
    fun cold_start_locks_when_enabled() {
        assertTrue(AppLockPolicy.shouldLock(enabled = true, lastBackgroundMs = 0L, nowMs = 10_000L, timeoutMs = 60_000L))
        assertTrue(AppLockPolicy.shouldLock(enabled = true, lastBackgroundMs = -1L, nowMs = 10_000L, timeoutMs = 1_800_000L))
    }

    @Test
    fun immediate_timeout_locks_without_delay() {
        assertTrue(AppLockPolicy.shouldLock(enabled = true, lastBackgroundMs = 10_000L, nowMs = 10_000L, timeoutMs = 0L))
        assertTrue(AppLockPolicy.shouldLock(enabled = true, lastBackgroundMs = 10_000L, nowMs = 10_001L, timeoutMs = 0L))
        assertTrue(AppLockPolicy.shouldLock(enabled = true, lastBackgroundMs = 10_000L, nowMs = 10_000L, timeoutMs = -500L))
    }

    @Test
    fun timeout_threshold_boundary() {
        val bg = 1_000_000L
        val timeout = AppLockTimeout.ONE_MIN.timeoutMs
        assertFalse(AppLockPolicy.shouldLock(enabled = true, lastBackgroundMs = bg, nowMs = bg, timeoutMs = timeout))
        assertFalse(AppLockPolicy.shouldLock(enabled = true, lastBackgroundMs = bg, nowMs = bg + timeout - 1L, timeoutMs = timeout))
        assertTrue(AppLockPolicy.shouldLock(enabled = true, lastBackgroundMs = bg, nowMs = bg + timeout, timeoutMs = timeout))
        assertTrue(AppLockPolicy.shouldLock(enabled = true, lastBackgroundMs = bg, nowMs = bg + timeout + 5_000L, timeoutMs = timeout))
    }

    @Test
    fun backward_clock_fails_closed() {
        assertTrue(
            AppLockPolicy.shouldLock(
                enabled = true,
                lastBackgroundMs = 100_000L,
                nowMs = 99_999L,
                timeoutMs = AppLockTimeout.THIRTY_MIN.timeoutMs,
            ),
        )
    }

    @Test
    fun app_lock_timeout_from_key_round_trips_and_defaults_to_one_min() {
        assertEquals(0L, AppLockTimeout.IMMEDIATE.timeoutMs)
        assertEquals(60_000L, AppLockTimeout.ONE_MIN.timeoutMs)
        assertEquals(300_000L, AppLockTimeout.FIVE_MIN.timeoutMs)
        assertEquals(1_800_000L, AppLockTimeout.THIRTY_MIN.timeoutMs)

        for (timeout in AppLockTimeout.entries) {
            assertEquals(timeout, AppLockTimeout.fromKey(timeout.key))
        }
        assertEquals(AppLockTimeout.entries.size, AppLockTimeout.entries.map { it.key }.toSet().size)

        assertEquals(AppLockTimeout.ONE_MIN, AppLockTimeout.fromKey(null))
        assertEquals(AppLockTimeout.ONE_MIN, AppLockTimeout.fromKey(""))
        assertEquals(AppLockTimeout.ONE_MIN, AppLockTimeout.fromKey("unknown"))
        assertEquals(AppLockTimeout.ONE_MIN, AppLockTimeout.fromKey("120"))
    }

    @Test
    fun conversation_shortcut_id_round_trips() {
        val userRef = ThreadRef("user", "123456789")
        val groupRef = ThreadRef("group", "grp_987_abc")
        assertEquals("chat_user_123456789", ConversationShortcuts.shortcutId(userRef))
        assertEquals("chat_group_grp_987_abc", ConversationShortcuts.shortcutId(groupRef))
        assertEquals(userRef, ConversationShortcuts.parseShortcutId(ConversationShortcuts.shortcutId(userRef)))
        assertEquals(groupRef, ConversationShortcuts.parseShortcutId(ConversationShortcuts.shortcutId(groupRef)))

        assertNull(ConversationShortcuts.parseShortcutId(null))
        assertNull(ConversationShortcuts.parseShortcutId(""))
        assertNull(ConversationShortcuts.parseShortcutId("other_user_123"))
        assertNull(ConversationShortcuts.parseShortcutId("chat_user_"))
        assertNull(ConversationShortcuts.parseShortcutId("chat_channel_123"))
    }
}
