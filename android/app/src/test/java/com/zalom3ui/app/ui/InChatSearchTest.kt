package com.zalom3ui.app.ui

import com.zalom3ui.app.data.Attachment
import com.zalom3ui.app.data.ChatMessage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class InChatSearchTest {
    private fun msg(
        id: String,
        ts: Long,
        text: String = "",
        fromName: String = "",
        recalled: Boolean = false,
        attachment: Attachment? = null,
    ): ChatMessage = ChatMessage(
        id = id,
        threadId = "t1",
        threadKind = "group",
        fromName = fromName,
        ts = ts,
        text = text,
        recalled = recalled,
        attachment = attachment,
    )

    @Test
    fun foldVietnamese_stripsDiacriticsAndFoldsDStrokeAndCase() {
        assertEquals("chao dep", foldVietnamese("Chào đệp"))
        assertEquals("nguyen van duc", foldVietnamese("Nguyễn Văn Đức"))
        assertEquals("duong pho ha noi", foldVietnamese("ĐƯỜNG PHỐ HÀ NỘI"))
        assertEquals("hello 123", foldVietnamese("Hello 123"))
    }

    @Test
    fun findInChatMatches_matchesVietnameseDiacriticsInChronologicalOrder() {
        val messages = listOf(
            msg(id = "m3", ts = 300L, text = "Hẹn gặp lại nhé"),
            msg(id = "m1", ts = 100L, text = "Chào đệp buổi sáng"),
            msg(id = "m2", ts = 200L, text = "chao ca nha"),
        )
        assertEquals(listOf("m1", "m2"), findInChatMatches(messages, "chao"))
        assertEquals(listOf("m1"), findInChatMatches(messages, "dep"))
        assertEquals(listOf("m1"), findInChatMatches(messages, "ĐỆP"))
    }

    @Test
    fun findInChatMatches_matchesAttachmentTitleDescriptionAndSenderName() {
        val messages = listOf(
            msg(
                id = "m1",
                ts = 100L,
                text = "",
                attachment = Attachment(kind = "file", title = "Báo cáo quý 4.pdf", description = "Tài liệu nội bộ"),
            ),
            msg(
                id = "m2",
                ts = 200L,
                text = "",
                attachment = Attachment(kind = "link", title = "Release notes", description = "Hướng dẫn cài đặt"),
            ),
            msg(
                id = "m3",
                ts = 300L,
                text = "ok",
                fromName = "Trần Minh Đức",
            ),
        )
        assertEquals(listOf("m1"), findInChatMatches(messages, "bao cao"))
        assertEquals(listOf("m2"), findInChatMatches(messages, "cai dat"))
        assertEquals(listOf("m3"), findInChatMatches(messages, "minh duc"))
    }

    @Test
    fun findInChatMatches_skipsRecalledMessages() {
        val messages = listOf(
            msg(id = "m1", ts = 100L, text = "Chào đệp", recalled = true),
            msg(
                id = "m2",
                ts = 200L,
                text = "",
                recalled = true,
                attachment = Attachment(kind = "file", title = "Chào đệp.txt"),
            ),
            msg(id = "m3", ts = 300L, text = "Chào bạn"),
        )
        assertEquals(listOf("m3"), findInChatMatches(messages, "chao"))
    }

    @Test
    fun findInChatMatches_returnsEmptyForEmptyOrBlankQuery() {
        val messages = listOf(
            msg(id = "m1", ts = 100L, text = "Hello"),
            msg(id = "m2", ts = 200L, text = "World"),
        )
        assertEquals(emptyList<String>(), findInChatMatches(messages, ""))
        assertEquals(emptyList<String>(), findInChatMatches(messages, "   \t\n "))
    }

    @Test
    fun formatInChatMatchCounter_handlesBlankZeroAndActiveIndex() {
        assertNull(formatInChatMatchCounter("", 0, 0))
        assertNull(formatInChatMatchCounter("   ", 0, 5))
        assertEquals("0 / 0", formatInChatMatchCounter("abc", 0, 0))
        assertEquals("3 / 12", formatInChatMatchCounter("abc", 2, 12))
        assertEquals("12 / 12", formatInChatMatchCounter("abc", 99, 12))
    }
}
