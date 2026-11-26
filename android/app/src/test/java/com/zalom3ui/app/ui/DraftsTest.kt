package com.zalom3ui.app.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Test

class DraftsTest {
    private val drafts = mapOf("user:1" to "On my way", "group:2" to "Hello")

    @Test
    fun withDraft_setsReplacesAndRemoves() {
        assertEquals(drafts + ("user:3" to "Hi"), AppViewModel.withDraft(drafts, "user:3", "Hi"))
        assertEquals(drafts + ("user:1" to "Almost there"), AppViewModel.withDraft(drafts, "user:1", "Almost there"))
        assertEquals(mapOf("group:2" to "Hello"), AppViewModel.withDraft(drafts, "user:1", ""))
        assertEquals(mapOf("group:2" to "Hello"), AppViewModel.withDraft(drafts, "user:1", "  \n"))
    }

    @Test
    fun withDraft_keepsTheSameMapWhenNothingChanges() {
        assertSame(drafts, AppViewModel.withDraft(drafts, "user:1", "On my way"))
        assertSame(drafts, AppViewModel.withDraft(drafts, "user:3", ""))
        assertSame(drafts, AppViewModel.withDraft(drafts, "user:3", " "))
    }
}
