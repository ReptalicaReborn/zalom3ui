package com.zalom3ui.app

import android.app.Application
import android.os.Build
import coil.ImageLoader
import coil.ImageLoaderFactory
import coil.decode.GifDecoder
import coil.decode.ImageDecoderDecoder
import com.zalom3ui.app.call.CallMediaFactory
import com.zalom3ui.app.call.EngineCallMedia
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.server.StayConnectedCoordinator

/** Gives Coil animated image decoding, so stickers (animated WebP) and GIFs move as they do in Zalo. */
class ZalooApp : Application(), ImageLoaderFactory {
    override fun onCreate() {
        super.onCreate()
        // "Stay connected": an install from before the setting keeps "Always"; then the coordinator follows the lifecycle.
        val upgrade = runCatching {
            packageManager.getPackageInfo(packageName, 0).let { it.firstInstallTime != it.lastUpdateTime }
        }.getOrDefault(true)
        SettingsStore(this).ensureStayConnectedInitialised(isUpgrade = upgrade)
        StayConnectedCoordinator.init(this)
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
