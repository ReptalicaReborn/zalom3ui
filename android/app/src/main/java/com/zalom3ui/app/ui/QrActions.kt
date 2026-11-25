package com.zalom3ui.app.ui

import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import androidx.core.content.FileProvider
import java.io.File
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * The login QR code has to be scanned by the Zalo app, which usually lives on this same phone.
 * Zalo's QR scanner can read a code from a picture in the gallery, so offer to save or share it.
 */
object QrActions {
    private const val SHARED_FILE_MAX_AGE_MS = 60 * 60_000L

    /** Saves [qr] to Pictures/Zalỏ. Returns false when it couldn't. */
    suspend fun saveToGallery(context: Context, qr: Bitmap): Boolean = withContext(Dispatchers.IO) {
        val prepared = try {
            withQuietZone(qr)
        } catch (_: Exception) {
            return@withContext false
        }
        try {
            save(context, prepared, "zalo-login-${System.currentTimeMillis()}.png")
        } catch (_: Exception) {
            false
        } finally {
            prepared.recycle()
        }
    }

    private fun save(context: Context, image: Bitmap, name: String): Boolean {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val resolver = context.contentResolver
            val values = ContentValues().apply {
                put(MediaStore.Images.Media.DISPLAY_NAME, name)
                put(MediaStore.Images.Media.MIME_TYPE, "image/png")
                put(MediaStore.Images.Media.RELATIVE_PATH, "${Environment.DIRECTORY_PICTURES}/Zalỏ")
            }
            val uri = resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values)
                ?: return false
            try {
                val out = resolver.openOutputStream(uri)
                if (out == null) {
                    runCatching { resolver.delete(uri, null, null) }
                    return false
                }
                val ok = out.use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
                if (!ok) {
                    runCatching { resolver.delete(uri, null, null) }
                    return false
                }
            } catch (e: Exception) {
                runCatching { resolver.delete(uri, null, null) }
                throw e
            }
        } else {
            @Suppress("DEPRECATION")
            val dir = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_PICTURES), "Zalỏ")
            dir.mkdirs()
            val file = File(dir, name)
            val ok = file.outputStream().use { image.compress(Bitmap.CompressFormat.PNG, 100, it) }
            if (!ok) {
                file.delete()
                return false
            }
            @Suppress("DEPRECATION")
            context.sendBroadcast(Intent(Intent.ACTION_MEDIA_SCANNER_SCAN_FILE, android.net.Uri.fromFile(file)))
        }
        return true
    }

    /** Opens the share sheet with the QR image, e.g. to send it to a computer or another phone. */
    suspend fun share(context: Context, qr: Bitmap) {
        val uri = withContext(Dispatchers.IO) {
            runCatching {
                val dir = sharedDir(context)
                val file = File(dir, "zalo-login.png")
                val prepared = withQuietZone(qr)
                try {
                    file.outputStream().use { prepared.compress(Bitmap.CompressFormat.PNG, 100, it) }
                } finally {
                    prepared.recycle()
                }
                FileProvider.getUriForFile(context, "${context.packageName}.files", file)
            }.getOrNull()
        } ?: return
        val send = Intent(Intent.ACTION_SEND).apply {
            type = "image/png"
            putExtra(Intent.EXTRA_STREAM, uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        withContext(Dispatchers.Main) {
            runCatching {
                context.startActivity(Intent.createChooser(send, "Share login QR code").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
        }
    }

    /** Opens the share sheet with the call log as a text file (to send it for the call research). */
    suspend fun shareCallLog(context: Context, log: String) = shareText(context, "zaloo-call-log.txt", "Share call log", log)

    /** Opens the share sheet with [text] as a file named [fileName]. */
    suspend fun shareText(context: Context, fileName: String, title: String, text: String) {
        val uri = withContext(Dispatchers.IO) {
            runCatching {
                val dir = sharedDir(context)
                val file = File(dir, fileName)
                file.writeText(text)
                FileProvider.getUriForFile(context, "${context.packageName}.files", file)
            }.getOrNull()
        } ?: return
        val send = Intent(Intent.ACTION_SEND).apply {
            type = "text/plain"
            putExtra(Intent.EXTRA_STREAM, uri)
            addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
        }
        withContext(Dispatchers.Main) {
            runCatching {
                context.startActivity(Intent.createChooser(send, title).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            }
        }
    }

    /** Removes stale temp files in cacheDir/shared before writing a new shared file. */
    private fun sharedDir(context: Context): File {
        val dir = File(context.cacheDir, "shared").apply { mkdirs() }
        val cutoff = System.currentTimeMillis() - SHARED_FILE_MAX_AGE_MS
        dir.listFiles()?.forEach { file ->
            if (file.isFile && file.lastModified() < cutoff) file.delete()
        }
        return dir
    }

    /** Scanners need a white border around the code; the image from Zalo is tight-cropped. */
    private fun withQuietZone(qr: Bitmap): Bitmap {
        val scale = maxOf(1, 600 / maxOf(qr.width, 1))
        val scaled = Bitmap.createScaledBitmap(qr, qr.width * scale, qr.height * scale, false)
        val pad = scaled.width / 8
        val out = Bitmap.createBitmap(scaled.width + 2 * pad, scaled.height + 2 * pad, Bitmap.Config.ARGB_8888)
        val canvas = android.graphics.Canvas(out)
        canvas.drawColor(android.graphics.Color.WHITE)
        canvas.drawBitmap(scaled, pad.toFloat(), pad.toFloat(), null)
        if (scaled !== qr) scaled.recycle()
        return out
    }
}
