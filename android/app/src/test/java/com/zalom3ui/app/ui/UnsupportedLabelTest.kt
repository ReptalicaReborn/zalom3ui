package com.zalom3ui.app.ui

import com.zalom3ui.app.data.ChatMessage
import org.junit.Assert.assertEquals
import org.junit.Test

class UnsupportedLabelTest {
    private fun msg(text: String, type: String) =
        ChatMessage(id = "1", threadId = "t", threadKind = "group", ts = 1L, msgType = type, text = text)

    @Test
    fun unsupportedBubbleNamesItsType() {
        assertEquals("[Unsupported message · chat.xyz]", bubbleText(msg("[Unsupported message]", "chat.xyz")))
    }

    @Test
    fun otherTextAndBlankTypeAreUnchanged() {
        assertEquals("hello", bubbleText(msg("hello", "webchat")))
        assertEquals("[Unsupported message]", bubbleText(msg("[Unsupported message]", "")))
    }
}
