package com.zalom3ui.app.perf

import android.content.Context
import android.os.Build
import java.io.File
import java.io.RandomAccessFile
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

/**
 * `perf.log`, next to `startup.log`: what the performance monitor measured (see docs/performance-monitor.md). Written
 * the way [com.zalom3ui.app.server.StartupLog] is: one background writer, a file lock, and when the file passes
 * [MAX_BYTES] only its newest half is kept. Numbers and screen names only: no tokens, messages, names or phone numbers.
 */
object PerfLog {
    private const val MAX_BYTES = 256 * 1024

    fun file(context: Context): File = File(File(context.filesDir, "zalo").apply { mkdirs() }, "perf.log")

    /** One ordered background writer, so callers (the sampler, the main thread) never wait for the file lock or the disk. */
    private val executor by lazy {
        Executors.newSingleThreadExecutor { r -> Thread(r, "perf-log").apply { isDaemon = true } }
    }

    /** Only used on the [executor] thread. */
    private val timeFormat = SimpleDateFormat("HH:mm:ss.SSS", Locale.US)
    private val dateTimeFormat = SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US)

    /** Queues [line] (stamped `HH:mm:ss.SSS`, now) for the log file; returns at once. [flush] waits until it is on disk. */
    fun write(context: Context, line: String) {
        val app = context.applicationContext ?: context
        val now = System.currentTimeMillis()
        runCatching { executor.execute { append(app, timeFormat.format(Date(now)) + " " + line) } }
    }

    /** Starts a new section of the log: a line with the full date, then [details] (what the phone and the app are). */
    fun writeHeader(context: Context, details: String) {
        val app = context.applicationContext ?: context
        val now = System.currentTimeMillis()
        val pid = android.os.Process.myPid()
        runCatching {
            executor.execute {
                append(app, "\n==== PERF LOG START ${dateTimeFormat.format(Date(now))} (pid $pid)\n$details")
            }
        }
    }

    /** Empties the log; lines written before this call are gone, lines after it start the file again. */
    fun clear(context: Context) {
        val app = context.applicationContext ?: context
        runCatching {
            executor.execute {
                runCatching {
                    RandomAccessFile(file(app), "rw").use { raf -> raf.channel.lock().use { raf.setLength(0) } }
                }
            }
        }
        flush()
    }

    /** Waits (briefly) for every line written so far to reach the file. Not for the writer thread itself. */
    fun flush() {
        runCatching { executor.submit {}.get(2, TimeUnit.SECONDS) }
    }

    private fun append(context: Context, text: String) {
        runCatching {
            RandomAccessFile(file(context), "rw").use { raf ->
                raf.channel.lock().use {
                    if (raf.length() > MAX_BYTES) {
                        val keep = MAX_BYTES / 2
                        raf.seek(raf.length() - keep)
                        val tail = ByteArray(keep)
                        raf.readFully(tail)
                        val start = LogTrim.firstLineStart(tail)
                        raf.seek(0)
                        raf.write(tail, start, tail.size - start)
                        raf.setLength((tail.size - start).toLong())
                    }
                    raf.seek(raf.length())
                    raf.write("$text\n".toByteArray(Charsets.UTF_8))
                }
            }
        }
    }

    /** Everything worth sending when comparing phones: the app, the phone and the newest part of the log. */
    fun report(context: Context): String = buildString {
        flush()
        val info = runCatching { context.packageManager.getPackageInfo(context.packageName, 0) }.getOrNull()
        appendLine("Zalỏ ${info?.versionName} (${context.packageName})")
        appendLine("Phone: ${Build.MANUFACTURER} ${Build.MODEL}, Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT}), ${Build.SUPPORTED_ABIS.joinToString()}")
        appendLine("Numbers only: no messages, names or phone numbers. Fields are explained in docs/performance-monitor.md.")
        appendLine()
        appendLine("== perf.log")
        append(runCatching { readTail(file(context), MAX_BYTES) }.getOrDefault("(none)\n"))
    }

    private fun readTail(file: File, maxBytes: Int): String {
        if (!file.isFile) return "(none)\n"
        RandomAccessFile(file, "r").use { raf ->
            val len = raf.length()
            if (len == 0L) return "(none)\n"
            val toRead = minOf(len, maxBytes.toLong()).toInt()
            raf.seek(len - toRead)
            val buf = ByteArray(toRead)
            raf.readFully(buf)
            return String(buf, Charsets.UTF_8)
        }
    }
}
