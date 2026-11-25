package com.zalom3ui.app.call

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class JitterBufferTest {
    private fun p(n: Int) = byteArrayOf(n.toByte())

    private fun JitterBuffer.popId(): Int? = pop()?.let { it[0].toInt() and 0xFF }

    @Test
    fun waitsForTargetBeforePlaying() {
        val jb = JitterBuffer(targetFrames = 3)
        jb.push(10, p(1))
        jb.push(11, p(2))
        assertFalse(jb.isPlaying)
        assertNull(jb.pop())
        jb.push(12, p(3))
        assertTrue(jb.isPlaying)
        assertEquals(1, jb.popId())
    }

    @Test
    fun playsInOrder() {
        val jb = JitterBuffer(3)
        for (i in 0..4) jb.push(100 + i, p(i))
        assertEquals((0..4).toList(), (0..4).map { jb.popId() })
    }

    @Test
    fun reorderedPacketsComeOutInOrder() {
        val jb = JitterBuffer(3)
        jb.push(5, p(5))
        jb.push(7, p(7))
        jb.push(6, p(6))
        assertEquals(listOf(5, 6, 7), listOf(jb.popId(), jb.popId(), jb.popId()))
    }

    @Test
    fun reorderBeforeStartKeepsEarliestFirst() {
        val jb = JitterBuffer(3)
        jb.push(7, p(7))
        jb.push(5, p(5))
        jb.push(6, p(6))
        assertEquals(listOf(5, 6, 7), listOf(jb.popId(), jb.popId(), jb.popId()))
    }

    @Test
    fun missingSequenceIsNullAndPlayoutMovesOn() {
        val jb = JitterBuffer(3)
        jb.push(1, p(1))
        jb.push(3, p(3))
        jb.push(4, p(4))
        assertEquals(1, jb.popId())
        assertNull(jb.pop())
        assertEquals(3, jb.popId())
        assertEquals(4, jb.popId())
    }

    @Test
    fun duplicatesAndOldPacketsAreDropped() {
        val jb = JitterBuffer(3)
        jb.push(1, p(1))
        jb.push(1, p(99))
        jb.push(2, p(2))
        jb.push(3, p(3))
        assertEquals(1, jb.popId())
        jb.push(1, p(1)) // already played
        jb.push(2, p(98)) // duplicate of a queued one
        assertEquals(2, jb.popId())
        assertEquals(3, jb.popId())
        assertNull(jb.pop())
    }

    @Test
    fun sequenceNumbersWrapAround() {
        val jb = JitterBuffer(3)
        jb.push(65534, p(1))
        jb.push(0, p(3))
        jb.push(65535, p(2))
        jb.push(1, p(4))
        assertEquals(listOf(1, 2, 3, 4), (0..3).map { jb.popId() })
    }

    @Test
    fun overflowSkipsAheadToKeepLatencyBounded() {
        val jb = JitterBuffer(targetFrames = 2, maxFrames = 5)
        for (i in 0..5) jb.push(i, p(i))
        // 6 queued > 5: drop to the newest two.
        assertEquals(4, jb.popId())
        assertEquals(5, jb.popId())
    }

    @Test
    fun hugeJumpRestartsBuffering() {
        val jb = JitterBuffer(2, 5)
        jb.push(1, p(1))
        jb.push(2, p(2))
        assertEquals(1, jb.popId())
        jb.push(1000, p(9))
        assertFalse(jb.isPlaying)
        jb.push(1001, p(10))
        assertEquals(9, jb.popId())
    }

    @Test
    fun runningDryResetsToBuffering() {
        val jb = JitterBuffer(2)
        jb.push(1, p(1))
        jb.push(2, p(2))
        repeat(60) { jb.pop() }
        assertFalse(jb.isPlaying)
        jb.push(500, p(7))
        assertFalse(jb.isPlaying)
        jb.push(501, p(8))
        assertTrue(jb.isPlaying)
        assertEquals(7, jb.popId())
    }
}
