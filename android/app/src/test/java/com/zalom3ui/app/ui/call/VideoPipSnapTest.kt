package com.zalom3ui.app.ui.call

import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class VideoPipSnapTest {
    @Test
    fun snapsToTopLeftSafeCorner() {
        val (sx, sy) = snapPipCorner(
            x = 40f,
            y = 110f,
            maxX = 300f,
            maxY = 600f,
            topInset = 72f,
            bottomInset = 104f,
        )
        assertEquals(0f, sx, 0.001f)
        assertEquals(72f, sy, 0.001f)
    }

    @Test
    fun snapsToTopRightSafeCorner() {
        val (sx, sy) = snapPipCorner(
            x = 240f,
            y = 120f,
            maxX = 300f,
            maxY = 600f,
            topInset = 72f,
            bottomInset = 104f,
        )
        assertEquals(300f, sx, 0.001f)
        assertEquals(72f, sy, 0.001f)
    }

    @Test
    fun snapsToBottomLeftSafeCorner() {
        val (sx, sy) = snapPipCorner(
            x = 60f,
            y = 450f,
            maxX = 300f,
            maxY = 600f,
            topInset = 72f,
            bottomInset = 104f,
        )
        assertEquals(0f, sx, 0.001f)
        assertEquals(496f, sy, 0.001f)
    }

    @Test
    fun snapsToBottomRightSafeCorner() {
        val (sx, sy) = snapPipCorner(
            x = 260f,
            y = 510f,
            maxX = 300f,
            maxY = 600f,
            topInset = 72f,
            bottomInset = 104f,
        )
        assertEquals(300f, sx, 0.001f)
        assertEquals(496f, sy, 0.001f)
    }

    @Test
    fun respectsTopAndBottomInsetsWhenDraggedIntoChromeArea() {
        val (topX, topY) = snapPipCorner(
            x = 280f,
            y = 0f,
            maxX = 300f,
            maxY = 600f,
            topInset = 80f,
            bottomInset = 120f,
        )
        assertEquals(300f, topX, 0.001f)
        assertEquals(80f, topY, 0.001f)

        val (botX, botY) = snapPipCorner(
            x = 10f,
            y = 600f,
            maxX = 300f,
            maxY = 600f,
            topInset = 80f,
            bottomInset = 120f,
        )
        assertEquals(0f, botX, 0.001f)
        assertEquals(480f, botY, 0.001f)
    }

    @Test
    fun snapsToTrueCornersWhenChromeInsetsAreZero() {
        val (tlX, tlY) = snapPipCorner(x = 10f, y = 10f, maxX = 300f, maxY = 600f, topInset = 0f, bottomInset = 0f)
        assertEquals(0f, tlX, 0.001f)
        assertEquals(0f, tlY, 0.001f)

        val (brX, brY) = snapPipCorner(x = 290f, y = 590f, maxX = 300f, maxY = 600f, topInset = 0f, bottomInset = 0f)
        assertEquals(300f, brX, 0.001f)
        assertEquals(600f, brY, 0.001f)
    }

    @Test
    fun handlesZeroAndNegativeBoundsSafely() {
        val (zeroX, zeroY) = snapPipCorner(
            x = 50f,
            y = 120f,
            maxX = 0f,
            maxY = 0f,
            topInset = 72f,
            bottomInset = 104f,
        )
        assertEquals(0f, zeroX, 0.001f)
        assertEquals(0f, zeroY, 0.001f)

        val (negX, negY) = snapPipCorner(
            x = -20f,
            y = -30f,
            maxX = -10f,
            maxY = -5f,
            topInset = 40f,
            bottomInset = 40f,
        )
        assertEquals(0f, negX, 0.001f)
        assertEquals(0f, negY, 0.001f)
    }

    @Test
    fun handlesOverlappingInsetsGracefully() {
        val (sx, sy) = snapPipCorner(
            x = 180f,
            y = 90f,
            maxX = 200f,
            maxY = 100f,
            topInset = 80f,
            bottomInset = 80f,
        )
        assertEquals(200f, sx, 0.001f)
        assertTrue("snapped Y ($sy) stays within [0, maxY]", sy in 0f..100f)
        assertEquals(50f, sy, 0.001f)
    }
}
