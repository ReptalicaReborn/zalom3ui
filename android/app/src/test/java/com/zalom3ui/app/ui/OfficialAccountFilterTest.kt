package com.zalom3ui.app.ui

import com.zalom3ui.app.data.ThreadSummary
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class OfficialAccountFilterTest {
    private fun thread(
        id: String,
        kind: String = "user",
        unread: Int = 0,
        official: Boolean = false,
    ) = ThreadSummary(id = id, kind = kind, name = "Chat $id", unread = unread, isOfficialAccount = official)

    private val chats = listOf(
        thread("friend", unread = 1),
        thread("oa-news", unread = 4, official = true),
        thread("group", kind = "group"),
        thread("oa-shop", official = true),
    )

    @Test
    fun hideOff_showsEverythingInAllFilter() {
        val result = chatsForFilter(chats, ChatFilter.ALL, hideOfficialAccounts = false)
        assertEquals(chats.map { it.id }, result.map { it.id })
    }

    @Test
    fun hideOn_removesOfficialAccountsFromAllUnreadAndGroups() {
        assertEquals(listOf("friend", "group"), chatsForFilter(chats, ChatFilter.ALL, true).map { it.id })
        assertEquals(listOf("friend"), chatsForFilter(chats, ChatFilter.UNREAD, true).map { it.id })
        assertEquals(listOf("group"), chatsForFilter(chats, ChatFilter.GROUPS, true).map { it.id })
    }

    @Test
    fun officialFilter_listsOnlyOfficialAccountsInOrder_evenWhenHidden() {
        assertEquals(listOf("oa-news", "oa-shop"), chatsForFilter(chats, ChatFilter.OFFICIAL, true).map { it.id })
        assertEquals(listOf("oa-news", "oa-shop"), chatsForFilter(chats, ChatFilter.OFFICIAL, false).map { it.id })
    }

    @Test
    fun hiddenCount_isZeroUnlessHiding() {
        assertEquals(2, hiddenOfficialAccountCount(chats, hideOfficialAccounts = true))
        assertEquals(0, hiddenOfficialAccountCount(chats, hideOfficialAccounts = false))
        assertEquals(0, hiddenOfficialAccountCount(emptyList(), hideOfficialAccounts = true))
    }

    @Test
    fun threadSummary_defaultsToNotOfficial_andParsesFlag() {
        assertFalse(thread("x").isOfficialAccount)
        assertTrue(thread("y", official = true).isOfficialAccount)
        val json = kotlinx.serialization.json.Json { ignoreUnknownKeys = true }
        val parsed = json.decodeFromString(
            ThreadSummary.serializer(),
            """{"id":"1","kind":"user","name":"Báo Mới","isOfficialAccount":true}""",
        )
        assertTrue(parsed.isOfficialAccount)
        val old = json.decodeFromString(ThreadSummary.serializer(), """{"id":"1","kind":"user","name":"A"}""")
        assertFalse(old.isOfficialAccount)
    }
}
