package com.zalom3ui.app.server

import android.content.Context
import android.os.Build
import java.io.File
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

    fun file(context: Context): File = File(File(context.filesDir, "zalo").apply { mkdirs() }, "startup.log")

    /** The server's own output (Node's console), see LOG_FILE in src/index.ts. */
    fun serverLog(context: Context): File = File(File(context.filesDir, "zalo").apply { mkdirs() }, "server.log")

    @Synchronized
    fun write(context: Context, line: String) {
        runCatching {
            val f = file(context)
            if (f.length() > MAX_BYTES) f.writeText(f.readText().takeLast(MAX_BYTES / 2))
            val time = SimpleDateFormat("yyyy-MM-dd HH:mm:ss.SSS", Locale.US).format(Date())
            f.appendText("$time [${android.os.Process.myPid()}] $line\n")
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
        append(runCatching { file(context).readText() }.getOrDefault("(none)\n"))
        appendLine()
        appendLine("== server.log (last part)")
        append(runCatching { serverLog(context).readText().takeLast(24 * 1024) }.getOrDefault("(none)\n"))
    }
}
