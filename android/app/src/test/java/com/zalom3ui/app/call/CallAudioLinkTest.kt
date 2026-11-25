package com.zalom3ui.app.call

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Test

class CallAudioLinkTest {
    @Test
    fun outgoingIsTypeByteThenPacket() {
        assertArrayEquals(byteArrayOf(2, 7, 8, 9), CallAudioLink.encodeOutgoing(byteArrayOf(7, 8, 9)))
    }

    @Test
    fun incomingParsesSequenceTimestampAndPayload() {
        val msg = byteArrayOf(1, 0x12, 0x34, 0xDE.toByte(), 0xAD.toByte(), 0xBE.toByte(), 0xEF.toByte(), 5, 6)
        val m = CallAudioLink.decodeIncoming(msg)
        assertNotNull(m)
        assertEquals(0x1234, m!!.seq)
        assertEquals(0xDEADBEEFL, m.timestamp)
        assertArrayEquals(byteArrayOf(5, 6), m.opus)
    }

    @Test
    fun incomingKeepsHighSequenceUnsigned() {
        val m = CallAudioLink.decodeIncoming(byteArrayOf(1, 0xFF.toByte(), 0xFF.toByte(), 0, 0, 0, 1, 42))!!
        assertEquals(65535, m.seq)
        assertEquals(1L, m.timestamp)
    }

    @Test
    fun shortOrUnknownMessagesAreIgnored() {
        assertNull(CallAudioLink.decodeIncoming(ByteArray(0)))
        assertNull(CallAudioLink.decodeIncoming(byteArrayOf(1, 0, 1)))
        assertNull(CallAudioLink.decodeIncoming(byteArrayOf(1, 0, 1, 0, 0, 0, 1))) // header only, no payload
        assertNull(CallAudioLink.decodeIncoming(byteArrayOf(9, 0, 1, 0, 0, 0, 1, 5)))
        assertNull(CallAudioLink.decodeIncoming(byteArrayOf(2, 0, 1, 0, 0, 0, 1, 5)))
    }

    @Test
    fun urlUsesWsSchemeAndToken() {
        assertEquals(
            "ws://127.0.0.1:41234/ws/call-audio?token=a%2Bb",
            CallAudioLink.buildUrl("http://127.0.0.1:41234/", "a+b"),
        )
        assertEquals("ws://127.0.0.1:1/ws/call-audio", CallAudioLink.buildUrl("http://127.0.0.1:1", null))
    }

    @Test
    fun videoOutIsTypeFlagsThenAccessUnit() {
        val au = byteArrayOf(0, 0, 0, 1, 0x65, 9)
        assertArrayEquals(byteArrayOf(4, 1, 0, 0, 0, 1, 0x65, 9), CallAudioLink.encodeVideoOut(true, au))
        assertArrayEquals(byteArrayOf(4, 0, 0, 0, 0, 1, 0x41, 9), CallAudioLink.encodeVideoOut(false, byteArrayOf(0, 0, 0, 1, 0x41, 9)))
    }

    @Test
    fun keyFrameNeededIsOneByte() {
        assertArrayEquals(byteArrayOf(6), CallAudioLink.encodeKeyFrameNeeded())
    }

    @Test
    fun videoInParsesFlagsTimestampAndUnit() {
        val msg = byteArrayOf(3, 1, 0xDE.toByte(), 0xAD.toByte(), 0xBE.toByte(), 0xEF.toByte(), 0, 0, 0, 1, 0x67)
        val v = CallAudioLink.decodeVideoIn(msg)!!
        assertEquals(true, v.key)
        assertEquals(0xDEADBEEFL, v.timestamp)
        assertArrayEquals(byteArrayOf(0, 0, 0, 1, 0x67), v.accessUnit)
        // Only bit 0 means key frame.
        assertEquals(false, CallAudioLink.decodeVideoIn(byteArrayOf(3, 2, 0, 0, 0, 1, 5))!!.key)
    }

    @Test
    fun videoInIgnoresOtherTypesAndEmptyUnits() {
        assertNull(CallAudioLink.decodeVideoIn(ByteArray(0)))
        assertNull(CallAudioLink.decodeVideoIn(byteArrayOf(3, 1, 0, 0, 0, 1))) // header only
        assertNull(CallAudioLink.decodeVideoIn(byteArrayOf(1, 1, 0, 0, 0, 1, 5)))
        // Audio parsing is not confused by a video message.
        assertNull(CallAudioLink.decodeIncoming(byteArrayOf(3, 1, 0, 0, 0, 1, 5, 6)))
    }

    @Test
    fun keyFrameWantedIsTypeFive() {
        assertEquals(true, CallAudioLink.isKeyFrameWanted(byteArrayOf(5)))
        assertEquals(false, CallAudioLink.isKeyFrameWanted(byteArrayOf(6)))
        assertEquals(false, CallAudioLink.isKeyFrameWanted(ByteArray(0)))
    }
}
