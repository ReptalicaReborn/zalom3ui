package com.zalom3ui.app.data

import java.util.concurrent.atomic.AtomicInteger

/**
 * Counts work that must not be cut off by pausing the Zalo listener (a photo, file or voice message being uploaded), so
 * the "Stay connected" logic postpones the pause until it is done.
 */
object InFlightWork {
    private val count = AtomicInteger(0)

    /** How many tracked jobs are running. */
    val active: Int get() = count.get()

    fun begin() {
        count.incrementAndGet()
    }

    fun end() {
        // Never below zero, whatever the callers do.
        count.updateAndGet { if (it > 0) it - 1 else 0 }
    }

    /** Runs [block], counted as in flight for its whole duration (also when it throws or is cancelled). */
    suspend inline fun <T> track(block: () -> T): T {
        begin()
        try {
            return block()
        } finally {
            end()
        }
    }
}
