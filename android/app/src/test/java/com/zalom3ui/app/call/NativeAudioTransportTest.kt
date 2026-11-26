package com.zalom3ui.app.call

import com.zalom3ui.app.data.CoreNatives
import com.zalom3ui.app.data.FakeNatives
import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicBoolean

/** A scripted call-audio link in the core: frames to hand out, then a close with a reason. */
private class FakeAudioNatives(private val openResult: Long = 5L) : CoreNatives by FakeNatives() {
    val frames = LinkedBlockingQueue<ByteArray>()
    val sent = CopyOnWriteArrayList<ByteArray>()
    val closed = AtomicBoolean(false)
    @Volatile var reason: String? = null
    val closedHandles = CopyOnWriteArrayList<Long>()

    override fun audioOpen(): Long = openResult

    override fun audioSend(handle: Long, data: ByteArray): Boolean {
        if (closed.get()) return false
        sent += data
        return true
    }

    override fun audioNext(handle: Long, timeoutMs: Long): ByteArray? {
        val deadline = System.nanoTime() + TimeUnit.MILLISECONDS.toNanos(timeoutMs)
        while (System.nanoTime() < deadline) {
            frames.poll(5, TimeUnit.MILLISECONDS)?.let { return it }
            if (closed.get()) return ByteArray(0)
        }
        return null
    }

    override fun audioCloseReason(handle: Long): String? = if (closed.get()) reason else null

    override fun audioClose(handle: Long) {
        closedHandles += handle
        closed.set(true)
    }
}

private class Recorder : AudioTransport.Listener {
    val messages = LinkedBlockingQueue<ByteArray>()
    val closes = LinkedBlockingQueue<Pair<Boolean, String?>>()

    override fun onMessage(msg: ByteArray) {
        messages += msg
    }

    override fun onClosed(reason: String?) {
        closes += true to reason
    }
}

class NativeAudioTransportTest {
    @Test
    fun noActiveCall_closesWithTheSameReasonAsTheWebSocketPath() {
        val rec = Recorder()
        NativeAudioTransport(FakeAudioNatives(CoreNatives.AUDIO_NO_ACTIVE_CALL)).connect(rec)
        assertEquals("No active call", rec.closes.poll(5, TimeUnit.SECONDS)?.second)
        assertTrue(rec.messages.isEmpty())
    }

    @Test
    fun coreNotRunning_closesWithAMessage() {
        val rec = Recorder()
        NativeAudioTransport(FakeAudioNatives(-1L)).connect(rec)
        assertEquals("The Zalo core is not running", rec.closes.poll(5, TimeUnit.SECONDS)?.second)

        val rec2 = Recorder()
        NativeAudioTransport(FakeAudioNatives(), ensureStarted = { false }).connect(rec2)
        assertEquals("The Zalo core is not running", rec2.closes.poll(5, TimeUnit.SECONDS)?.second)
    }

    @Test
    fun framesFlowBothWaysAndTheCallEndClosesWithItsReason() {
        val natives = FakeAudioNatives()
        val t = NativeAudioTransport(natives)
        val rec = Recorder()
        assertFalse("not connected yet", t.send(byteArrayOf(2, 1)))

        t.connect(rec)
        natives.frames += byteArrayOf(1, 0, 1, 0, 0, 0, 9, 42)
        assertArrayEquals(byteArrayOf(1, 0, 1, 0, 0, 0, 9, 42), rec.messages.poll(5, TimeUnit.SECONDS))

        // Sends reach the core once the link is open.
        val deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(5)
        while (!t.send(byteArrayOf(2, 7, 8)) && System.nanoTime() < deadline) Thread.sleep(5)
        assertArrayEquals(byteArrayOf(2, 7, 8), natives.sent.last())
        assertEquals(0L, t.queueSize())

        natives.reason = "1000 call ended"
        natives.closed.set(true)
        assertEquals("call ended", rec.closes.poll(5, TimeUnit.SECONDS)?.second)
        assertNull("closes once", rec.closes.poll(100, TimeUnit.MILLISECONDS))
        assertTrue(natives.closedHandles.contains(5L))
        assertFalse(t.send(byteArrayOf(2, 1)))
    }

    @Test
    fun replacedByANewerClient_reportsReplaced() {
        val natives = FakeAudioNatives()
        val rec = Recorder()
        NativeAudioTransport(natives).connect(rec)
        natives.reason = "1000 replaced"
        natives.closed.set(true)
        assertEquals("replaced", rec.closes.poll(5, TimeUnit.SECONDS)?.second)
    }

    @Test
    fun closeFromTheAppEndsTheLinkAndWakesTheReader() {
        val natives = FakeAudioNatives()
        val t = NativeAudioTransport(natives)
        val rec = Recorder()
        t.connect(rec)
        natives.frames += byteArrayOf(1, 0, 1, 0, 0, 0, 1, 1)
        assertTrue(rec.messages.poll(5, TimeUnit.SECONDS) != null)

        t.close()
        assertTrue(rec.closes.poll(5, TimeUnit.SECONDS) != null)
        assertTrue(natives.closed.get())
        assertFalse(t.send(byteArrayOf(2, 1)))
    }

    @Test
    fun closeReasonText_dropsTheCode() {
        assertEquals("call ended", NativeAudioTransport.closeReasonText("1000 call ended"))
        assertNull(NativeAudioTransport.closeReasonText("1000"))
        assertNull(NativeAudioTransport.closeReasonText(null))
    }

    @Test
    fun connectIsOnlyHonouredOnce() {
        val natives = FakeAudioNatives()
        val t = NativeAudioTransport(natives)
        val rec = Recorder()
        t.connect(rec)
        t.connect(rec)
        natives.closed.set(true)
        assertTrue(rec.closes.poll(5, TimeUnit.SECONDS) != null)
        assertNull(rec.closes.poll(200, TimeUnit.MILLISECONDS))
    }
}

/** [CallAudioLink] does its framing and backlog limits the same over any transport. */
class CallAudioLinkOverTransportTest {
    private class FakeTransport : AudioTransport {
        var listener: AudioTransport.Listener? = null
        val sent = CopyOnWriteArrayList<ByteArray>()
        var queue = 0L
        var closedByApp = false

        override fun connect(listener: AudioTransport.Listener) {
            this.listener = listener
        }

        override fun send(msg: ByteArray): Boolean {
            sent += msg
            return true
        }

        override fun queueSize(): Long = queue

        override fun close() {
            closedByApp = true
        }
    }

    @Test
    fun framesAreDecodedAndDispatched() {
        val t = FakeTransport()
        val audio = mutableListOf<Triple<Int, Long, List<Byte>>>()
        val video = mutableListOf<Triple<Boolean, Long, List<Byte>>>()
        var keyWanted = 0
        val link = CallAudioLink(
            t,
            onAudio = { seq, ts, opus -> audio += Triple(seq, ts, opus.toList()) },
            onClosed = {},
            onVideo = { key, ts, au -> video += Triple(key, ts, au.toList()) },
            onKeyFrameWanted = { keyWanted++ },
        )
        link.connect()
        t.listener!!.onMessage(byteArrayOf(1, 0, 3, 0, 0, 0, 5, 9, 9))
        t.listener!!.onMessage(byteArrayOf(3, 1, 0, 0, 0, 7, 0, 0, 0, 1, 0x65))
        t.listener!!.onMessage(byteArrayOf(5))
        t.listener!!.onMessage(byteArrayOf(9, 1, 2)) // unknown: ignored
        assertEquals(listOf(Triple(3, 5L, listOf<Byte>(9, 9))), audio)
        assertEquals(listOf(Triple(true, 7L, listOf<Byte>(0, 0, 0, 1, 0x65))), video)
        assertEquals(1, keyWanted)
    }

    @Test
    fun outgoingFramesUseTheSameBinaryFraming() {
        val t = FakeTransport()
        val link = CallAudioLink(t, onAudio = { _, _, _ -> }, onClosed = {})
        link.connect()
        assertTrue(link.send(byteArrayOf(7, 8)))
        assertTrue(link.sendVideo(true, byteArrayOf(0, 0, 0, 1, 0x65)))
        assertTrue(link.sendKeyFrameNeeded())
        assertArrayEquals(byteArrayOf(2, 7, 8), t.sent[0])
        assertArrayEquals(byteArrayOf(4, 1, 0, 0, 0, 1, 0x65), t.sent[1])
        assertArrayEquals(byteArrayOf(6), t.sent[2])
    }

    @Test
    fun deltaVideoIsDroppedWhenTheLinkIsBackedUp() {
        val t = FakeTransport()
        val link = CallAudioLink(t, onAudio = { _, _, _ -> }, onClosed = {})
        link.connect()
        t.queue = CallAudioLink.MAX_VIDEO_BACKLOG + 1
        assertFalse(link.sendVideo(false, byteArrayOf(1, 2, 3)))
        assertTrue("key frames still go", link.sendVideo(true, byteArrayOf(1, 2, 3)))
        assertEquals(1, t.sent.size)
    }

    @Test
    fun closeCallbackFiresOnceWithTheReason() {
        val t = FakeTransport()
        val reasons = mutableListOf<String?>()
        val link = CallAudioLink(t, onAudio = { _, _, _ -> }, onClosed = { reasons += it })
        link.connect()
        t.listener!!.onClosed("call ended")
        t.listener!!.onClosed("again")
        assertEquals(listOf<String?>("call ended"), reasons)
        link.close()
        assertTrue(t.closedByApp)
    }

    @Test
    fun aClosedLinkDoesNotConnect() {
        val t = FakeTransport()
        val link = CallAudioLink(t, onAudio = { _, _, _ -> }, onClosed = {})
        link.close()
        link.connect()
        assertNull(t.listener)
    }
}
