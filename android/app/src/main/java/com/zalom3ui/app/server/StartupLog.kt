package com.zalom3ui.app.server

import android.content.Context
import android.os.Build
import java.io.File
import java.io.RandomAccessFile
import java.text.SimpleDateFormat
import java.util.Date
import java.util.Locale

/**
 * What happened while the on-phone Zalo server started: the app asking for the service, the service starting Node,
 * Node exiting, and any error on the way. Written by both processes (UI and ":node") to one small file, so a phone that
 * stays on "Starting Zalo…" can share why. Holds no tokens or messages.
 */
object StartupLog {
    private const val MAX_BYTES = 64 * 1024
    private const val SERVER_LOG_TAIL_BYTES = 24 * 1024

    fun file(context: Context): File = File(File(context.filesDir, "zalo").apply { mkdirs() }, "startup.log")

    /** The server's own output (Node's console), see LOG_FILE in src/index.ts. */
    fun serverLog(context: Context): File = File(File(context.filesDir, "zalo").apply { mkdirs() }, "server.log")

    @Synchronized
    fun write(context: Context, line: String) {
        runCatching {
            val f = file(context)
            RandomAccessFile(f, "rw").use { raf ->
                raf.channel.lock().use {
                    if (raf.length() > MAX_BYTES) {
                        val keep = MAX_BYTES / 2
                        raf.seek(raf.length() - keep)
                        val tail = ByteArray(keep)
                        raf.readFully(tail)
                        val nl = tail.indexOf('\n'.code.toByte())
                        val start = if (nl in 0 until tail.size - 1) nl + 1 else 0
                        raf.seek(0)
                        raf.write(tail, start, tail.size - start)
                        raf.setLength((tail.size - start).toLong())
                    }
                    raf.seek(raf.length())
                    val time = SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US).format(Date())
                    raf.write("$time [${android.os.Process.myPid()}] $line\n".toByteArray(Charsets.UTF_8))
                }
            }
        }
    }

    fun error(context: Context, what: String, e: Throwable) {
        write(context, "$what: ${e.javaClass.name}: ${e.message}\n" + e.stackTrace.take(8).joinToString("\n") { "    at $it" })
    }

    /** Everything worth sending when the server doesn't start: phone, app, startup steps and the server's output. */
    fun report(context: Context): String = buildString {
        val info = runCatching { context.packageManager.getPackageInfo(context.packageName, 0) }.getOrNull()
        appendLine("Zalỏ ${info?.versionName} (${context.packageName})")
        appendLine("Phone: ${Build.MANUFACTURER} ${Build.MODEL}, Android ${Build.VERSION.RELEASE} (API ${Build.VERSION.SDK_INT}), ${Build.SUPPORTED_ABIS.joinToString()}")
        appendLine("Local port: ${LocalServer.port(context)}")
        appendLine()
        appendLine("== startup.log")
        append(runCatching { readTail(file(context), MAX_BYTES) }.getOrDefault("(none)\n"))
        appendLine()
        appendLine("== server.log (last part)")
        append(runCatching { readTail(serverLog(context), SERVER_LOG_TAIL_BYTES) }.getOrDefault("(none)\n"))
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
