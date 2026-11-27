package com.zalom3ui.app.server

import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.graphics.BitmapShader
import android.graphics.Canvas
import android.graphics.Matrix
import android.graphics.Paint
import android.graphics.Shader
import android.graphics.Typeface
import androidx.core.app.Person
import androidx.core.content.pm.ShortcutInfoCompat
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.core.graphics.drawable.IconCompat
import androidx.core.graphics.drawable.toBitmap
import coil.imageLoader
import coil.request.ImageRequest
import coil.request.SuccessResult
import coil.size.Scale
import com.zalom3ui.app.MainActivity
import com.zalom3ui.app.data.ThreadRef
import com.zalom3ui.app.data.ThreadSummary
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext
import kotlinx.coroutines.withTimeoutOrNull
import kotlin.math.abs

/**
 * Publishes Android Conversation Shortcuts & Direct Share targets for Zalỏ chats.
 *
 * Notifications that reference [shortcutId] via `setShortcutId` appear in Android 11+'s dedicated
 * Conversations section, and shortcuts carrying [SHARE_CATEGORY] show up as Direct Share targets
 * in the system share sheet (wired through `res/xml/shortcuts.xml`).
 */
object ConversationShortcuts {
    const val SHARE_CATEGORY = "com.zalom3ui.app.category.SHARE_TARGET"

    private const val SHORTCUT_PREFIX = "chat_"
    private const val AVATAR_SIZE_PX = 192
    private const val LOAD_TIMEOUT_MS = 1_500L
    private const val MAX_CACHED_AVATARS = 32

    private val avatarCacheLock = Any()
    private val avatarCache = object : LinkedHashMap<String, Bitmap>(MAX_CACHED_AVATARS, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, Bitmap>?): Boolean =
            size > MAX_CACHED_AVATARS
    }

    /** Stable shortcut ID for a conversation. */
    fun shortcutId(ref: ThreadRef): String = "$SHORTCUT_PREFIX${ref.kind}_${ref.id}"

    /** Parses a shortcut ID created by [shortcutId] back into its [ThreadRef], or null if unrecognized. */
    fun parseShortcutId(shortcutId: String?): ThreadRef? {
        if (shortcutId.isNullOrBlank() || !shortcutId.startsWith(SHORTCUT_PREFIX)) return null
        val rest = shortcutId.removePrefix(SHORTCUT_PREFIX)
        val sep = rest.indexOf('_')
        if (sep <= 0 || sep >= rest.length - 1) return null
        val kind = rest.substring(0, sep)
        val id = rest.substring(sep + 1)
        if (kind != "user" && kind != "group") return null
        return ThreadRef(kind, id)
    }

    /**
     * Pushes or refreshes a long-lived dynamic conversation shortcut for [ref].
     * Safe to call from any coroutine; network/image decode is bounded to 1.5s with an initials fallback.
     */
    suspend fun pushShortcut(
        context: Context,
        ref: ThreadRef,
        title: String,
        avatarUrl: String?,
        isGroup: Boolean,
    ) {
        val safeTitle = title.trim().ifEmpty { "Chat" }
        val bitmap = loadCircularAvatarBitmap(context, avatarUrl)
            ?: createInitialsBitmap(safeTitle, ref.key, isGroup)
        val icon = bitmap?.let { runCatching { IconCompat.createWithBitmap(it) }.getOrNull() }
        val person = Person.Builder()
            .setKey(ref.key)
            .setName(safeTitle)
            .apply { if (icon != null) setIcon(icon) }
            .build()
        val intent = MainActivity.openChatIntent(context, ref).apply {
            if (action == null) action = Intent.ACTION_VIEW
        }
        val shortcut = ShortcutInfoCompat.Builder(context, shortcutId(ref))
            .setShortLabel(safeTitle)
            .setLongLabel(safeTitle)
            .setLongLived(true)
            .setPerson(person)
            .apply { if (icon != null) setIcon(icon) }
            .setCategories(setOf(SHARE_CATEGORY))
            .setIntent(intent)
            .build()
        runCatching { ShortcutManagerCompat.pushDynamicShortcut(context, shortcut) }
    }

    /**
     * Publishes shortcuts for the most recent conversations (for Direct Share targets).
     * Pushes in reverse order so the top chat receives the highest shortcut rank.
     */
    suspend fun syncRecentShortcuts(
        context: Context,
        threads: List<ThreadSummary>,
        maxCount: Int = 8,
    ) {
        val limit = maxCount.coerceIn(1, ShortcutManagerCompat.getMaxShortcutCountPerActivity(context).coerceAtLeast(1))
        val top = threads.take(limit)
        for (thread in top.asReversed()) {
            pushShortcut(
                context = context,
                ref = ThreadRef(thread.kind, thread.id),
                title = thread.name,
                avatarUrl = thread.avatar.takeIf { it.isNotBlank() },
                isGroup = thread.kind == "group",
            )
        }
    }

    /** Removes a single conversation's dynamic and cached long-lived shortcut. */
    fun removeShortcut(context: Context, ref: ThreadRef) {
        val ids = listOf(shortcutId(ref))
        runCatching { ShortcutManagerCompat.removeDynamicShortcuts(context, ids) }
        runCatching { ShortcutManagerCompat.removeLongLivedShortcuts(context, ids) }
    }

    /** Clears all conversation shortcuts (e.g. on logout) and flushes the in-memory avatar cache. */
    fun clearAll(context: Context) {
        runCatching { ShortcutManagerCompat.removeAllDynamicShortcuts(context) }
        runCatching {
            val cached = ShortcutManagerCompat.getShortcuts(context, ShortcutManagerCompat.FLAG_MATCH_CACHED)
                .map { it.id }
            if (cached.isNotEmpty()) {
                ShortcutManagerCompat.removeLongLivedShortcuts(context, cached)
            }
        }
        synchronized(avatarCacheLock) { avatarCache.clear() }
    }

    /**
     * Loads a circular avatar [IconCompat] via Coil with a 1.5s timeout and a 32-entry in-memory LRU cache.
     * Returns null when [avatarUrl] is blank or cannot be loaded in time.
     */
    suspend fun loadCircularAvatarIcon(context: Context, avatarUrl: String?): IconCompat? {
        val bitmap = loadCircularAvatarBitmap(context, avatarUrl) ?: return null
        return runCatching { IconCompat.createWithBitmap(bitmap) }.getOrNull()
    }

    /**
     * Loads a circular avatar [Bitmap] via Coil with a 1.5s timeout and a 32-entry in-memory LRU cache.
     * Returns null when [avatarUrl] is blank or cannot be loaded in time.
     */
    suspend fun loadCircularAvatarBitmap(context: Context, avatarUrl: String?): Bitmap? {
        val url = avatarUrl?.trim()?.takeIf {
            it.startsWith("http://", ignoreCase = true) || it.startsWith("https://", ignoreCase = true)
        } ?: return null

        synchronized(avatarCacheLock) {
            avatarCache[url]?.takeIf { !it.isRecycled }?.let { return it }
        }

        val loaded = try {
            withContext(Dispatchers.IO) {
                withTimeoutOrNull(LOAD_TIMEOUT_MS) {
                    val request = ImageRequest.Builder(context)
                        .data(url)
                        .size(AVATAR_SIZE_PX, AVATAR_SIZE_PX)
                        .scale(Scale.FILL)
                        .allowHardware(false)
                        .build()
                    val result = context.imageLoader.execute(request) as? SuccessResult ?: return@withTimeoutOrNull null
                    val drawable = result.drawable
                    val rawW = drawable.intrinsicWidth.takeIf { it > 0 } ?: AVATAR_SIZE_PX
                    val rawH = drawable.intrinsicHeight.takeIf { it > 0 } ?: AVATAR_SIZE_PX
                    val raw = drawable.toBitmap(
                        width = rawW.coerceAtMost(AVATAR_SIZE_PX * 2),
                        height = rawH.coerceAtMost(AVATAR_SIZE_PX * 2),
                        config = Bitmap.Config.ARGB_8888,
                    )
                    cropCircle(raw, AVATAR_SIZE_PX)
                }
            }
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            null
        }

        if (loaded != null) {
            synchronized(avatarCacheLock) {
                avatarCache[url] = loaded
            }
        }
        return loaded
    }

    /** Generates a circular initial avatar [IconCompat] fallback when no remote avatar is available. */
    fun createInitialsIcon(title: String, key: String, isGroup: Boolean = false): IconCompat? {
        val bitmap = createInitialsBitmap(title, key, isGroup) ?: return null
        return runCatching { IconCompat.createWithBitmap(bitmap) }.getOrNull()
    }

    /** Generates a circular initial avatar [Bitmap] fallback when no remote avatar is available. */
    fun createInitialsBitmap(title: String, key: String, isGroup: Boolean = false): Bitmap? = runCatching {
        val size = AVATAR_SIZE_PX
        val out = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(out)
        val paletteIndex = abs(key.hashCode()) % AVATAR_PALETTE.size
        val (bgColor, fgColor) = AVATAR_PALETTE[paletteIndex]

        val bgPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = bgColor
            style = Paint.Style.FILL
        }
        val radius = size / 2f
        canvas.drawCircle(radius, radius, radius, bgPaint)

        val label = initialLabel(title, isGroup)
        val textPaint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
            color = fgColor
            textSize = size * 0.44f
            textAlign = Paint.Align.CENTER
            typeface = Typeface.create(Typeface.DEFAULT, Typeface.BOLD)
        }
        val fm = textPaint.fontMetrics
        val baseline = radius - (fm.ascent + fm.descent) / 2f
        canvas.drawText(label, radius, baseline, textPaint)
        out
    }.getOrNull()

    private fun initialLabel(title: String, isGroup: Boolean): String {
        val first = title.trim().firstOrNull { it.isLetterOrDigit() }
        if (first != null) return first.uppercase()
        return if (isGroup) "G" else "?"
    }

    private fun cropCircle(source: Bitmap, size: Int): Bitmap {
        val out = Bitmap.createBitmap(size, size, Bitmap.Config.ARGB_8888)
        val canvas = Canvas(out)
        val shader = BitmapShader(source, Shader.TileMode.CLAMP, Shader.TileMode.CLAMP)
        val srcW = source.width.coerceAtLeast(1).toFloat()
        val srcH = source.height.coerceAtLeast(1).toFloat()
        val scale = maxOf(size / srcW, size / srcH)
        val dx = (size - srcW * scale) / 2f
        val dy = (size - srcH * scale) / 2f
        val matrix = Matrix().apply {
            setScale(scale, scale)
            postTranslate(dx, dy)
        }
        shader.setLocalMatrix(matrix)
        val paint = Paint(Paint.ANTI_ALIAS_FLAG or Paint.FILTER_BITMAP_FLAG).apply {
            this.shader = shader
        }
        val radius = size / 2f
        canvas.drawCircle(radius, radius, radius, paint)
        return out
    }

    // Material 3 tonal container/on-container pairs for deterministic initials fallbacks.
    private val AVATAR_PALETTE = listOf(
        0xFFD8E2FF.toInt() to 0xFF001A41.toInt(),
        0xFFD3E4FF.toInt() to 0xFF001C38.toInt(),
        0xFFC8E6C9.toInt() to 0xFF0A3818.toInt(),
        0xFFFFDDB3.toInt() to 0xFF291800.toInt(),
        0xFFFFD8E4.toInt() to 0xFF31111D.toInt(),
        0xFFEADDFF.toInt() to 0xFF21005D.toInt(),
        0xFFB2EBF2.toInt() to 0xFF002022.toInt(),
    )
}
