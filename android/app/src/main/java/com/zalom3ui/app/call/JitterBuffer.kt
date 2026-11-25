package com.zalom3ui.app.call

/**
 * Reorders incoming packets by 16-bit RTP sequence number and hands them out one per 20 ms tick.
 *
 * It first waits for [targetFrames] packets so small network jitter does not cause gaps, then plays in order.
 * A missing sequence number comes out as null (the decoder conceals it). Thread-safe: push from the network
 * thread, pop from the playback thread.
 */
class JitterBuffer(private val targetFrames: Int = 3, private val maxFrames: Int = 25) {
    /** Result of an atomic [popIfPlaying] tick. */
    sealed class PopResult {
        /** Buffer is still filling to [targetFrames]; play silence without counting concealment. */
        data object Buffering : PopResult()

        /** Playout slot is active: [packet] is the Opus payload, or null when missing (conceal). */
        class Frame(val packet: ByteArray?) : PopResult()
    }

    private val lock = Any()
    private val packets = HashMap<Int, ByteArray>()
    private val hardResetAhead = (maxFrames * RESET_MULTIPLIER).coerceAtMost(HALF - 1)

    private var playing = false

    /** While buffering: the oldest sequence seen. While playing: the next sequence to hand out. */
    private var next = 0
    private var dryPops = 0

    /** True once enough packets arrived and [pop] is returning real playout slots (frames or losses). */
    val isPlaying: Boolean get() = synchronized(lock) { playing }

    fun push(seq: Int, packet: ByteArray) {
        val s = seq and MASK
        synchronized(lock) {
            if (packets.isEmpty() && !playing) {
                next = s
            } else {
                val ahead = (s - next) and MASK
                if (ahead >= HALF) {
                    // Older than the playout point. While buffering, it may still be the new oldest (reordering).
                    if (playing) return
                    next = s
                    // Prune packets that are now too far ahead of the rewound start so a buffering gap can't hang PLC.
                    packets.keys.removeAll { ((it - next) and MASK) > maxFrames }
                } else if (!playing && ahead > maxFrames) {
                    // While buffering, keep the window bounded to maxFrames behind the newest sequence.
                    packets.keys.removeAll { ((s - it) and MASK) > maxFrames }
                    next = packets.keys.maxByOrNull { (s - it) and MASK } ?: s
                } else if (playing && ahead > hardResetAhead) {
                    // A jump this large is not jitter or clock drift: the stream restarted or we lost it. Re-buffer from here.
                    packets.clear()
                    playing = false
                    dryPops = 0
                    next = s
                }
            }
            if (packets.containsKey(s)) return
            packets[s] = packet
            if (!playing && packets.size >= targetFrames) {
                playing = true
                dryPops = 0
            }
        }
    }

    /** Atomically checks whether playout is active and pops the next slot, avoiding an [isPlaying]/[pop] TOCTOU race. */
    fun popIfPlaying(): PopResult = synchronized(lock) {
        if (!playing) return PopResult.Buffering
        PopResult.Frame(popLocked())
    }

    /** Next frame in order, or null when it is missing or the buffer has not started playing yet. */
    fun pop(): ByteArray? = synchronized(lock) {
        if (!playing) return null
        popLocked()
    }

    private fun popLocked(): ByteArray? {
        if (packets.size > maxFrames) skipAhead()
        val out = packets.remove(next)
        next = (next + 1) and MASK
        if (packets.isEmpty()) {
            if (++dryPops >= DRY_RESET) {
                playing = false
                dryPops = 0
            }
        } else {
            dryPops = 0
        }
        return out
    }

    /** Drops the oldest frames so only [targetFrames] remain, bounding latency after a burst or clock drift. */
    private fun skipAhead() {
        val sorted = packets.keys.sortedBy { (it - next) and MASK }
        val keep = targetFrames.coerceIn(1, sorted.size)
        for (k in sorted.dropLast(keep)) packets.remove(k)
        next = sorted[sorted.size - keep]
    }

    private companion object {
        const val MASK = 0xFFFF
        const val HALF = 0x8000
        const val DRY_RESET = 50
        const val RESET_MULTIPLIER = 3
    }
}
