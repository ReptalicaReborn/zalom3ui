package com.zalom3ui.app

import android.app.Application
import android.os.Build
import coil.ImageLoader
import coil.ImageLoaderFactory
import coil.decode.GifDecoder
import coil.decode.ImageDecoderDecoder
import com.zalom3ui.app.call.CallMediaFactory
import com.zalom3ui.app.call.EngineCallMedia

/** Gives Coil animated image decoding, so stickers (animated WebP) and GIFs move as they do in Zalo. */
class ZalooApp : Application(), ImageLoaderFactory {
    override fun onCreate() {
        super.onCreate()
        // Calls get the real microphone/speaker/Opus engine (CallService asks the factory when a call connects).
        CallMediaFactory.create = { context -> EngineCallMedia(context) }
    }

    override fun newImageLoader(): ImageLoader =
        ImageLoader.Builder(this)
            .components {
                // Android 9+ decodes animated WebP and GIF; older phones animate GIFs only.
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) add(ImageDecoderDecoder.Factory()) else add(GifDecoder.Factory())
            }
            .build()
}
