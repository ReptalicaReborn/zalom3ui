package com.zalom3ui.app.server

import com.zalom3ui.app.server.ZaloNotificationRules.Kind
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class ZaloNotificationRulesTest {
    private fun zalo(
        category: String? = "msg",
        ongoing: Boolean = false,
        fullScreen: Boolean = false,
        summary: Boolean = false,
        title: String? = "Alice",
        conversationTitle: String? = null,
        pkg: String = "com.zing.zalo",
    ) = NotificationInfo(pkg, category, ongoing, fullScreen, summary, title, conversationTitle)

    @Test
    fun the_zalo_package_is_the_official_app() {
        assertEquals("com.zing.zalo", MessageNotifier.ZALO_PACKAGE)
    }

    @Test
    fun other_apps_are_not_our_business() {
        assertEquals(Kind.NOT_ZALO, ZaloNotificationRules.classify(zalo(pkg = "com.whatsapp")))
        assertEquals(Kind.NOT_ZALO, ZaloNotificationRules.classify(zalo(pkg = "com.zalom3ui.app")))
    }

    @Test
    fun message_notifications_bring_zalo_online() {
        assertEquals(Kind.MESSAGE, ZaloNotificationRules.classify(zalo(category = "msg")))
        // Zalo may set no category at all.
        assertEquals(Kind.MESSAGE, ZaloNotificationRules.classify(zalo(category = null)))
        assertEquals(Kind.MESSAGE, ZaloNotificationRules.classify(zalo(category = "social")))
    }

    @Test
    fun call_notifications_are_calls_and_nothing_else() {
        assertEquals(Kind.CALL, ZaloNotificationRules.classify(zalo(category = "call")))
        assertEquals(Kind.CALL, ZaloNotificationRules.classify(zalo(category = "CALL")))
        assertEquals(Kind.CALL, ZaloNotificationRules.classify(zalo(category = "missed_call")))
        // A full-screen intent marks an incoming call whatever the category says.
        assertEquals(Kind.CALL, ZaloNotificationRules.classify(zalo(category = "msg", fullScreen = true)))
        assertEquals(Kind.CALL, ZaloNotificationRules.classify(zalo(category = null, fullScreen = true, ongoing = true)))
    }

    @Test
    fun running_progress_and_summary_notifications_are_not_news() {
        assertEquals(Kind.IGNORED, ZaloNotificationRules.classify(zalo(ongoing = true)))
        assertEquals(Kind.IGNORED, ZaloNotificationRules.classify(zalo(category = "service")))
        assertEquals(Kind.IGNORED, ZaloNotificationRules.classify(zalo(category = "progress")))
        assertEquals(Kind.IGNORED, ZaloNotificationRules.classify(zalo(category = "promo")))
        // A group summary would re-wake after Zalo's own notification was hidden.
        assertEquals(Kind.IGNORED, ZaloNotificationRules.classify(zalo(summary = true)))
    }

    @Test
    fun a_zalo_notification_matches_the_chat_by_title() {
        val n = zalo(title = "Alice Nguyen")
        assertTrue(ZaloNotificationRules.isForChat(n, "Alice Nguyen"))
        assertTrue(ZaloNotificationRules.isForChat(n, "  alice   nguyen "))
        assertFalse(ZaloNotificationRules.isForChat(n, "Alice"))
        assertFalse(ZaloNotificationRules.isForChat(n, "Bob"))
        assertFalse(ZaloNotificationRules.isForChat(n, ""))
        assertFalse(ZaloNotificationRules.isForChat(zalo(title = null), "Alice"))
    }

    @Test
    fun a_group_title_with_the_sender_around_it_still_matches() {
        assertTrue(ZaloNotificationRules.isForChat(zalo(title = "Family: Mom"), "Family"))
        assertTrue(ZaloNotificationRules.isForChat(zalo(title = "Mom @ Family"), "Family"))
        assertTrue(ZaloNotificationRules.isForChat(zalo(title = "Mom", conversationTitle = "Family"), "Family"))
        assertFalse(ZaloNotificationRules.isForChat(zalo(title = "Family reunion planning"), "Family"))
        assertFalse(ZaloNotificationRules.isForChat(zalo(title = "Familyx: Mom"), "Family"))
    }

    @Test
    fun hiding_never_touches_calls_or_other_apps() {
        assertFalse(ZaloNotificationRules.isForChat(zalo(category = "call", title = "Alice"), "Alice"))
        assertFalse(ZaloNotificationRules.isForChat(zalo(fullScreen = true, title = "Alice"), "Alice"))
        assertFalse(ZaloNotificationRules.isForChat(zalo(pkg = "com.whatsapp", title = "Alice"), "Alice"))
        assertFalse(ZaloNotificationRules.isForChat(zalo(ongoing = true, title = "Alice"), "Alice"))
    }
}
