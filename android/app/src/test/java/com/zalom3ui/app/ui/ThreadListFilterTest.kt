package com.zalom3ui.app.ui

import com.zalom3ui.app.data.ThreadSummary
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
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

    @Test
    fun formatDraftSnippet_nullOrBlank_returnsNull() {
        assertNull(formatDraftSnippet(null))
        assertNull(formatDraftSnippet(""))
        assertNull(formatDraftSnippet("   "))
        assertNull(formatDraftSnippet("\n\t  \r\n "))
    }

    @Test
    fun formatDraftSnippet_trimsAndCollapsesNewlinesAndWhitespace() {
        assertEquals("Hello world", formatDraftSnippet("Hello world"))
        assertEquals("Hello world", formatDraftSnippet("  Hello world  "))
        assertEquals("Line one Line two Line three", formatDraftSnippet("Line one\nLine two\r\n\nLine three"))
        assertEquals("a b c", formatDraftSnippet("\t a   \n  b\t\tc \n"))
    }

    @Test
    fun findHighlightRanges_caseAndDiacriticInsensitive() {
        assertEquals(listOf(4..8, 16..20), findHighlightRanges("Say Hello again hello!", "HELLO"))
        assertEquals(listOf(5..7), findHighlightRanges("Chào Đức nhé", "duc"))
        assertTrue(findHighlightRanges("Hello world", "   ").isEmpty())
        assertTrue(findHighlightRanges("Hello world", "xyz").isEmpty())
    }
}

class SwipeThresholdTest {
    @Test
    fun threshold_isFractionOfWidth_orFallback() {
        assertEquals(380f, swipeThresholdPx(1000f, 72f), 0.001f)
        assertEquals(72f, swipeThresholdPx(0f, 72f), 0.001f)
    }

    @Test
    fun triggers_onlyPastThreshold_inEitherDirection() {
        assertEquals(false, swipeTriggers(379f, 380f))
        assertEquals(true, swipeTriggers(380f, 380f))
        assertEquals(true, swipeTriggers(-400f, 380f))
        assertEquals(false, swipeTriggers(-100f, 380f))
    }
}
