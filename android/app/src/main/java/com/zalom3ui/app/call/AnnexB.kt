package com.zalom3ui.app.call

/** Small helpers for H.264 in Annex B form (start codes between NAL units). */
object AnnexB {
    private const val NAL_IDR = 5
    private const val NAL_SPS = 7

    /** NAL unit types in [data], in order (the 5 low bits of the byte after each 00 00 01 start code). */
    fun nalTypes(data: ByteArray, max: Int = Int.MAX_VALUE): List<Int> {
        val out = ArrayList<Int>()
        var i = 0
        while (i + 3 < data.size && out.size < max) {
            if (data[i].toInt() == 0 && data[i + 1].toInt() == 0 && data[i + 2].toInt() == 1) {
                out += data[i + 3].toInt() and 0x1F
                i += 3
            } else {
                i++
            }
        }
        return out
    }

    fun hasParameterSets(au: ByteArray): Boolean = NAL_SPS in nalTypes(au, max = 3)

    fun isKeyFrame(au: ByteArray): Boolean = NAL_IDR in nalTypes(au)

    /** Puts the SPS/PPS [config] in front of a key frame that doesn't carry them; other frames pass through. */
    fun withParameterSets(config: ByteArray?, key: Boolean, au: ByteArray): ByteArray {
        if (!key || config == null || config.isEmpty() || hasParameterSets(au)) return au
        return config + au
    }
}
