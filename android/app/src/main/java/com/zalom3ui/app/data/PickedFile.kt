package com.zalom3ui.app.data

import android.content.ContentResolver
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import android.media.MediaMetadataRetriever
import android.net.Uri
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import androidx.exifinterface.media.ExifInterface
import java.io.ByteArrayOutputStream
import java.io.File
import java.io.InputStream

/** A photo or file picked to send: what the upload needs to know about it. */
data class PickedFile(
    val name: String,
    val mimeType: String?,
    val length: Long,
    /** Pixel size, as shown (EXIF rotation applied); photos only. */
    val width: Int? = null,
    val height: Int? = null,
    val open: () -> InputStream,
    val cleanup: () -> Unit = {},
    /** What Zalo needs to show a video as a video message; null for anything else or when it can't be read. */
    val video: VideoInfo? = null,
)

/** A video's shown size (rotation applied), duration and a JPEG thumbnail. */
class VideoInfo(
    val durationMs: Long,
    val width: Int,
    val height: Int,
    val thumbnailJpeg: ByteArray,
)

/** Extensions of files that are videos (the picker's MIME type counts too). */
val VIDEO_EXTENSIONS = setOf("mp4", "mov", "3gp", "webm", "m4v", "mkv")

fun isVideoFile(name: String, mimeType: String?): Boolean =
    mimeType?.lowercase()?.startsWith("video/") == true || extensionOf(name) in VIDEO_EXTENSIONS

/** Only .mp4 is sent as a video message: the core uploads other containers as files. */
fun sendsAsVideoMessage(name: String): Boolean = extensionOf(name) == "mp4"

/** Width and height as the video is shown: rotations of 90 and 270 degrees turn it on its side. */
fun rotatedVideoSize(width: Int, height: Int, rotationDegrees: Int): Pair<Int, Int> =
    if (((rotationDegrees % 360) + 360) % 360 in setOf(90, 270)) height to width else width to height

/** Longest side of a video thumbnail, in pixels. */
private const val VIDEO_THUMB_SIDE = 480

/** Zalo sends these as photos; any other picture is converted to JPEG first. */
val PHOTO_EXTENSIONS = setOf("jpg", "jpeg", "png", "webp")

/** Longest side of a photo converted to JPEG (HEIC, BMP…), like Zalo's own resize. */
private const val MAX_CONVERTED_SIDE = 2560

/**
 * A file name with an extension, which decides how Zalo sends it: the picker's name, or "<prefix>.<ext>" from the MIME type.
 * Path separators are dropped.
 */
fun nameWithExtension(displayName: String?, mimeType: String?, fallbackExtension: (String) -> String? = ::extensionForMime): String {
    val base = displayName?.substringAfterLast('/')?.trim().orEmpty()
    val fromMime = mimeType?.let(fallbackExtension)
    return when {
        base.isNotEmpty() && base.substringAfterLast('.', "").isNotEmpty() && '.' in base.drop(1) -> base
        base.isNotEmpty() && fromMime != null -> "$base.$fromMime"
        fromMime != null -> "${if (mimeType?.startsWith("image/") == true) "photo" else "file"}.$fromMime"
        base.isNotEmpty() -> "$base.bin"
        else -> "file.bin"
    }
}

fun extensionOf(name: String): String = name.substringAfterLast('.', "").lowercase()

/** A picture Zalo would send as a file because of its format (HEIC, BMP, AVIF…): convert it to JPEG instead. */
fun needsJpegConversion(name: String, mimeType: String?): Boolean {
    val ext = extensionOf(name)
    if (ext in PHOTO_EXTENSIONS || ext == "gif") return false
    return mimeType?.startsWith("image/") == true || ext in setOf("heic", "heif", "bmp", "avif")
}

/** Width and height as the photo is shown: EXIF orientations 5–8 turn it on its side. */
fun orientedSize(width: Int, height: Int, exifOrientation: Int): Pair<Int, Int> =
    if (exifOrientation in 5..8) height to width else width to height

/** Largest power-of-two subsampling factor keeping the longest side at least [max] pixels (before exact scaling). */
fun sampleSizeFor(width: Int, height: Int, max: Int): Int {
    if (max <= 0) return 1
    var sample = 1
    while (maxOf(width, height) / (sample * 2) >= max) sample *= 2
    return sample
}

private fun extensionForMime(mimeType: String): String? = when (mimeType.lowercase()) {
    "image/jpeg" -> "jpg"
    else -> MimeTypeMap.getSingleton().getExtensionFromMimeType(mimeType)
}

/**
 * Reads what an upload needs about [uri]. Pictures in formats Zalo does not send as photos are re-encoded to a JPEG in
 * [cacheDir] (rotation applied); other photos keep their bytes and get their shown size.
 */
fun readPickedFile(resolver: ContentResolver, uri: Uri, cacheDir: File): PickedFile {
    var displayName: String? = null
    var size = -1L
    resolver.query(uri, arrayOf(OpenableColumns.DISPLAY_NAME, OpenableColumns.SIZE), null, null, null)?.use { c ->
        if (c.moveToFirst()) {
            c.getColumnIndex(OpenableColumns.DISPLAY_NAME).takeIf { it >= 0 && !c.isNull(it) }?.let { displayName = c.getString(it) }
            c.getColumnIndex(OpenableColumns.SIZE).takeIf { it >= 0 && !c.isNull(it) }?.let { size = c.getLong(it) }
        }
    }
    val mime = resolver.getType(uri)
    val name = nameWithExtension(displayName, mime)
    val open = { resolver.openInputStream(uri) ?: throw ApiException("Can't read $name") }

    if (needsJpegConversion(name, mime)) return convertToJpeg(name, open, cacheDir)
    if (extensionOf(name) !in PHOTO_EXTENSIONS) {
        val video = if (sendsAsVideoMessage(name) && isVideoFile(name, mime)) readVideoInfo(resolver, uri) else null
        return PickedFile(name, mime, size, open = open, video = video)
    }

    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    open().use { BitmapFactory.decodeStream(it, null, bounds) }
    val orientation = runCatching { open().use { ExifInterface(it).getAttributeInt(ExifInterface.TAG_ORIENTATION, 1) } }.getOrDefault(1)
    val (w, h) = orientedSize(bounds.outWidth, bounds.outHeight, orientation)
    return PickedFile(name, mime, size, width = w.takeIf { it > 0 }, height = h.takeIf { it > 0 }, open = open)
}

/** Duration, shown size and a frame at about one second of the video at [uri]; null when the phone can't read it. */
private fun readVideoInfo(resolver: ContentResolver, uri: Uri): VideoInfo? {
    val retriever = MediaMetadataRetriever()
    var frame: Bitmap? = null
    var scaled: Bitmap? = null
    try {
        resolver.openFileDescriptor(uri, "r")?.use { retriever.setDataSource(it.fileDescriptor) } ?: return null
        fun meta(key: Int) = retriever.extractMetadata(key)?.toLongOrNull()
        val duration = meta(MediaMetadataRetriever.METADATA_KEY_DURATION)?.takeIf { it > 0 } ?: return null
        val rawW = meta(MediaMetadataRetriever.METADATA_KEY_VIDEO_WIDTH)?.toInt()?.takeIf { it > 0 } ?: return null
        val rawH = meta(MediaMetadataRetriever.METADATA_KEY_VIDEO_HEIGHT)?.toInt()?.takeIf { it > 0 } ?: return null
        val rotation = meta(MediaMetadataRetriever.METADATA_KEY_VIDEO_ROTATION)?.toInt() ?: 0
        val (w, h) = rotatedVideoSize(rawW, rawH, rotation)
        val atUs = if (duration >= 1500) 1_000_000L else 0L
        val decoded = retriever.getFrameAtTime(atUs, MediaMetadataRetriever.OPTION_CLOSEST_SYNC) ?: return null
        frame = decoded
        val longest = maxOf(decoded.width, decoded.height)
        val out = if (longest > VIDEO_THUMB_SIDE) {
            val k = VIDEO_THUMB_SIDE.toFloat() / longest
            Bitmap.createScaledBitmap(decoded, maxOf(1, (decoded.width * k).toInt()), maxOf(1, (decoded.height * k).toInt()), true)
                .also { scaled = it }
        } else {
            decoded
        }
        val bytes = ByteArrayOutputStream().also { out.compress(Bitmap.CompressFormat.JPEG, 80, it) }.toByteArray()
        return if (bytes.isEmpty()) null else VideoInfo(duration, w, h, bytes)
    } catch (e: Exception) {
        return null
    } finally {
        scaled?.recycle()
        frame?.recycle()
        runCatching { retriever.release() }
    }
}

private fun convertToJpeg(name: String, open: () -> InputStream, cacheDir: File): PickedFile {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    open().use { BitmapFactory.decodeStream(it, null, bounds) }
    if (bounds.outWidth <= 0) throw ApiException("$name is a picture this phone can't read")
    val decoded = open().use {
        BitmapFactory.decodeStream(
            it,
            null,
            BitmapFactory.Options().apply { inSampleSize = sampleSizeFor(bounds.outWidth, bounds.outHeight, MAX_CONVERTED_SIDE) },
        )
    } ?: throw ApiException("$name is a picture this phone can't read")
    var bitmap: Bitmap? = null
    try {
        val orientation = runCatching { open().use { ExifInterface(it).getAttributeInt(ExifInterface.TAG_ORIENTATION, 1) } }.getOrDefault(1)
        val transformed = transform(decoded, orientation, MAX_CONVERTED_SIDE)
        bitmap = transformed
        val dir = File(cacheDir, "uploads").apply { mkdirs() }
        dir.listFiles()?.filter { System.currentTimeMillis() - it.lastModified() > 6 * 3_600_000L }?.forEach { it.delete() }
        val out = File(dir, name.substringBeforeLast('.') + "-" + System.nanoTime() + ".jpg")
        try {
            out.outputStream().use { transformed.compress(Bitmap.CompressFormat.JPEG, 90, it) }
        } catch (e: Throwable) {
            out.delete()
            throw e
        }
        return PickedFile(
            name = name.substringBeforeLast('.') + ".jpg",
            mimeType = "image/jpeg",
            length = out.length(),
            width = transformed.width,
            height = transformed.height,
            open = { out.inputStream() },
            cleanup = { out.delete() },
        )
    } finally {
        val b = bitmap
        if (b != null && b !== decoded) b.recycle()
        decoded.recycle()
    }
}

private fun transform(bitmap: Bitmap, orientation: Int, maxSide: Int): Bitmap {
    val m = Matrix()
    var changed = true
    when (orientation) {
        ExifInterface.ORIENTATION_ROTATE_90 -> m.postRotate(90f)
        ExifInterface.ORIENTATION_ROTATE_180 -> m.postRotate(180f)
        ExifInterface.ORIENTATION_ROTATE_270 -> m.postRotate(270f)
        ExifInterface.ORIENTATION_FLIP_HORIZONTAL -> m.postScale(-1f, 1f)
        ExifInterface.ORIENTATION_FLIP_VERTICAL -> m.postScale(1f, -1f)
        ExifInterface.ORIENTATION_TRANSPOSE -> { m.postRotate(90f); m.postScale(-1f, 1f) }
        ExifInterface.ORIENTATION_TRANSVERSE -> { m.postRotate(270f); m.postScale(-1f, 1f) }
        else -> changed = false
    }
    val longest = maxOf(bitmap.width, bitmap.height)
    if (longest > maxSide) {
        val scale = maxSide.toFloat() / longest.toFloat()
        m.postScale(scale, scale)
        changed = true
    }
    if (!changed) return bitmap
    return Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, m, true)
}
