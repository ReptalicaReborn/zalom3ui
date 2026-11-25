package com.zalom3ui.app.call

import android.content.Context
import android.view.Surface

/** News from the video side of a call, delivered on a worker thread. */
interface VideoListener {
    /** The other side's picture has this size (again after it changes resolution). */
    fun onRemoteVideoSize(width: Int, height: Int)

    /** The camera stopped by itself (disconnected, taken by another app, encoder failure). */
    fun onCameraStopped(message: String)
}

/** What [CallService] needs from the audio side of a call, so the service doesn't depend on the engine directly. */
interface CallMedia {
    /** Starts capture, playback and the link to the server. Called once the call is connected. */
    fun start()

    /** Stops everything and releases the audio devices. Safe to call twice. */
    fun stop()

    var muted: Boolean
    var speakerOn: Boolean

    // Video. Voice-only media ignores all of it.

    fun setVideoListener(listener: VideoListener?) = Unit

    /** Where the other side's picture goes; null when the screen's surface is going away. */
    fun setRemoteSurface(surface: Surface?) = Unit

    /** The small self-view; null when its surface is going away. */
    fun setPreviewSurface(surface: Surface?) = Unit

    /** Starts or stops sending our camera. Turning on throws if the camera can't be started. */
    fun setCameraOn(on: Boolean) = Unit

    fun switchCamera() = Unit
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
