package com.zalom3ui.app.ui

/**
 * One-time code detection for message text and notification text. The same algorithm and the same test vectors as the
 * server's `extract_otp_code` (`rust/zalo-server/src/store.rs`); keep them in step. The server's answer (`otp` on the
 * message) is preferred in the UI; this is the fallback and the only detector for the official Zalo app's notifications.
 *
 * It works on diacritic-folded lowercase text, one line at a time, without look-around:
 *  1. a strong keyword (OTP, "ma xac thuc", "verification code", ...) followed, in the same sentence and within 100
 *     characters, by the first code-like number;
 *  2. a weak keyword that must be directly followed by the code ("code: 123456", "your code is 123456", "ma: 123456",
 *     "ma cua ban la 1234");
 *  3. a code-like number directly before a keyword ("739201 la ma xac nhan", "123456 is your verification code",
 *     "G-123456 is your Google verification code");
 *  4. a line that is only a code, right under a line with a strong keyword.
 *
 * Code-like: 4-8 plain digits ("482910") or two groups of three ("482 910", "482-910", returned as "482910"). Not
 * code-like: amounts ("5.000.000", "5000d"), dates, times, 9+ digit numbers (phone and account numbers), numbers after
 * tk/stk/sdt/hotline/..., and a year after "nam"/"year".
 */
private const val OTP_STRONG =
    """\b(?:otp|ma\s*(?:xac\s*(?:thuc|nhan)|bao\s*mat|dang\s*nhap|kich\s*hoat|xac\s*minh|giao\s*dich|pin)|verification\s*code|security\s*code|auth(?:entication)?\s*code|confirm(?:ation)?\s*code|passcode|mat\s*khau\s*(?:dung\s*)?mot\s*lan|one[- ]time(?:\s*(?:code|password|pin))?)\b"""
private const val OTP_WEAK =
    """(?:^|[.!?:;,]\s*|\byour\s+|\bur\s+|\bsms\s+|\bthe\s+)code\s*(?::|\bis\b)\s*(?:g-)?|\bma\s*:\s*(?:g-)?|\bma\s+cua\s+(?:ban|quy\s+khach|minh)\s*(?:\bla\b|:)\s*(?:g-)?"""
private const val OTP_BEFORE =
    """\b(?:(?:la|is)\s+(?:your|the)\s+(?:[a-z]+\s+)?(?:(?:verification|security|login|confirmation|authentication)\s+)?code|(?:la\s+)?(?:otp|ma\s*(?:xac\s*(?:thuc|nhan)|bao\s*mat|dang\s*nhap|kich\s*hoat|xac\s*minh|giao\s*dich)|verification\s*code|security\s*code|passcode)|la\s+ma\s+cua\s+ban)\b"""

private val OTP_STRONG_RE = Regex(OTP_STRONG)
private val OTP_WEAK_RE = Regex(OTP_WEAK)
private val OTP_BEFORE_RE = Regex(OTP_BEFORE)
private val OTP_TOKEN_RE = Regex("""[0-9]+(?:[.,/:\-][0-9]+)*""")
private val OTP_UNIT_AFTER_RE = Regex("""^\s*(?:d|vnd|dong|usd|eur|k|tr|trieu|nghin|ngan|ty|%)(?:[^a-z0-9]|$)""")
private val OTP_ACCOUNT_BEFORE_RE = Regex(
    """\b(?:tk|stk|sdt|hotline|tel|phone|account|acc|a/c|card|cmnd|cccd|mst|tai\s+khoan|so\s+tk|so\s+the)\s*(?:so\b|no\b|number\b)?\s*[:#.]?\s*$""",
)
private val OTP_YEAR_BEFORE_RE = Regex("""\b(?:nam|year)\s*$""")
private val OTP_SENTENCE_END_RE = Regex("""[.!?](?:\s|$)""")
private const val OTP_WINDOW = 100
private const val OTP_BEFORE_WINDOW = 24

private class OtpToken(val start: Int, val end: Int)

private fun allDigits(s: String): Boolean = s.isNotEmpty() && s.all { it in '0'..'9' }

private fun isAsciiLetter(c: Char): Boolean = c in 'a'..'z' || c in 'A'..'Z'

/** The code a token stands for, or null if it is not code-like. */
private fun otpCandidate(line: String, toks: List<OtpToken>, i: Int): String? {
    val t = toks[i]
    val text = line.substring(t.start, t.end)
    val prefix = line.substring(0, t.start)

    // Glued to letters ("abc1234", "1234abc"): not a standalone code.
    if (t.start > 0 && isAsciiLetter(line[t.start - 1])) return null
    if (t.end < line.length && isAsciiLetter(line[t.end])) return null
    if (prefix.endsWith("$")) return null

    val code: String
    if (allDigits(text) && text.length in 4..8) {
        code = text
    } else if (text.length == 7 && text[3] == '-' && allDigits(text.replace("-", "")) && text.split('-').all { it.length == 3 }) {
        code = text.replace("-", "")
    } else if (allDigits(text) && text.length == 3) {
        // "482 910": two groups of three, one space apart, and not part of a longer chain.
        val next = toks.getOrNull(i + 1) ?: return null
        val nextText = line.substring(next.start, next.end)
        val gapOk = next.start == t.end + 1 && line[t.end] == ' '
        if (!(gapOk && allDigits(nextText) && nextText.length == 3)) return null
        val chainedBefore = i > 0 && toks[i - 1].let { p ->
            p.end + 1 == t.start && line[p.end] == ' ' && allDigits(line.substring(p.start, p.end)) && p.end - p.start <= 3
        }
        val chainedAfter = toks.getOrNull(i + 2)?.let { n ->
            n.start == next.end + 1 && line[next.end] == ' ' && allDigits(line.substring(n.start, n.end)) && n.end - n.start == 3
        } == true
        if (chainedBefore || chainedAfter) return null
        code = text + nextText
    } else {
        return null
    }

    // The token that ends the code (the second group for "482 910").
    val end = if (text.length == 3 && allDigits(text)) toks[i + 1].end else t.end
    if (end < line.length && isAsciiLetter(line[end])) return null
    if (end < line.length && (line[end] == '.' || line[end] == ',') && end + 1 < line.length && line[end + 1] in '0'..'9') return null
    if (OTP_UNIT_AFTER_RE.containsMatchIn(line.substring(end))) return null
    if (OTP_ACCOUNT_BEFORE_RE.containsMatchIn(prefix)) return null
    if (code.length == 4 && (code.startsWith("19") || code.startsWith("20")) && OTP_YEAR_BEFORE_RE.containsMatchIn(prefix)) return null
    return code
}

private fun otpInLine(line: String): String? {
    val toks = OTP_TOKEN_RE.findAll(line).map { OtpToken(it.range.first, it.range.last + 1) }.toList()
    if (toks.isEmpty()) return null

    // 1. strong keyword, then the first code-like number in the same sentence.
    for (m in OTP_STRONG_RE.findAll(line)) {
        val mEnd = m.range.last + 1
        val limit = minOf(line.length, mEnd + OTP_WINDOW)
        for ((i, t) in toks.withIndex()) {
            if (t.start < mEnd || t.start > limit) continue
            if (OTP_SENTENCE_END_RE.containsMatchIn(line.substring(mEnd, t.start))) break
            otpCandidate(line, toks, i)?.let { return it }
        }
    }

    // 2. weak keyword directly followed by the code.
    for (m in OTP_WEAK_RE.findAll(line)) {
        val mEnd = m.range.last + 1
        val i = toks.indexOfFirst { it.start == mEnd }
        if (i >= 0) otpCandidate(line, toks, i)?.let { return it }
    }

    // 3. a code directly before a keyword.
    for (m in OTP_BEFORE_RE.findAll(line)) {
        val mStart = m.range.first
        val i = toks.indexOfLast { it.end <= mStart }
        if (i < 0) continue
        // "482 910 la ..." ends on the second group; step back to the first.
        val first = if (i > 0 && toks[i].end - toks[i].start == 3 && toks[i - 1].end + 1 == toks[i].start) i - 1 else i
        val gap = line.substring(toks[i].end, mStart)
        if (gap.length > OTP_BEFORE_WINDOW || OTP_SENTENCE_END_RE.containsMatchIn(gap)) continue
        otpCandidate(line, toks, first)?.let { return it }
    }
    return null
}

/** Extracts a one-time code (4-8 digits, digits only) from message or notification text, or null. */
internal fun extractOtpCode(text: String): String? {
    if (text.isBlank()) return null
    val folded = foldForSearch(text)
    var prevStrong = false
    for (raw in folded.lines()) {
        val line = raw.trim()
        otpInLine(line)?.let { return it }
        // A line that is only a code, under a line with a strong keyword.
        if (prevStrong && allDigits(line)) {
            otpCandidate(line, listOf(OtpToken(0, line.length)), 0)?.let { return it }
        }
        prevStrong = OTP_STRONG_RE.containsMatchIn(line)
    }
    return null
}
