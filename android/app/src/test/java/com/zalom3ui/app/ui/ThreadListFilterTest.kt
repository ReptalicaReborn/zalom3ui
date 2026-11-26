package com.zalom3ui.app.ui

import com.zalom3ui.app.data.ThreadSummary
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ThreadListFilterTest {
    private fun thread(
        id: String,
        kind: String = "user",
        name: String = "Chat $id",
        unread: Int = 0,
        pinned: Boolean = false,
        mutedUntil: Long? = null,
    ) = ThreadSummary(
        id = id,
        kind = kind,
        name = name,
        unread = unread,
        pinned = pinned,
        mutedUntil = mutedUntil,
    )

    private val sampleThreads = listOf(
        thread(id = "u-pin-read", kind = "user", unread = 0, pinned = true),
        thread(id = "g-pin-unread", kind = "group", unread = 3, pinned = true),
        thread(id = "u-muted-unread", kind = "user", unread = 5, mutedUntil = -1L),
        thread(id = "u-read", kind = "user", unread = 0),
        thread(id = "g-muted-read", kind = "group", unread = 0, mutedUntil = 1_700_000_000_000L),
        thread(id = "g-unread", kind = "group", unread = 1),
    )

    @Test
    fun filterChats_all_returnsAllPreservingOrder() {
        val result = filterChats(sampleThreads, ChatFilter.ALL)
        assertEquals(sampleThreads.map { it.id }, result.map { it.id })
    }

    @Test
    fun filterChats_unread_includesPinnedMutedAndGroupUnreadOnly() {
        val result = filterChats(sampleThreads, ChatFilter.UNREAD)
        assertEquals(
            listOf("g-pin-unread", "u-muted-unread", "g-unread"),
            result.map { it.id },
        )
    }

    @Test
    fun filterChats_groups_includesOnlyGroupThreadsRegardlessOfReadPinnedOrMuted() {
        val result = filterChats(sampleThreads, ChatFilter.GROUPS)
        assertEquals(
            listOf("g-pin-unread", "g-muted-read", "g-unread"),
            result.map { it.id },
        )
    }

    @Test
    fun filterChats_emptyList_returnsEmptyForAllFilters() {
        for (filter in ChatFilter.entries) {
            assertTrue(filterChats(emptyList(), filter).isEmpty())
        }
    }

    @Test
    fun filterChats_noMatches_returnsEmpty() {
        val readDirects = listOf(
            thread(id = "u1", kind = "user", unread = 0, pinned = true),
            thread(id = "u2", kind = "user", unread = 0, mutedUntil = -1L),
        )
        assertTrue(filterChats(readDirects, ChatFilter.UNREAD).isEmpty())
        assertTrue(filterChats(readDirects, ChatFilter.GROUPS).isEmpty())
        assertEquals(2, filterChats(readDirects, ChatFilter.ALL).size)
    }
}
