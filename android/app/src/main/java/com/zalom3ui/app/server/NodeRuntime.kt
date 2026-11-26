package com.zalom3ui.app.server

/** Entry point into the embedded Rust Zalo server (libzalocore.so). */
object NodeRuntime {
    init {
        System.loadLibrary("zalocore")
    }

    /** Starts the embedded Rust server and blocks until it exits or [stopServer] is called. */
    external fun startServer(
        host: String,
        port: Int,
        token: String,
        dataDir: String,
        logFile: String,
    ): Int

    /** Requests a clean shutdown of the running embedded Rust server. */
    external fun stopServer()
}
