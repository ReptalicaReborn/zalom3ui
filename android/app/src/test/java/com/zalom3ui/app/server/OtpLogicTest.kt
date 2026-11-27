package com.zalom3ui.app.server

import com.zalom3ui.app.data.Attachment
import com.zalom3ui.app.data.ChatMessage
import com.zalom3ui.app.data.StayConnected
import com.zalom3ui.app.server.ListenerJobs.Job
import com.zalom3ui.app.server.ZaloNotificationRules.Kind
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class OtpLogicTest {
    @Test
    fun zaloNotificationIsHiddenOnlyWhileZaloShowsTheCodeItself() {
        // Posted now, or already shown by Zalỏ: hide Zalo's duplicate when the user wants that.
        assertTrue(OtpRescue.shouldHideZalo(RescueResult.POSTED, hideSetting = true))
        assertTrue(OtpRescue.shouldHideZalo(RescueResult.ALREADY_SHOWN, hideSetting = true))
        // Zalỏ couldn't show it: keep Zalo's, or the user would have no notification at all.
        assertFalse(OtpRescue.shouldHideZalo(RescueResult.NOT_SHOWN, hideSetting = true))
        // The switch is off: never hide.
        assertFalse(OtpRescue.shouldHideZalo(RescueResult.POSTED, hideSetting = false))
        assertFalse(OtpRescue.shouldHideZalo(RescueResult.ALREADY_SHOWN, hideSetting = false))
    }

    private fun msg(
        text: String = "hi",
        otp: String? = null,
        self: Boolean = false,
        recalled: Boolean = false,
        attachment: Attachment? = null,
    ) = ChatMessage(
        id = "1", threadId = "t", threadKind = "user", ts = 1L, text = text,
        otp = otp, isSelf = self, recalled = recalled, attachment = attachment,
    )

    // ---- Notification words ----

    @Test
    fun the_code_comes_first() {
        assertEquals("Code 482910 · Zalo OTP", OtpText.title("482910", "Zalo OTP"))
        assertEquals("Code 482910", OtpText.title("482910", null))
        assertEquals("Code 482910", OtpText.title("482910", "  "))
        assertEquals("482910 · Mã OTP là 482910", OtpText.line("482910", "Mã OTP là 482910"))
        assertEquals("Copy 482910", OtpText.copyLabel("482910"))
        assertEquals("Copied 482910", OtpText.copiedToast("482910"))
    }

    @Test
    fun the_public_version_names_no_code() {
        assertFalse(OtpText.PUBLIC_TEXT.any { it.isDigit() })
        assertFalse(OtpText.PUBLIC_TITLE.any { it.isDigit() })
    }

    @Test
    fun only_four_to_eight_digits_are_a_valid_code() {
        assertTrue(OtpText.isValidCode("1234"))
        assertTrue(OtpText.isValidCode("12345678"))
        assertFalse(OtpText.isValidCode("123"))
        assertFalse(OtpText.isValidCode("123456789"))
        assertFalse(OtpText.isValidCode("12a456"))
        assertFalse(OtpText.isValidCode(null))
    }

    // ---- Which messages carry a code ----

    @Test
    fun the_servers_code_wins_then_the_card_then_the_text() {
        assertEquals("111111", OtpText.messageCode(msg(text = "Mã OTP là 222222", otp = "111111")))
        assertEquals("333333", OtpText.messageCode(msg(text = "card", attachment = Attachment(kind = "link", otp = "333333"))))
        assertEquals("222222", OtpText.messageCode(msg(text = "Mã OTP là 222222")))
        assertNull(OtpText.messageCode(msg(text = "Hẹn gặp lúc 14:30")))
    }

    @Test
    fun my_own_and_recalled_messages_carry_no_code() {
        assertNull(OtpText.messageCode(msg(text = "Mã OTP là 222222", self = true)))
        assertNull(OtpText.messageCode(msg(text = "Mã OTP là 222222", recalled = true)))
        assertNull(OtpText.messageCode(msg(otp = "12", text = "x")))
    }

    // ---- De-duplication ----

    @Test
    fun a_code_zalo_announced_a_moment_ago_is_not_rescued() {
        val d = OtpDedupe()
        d.noteNotified("482910", now = 1_000)
        assertFalse(d.tryRescue("482910", now = 1_000 + OtpDedupe.WEB_WINDOW_MS))
        // A different code is fine.
        assertTrue(d.tryRescue("111111", now = 2_000))
    }

    @Test
    fun an_old_web_notification_does_not_block_a_rescue() {
        val d = OtpDedupe()
        d.noteNotified("482910", now = 0)
        assertTrue(d.tryRescue("482910", now = OtpDedupe.WEB_WINDOW_MS + 1))
    }

    @Test
    fun zalo_updating_its_notification_does_not_post_the_same_code_twice() {
        val d = OtpDedupe()
        assertTrue(d.tryRescue("482910", now = 0))
        assertFalse(d.tryRescue("482910", now = 5_000))
        assertFalse(d.tryRescue("482910", now = OtpDedupe.RESCUE_MEMORY_MS))
        // Much later it is a new code with the same digits.
        assertTrue(d.tryRescue("482910", now = OtpDedupe.RESCUE_MEMORY_MS + 1))
    }

    @Test
    fun the_memory_is_bounded() {
        val d = OtpDedupe()
        for (i in 0 until 500) assertTrue(d.tryRescue((100000 + i).toString(), now = i.toLong()))
        // The oldest were forgotten; the newest are remembered.
        assertTrue(d.tryRescue("100000", now = 501))
        assertFalse(d.tryRescue("100499", now = 502))
    }

    // ---- Reading the Zalo notification ----

    @Test
    fun a_code_is_found_in_any_text_field() {
        assertEquals("482910", OtpRescue.findCode(ZaloNotificationText("Zalo OTP", listOf("Mã OTP của bạn là 482910"))))
        assertEquals("482910", OtpRescue.findCode(ZaloNotificationText("Zalo OTP", listOf("hi", "", "Mã OTP: 482910"))))
        assertEquals("482910", OtpRescue.findCode(ZaloNotificationText("Mã OTP 482910", emptyList())))
    }

    @Test
    fun a_title_keyword_over_a_text_that_is_only_the_code() {
        assertEquals("849201", OtpRescue.findCode(ZaloNotificationText("Mã xác thực OTP", listOf("849201"))))
    }

    @Test
    fun plain_chat_in_a_notification_has_no_code() {
        assertNull(OtpRescue.findCode(ZaloNotificationText("Alice", listOf("Hẹn gặp lúc 14:30", "gọi 0901234567"))))
        assertNull(OtpRescue.findCode(ZaloNotificationText(null, emptyList())))
        assertNull(OtpRescue.findCode(ZaloNotificationText("", listOf("   "))))
    }

    @Test
    fun only_recent_messaging_style_messages_count() {
        val now = 10 * 60_000L
        assertTrue(OtpRescue.isFresh(now - 1_000, now))
        assertTrue(OtpRescue.isFresh(now - OtpRescue.FRESH_MS, now))
        assertFalse(OtpRescue.isFresh(now - OtpRescue.FRESH_MS - 1, now))
        // No timestamp: cannot tell, so it counts.
        assertTrue(OtpRescue.isFresh(0, now))
    }

    // ---- Which job runs ----

    @Test
    fun the_component_is_on_for_the_bridge_or_the_rescue_alone() {
        assertFalse(ListenerJobs.componentEnabled(onDevice = true, mode = StayConnected.ALWAYS, otpRescue = false))
        assertFalse(ListenerJobs.componentEnabled(onDevice = true, mode = StayConnected.WHILE_OPEN, otpRescue = false))
        assertTrue(ListenerJobs.componentEnabled(onDevice = true, mode = StayConnected.WHILE_OPEN_BRIDGE, otpRescue = false))
        assertTrue(ListenerJobs.componentEnabled(onDevice = true, mode = StayConnected.ALWAYS, otpRescue = true))
        assertTrue(ListenerJobs.componentEnabled(onDevice = true, mode = StayConnected.WHILE_OPEN, otpRescue = true))
        assertTrue(ListenerJobs.componentEnabled(onDevice = false, mode = StayConnected.ALWAYS, otpRescue = true))
        // The bridge only exists for the on-phone core.
        assertFalse(ListenerJobs.componentEnabled(onDevice = false, mode = StayConnected.WHILE_OPEN_BRIDGE, otpRescue = false))
    }

    @Test
    fun each_job_is_gated_on_its_own_setting() {
        for (mode in StayConnected.entries) {
            assertEquals(
                if (mode.usesBridge) setOf(Job.BRIDGE) else emptySet<Job>(),
                ListenerJobs.jobsFor(Kind.MESSAGE, mode, otpRescue = false),
            )
            val withRescue = ListenerJobs.jobsFor(Kind.MESSAGE, mode, otpRescue = true)
            assertTrue(Job.OTP_RESCUE in withRescue)
            // The rescue never turns the bridge on in the other modes.
            assertEquals(mode.usesBridge, Job.BRIDGE in withRescue)
        }
    }

    @Test
    fun calls_and_other_notifications_get_no_job() {
        for (kind in listOf(Kind.CALL, Kind.IGNORED, Kind.NOT_ZALO)) {
            assertTrue(ListenerJobs.jobsFor(kind, StayConnected.WHILE_OPEN_BRIDGE, otpRescue = true).isEmpty())
        }
    }

    @Test
    fun the_rescue_defaults_to_off() {
        assertFalse(com.zalom3ui.app.data.StaySettings().otpFromZaloNotifications)
    }
}
