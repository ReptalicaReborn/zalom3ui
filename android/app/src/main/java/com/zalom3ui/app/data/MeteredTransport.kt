package com.zalom3ui.app.data

import com.zalom3ui.app.perf.CoreMeter
import com.zalom3ui.app.perf.PerfCounters
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.onEach

/**
 * Counts what goes over [delegate] for the performance monitor (see docs/performance-monitor.md): requests, how long
 * each took, how many are in flight, and the events that arrive. It is the one place both [NativeTransport] and
 * [HttpTransport] go through (ApiClient wraps its transport in it), and it changes nothing about a call: the answer,
 * the errors and cancellation pass through as they are.
 */
internal class MeteredTransport(
    private val delegate: Transport,
    private val meter: CoreMeter = PerfCounters.core,
) : Transport {
    override suspend fun execute(method: String, path: String, body: ApiBody, upload: Boolean): TransportResponse {
        val startNs = meter.requestStarted()
        var failed = true
        var bytesDown = 0L
        try {
            val response = delegate.execute(method, path, body, upload)
            failed = false
            bytesDown = response.body.length.toLong() // characters, close enough to bytes for a counter
            return response
        } finally {
            // A call that is cancelled counts as failed too: it did not get an answer.
            meter.requestFinished(startNs, timed = !upload, failed = failed, bytesUp = bytesSent(body), bytesDown = bytesDown)
        }
    }

    override fun events(): Flow<String> = delegate.events().onEach { meter.eventReceived(it.length) }

    private fun bytesSent(body: ApiBody): Long = when (body) {
        ApiBody.None -> 0L
        is ApiBody.Bytes -> body.bytes.size.toLong()
        is ApiBody.Stream -> maxOf(body.length, 0L)
        // Not counted: the file's size would be a disk access, possibly on the main thread.
        is ApiBody.VoiceFile -> 0L
    }
}
