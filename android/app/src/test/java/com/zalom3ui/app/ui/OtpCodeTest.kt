package com.zalom3ui.app.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** The same vectors as the server's `extract_otp_code` tests (rust/zalo-server/src/store.rs). Keep them in step. */
class OtpCodeTest {
    private val positive = listOf(
        "Mã xác thực OTP của bạn là: 482910 (hiệu lực 2 phút)" to "482910",
        "Your verification code is 9912" to "9912",
        "739201 là mã xác nhận giao dịch" to "739201",
        "Mã OTP xác thực giao dịch chuyển khoản 5.000.000đ đến TK 0123456789 là 482910" to "482910",
        "Mã OTP của bạn là 482 910" to "482910",
        "Mã OTP: 482-910 (không chia sẻ)" to "482910",
        "Your code is 123456" to "123456",
        "code: 123456" to "123456",
        "Your OTP: 5566. Do not share." to "5566",
        "mã: 123456" to "123456",
        "Mã của bạn là 1234" to "1234",
        "123456 is your verification code" to "123456",
        "G-123456 is your Google verification code" to "123456",
        "123 456 is your code" to "123456",
        "Mã OTP giao dịch ngày 12/05/2024 là 482910" to "482910",
        "Mã OTP xác thực giao dịch năm 2024 là 482910" to "482910",
        "Mã xác thực OTP\n849201" to "849201",
        "Mã OTP là 2024" to "2024",
    )

    private val negative = listOf(
        "Hẹn gặp lúc 14:30 nhé",
        "Chiều nay 17:30 đi ăn nhé",
        "gọi 0901234567",
        "Mã OTP là 0901234567",
        "Chuyển cho mình 5.000.000 nhé",
        "Mã giao dịch 5.000.000đ",
        "Mã OTP 123456789",
        "Mã OTP giao dịch ngày 12/05/2024",
        "Mã OTP hết hạn năm 2025",
        "Sinh nhật mình 12/05/2024",
        "Tôi đi mà 1234 người đến",
        "Đây là mã 123456",
        "Số tài khoản 12345678 của mình",
        "OTP hotline 1900",
        "Mã OTP của bạn đã hết hạn. Gọi 1900 để được hỗ trợ",
        "Bạn đang yêu cầu cài đặt lại mã PIN tài khoản Giang với mã tài khoản 0378795621 tại The 1.",
        "zip code is 70000",
        "Mã OTP 5 000 000đ",
    )

    @Test
    fun positive_vectors() {
        for ((text, code) in positive) assertEquals(text, code, extractOtpCode(text))
    }

    @Test
    fun negative_vectors() {
        for (text in negative) assertNull(text, extractOtpCode(text))
    }

    @Test
    fun blank_text_has_no_code() {
        assertNull(extractOtpCode(""))
        assertNull(extractOtpCode("   \n "))
    }
}
