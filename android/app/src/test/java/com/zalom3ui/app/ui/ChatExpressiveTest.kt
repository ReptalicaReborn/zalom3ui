package com.zalom3ui.app.ui

import com.zalom3ui.app.data.Attachment
import com.zalom3ui.app.data.CallInfo
import com.zalom3ui.app.data.ChatMessage
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.Calendar

class ChatExpressiveTest {
    private fun msg(
        id: String,
        text: String,
        ts: Long,
        recalled: Boolean = false,
        title: String? = null,
        msgType: String = "webchat",
        attachment: Attachment? = title?.let { Attachment(kind = "file", title = it) },
    ) = ChatMessage(
        id = id,
        threadId = "t",
        threadKind = "user",
        ts = ts,
        msgType = msgType,
        text = text,
        recalled = recalled,
        attachment = attachment,
    )

    private fun callMsg(id: String, ts: Long, missed: Boolean = false, video: Boolean = false) = msg(
        id = id,
        text = if (missed) "[You declined the call]" else "[Incoming voice call]",
        ts = ts,
        attachment = Attachment(
            kind = "call",
            title = if (missed) "You declined the call" else "Incoming voice call",
            duration = if (missed) null else 3000L,
            call = CallInfo(video = video, missed = missed, outgoing = false),
        ),
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

    @Test
    fun formatPresenceUsesCompactFormat() {
        val now = Calendar.getInstance().apply {
            set(2026, Calendar.NOVEMBER, 26, 15, 0, 0)
            set(Calendar.MILLISECOND, 0)
        }.timeInMillis
        assertEquals("Active now", formatPresence(now - 2 * 60_000L, now))
        assertEquals("Active 5m ago", formatPresence(now - 5 * 60_000L, now))
        assertEquals("Active 2h ago", formatPresence(now - 2 * 3_600_000L, now))
        assertEquals("Active yesterday", formatPresence(now - 26 * 3_600_000L, now))
        assertEquals("Active 3d ago", formatPresence(now - 3 * 86_400_000L, now))
        // Seconds timestamps are also normalized.
        assertEquals("Active 10m ago", formatPresence((now - 10 * 60_000L) / 1000L, now))
    }

    @Test
    fun systemMessageLabelMapsKnownPromptsAndIgnoresNormalMessages() {
        assertEquals("Call audio issue?", systemMessageLabel(msg("1", "Âm thanh cuộc gọi có vấn đề?", 1)))
        assertEquals("Logged in to Zalo on PC", systemMessageLabel(msg("2", "Đăng nhập Zalo trên máy tính thành công", 2)))
        assertEquals("You are now connected with Lanh", systemMessageLabel(msg("3", "Bạn vừa kết bạn với Lanh", 3)))
        assertEquals("You joined the community", systemMessageLabel(msg("4", "Bạn trở thành thành viên của cộng đồng", 4)))
        assertNull(systemMessageLabel(msg("5", "Hello there!", 5)))
        assertNull(systemMessageLabel(callMsg("6", 6)))
    }

    @Test
    fun buildChatItemsGroupsConsecutiveCallsWithinWindow() {
        val base = Calendar.getInstance().apply {
            set(2026, Calendar.NOVEMBER, 26, 12, 57, 0)
            set(Calendar.MILLISECOND, 0)
        }.timeInMillis
        val list = listOf(
            msg("m0", "hi", base - 60_000L),
            callMsg("c1", base, missed = false),
            callMsg("c2", base + 60_000L, missed = true),
            callMsg("c3", base + 120_000L, missed = true),
            // 10 minutes later: separate single call.
            callMsg("c4", base + 12 * 60_000L, missed = false),
            msg("m1", "ok", base + 13 * 60_000L),
        )
        val items = buildChatItems(list, firstUnreadId = null)
        assertEquals(4, items.size)
        assertTrue(items[0] is ChatItem.Single)
        val group = items[1] as ChatItem.CallGroup
        assertEquals(listOf("c1", "c2", "c3"), group.calls.map { it.id })
        assertTrue(items[2] is ChatItem.Single)
        assertEquals("c4", (items[2] as ChatItem.Single).msg.id)
        assertTrue(items[3] is ChatItem.Single)

        // Unread boundary splits a call group so UnreadDivider stays anchored accurately.
        val split = buildChatItems(list, firstUnreadId = "c2")
        assertEquals(5, split.size)
        assertEquals("c1", (split[1] as ChatItem.Single).msg.id)
        val unreadGroup = split[2] as ChatItem.CallGroup
        assertEquals(listOf("c2", "c3"), unreadGroup.calls.map { it.id })
        assertTrue(unreadGroup.firstUnreadAnchor)
    }

    @Test
    fun extractChatPhotoUrlsCollectsNonRecalledPhotosAndGifsInOrder() {
        val messages = listOf(
            msg("m1", "plain text", 1),
            msg("p1", "", 2, attachment = Attachment(kind = "photo", thumb = "https://cdn/t1.jpg", href = "https://cdn/full1.jpg")),
            msg("p2", "", 3, recalled = true, attachment = Attachment(kind = "photo", href = "https://cdn/recalled.jpg")),
            msg("v1", "", 4, attachment = Attachment(kind = "video", thumb = "https://cdn/vthumb.jpg", href = "https://cdn/video.mp4")),
            msg("s1", "", 5, attachment = Attachment(kind = "sticker", thumb = "https://cdn/sticker.webp")),
            msg("g1", "", 6, attachment = Attachment(kind = "gif", thumb = "https://cdn/g1.gif")),
            msg("p3", "", 7, attachment = Attachment(kind = "photo", href = "   ", thumb = "   ")),
            msg("p4", "", 8, attachment = Attachment(kind = "photo", href = "https://cdn/full1.jpg")),
        )

        assertEquals(
            listOf("https://cdn/full1.jpg", "https://cdn/g1.gif", "https://cdn/full1.jpg"),
            extractChatPhotoUrls(messages),
        )

        val entries = extractChatPhotoEntries(messages)
        // Duplicate URLs resolve to the exact message tapped when msgId is provided.
        assertEquals(0, findChatPhotoIndex(entries, "p1", "https://cdn/full1.jpg"))
        assertEquals(2, findChatPhotoIndex(entries, "p4", "https://cdn/full1.jpg"))
        assertEquals(1, findChatPhotoIndex(entries, null, "https://cdn/g1.gif"))
        assertEquals(-1, findChatPhotoIndex(entries, "missing", "https://cdn/missing.jpg"))
    }
}
