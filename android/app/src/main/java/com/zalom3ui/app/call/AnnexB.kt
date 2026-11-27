package com.zalom3ui.app.call

/** Small helpers for H.264 in Annex B form (start codes between NAL units). */
object AnnexB {
    private const val NAL_IDR = 5
    private const val NAL_SPS = 7
    private const val NAL_PPS = 8
    private const val PARAMETER_SET_SCAN_LIMIT = 16

    /** NAL unit types in [data], in order (the 5 low bits of the byte after each 00 00 01 or 00 00 00 01 start code). */
    fun nalTypes(data: ByteArray, max: Int = Int.MAX_VALUE): List<Int> {
        val out = ArrayList<Int>()
        var i = 0
        while (i + 3 < data.size && out.size < max) {
            if (data[i].toInt() == 0 && data[i + 1].toInt() == 0) {
                if (data[i + 2].toInt() == 1) {
                    out += data[i + 3].toInt() and 0x1F
                    i += 4
                    continue
                }
                if (i + 4 < data.size && data[i + 2].toInt() == 0 && data[i + 3].toInt() == 1) {
                    out += data[i + 4].toInt() and 0x1F
                    i += 5
                    continue
                }
            }
            i++
        }
        return out
    }

    fun hasParameterSets(au: ByteArray): Boolean {
        var hasSps = false
        var hasPps = false
        for (nalType in nalTypes(au, max = PARAMETER_SET_SCAN_LIMIT)) {
            when (nalType) {
                NAL_SPS -> hasSps = true
                NAL_PPS -> hasPps = true
                in 1..NAL_IDR -> break
            }
            if (hasSps && hasPps) return true
        }
        return false
    }

    fun isKeyFrame(au: ByteArray): Boolean = NAL_IDR in nalTypes(au)

    /** Puts the SPS/PPS [config] in front of a key frame that doesn't carry them; other frames pass through. */
    fun withParameterSets(config: ByteArray?, key: Boolean, au: ByteArray): ByteArray {
        if (!key || config == null || config.isEmpty() || hasParameterSets(au)) return au
        return config + au
    }

    /** The NAL units of [data] without start codes (trailing zero bytes trimmed), in order. */
    fun nalUnits(data: ByteArray): List<ByteArray> {
        val out = ArrayList<ByteArray>()
        var start = -1
        var i = 0
        while (i + 2 < data.size) {
            if (data[i].toInt() == 0 && data[i + 1].toInt() == 0 && data[i + 2].toInt() == 1) {
                if (start >= 0) addNal(out, data, start, i)
                start = i + 3
                i += 3
            } else {
                i++
            }
        }
        if (start >= 0) addNal(out, data, start, data.size)
        return out
    }

    private fun addNal(out: MutableList<ByteArray>, data: ByteArray, start: Int, endExclusive: Int) {
        var end = endExclusive
        while (end > start && data[end - 1].toInt() == 0) end--
        if (end > start) out += data.copyOfRange(start, end)
    }

    /** The first NAL unit of [type] in [data] (without start code), or null. */
    fun findNal(data: ByteArray, type: Int): ByteArray? = nalUnits(data).firstOrNull { (it[0].toInt() and 0x1F) == type }

    /** [nal] with a 4-byte start code in front, the form MediaCodec wants for csd-0 / csd-1. */
    fun withStartCode(nal: ByteArray): ByteArray = byteArrayOf(0, 0, 0, 1) + nal

    /** Picture width and height (after cropping) from an SPS NAL unit (with its header byte), or null if it can't be read. */
    fun spsSize(sps: ByteArray): Pair<Int, Int>? {
        if (sps.size < 4 || (sps[0].toInt() and 0x1F) != NAL_SPS) return null
        // Drop emulation prevention bytes (00 00 03 -> 00 00) after the header byte.
        val b = ArrayList<Int>(sps.size)
        var zeros = 0
        for (k in 1 until sps.size) {
            val v = sps[k].toInt() and 0xFF
            if (zeros >= 2 && v == 3) {
                zeros = 0
                continue
            }
            zeros = if (v == 0) zeros + 1 else 0
            b += v
        }
        var pos = 0
        fun bit(): Int {
            if (pos >= b.size * 8) throw IllegalStateException("eof")
            val v = (b[pos shr 3] shr (7 - (pos and 7))) and 1
            pos++
            return v
        }
        fun bits(n: Int): Long {
            var v = 0L
            repeat(n) { v = v * 2 + bit() }
            return v
        }
        fun ue(): Long {
            var z = 0
            while (bit() == 0) if (++z > 31) throw IllegalStateException("ue")
            return bits(z) + ((1L shl z) - 1)
        }
        fun se(): Long {
            val k = ue()
            return if ((k and 1L) == 1L) (k + 1) / 2 else -(k / 2)
        }
        return try {
            val profile = bits(8).toInt()
            bits(16)
            ue()
            var chroma = 1L
            if (profile in HIGH_PROFILES) {
                chroma = ue()
                if (chroma == 3L) bit()
                ue()
                ue()
                bit()
                if (bit() == 1) {
                    for (n in 0 until (if (chroma == 3L) 12 else 8)) {
                        if (bit() == 0) continue
                        var last = 8L
                        var next = 8L
                        for (j in 0 until (if (n < 6) 16 else 64)) {
                            if (next != 0L) next = (last + se() + 256) % 256
                            last = if (next != 0L) next else last
                        }
                    }
                }
            }
            ue()
            val poc = ue()
            if (poc == 0L) {
                ue()
            } else if (poc == 1L) {
                bit()
                se()
                se()
                val n = ue()
                for (j in 0 until n) se()
            }
            ue()
            bit()
            val wMbs = ue() + 1
            val hMap = ue() + 1
            val frameMbsOnly = bit()
            if (frameMbsOnly == 0) bit()
            bit()
            var cl = 0L
            var cr = 0L
            var ct = 0L
            var cb = 0L
            if (bit() == 1) {
                cl = ue()
                cr = ue()
                ct = ue()
                cb = ue()
            }
            val cx = if (chroma == 0L) 1 else 2
            val cy = (if (chroma == 1L) 2 else 1) * (2 - frameMbsOnly)
            val w = wMbs * 16 - cx * (cl + cr)
            val h = (2 - frameMbsOnly) * hMap * 16 - cy * (ct + cb)
            if (w in 1..16384 && h in 1..16384) Pair(w.toInt(), h.toInt()) else null
        } catch (_: IllegalStateException) {
            null
        }
    }

    private val HIGH_PROFILES = setOf(100, 110, 122, 244, 44, 83, 86, 118, 128, 138, 139, 134, 135)
}
