package com.zalom3ui.app.data

import android.content.ContentResolver
import android.graphics.Bitmap
import android.graphics.BitmapFactory
import android.graphics.Matrix
import android.net.Uri
import android.provider.OpenableColumns
import android.webkit.MimeTypeMap
import androidx.exifinterface.media.ExifInterface
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
)

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

/** Sample size that brings the longest side to at most [max] pixels. */
fun sampleSizeFor(width: Int, height: Int, max: Int): Int {
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
    if (extensionOf(name) !in PHOTO_EXTENSIONS) return PickedFile(name, mime, size, open = open)

    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    open().use { BitmapFactory.decodeStream(it, null, bounds) }
    val orientation = runCatching { open().use { ExifInterface(it).getAttributeInt(ExifInterface.TAG_ORIENTATION, 1) } }.getOrDefault(1)
    val (w, h) = orientedSize(bounds.outWidth, bounds.outHeight, orientation)
    return PickedFile(name, mime, size, width = w.takeIf { it > 0 }, height = h.takeIf { it > 0 }, open = open)
}

private fun convertToJpeg(name: String, open: () -> InputStream, cacheDir: File): PickedFile {
    val bounds = BitmapFactory.Options().apply { inJustDecodeBounds = true }
    open().use { BitmapFactory.decodeStream(it, null, bounds) }
    if (bounds.outWidth <= 0) throw ApiException("$name is a picture this phone can't read")
    val decoded = open().use {
        BitmapFactory.decodeStream(it, null, BitmapFactory.Options().apply { inSampleSize = sampleSizeFor(bounds.outWidth, bounds.outHeight, MAX_CONVERTED_SIDE) })
    } ?: throw ApiException("$name is a picture this phone can't read")
    val orientation = runCatching { open().use { ExifInterface(it).getAttributeInt(ExifInterface.TAG_ORIENTATION, 1) } }.getOrDefault(1)
    val bitmap = rotate(decoded, orientation)
    val dir = File(cacheDir, "uploads").apply { mkdirs() }
    dir.listFiles()?.filter { System.currentTimeMillis() - it.lastModified() > 6 * 3_600_000L }?.forEach { it.delete() }
    val out = File(dir, name.substringBeforeLast('.') + "-" + System.nanoTime() + ".jpg")
    out.outputStream().use { bitmap.compress(Bitmap.CompressFormat.JPEG, 90, it) }
    val result = PickedFile(
        name = name.substringBeforeLast('.') + ".jpg",
        mimeType = "image/jpeg",
        length = out.length(),
        width = bitmap.width,
        height = bitmap.height,
        open = { out.inputStream() },
    )
    if (bitmap !== decoded) decoded.recycle()
    bitmap.recycle()
    return result
}

private fun rotate(bitmap: Bitmap, orientation: Int): Bitmap {
    val m = Matrix()
    when (orientation) {
        ExifInterface.ORIENTATION_ROTATE_90 -> m.postRotate(90f)
        ExifInterface.ORIENTATION_ROTATE_180 -> m.postRotate(180f)
        ExifInterface.ORIENTATION_ROTATE_270 -> m.postRotate(270f)
        ExifInterface.ORIENTATION_FLIP_HORIZONTAL -> m.postScale(-1f, 1f)
        ExifInterface.ORIENTATION_FLIP_VERTICAL -> m.postScale(1f, -1f)
        ExifInterface.ORIENTATION_TRANSPOSE -> { m.postRotate(90f); m.postScale(-1f, 1f) }
        ExifInterface.ORIENTATION_TRANSVERSE -> { m.postRotate(270f); m.postScale(-1f, 1f) }
        else -> return bitmap
    }
    return Bitmap.createBitmap(bitmap, 0, 0, bitmap.width, bitmap.height, m, true)
}
