package com.zalom3ui.app.ui

import com.zalom3ui.app.data.Attachment
import com.zalom3ui.app.data.ChatMessage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class ChatExpressiveTest {
    private fun msg(id: String, text: String, ts: Long, recalled: Boolean = false, title: String? = null) =
        ChatMessage(
            id = id,
            threadId = "t",
            threadKind = "user",
            ts = ts,
            text = text,
            recalled = recalled,
            attachment = title?.let { Attachment(kind = "file", title = it) },
        )

    @Test
    fun emptyQueryHasNoMatches() {
        assertTrue(findMessageMatches(listOf(msg("1", "hello", 1)), "  ").isEmpty())
    }

    @Test
    fun matchesAreCaseAndDiacriticInsensitiveNewestFirst() {
        val messages = listOf(msg("1", "Xin chào bạn", 1), msg("2", "nothing", 2), msg("3", "CHAO nhé", 3))
        assertEquals(listOf("3", "1"), findMessageMatches(messages, "chao"))
    }

    @Test
    fun recalledMessagesAreSkippedAndAttachmentTitlesMatch() {
        val messages = listOf(
            msg("1", "report", 1, recalled = true),
            msg("2", "see file", 2, title = "Quarterly report.pdf"),
        )
        assertEquals(listOf("2"), findMessageMatches(messages, "report"))
    }

    @Test
    fun extractUrlsFindsLinksAndTrimsTrailingPunctuation() {
        val text = "see https://example.com/a?b=1, and (http://foo.bar/x)."
        val found = extractUrls(text).map { text.substring(it) }
        assertEquals(listOf("https://example.com/a?b=1", "http://foo.bar/x"), found)
    }

    @Test
    fun extractUrlsIgnoresPlainText() {
        assertTrue(extractUrls("no links here").isEmpty())
        assertTrue(extractUrls("").isEmpty())
    }
}
