package com.zalom3ui.app.data

import com.zalom3ui.app.perf.CoreMeter
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.flowOf
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import org.junit.Assert.assertEquals
import org.junit.Assert.assertSame
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Test
import java.io.IOException

/** The performance counters around a transport change nothing about a call: same answer, same errors, same events. */
class MeteredTransportTest {
    private class Fake(private val answer: () -> TransportResponse) : Transport {
        override suspend fun execute(method: String, path: String, body: ApiBody, upload: Boolean): TransportResponse = answer()

        override fun events(): Flow<String> = flowOf("""{"type":"a"}""", """{"type":"bb"}""")
    }

    @Test
    fun theAnswerPassesThroughAndIsCounted() = runBlocking {
        val meter = CoreMeter()
        val response = TransportResponse(404, """{"error":"nope"}""")
        val transport = MeteredTransport(Fake { response }, meter)
        val got = transport.execute("POST", "/api/x", ApiBody.Bytes("application/json", ByteArray(10)))
        assertSame(response, got)
        val totals = meter.drain()
        assertEquals(1L, totals.requests)
        // A 404 is an answer, not a failure of the call.
        assertEquals(0L, totals.errors)
        assertEquals(1L, totals.latencyCount)
        assertEquals(10L, totals.bytesUp)
        assertEquals(response.body.length.toLong(), totals.bytesDown)
        assertEquals(0, totals.inFlight)
    }

    @Test
    fun anErrorIsRethrownUnchangedAndCountedAsFailed() = runBlocking {
        val meter = CoreMeter()
        val boom = IOException("no route")
        val transport = MeteredTransport(Fake { throw boom }, meter)
        try {
            transport.execute("GET", "/api/x")
            fail("expected the IOException")
        } catch (e: IOException) {
            assertSame(boom, e)
        }
        val totals = meter.drain()
        assertEquals(1L, totals.requests)
        assertEquals(1L, totals.errors)
        assertEquals(0, totals.inFlight)
    }

    @Test
    fun uploadsDoNotCountForLatency() = runBlocking {
        val meter = CoreMeter()
        val transport = MeteredTransport(Fake { TransportResponse(200, "{}") }, meter)
        transport.execute("POST", "/api/upload", ApiBody.Bytes("image/jpeg", ByteArray(2048)), upload = true)
        val totals = meter.drain()
        assertEquals(1L, totals.requests)
        assertEquals(0L, totals.latencyCount)
        assertEquals(2048L, totals.bytesUp)
    }

    @Test
    fun eventsPassThroughAndAreCounted() = runBlocking {
        val meter = CoreMeter()
        val transport = MeteredTransport(Fake { TransportResponse(200, "{}") }, meter)
        assertEquals(listOf("""{"type":"a"}""", """{"type":"bb"}"""), transport.events().toList())
        val totals = meter.drain()
        assertEquals(2L, totals.events)
        assertTrue(totals.bytesDown > 0)
    }
}
