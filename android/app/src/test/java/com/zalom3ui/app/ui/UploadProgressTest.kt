package com.zalom3ui.app.ui

import com.zalom3ui.app.data.ServerEvent
import com.zalom3ui.app.data.ThreadRef
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Test

class UploadProgressTest {
    private val ref = ThreadRef("user", "1")
    private fun ev(id: String, sent: Long, total: Long, done: Boolean = false, thread: ThreadRef = ref) =
        ServerEvent.UploadProgress(thread, id, sent, total, done)

    @Test
    fun noEntries_isNull() {
        assertNull(aggregateUploadProgress(null))
        assertNull(aggregateUploadProgress(emptyMap()))
        assertNull(aggregateUploadProgress(mapOf("a" to UploadBytes(0, 0))))
    }

    @Test
    fun sumsBytesAcrossUploads() {
        var m: UploadsByChat = emptyMap()
        m = applyUploadProgress(m, ev("a", 50, 100))
        m = applyUploadProgress(m, ev("b", 0, 300))
        assertEquals(0.125f, aggregateUploadProgress(m[ref.key])!!, 1e-6f)
        m = applyUploadProgress(m, ev("b", 300, 300))
        assertEquals(350f / 400f, aggregateUploadProgress(m[ref.key])!!, 1e-6f)
    }

    @Test
    fun doneDropsEntryAndEmptyChat() {
        var m = applyUploadProgress(emptyMap(), ev("a", 10, 10))
        m = applyUploadProgress(m, ev("a", 10, 10, done = true))
        assertTrue(m.isEmpty())
        assertTrue(uploadProgressByChat(m).isEmpty())
    }

    @Test
    fun unchangedEventReturnsSameMap() {
        val m = applyUploadProgress(emptyMap(), ev("a", 1, 10))
        assertSame(m, applyUploadProgress(m, ev("a", 1, 10)))
        assertSame(m, applyUploadProgress(m, ev("zzz", 1, 10, done = true)))
    }

    @Test
    fun chatsAreIndependent_andClearIsPerChat() {
        val other = ThreadRef("group", "9")
        var m = applyUploadProgress(emptyMap(), ev("a", 1, 4))
        m = applyUploadProgress(m, ev("a", 3, 4, thread = other))
        val p = uploadProgressByChat(m)
        assertEquals(0.25f, p[ref.key]!!, 1e-6f)
        assertEquals(0.75f, p[other.key]!!, 1e-6f)
        m = clearUploads(m, ref.key)
        assertEquals(setOf(other.key), m.keys)
    }

    @Test
    fun clampsOvershootAndNegatives() {
        val m = applyUploadProgress(emptyMap(), ev("a", 500, 100))
        assertEquals(1f, aggregateUploadProgress(m[ref.key])!!, 0f)
        val n = applyUploadProgress(emptyMap(), ev("a", -5, 100))
        assertEquals(0f, aggregateUploadProgress(n[ref.key])!!, 0f)
    }
}
