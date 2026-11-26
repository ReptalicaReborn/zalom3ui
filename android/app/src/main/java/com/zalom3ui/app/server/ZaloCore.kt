package com.zalom3ui.app.server

import android.content.Context
import android.util.Log
import com.zalom3ui.app.data.CoreNatives
import java.io.File

/**
 * The Zalo core (libzalocore.so, Rust) running inside this app's process, called directly over JNI: no socket, port or
 * token. One per process; [ensureStarted] starts it on first use and is cheap afterwards, whoever asks first (the
 * screen, the notifier, a call), so it does not depend on [ZaloService]. The service only keeps the process alive in
 * the background.
 *
 * Pull model: the native functions block, and the app calls them from its own background threads. The core never calls
 * back into the JVM. The `external` functions below must match the `Java_com_zalom3ui_app_server_ZaloCore_*` symbols in
 * rust/zalo-android/src/lib.rs (and be kept by proguard-rules.pro).
 */
object ZaloCore : CoreNatives {
    private const val TAG = "ZaloCore"

    init {
        System.loadLibrary("zalocore")
    }

    @Volatile
    private var started = false

    /** The core is running in this process (without starting it, unlike [ensureStarted]). */
    val isStarted: Boolean get() = started

    /**
     * Starts the core if it is not running. Blocks for a moment on the first call (it opens the message store), so call
     * it from a background thread. False if it could not start; the reason is in the startup log.
     */
    @Synchronized
    fun ensureStarted(context: Context): Boolean {
        if (started) return true
        val app = context.applicationContext ?: context
        return try {
            val dataDir = File(app.filesDir, "zalo").apply { mkdirs() }
            // Left over from when the UI talked to the core over a loopback port.
            File(app.filesDir, "local-server.properties").delete()
            File(app.filesDir, "local-server.lock").delete()
            StartupLog.write(app, "core: starting")
            val code = start(dataDir.absolutePath, StartupLog.serverLog(app).absolutePath)
            started = code == 0
            StartupLog.write(app, if (started) "core: started" else "core: failed to start (code $code)")
            // In the "While open" modes, a start from anywhere but the screen is paused again after a grace period.
            if (started) StayConnectedCoordinator.onCoreStarted()
            started
        } catch (e: Throwable) {
            Log.e(TAG, "Could not start the Zalo core", e)
            StartupLog.error(app, "core: could not start", e)
            false
        }
    }

    /** Stops the core (flushing the message store); [ensureStarted] starts a fresh one. Blocks briefly. */
    @Synchronized
    fun shutdown() {
        if (!started) return
        started = false
        try {
            stop()
        } catch (e: Throwable) {
            Log.e(TAG, "Could not stop the Zalo core", e)
        }
    }

    /** Stops the core and starts it again in this process, e.g. when it got stuck. */
    @Synchronized
    fun restart(context: Context): Boolean {
        shutdown()
        return ensureStarted(context)
    }

    /** Opens the store and starts the session. Idempotent; 0 on success. */
    private external fun start(dataDir: String, logFile: String): Int

    private external fun stop()

    external override fun request(method: String, pathAndQuery: String, contentType: String, body: ByteArray?): ByteArray

    external override fun requestFile(method: String, pathAndQuery: String, contentType: String, filePath: String): ByteArray

    external override fun eventsOpen(): Long

    external override fun eventsNext(handle: Long, timeoutMs: Long): String?

    external override fun eventsClose(handle: Long)

    external override fun audioOpen(): Long

    external override fun audioSend(handle: Long, data: ByteArray): Boolean

    external override fun audioNext(handle: Long, timeoutMs: Long): ByteArray?

    external override fun audioCloseReason(handle: Long): String?

    external override fun audioClose(handle: Long)
}
