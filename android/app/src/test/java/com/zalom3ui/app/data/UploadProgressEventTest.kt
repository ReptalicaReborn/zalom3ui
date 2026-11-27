package com.zalom3ui.app.data

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class UploadProgressEventTest {
    private val api = ApiClient("127.0.0.1:3000", null)

    @Test
    fun uploadProgress_isParsed() {
        val event = api.parseEvent(
            """{"type":"upload_progress","threadKind":"group","threadId":"g1","uploadId":"u-7","sent":1024,"total":4096,"done":false}""",
        )
        assertEquals(ServerEvent.UploadProgress(ThreadRef("group", "g1"), "u-7", 1024L, 4096L, false), event)
    }

    @Test
    fun uploadProgress_doneAndMissingFieldsDefault() {
        val event = api.parseEvent(
            """{"type":"upload_progress","threadKind":"user","threadId":"42","uploadId":"x","done":true}""",
        )
        assertEquals(ServerEvent.UploadProgress(ThreadRef("user", "42"), "x", 0L, 0L, true), event)
    }

    @Test
    fun uploadProgress_withoutId_isIgnored() {
        assertNull(api.parseEvent("""{"type":"upload_progress","threadKind":"user","threadId":"42","sent":1,"total":2}"""))
    }
}
