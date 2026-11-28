package com.zalom3ui.app.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class VideoUploadTest {
    @Test
    fun rotatedVideoSize_swapsForSidewaysRotations() {
        assertEquals(1920 to 1080, rotatedVideoSize(1920, 1080, 0))
        assertEquals(1080 to 1920, rotatedVideoSize(1920, 1080, 90))
        assertEquals(1920 to 1080, rotatedVideoSize(1920, 1080, 180))
        assertEquals(1080 to 1920, rotatedVideoSize(1920, 1080, 270))
        assertEquals(1080 to 1920, rotatedVideoSize(1920, 1080, -90))
    }

    @Test
    fun isVideoFile_byMimeTypeOrExtension() {
        assertTrue(isVideoFile("905.mp4", null))
        assertTrue(isVideoFile("clip.MOV", "application/octet-stream"))
        assertTrue(isVideoFile("noext", "video/x-matroska"))
        assertFalse(isVideoFile("a.jpg", "image/jpeg"))
        assertFalse(isVideoFile("notes.pdf", null))
    }

    @Test
    fun sendsAsVideoMessage_onlyMp4() {
        assertTrue(sendsAsVideoMessage("905.mp4"))
        assertTrue(sendsAsVideoMessage("905.MP4"))
        assertFalse(sendsAsVideoMessage("a.mov"))
    }

    @Test
    fun attachmentQuery_videoCarriesMetadataAndThumbSize() {
        val video = VideoInfo(12_500, 720, 1280, ByteArray(7))
        assertEquals(
            listOf("name" to "a.mp4", "width" to "720", "height" to "1280", "durationMs" to "12500", "thumbSize" to "7"),
            attachmentQuery("a.mp4", null, null, video),
        )
    }

    @Test
    fun attachmentQuery_withoutVideoIsAsBefore() {
        assertEquals(listOf("name" to "a.jpg", "width" to "4", "height" to "3"), attachmentQuery("a.jpg", 4, 3, null))
        assertEquals(listOf("name" to "a.mp4"), attachmentQuery("a.mp4", null, null, null))
        assertEquals(listOf("name" to "a.mp4"), attachmentQuery("a.mp4", null, null, VideoInfo(1, 1, 1, ByteArray(0))))
    }
}
