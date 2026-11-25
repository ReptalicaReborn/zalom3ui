package com.zalom3ui.app.ui

import androidx.compose.ui.unit.dp
import com.zalom3ui.app.data.ChatMessage
import com.zalom3ui.app.data.ThreadSummary
import org.junit.Assert.assertEquals
import org.junit.Test

class LayoutTest {
    private fun thread(id: String, ts: Long? = null, pinned: Boolean = false) = ThreadSummary(
        id = id,
        kind = "user",
        name = id,
        pinned = pinned,
        lastMessage = ts?.let { ChatMessage(id = "m$id", threadId = id, threadKind = "user", ts = it, text = "") },
    )

    @Test
    fun sortingMixesChatsWithAndWithoutMessages() {
        // Friends without messages are in the list too; their fallback used to be an Int, compared with a Long:
        // "java.lang.Integer cannot be cast to java.lang.Long".
        val sorted = AppViewModel.sortThreads(listOf(thread("friend"), thread("old", 1_000), thread("new", 2_000), thread("pin", 10, pinned = true)))
        assertEquals(listOf("pin", "new", "old", "friend"), sorted.map { it.id })
    }

    @Test
    fun theListPaneTakesAboutAThird() {
        assertEquals(360.dp, listPaneWidth(1000))
        assertEquals("narrow tablets keep a usable list", 320.dp, listPaneWidth(700))
        assertEquals("wide screens don't stretch it", 440.dp, listPaneWidth(1600))
    }
}
