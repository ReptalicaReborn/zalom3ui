package com.zalom3ui.app.ui

import android.content.ActivityNotFoundException
import android.content.Context
import android.content.Intent
import android.media.AudioAttributes
import android.media.MediaPlayer
import android.net.Uri
import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.outlined.Call
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Image
import androidx.compose.material.icons.outlined.Link
import androidx.compose.material.icons.outlined.LocationOn
import androidx.compose.material.icons.outlined.MissedVideoCall
import androidx.compose.material.icons.outlined.PhoneMissed
import androidx.compose.material.icons.outlined.Videocam
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import coil.compose.SubcomposeAsyncImage
import com.zalom3ui.app.data.Attachment
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** Kinds drawn as their own content (picture, player, card) instead of a "[Photo]" label. */
private val SHOWN_KINDS = setOf("photo", "gif", "video", "voice", "file", "link", "contact", "location", "call", "poll")

/** Whether [AttachmentContent] draws this attachment (otherwise the message shows its text label). */
fun Attachment.isShown(): Boolean = kind in SHOWN_KINDS && (href != null || thumb != null || title != null)

/** Caption under a photo or video, when the sender wrote one. */
fun Attachment.caption(): String? = title?.takeIf { kind in setOf("photo", "gif", "video") && it.isNotBlank() }

/** "4:05" for a voice or video length in ms. */
fun formatDuration(ms: Long): String {
    val total = (ms + 500) / 1000
    return "%d:%02d".format(total / 60, total % 60)
}

/** "1.2 MB" for a file size. */
fun formatSize(bytes: Long): String = when {
    bytes >= 1L shl 30 -> "%.1f GB".format(bytes / (1L shl 30).toDouble())
    bytes >= 1L shl 20 -> "%.1f MB".format(bytes / (1L shl 20).toDouble())
    bytes >= 1L shl 10 -> "%.0f KB".format(bytes / (1L shl 10).toDouble())
    else -> "$bytes B"
}

/** Width / height of a photo or video for its frame, kept between a tall portrait and a wide panorama. */
fun mediaAspect(width: Int?, height: Int?): Float {
    if (width == null || height == null || width <= 0 || height <= 0) return 4f / 3f
    return (width.toFloat() / height).coerceIn(0.6f, 2f)
}

/** Opens a link, file or video in another app (browser, player, downloads). */
fun openExternally(context: Context, url: String, mimeType: String? = null) {
    val uri = runCatching { Uri.parse(url.trim()) }.getOrNull()
    val scheme = uri?.scheme?.lowercase()
    val browsable = scheme == "http" || scheme == "https"
    if (uri == null || scheme !in setOf("http", "https", "geo") || (browsable && uri.host.isNullOrBlank())) {
        Toast.makeText(context, "Can't open this link", Toast.LENGTH_SHORT).show()
        return
    }
    fun buildIntent(withMime: Boolean) = Intent(Intent.ACTION_VIEW).apply {
        if (withMime && mimeType != null) setDataAndType(uri, mimeType) else data = uri
        if (browsable) addCategory(Intent.CATEGORY_BROWSABLE)
    }
    try {
        context.startActivity(buildIntent(withMime = true))
    } catch (_: ActivityNotFoundException) {
        if (mimeType != null) {
            try {
                context.startActivity(buildIntent(withMime = false))
                return
            } catch (_: Exception) {
                // Fall through to toast.
            }
        }
        Toast.makeText(context, "No app can open this", Toast.LENGTH_SHORT).show()
    } catch (_: Exception) {
        Toast.makeText(context, "Can't open this link", Toast.LENGTH_SHORT).show()
    }
}

/** The picture, player or card of a non-text message, inside its bubble. */
@Composable
fun AttachmentContent(a: Attachment, onOutgoing: Boolean, onOpenImage: (String) -> Unit) {
    val context = LocalContext.current
    when (a.kind) {
        "photo", "gif" -> {
            val image = a.image ?: return
            MediaFrame(image, mediaAspect(a.width, a.height), onClick = { onOpenImage(a.href ?: image) })
        }
        "video" -> {
            Box(contentAlignment = Alignment.Center) {
                if (a.thumb != null) {
                    MediaFrame(a.thumb, mediaAspect(a.width, a.height), onClick = { a.href?.let { openExternally(context, it, "video/*") } })
                } else {
                    Box(Modifier.width(220.dp).aspectRatio(16f / 9f).clip(RoundedCornerShape(12.dp)).background(Color.Black))
                }
                Surface(shape = CircleShape, color = Color.Black.copy(alpha = 0.55f), contentColor = Color.White) {
                    Icon(Icons.Filled.PlayArrow, contentDescription = "Play video", modifier = Modifier.padding(10.dp).size(28.dp))
                }
                a.duration?.let {
                    Text(
                        formatDuration(it),
                        color = Color.White,
                        style = MaterialTheme.typography.labelMedium,
                        modifier = Modifier
                            .align(Alignment.BottomStart)
                            .padding(8.dp)
                            .clip(RoundedCornerShape(6.dp))
                            .background(Color.Black.copy(alpha = 0.55f))
                            .padding(horizontal = 6.dp, vertical = 2.dp),
                    )
                }
            }
        }
        "voice" -> VoiceClip(a, onOutgoing)
        "file" -> CardRow(
            icon = { FileBadge(a.fileExt ?: a.title?.substringAfterLast('.', "")) },
            title = a.title ?: "File",
            subtitle = listOfNotNull(a.size?.let(::formatSize), a.fileExt?.uppercase()).joinToString(" · ").ifBlank { null },
            onOutgoing = onOutgoing,
            onClick = a.href?.let { href -> { openExternally(context, href) } },
        )
        "link" -> LinkCard(a, onOutgoing, onClick = a.href?.let { href -> { openExternally(context, href) } })
        "contact" -> CardRow(
            icon = { Avatar(a.title.orEmpty(), a.thumb.orEmpty(), key = a.userId ?: a.title.orEmpty(), isGroup = false, size = 40.dp) },
            title = a.title ?: "Contact",
            subtitle = "Contact card",
            onOutgoing = onOutgoing,
            onClick = null,
        )
        "call" -> CallCard(a, onOutgoing)
        "poll" -> a.poll?.let { PollCard(it, onOutgoing) }
        "location" -> CardRow(
            icon = { Icon(Icons.Outlined.LocationOn, null, modifier = Modifier.size(32.dp)) },
            title = a.title ?: "Location",
            subtitle = a.description,
            onOutgoing = onOutgoing,
            onClick = a.href?.let { href -> { openExternally(context, href) } },
        )
    }
}

/** A sticker: the picture alone, no bubble. */
@Composable
fun StickerImage(url: String?, modifier: Modifier = Modifier) {
    Box(modifier.size(128.dp), contentAlignment = Alignment.Center) {
        if (url != null) {
            SubcomposeAsyncImage(
                model = url,
                contentDescription = "Sticker",
                contentScale = ContentScale.Fit,
                modifier = Modifier.fillMaxSize(),
                loading = { StickerPlaceholder() },
                error = { StickerPlaceholder() },
            )
        } else {
            StickerPlaceholder()
        }
    }
}

@Composable
private fun StickerPlaceholder() {
    Box(Modifier.fillMaxSize().clip(RoundedCornerShape(16.dp)).background(MaterialTheme.colorScheme.surfaceContainerHigh))
}

@Composable
private fun MediaFrame(url: String, aspect: Float, onClick: () -> Unit) {
    SubcomposeAsyncImage(
        model = url,
        contentDescription = "Photo",
        contentScale = ContentScale.Crop,
        modifier = Modifier
            .widthIn(max = 260.dp)
            .fillMaxWidth()
            .aspectRatio(aspect)
            .heightIn(max = 360.dp)
            .clip(RoundedCornerShape(12.dp))
            .clickable(onClick = onClick),
        loading = { MediaPlaceholder() },
        error = { MediaPlaceholder() },
    )
}

@Composable
private fun MediaPlaceholder() {
    Box(
        Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surfaceContainerHighest),
        contentAlignment = Alignment.Center,
    ) { Icon(Icons.Outlined.Image, null, tint = MaterialTheme.colorScheme.onSurfaceVariant) }
}

@Composable
private fun CardRow(
    icon: @Composable () -> Unit,
    title: String,
    subtitle: String?,
    onOutgoing: Boolean,
    onClick: (() -> Unit)?,
) {
    val tint = if (onOutgoing) Color.White.copy(alpha = 0.18f) else MaterialTheme.colorScheme.surface.copy(alpha = 0.6f)
    Row(
        Modifier
            .widthIn(min = 200.dp, max = 280.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(tint)
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        icon()
        Spacer(Modifier.width(10.dp))
        Column(Modifier.weight(1f, fill = false)) {
            Text(title, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.Medium, maxLines = 2, overflow = TextOverflow.Ellipsis)
            subtitle?.let { Text(it, style = MaterialTheme.typography.bodySmall, maxLines = 2, overflow = TextOverflow.Ellipsis) }
        }
    }
}

/** A call log entry, drawn like the official client's call card (missed calls in red). Calls can't be placed from here. */
@Composable
private fun CallCard(a: Attachment, onOutgoing: Boolean) {
    val call = a.call
    val missed = call?.missed == true
    val icon = when {
        call?.video == true && missed -> Icons.Outlined.MissedVideoCall
        call?.video == true -> Icons.Outlined.Videocam
        missed -> Icons.Outlined.PhoneMissed
        else -> Icons.Outlined.Call
    }
    val alert = if (onOutgoing) Color.White else MaterialTheme.colorScheme.error
    CardRow(
        icon = { Icon(icon, null, tint = if (missed) alert else LocalContentColor.current, modifier = Modifier.size(32.dp)) },
        title = a.title ?: if (call?.video == true) "Video call" else "Voice call",
        subtitle = a.duration?.let(::formatDuration),
        onOutgoing = onOutgoing,
        onClick = null,
    )
}

@Composable
private fun FileBadge(ext: String?) {
    Box(
        Modifier.size(40.dp).clip(RoundedCornerShape(8.dp)).background(MaterialTheme.colorScheme.primary),
        contentAlignment = Alignment.Center,
    ) {
        val label = ext?.take(4)?.uppercase().orEmpty()
        if (label.isNotEmpty()) {
            Text(label, color = MaterialTheme.colorScheme.onPrimary, style = MaterialTheme.typography.labelSmall, fontWeight = FontWeight.Bold)
        } else {
            Icon(Icons.Outlined.Description, null, tint = MaterialTheme.colorScheme.onPrimary)
        }
    }
}

@Composable
private fun LinkCard(a: Attachment, onOutgoing: Boolean, onClick: (() -> Unit)?) {
    val tint = if (onOutgoing) Color.White.copy(alpha = 0.18f) else MaterialTheme.colorScheme.surface.copy(alpha = 0.6f)
    Column(
        Modifier
            .widthIn(max = 280.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(tint)
            .then(if (onClick != null) Modifier.clickable(onClick = onClick) else Modifier),
    ) {
        a.thumb?.let {
            SubcomposeAsyncImage(
                model = it,
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxWidth().height(140.dp),
                error = {},
            )
        }
        Column(Modifier.padding(10.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Outlined.Link, null, modifier = Modifier.size(16.dp))
                Spacer(Modifier.width(4.dp))
                Text(
                    a.href?.let { runCatching { Uri.parse(it).host }.getOrNull() }?.removePrefix("www.") ?: "Link",
                    style = MaterialTheme.typography.labelMedium,
                    maxLines = 1,
                )
            }
            a.title?.let { Text(it, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.Medium, maxLines = 2, overflow = TextOverflow.Ellipsis) }
            a.description?.let { Text(it, style = MaterialTheme.typography.bodySmall, maxLines = 3, overflow = TextOverflow.Ellipsis) }
        }
    }
}

// ---- Voice messages ----

/** Plays one voice message at a time, streamed from Zalo's CDN. */
object VoicePlayback {
    private var player: MediaPlayer? = null
    private val _playing = MutableStateFlow<String?>(null)
    /** URL of the clip playing (or loading), if any. */
    val playing: StateFlow<String?> = _playing.asStateFlow()

    fun toggle(context: Context, url: String) {
        if (_playing.value == url) stop() else play(context, url)
    }

    private fun play(context: Context, url: String) {
        stop()
        val uri = runCatching { Uri.parse(url.trim()) }.getOrNull()
        val scheme = uri?.scheme?.lowercase()
        if (uri == null || scheme !in setOf("http", "https") || uri.host.isNullOrBlank()) {
            return
        }
        val p = MediaPlayer()
        player = p
        _playing.value = url
        try {
            p.setAudioAttributes(
                AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build(),
            )
            p.setDataSource(uri.toString())
            p.setOnPreparedListener { if (player === it) it.start() }
            p.setOnCompletionListener { if (player === it) stop() }
            p.setOnErrorListener { mp, _, _ ->
                if (player === mp) {
                    stop()
                    Toast.makeText(context.applicationContext, "Couldn't play this voice message", Toast.LENGTH_SHORT).show()
                }
                true
            }
            p.prepareAsync()
        } catch (e: Exception) {
            stop()
            Toast.makeText(context.applicationContext, "Couldn't play this voice message", Toast.LENGTH_SHORT).show()
        }
    }

    fun stop() {
        player?.let { runCatching { it.release() } }
        player = null
        _playing.value = null
    }
}

@Composable
private fun VoiceClip(a: Attachment, onOutgoing: Boolean) {
    val context = LocalContext.current
    val playing by VoicePlayback.playing.collectAsStateWithLifecycle()
    val href = a.href
    val isPlaying = href != null && playing == href
    Row(Modifier.widthIn(min = 180.dp), verticalAlignment = Alignment.CenterVertically) {
        Surface(
            shape = CircleShape,
            color = if (onOutgoing) Color.White.copy(alpha = 0.25f) else MaterialTheme.colorScheme.primary,
            contentColor = if (onOutgoing) Color.White else MaterialTheme.colorScheme.onPrimary,
            modifier = Modifier.size(40.dp).clickable(enabled = href != null) { href?.let { VoicePlayback.toggle(context, it) } },
        ) {
            Box(contentAlignment = Alignment.Center) {
                Icon(if (isPlaying) Icons.Filled.Pause else Icons.Filled.PlayArrow, contentDescription = if (isPlaying) "Pause" else "Play voice message")
            }
        }
        Spacer(Modifier.width(10.dp))
        Column {
            Text("Voice message", style = MaterialTheme.typography.bodyLarge)
            a.duration?.let { Text(formatDuration(it), style = MaterialTheme.typography.bodySmall) }
        }
    }
}

// ---- Photo viewer ----

/** Full-screen photo; tap or back closes it. */
@Composable
fun PhotoViewer(url: String, onDismiss: () -> Unit) {
    val context = LocalContext.current
    Dialog(onDismissRequest = onDismiss, properties = DialogProperties(usePlatformDefaultWidth = false)) {
        Box(Modifier.fillMaxSize().background(Color.Black).clickable(onClick = onDismiss)) {
            SubcomposeAsyncImage(
                model = url,
                contentDescription = "Photo",
                contentScale = ContentScale.Fit,
                modifier = Modifier.fillMaxSize(),
                loading = { Box(Modifier.fillMaxSize()) },
            )
            Row(Modifier.align(Alignment.TopEnd).statusBarsPadding().padding(8.dp)) {
                IconButton(onClick = { openExternally(context, url, "image/*") }) {
                    Icon(Icons.AutoMirrored.Outlined.OpenInNew, contentDescription = "Open in another app", tint = Color.White)
                }
                IconButton(onClick = onDismiss) { Icon(Icons.Outlined.Close, contentDescription = "Close", tint = Color.White) }
            }
        }
    }
}
