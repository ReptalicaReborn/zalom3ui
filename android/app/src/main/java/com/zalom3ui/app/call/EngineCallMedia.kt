package com.zalom3ui.app.call

import android.content.Context
import android.view.Surface
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.server.LocalServer

/**
 * The call's real audio (docs/call-audio.md): microphone and speaker through [CallAudioEngine] with Opus, frames to and
 * from the on-phone server over [CallAudioLink], which carries them to Zalo's media server. A new one per call.
 */
class EngineCallMedia(private val context: Context) : CallMedia {
    private var engine: CallAudioEngine? = null
    private var link: CallAudioLink? = null
    private var decoder: VideoDecoder? = null
    private var sender: CameraVideoSender? = null
    private var listener: VideoListener? = null
    private var remoteSurface: Surface? = null
    private var previewSurface: Surface? = null
    private var frontCamera = true

    override var muted: Boolean = false
        set(value) {
            field = value
            engine?.muted = value
        }

    override var speakerOn: Boolean = false
        set(value) {
            field = value
            engine?.speakerOn = value
        }

    @Synchronized
    override fun start() {
        if (engine != null) return
        val l = CallAudioLink(
            LocalServer.url(context),
            SettingsStore(context).localToken(),
            onAudio = { seq, timestamp, opus -> engine?.onPacket(seq, timestamp, opus) },
            // The server closes the socket when the call ends; CallService hears about the end from the event socket.
            onClosed = { },
            // Video shares the socket (one per call).
            onVideo = { key, timestamp, au -> decoder?.submit(key, timestamp, au) },
            onKeyFrameWanted = { sender?.requestKeyFrame() },
        )
        val e = CallAudioEngine(context.applicationContext, OpusFrameCodec(), send = { opus -> l.send(opus) })
        e.muted = muted
        e.speakerOn = speakerOn
        link = l
        engine = e
        remoteSurface?.let { startDecoder(it) }
        l.connect()
        try {
            e.start()
        } catch (err: Exception) {
            stop()
            throw err
        }
    }

    @Synchronized
    override fun setVideoListener(listener: VideoListener?) {
        this.listener = listener
    }

    @Synchronized
    override fun setRemoteSurface(surface: Surface?) {
        remoteSurface = surface
        val d = decoder
        when {
            d != null -> d.setSurface(surface)
            surface != null && link != null -> startDecoder(surface)
        }
    }

    // Called with the lock held.
    private fun startDecoder(surface: Surface) {
        val d = VideoDecoder(
            requestKeyFrame = { link?.sendKeyFrameNeeded() },
            onSize = { w, h -> listener?.onRemoteVideoSize(w, h) },
        )
        decoder = d
        d.setSurface(surface)
    }

    @Synchronized
    override fun setPreviewSurface(surface: Surface?) {
        previewSurface = surface
        sender?.setPreviewSurface(surface)
    }

    @Synchronized
    override fun setCameraOn(on: Boolean) {
        if (!on) {
            sender?.stop()
            sender = null
            return
        }
        if (sender?.isRunning == true) return
        val l = link ?: error("The call isn't connected yet")
        val s = CameraVideoSender(
            context,
            onFrame = { key, au -> l.sendVideo(key, au) },
            onError = { message -> listener?.onCameraStopped(message) },
        )
        s.setPreviewSurface(previewSurface)
        s.start(frontCamera)
        sender = s
    }

    @Synchronized
    override fun switchCamera() {
        frontCamera = !frontCamera
        sender?.switchCamera()
    }

    @Synchronized
    override fun stop() {
        sender?.stop()
        sender = null
        decoder?.release()
        decoder = null
        link?.close()
        link = null
        engine?.stop()
        engine = null
    }
}
