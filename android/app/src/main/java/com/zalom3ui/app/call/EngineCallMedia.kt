package com.zalom3ui.app.call

import android.content.Context
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.server.LocalServer

/**
 * The call's real audio (docs/call-audio.md): microphone and speaker through [CallAudioEngine] with Opus, frames to and
 * from the on-phone server over [CallAudioLink], which carries them to Zalo's media server. A new one per call.
 */
class EngineCallMedia(private val context: Context) : CallMedia {
    private var engine: CallAudioEngine? = null
    private var link: CallAudioLink? = null

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
        )
        val e = CallAudioEngine(context.applicationContext, OpusFrameCodec(), send = { opus -> l.send(opus) })
        e.muted = muted
        e.speakerOn = speakerOn
        link = l
        engine = e
        l.connect()
        try {
            e.start()
        } catch (err: Exception) {
            stop()
            throw err
        }
    }

    @Synchronized
    override fun stop() {
        link?.close()
        link = null
        engine?.stop()
        engine = null
    }
}
