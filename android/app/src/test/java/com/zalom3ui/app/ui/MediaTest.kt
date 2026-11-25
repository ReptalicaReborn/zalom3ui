package com.zalom3ui.app.ui

import com.zalom3ui.app.data.Attachment
import com.zalom3ui.app.data.ChatMessage
import com.zalom3ui.app.data.Person
import com.zalom3ui.app.data.StickerRef
import com.zalom3ui.app.data.nameWithExtension
import com.zalom3ui.app.data.needsJpegConversion
import com.zalom3ui.app.data.orientedSize
import com.zalom3ui.app.data.sampleSizeFor
import com.zalom3ui.app.server.MessageNotifier
import kotlinx.serialization.json.Json
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

class MediaTest {
    private val json = Json { ignoreUnknownKeys = true }
    // MimeTypeMap is an Android stub in unit tests.
    private val ext: (String) -> String? = { mime -> mapOf("image/jpeg" to "jpg", "image/heic" to "heic", "application/pdf" to "pdf")[mime] }

    @Test
    fun decodesTheServersAttachmentDetails() {
        val msg = json.decodeFromString<ChatMessage>(
            """
            {"id":"1","threadId":"100","threadKind":"user","ts":1,"msgType":"chat.video.msg","text":"[Video]",
             "attachment":{"kind":"video","href":"https://x/v.mp4","thumb":"https://x/v.jpg","width":720,"height":1280,"duration":12500}}
            """.trimIndent(),
        )
        val a = msg.attachment!!
        assertEquals("video", a.kind)
        assertEquals(12_500L, a.duration)
        assertTrue(a.isShown())
        assertNull("only photos, GIFs and stickers have an image", a.image)

        val sticker = json.decodeFromString<Attachment>("""{"kind":"sticker","sticker":{"id":9,"catId":1,"type":7},"thumb":"https://s/9.webp"}""")
        assertEquals(StickerRef(9, 1, 7), sticker.sticker)
        assertEquals("https://s/9.webp", sticker.image)
    }

    @Test
    fun oldAttachmentsWithoutKindKeepTheirLabel() {
        val a = Attachment(href = "https://x/y.jpg")
        assertFalse(a.isShown())
        assertNull(a.caption())
    }

    @Test
    fun photoImageFallsBackToTheFullPicture() {
        assertEquals("https://x/t.jpg", Attachment(kind = "photo", href = "https://x/y.jpg", thumb = "https://x/t.jpg").image)
        assertEquals("https://x/y.jpg", Attachment(kind = "photo", href = "https://x/y.jpg").image)
        assertEquals("Nice", Attachment(kind = "photo", href = "h", title = "Nice").caption())
        assertNull("a file's title is its name, not a caption", Attachment(kind = "file", title = "a.pdf").caption())
    }

    @Test
    fun durationsSizesAndFrames() {
        assertEquals("0:05", formatDuration(4_600))
        assertEquals("1:05", formatDuration(65_000))
        assertEquals("512 B", formatSize(512))
        assertEquals("20 KB", formatSize(20_480))
        assertEquals("1.5 MB", formatSize(1_572_864))
        assertEquals(16f / 9f, mediaAspect(1920, 1080), 0.001f)
        assertEquals("very tall photos are cropped", 0.6f, mediaAspect(100, 1000), 0.001f)
        assertEquals("unknown size", 4f / 3f, mediaAspect(null, 10), 0.001f)
    }

    @Test
    fun pickedFilesGetAnExtensionZaloUnderstands() {
        assertEquals("Báo cáo.pdf", nameWithExtension("Báo cáo.pdf", "application/pdf", ext))
        assertEquals("IMG_1.jpg", nameWithExtension("IMG_1", "image/jpeg", ext))
        assertEquals("photo.jpg", nameWithExtension(null, "image/jpeg", ext))
        assertEquals("file.pdf", nameWithExtension("", "application/pdf", ext))
        assertEquals("notes.bin", nameWithExtension("notes", "application/x-unknown", ext))
        assertEquals("a.txt", nameWithExtension("dir/a.txt", null, ext))
        assertEquals(".hidden.pdf", nameWithExtension(".hidden", "application/pdf", ext))
    }

    @Test
    fun picturesZaloSendsAsFilesAreConverted() {
        assertTrue(needsJpegConversion("IMG.heic", "image/heic"))
        assertTrue(needsJpegConversion("scan.bmp", null))
        assertFalse(needsJpegConversion("a.jpg", "image/jpeg"))
        assertFalse(needsJpegConversion("a.webp", "image/webp"))
        assertFalse("GIFs stay GIFs", needsJpegConversion("a.gif", "image/gif"))
        assertFalse(needsJpegConversion("report.pdf", "application/pdf"))
    }

    @Test
    fun rotatedPhotosAreSentWithTheirShownSize() {
        assertEquals(3000 to 4000, orientedSize(4000, 3000, 6))
        assertEquals(4000 to 3000, orientedSize(4000, 3000, 1))
        assertEquals(1, sampleSizeFor(2000, 1000, 2560))
        assertEquals(2, sampleSizeFor(6000, 4000, 2560))
        assertEquals(4, sampleSizeFor(12000, 9000, 2560))
    }

    @Test
    fun notificationsNameWhatWasSent() {
        fun msg(attachment: Attachment?, text: String = "[x]") =
            ChatMessage(id = "1", threadId = "1", threadKind = "user", ts = 1, text = text, attachment = attachment)
        assertEquals("hello", MessageNotifier.notificationText(msg(null, "hello")))
        assertEquals("📷 Photo", MessageNotifier.notificationText(msg(Attachment(kind = "photo"))))
        assertEquals("📎 a.pdf", MessageNotifier.notificationText(msg(Attachment(kind = "file", title = "a.pdf"))))
        assertEquals("🎤 Voice message", MessageNotifier.notificationText(msg(Attachment(kind = "voice"))))
        assertEquals("[Link]", MessageNotifier.notificationText(msg(Attachment(kind = "link"), "[Link]")))
    }

    @Test
    fun friendSearchIgnoresDiacritics() {
        val people = listOf(Person("1", "Đức Anh"), Person("2", "Bình"), Person("3", "an"))
        assertEquals(listOf("an", "Đức Anh"), filterPeople(people, "an").map { it.name })
        assertEquals(listOf("Đức Anh"), filterPeople(people, "duc").map { it.name })
        assertEquals(3, filterPeople(people, " ").size)
    }
}
