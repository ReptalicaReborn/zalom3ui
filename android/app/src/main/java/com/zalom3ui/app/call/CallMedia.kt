package com.zalom3ui.app.call

import android.content.Context

/** What [CallService] needs from the audio side of a call, so the service doesn't depend on the engine directly. */
interface CallMedia {
    /** Starts capture, playback and the link to the server. Called once the call is connected. */
    fun start()

    /** Stops everything and releases the audio devices. Safe to call twice. */
    fun stop()

    var muted: Boolean
    var speakerOn: Boolean
}

/** Used until the real engine is plugged into [CallMediaFactory]: the call stays connected but silent. */
object NoCallMedia : CallMedia {
    override fun start() = Unit

    override fun stop() = Unit

    override var muted: Boolean = false
    override var speakerOn: Boolean = false
}

object CallMediaFactory {
    /** Replace this (e.g. in `ZalooApp.onCreate`) to give [CallService] the real audio engine. */
    @Volatile
    var create: (Context) -> CallMedia = { NoCallMedia }
}
