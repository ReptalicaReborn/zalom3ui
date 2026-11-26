package com.zalom3ui.app.ui

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class PredictiveBackTest {
    @Test
    fun onlyAPoppedScreenSlidesAway() {
        assertTrue("mid-gesture the leaving screen is still on top", leavingByBack(isTopEntry = true, stillOnBackStack = true))
        assertTrue("after back the leaving screen is off the back stack", leavingByBack(isTopEntry = false, stillOnBackStack = false))
        assertFalse("a screen covered by a newly opened one stays put", leavingByBack(isTopEntry = false, stillOnBackStack = true))
    }

    @Test
    fun scrimClearsAsTheScreenLeaves() {
        assertEquals(0.18f, backScrimAlpha(0f), 1e-6f)
        assertEquals(0.09f, backScrimAlpha(0.5f), 1e-6f)
        assertEquals(0f, backScrimAlpha(1f), 1e-6f)
        assertEquals("progress is clamped", 0f, backScrimAlpha(1.2f), 1e-6f)
    }

    @Test
    fun cornersRoundEarlyInTheDrag() {
        assertEquals(0f, backCornerFraction(0f), 1e-6f)
        assertEquals(0.6f, backCornerFraction(0.05f), 1e-6f)
        assertEquals(1f, backCornerFraction(0.2f), 1e-6f)
    }
}
