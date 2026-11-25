package com.zalom3ui.app.call

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class AnnexBTest {
    private fun b(vararg v: Int) = ByteArray(v.size) { v[it].toByte() }

    private val sps = b(0, 0, 0, 1, 0x67, 1, 2)
    private val pps = b(0, 0, 0, 1, 0x68, 3)
    private val idr = b(0, 0, 1, 0x65, 9, 9)
    private val delta = b(0, 0, 0, 1, 0x41, 7)

    @Test
    fun findsNalTypesWithShortAndLongStartCodes() {
        assertEquals(listOf(7, 8, 5), AnnexB.nalTypes(sps + pps + idr))
        assertEquals(listOf(7), AnnexB.nalTypes(sps + pps + idr, max = 1))
        assertEquals(emptyList<Int>(), AnnexB.nalTypes(b(1, 2, 3, 4)))
    }

    @Test
    fun keyFrameAndParameterSetDetection() {
        assertTrue(AnnexB.isKeyFrame(sps + pps + idr))
        assertFalse(AnnexB.isKeyFrame(delta))
        assertTrue(AnnexB.hasParameterSets(sps + pps + idr))
        assertFalse(AnnexB.hasParameterSets(idr))
    }

    @Test
    fun parameterSetsAreAddedToKeyFramesOnly() {
        val config = sps + pps
        assertArrayEquals(config + idr, AnnexB.withParameterSets(config, true, idr))
        assertSame(delta, AnnexB.withParameterSets(config, false, delta))
        val full = config + idr
        assertSame(full, AnnexB.withParameterSets(config, true, full))
        assertSame(idr, AnnexB.withParameterSets(null, true, idr))
    }
}
