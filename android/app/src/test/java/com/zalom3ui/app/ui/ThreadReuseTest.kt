package com.zalom3ui.app.ui

import com.zalom3ui.app.data.ChatMessage
import com.zalom3ui.app.data.ThreadSummary
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotSame
import org.junit.Assert.assertSame
import org.junit.Test

class ThreadReuseTest {
    private fun thread(id: String, text: String = "hi $id", unread: Int = 0) = ThreadSummary(
        id = id,
        kind = "user",
        name = "Chat $id",
        lastMessage = ChatMessage(id = "m-$id", threadId = id, threadKind = "user", ts = 1L, text = text),
        unread = unread,
    )

    private val current = listOf(thread("1"), thread("2"), thread("3"))

    @Test
    fun keepUnchanged_returnsTheCurrentListWhenNothingChanged() {
        val fresh = listOf(thread("1"), thread("2"), thread("3"))
        assertSame(current, AppViewModel.keepUnchanged(current, fresh))
    }

    @Test
    fun keepUnchanged_reusesUnchangedThreadsAndTakesChangedOnes() {
        val changed = thread("2", unread = 4)
        val result = AppViewModel.keepUnchanged(current, listOf(thread("1"), changed, thread("3")))
        assertSame(current[0], result[0])
        assertSame(changed, result[1])
        assertSame(current[2], result[2])
    }

    @Test
    fun keepUnchanged_followsTheFreshOrderAndMembers() {
        val added = thread("4")
        val result = AppViewModel.keepUnchanged(current, listOf(thread("3"), added, thread("1")))
        assertNotSame(current, result)
        assertEquals(listOf("3", "4", "1"), result.map { it.id })
        assertSame(current[2], result[0])
        assertSame(added, result[1])
        assertSame(current[0], result[2])
    }

    @Test
    fun keepUnchanged_withNothingShownYetTakesTheFreshList() {
        val fresh = listOf(thread("1"))
        assertSame(fresh, AppViewModel.keepUnchanged(emptyList(), fresh))
        assertEquals(emptyList<ThreadSummary>(), AppViewModel.keepUnchanged(current, emptyList()))
    }
}
