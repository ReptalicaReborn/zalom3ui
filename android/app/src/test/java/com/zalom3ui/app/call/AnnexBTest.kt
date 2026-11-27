package com.zalom3ui.app.call

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class AnnexBTest {
    private fun b(vararg v: Int) = ByteArray(v.size) { v[it].toByte() }

    private val aud = b(0, 0, 0, 1, 0x09, 0x10)
    private val sei = b(0, 0, 1, 0x06, 0x05, 0x01)
    private val sps = b(0, 0, 0, 1, 0x67, 1, 2)
    private val pps = b(0, 0, 0, 1, 0x68, 3)
    private val idr = b(0, 0, 1, 0x65, 9, 9)
    private val delta = b(0, 0, 0, 1, 0x41, 7)

    @Test
    fun findsNalTypesWithShortAndLongStartCodes() {
        assertEquals(listOf(7, 8, 5), AnnexB.nalTypes(sps + pps + idr))
        assertEquals(listOf(7), AnnexB.nalTypes(sps + pps + idr, max = 1))
        assertEquals(emptyList<Int>(), AnnexB.nalTypes(b(1, 2, 3, 4)))
        // A 4-byte start code followed by a 0x00 header byte and 00 01 payload must not re-match the header as a start code.
        assertEquals(listOf(0), AnnexB.nalTypes(b(0, 0, 0, 1, 0x00, 0, 1, 0x65)))
    }

    @Test
    fun keyFrameAndParameterSetDetection() {
        assertTrue(AnnexB.isKeyFrame(sps + pps + idr))
        assertFalse(AnnexB.isKeyFrame(delta))
        assertTrue(AnnexB.hasParameterSets(sps + pps + idr))
        assertFalse(AnnexB.hasParameterSets(idr))
        assertFalse(AnnexB.hasParameterSets(sps + idr))
        assertFalse(AnnexB.hasParameterSets(pps + idr))
    }

    @Test
    fun detectsParameterSetsPrecededByAudAndMultipleSeis() {
        val au = aud + sei + sei + sei + sps + pps + idr
        assertTrue(AnnexB.hasParameterSets(au))
        assertTrue(AnnexB.isKeyFrame(au))
        val config = sps + pps
        assertSame(au, AnnexB.withParameterSets(config, true, au))
    }

    @Test
    fun parameterSetsAreAddedToKeyFramesOnly() {
        val config = sps + pps
        assertArrayEquals(config + idr, AnnexB.withParameterSets(config, true, idr))
        assertArrayEquals(config + sps + idr, AnnexB.withParameterSets(config, true, sps + idr))
        assertSame(delta, AnnexB.withParameterSets(config, false, delta))
        val full = config + idr
        assertSame(full, AnnexB.withParameterSets(config, true, full))
        assertSame(idr, AnnexB.withParameterSets(null, true, idr))
    }

    private fun hex(h: String) = ByteArray(h.length / 2) { h.substring(it * 2, it * 2 + 2).toInt(16).toByte() }

    @Test
    fun splitsNalUnitsAndFindsByType() {
        val units = AnnexB.nalUnits(sps + pps + idr)
        assertEquals(3, units.size)
        assertArrayEquals(b(0x67, 1, 2), units[0])
        assertArrayEquals(b(0x68, 3), units[1])
        assertArrayEquals(b(0x65, 9, 9), units[2])
        assertArrayEquals(b(0x68, 3), AnnexB.findNal(sps + pps + idr, 8))
        assertEquals(null, AnnexB.findNal(delta, 8))
        assertEquals(emptyList<ByteArray>().size, AnnexB.nalUnits(b(1, 2, 3)).size)
        assertArrayEquals(b(0, 0, 0, 1, 0x68, 3), AnnexB.withStartCode(b(0x68, 3)))
    }

    @Test
    fun readsPictureSizeFromSps() {
        assertEquals(Pair(640, 480), AnnexB.spsSize(hex("6742001eda0280f640")))
        assertEquals(Pair(368, 640), AnnexB.spsSize(hex("6742001eda05c14640")))
        // High profile with a bottom crop of 8 lines: 1920x1088 -> 1920x1080.
        assertEquals(Pair(1920, 1080), AnnexB.spsSize(hex("6764001eacb403c0113f2a")))
        assertEquals(null, AnnexB.spsSize(b(0x68, 1, 2, 3)))
        assertEquals(null, AnnexB.spsSize(b(0x67, 0x42)))
    }
}
