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
}
