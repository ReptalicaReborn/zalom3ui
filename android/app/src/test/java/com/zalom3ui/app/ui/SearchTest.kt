package com.zalom3ui.app.ui

import com.zalom3ui.app.data.ChatAccent
import com.zalom3ui.app.data.Person
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SearchTest {
    @Test
    fun foldForSearch_ignoresCaseAndVietnameseDiacritics() {
        assertEquals("nguyen van duc", foldForSearch("Nguyễn Văn Đức"))
        assertEquals("tran thi hoa", foldForSearch("TRẦN THỊ HOA"))
    }

    @Test
    fun filterPeople_hidesOfficialSystemAccountsAndPreservesExactCasing() {
        val people = listOf(
            Person(id = "0", name = "Zalo", avatar = ""),
            Person(id = "999", name = "Zalo Official", avatar = ""),
            Person(id = "1", name = "mika", avatar = ""),
            Person(id = "2", name = "dAn", avatar = ""),
            Person(id = "3", name = "Đức", avatar = ""),
        )
        assertTrue(isSystemContact(people[0]))
        assertTrue(isSystemContact(people[1]))
        assertFalse(isSystemContact(people[2]))

        val filtered = filterPeople(people, "")
        assertEquals(listOf("dAn", "Đức", "mika"), filtered.map { it.name })
    }

    @Test
    fun chatAccent_includesReptDocGoldSwatch() {
        assertEquals(ChatAccent.REPTDOC_GOLD, ChatAccent.fromKey("gold"))
    }
}
