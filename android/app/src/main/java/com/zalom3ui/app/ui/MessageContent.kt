package com.zalom3ui.app.ui

import android.content.ActivityNotFoundException
import android.content.ContentValues
import android.content.Context
import android.content.Intent
import android.graphics.Bitmap
import android.media.AudioAttributes
import android.media.MediaPlayer
import android.net.Uri
import android.os.Build
import android.os.Environment
import android.provider.MediaStore
import android.widget.Toast
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.calculateCentroid
import androidx.compose.foundation.gestures.calculateCentroidSize
import androidx.compose.foundation.gestures.calculatePan
import androidx.compose.foundation.gestures.calculateZoom
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.gestures.detectTapGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import android.content.ClipData
import android.content.ClipboardManager
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.MissedVideoCall
import androidx.compose.material.icons.automirrored.outlined.OpenInNew
import androidx.compose.material.icons.automirrored.outlined.PhoneMissed
import androidx.compose.material.icons.filled.Pause
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.outlined.BrokenImage
import androidx.compose.material.icons.outlined.Call
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.Description
import androidx.compose.material.icons.outlined.Download
import androidx.compose.material.icons.outlined.Image
import androidx.compose.material.icons.outlined.Link
import androidx.compose.material.icons.outlined.LocationOn
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material.icons.outlined.Videocam
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.FloatingToolbarDefaults
import androidx.compose.material3.HorizontalFloatingToolbar
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.input.pointer.positionChanged
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.core.content.FileProvider
import androidx.core.graphics.drawable.toBitmap
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import coil.Coil
import coil.compose.AsyncImage
import coil.compose.AsyncImagePainter
import coil.request.ImageRequest
import coil.request.SuccessResult
import com.zalom3ui.app.data.Attachment
import com.zalom3ui.app.data.isLocalMediaUri
import java.io.File
import kotlin.math.PI
import kotlin.math.abs
import kotlin.math.sin
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/** Kinds drawn as their own content (picture, player, card) instead of a "[Photo]" label. */
private val SHOWN_KINDS = setOf("photo", "gif", "video", "voice", "file", "link", "contact", "location", "call", "poll")

/** Whether [AttachmentContent] draws this attachment (otherwise the message shows its text label). */
fun Attachment.isShown(): Boolean = kind in SHOWN_KINDS &&
    (href != null || thumb != null || title != null || description != null || otp != null || actionLabel != null || kind in setOf("photo", "gif", "video"))

/** Whether a photo, GIF, or video has a remote URL that can be rendered frameless. */
fun Attachment.hasRenderableMedia(): Boolean = when (kind) {
    "photo", "gif" -> image != null
    "video" -> remoteThumb != null || remoteHref != null
    else -> false
}

/** Caption under a photo or video, when the sender wrote one (ignoring local phone content/file URIs). */
fun Attachment.caption(): String? = title?.takeIf {
    kind in setOf("photo", "gif", "video") && it.isNotBlank() && !isLocalMediaUri(it)
}

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
fun AttachmentContent(
    a: Attachment,
    onOutgoing: Boolean,
    onOpenImage: (String) -> Unit,
    edgeToEdge: Boolean = false,
) {
    val context = LocalContext.current
    when (a.kind) {
        "photo", "gif" -> {
            val image = a.image
            if (image == null) {
                UnavailableMediaRow(
                    label = if (a.kind == "gif") "GIF unavailable" else "Photo unavailable",
                    isVideo = false,
                    onOutgoing = onOutgoing,
                )
                return
            }
            val fullUrl = a.remoteHref ?: image
            val fallback = fullUrl.takeIf { it != image }
            MediaFrame(
                url = image,
                aspect = mediaAspect(a.width, a.height),
                edgeToEdge = edgeToEdge,
                fallbackUrl = fallback,
                onClick = { onOpenImage(fullUrl) },
            )
        }
        "video" -> {
            val thumb = a.remoteThumb
            val videoUrl = a.remoteHref
            if (thumb == null && videoUrl == null) {
                UnavailableMediaRow(label = "Video unavailable", isVideo = true, onOutgoing = onOutgoing)
                return
            }
            Box(contentAlignment = Alignment.Center) {
                if (thumb != null) {
                    MediaFrame(
                        url = thumb,
                        aspect = mediaAspect(a.width, a.height),
                        edgeToEdge = edgeToEdge,
                        onClick = { videoUrl?.let { openExternally(context, it, "video/*") } },
                    )
                } else {
                    val placeholderMod = if (edgeToEdge) {
                        Modifier
                            .widthIn(max = 280.dp)
                            .fillMaxWidth()
                            .aspectRatio(16f / 9f)
                    } else {
                        Modifier
                            .width(220.dp)
                            .aspectRatio(16f / 9f)
                            .clip(RoundedCornerShape(12.dp))
                    }
                    Box(
                        placeholderMod
                            .background(Color.Black)
                            .then(
                                if (videoUrl != null) {
                                    Modifier.clickable { openExternally(context, videoUrl, "video/*") }
                                } else {
                                    Modifier
                                },
                            ),
                    )
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
        "link" -> LinkCard(a, onOutgoing, onClick = a.remoteHref?.let { href -> { openExternally(context, href) } })
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
            // Placeholder under the sticker while it loads or if it fails; stickers are transparent, so it hides once in.
            var loaded by remember(url) { mutableStateOf(false) }
            if (!loaded) StickerPlaceholder()
            AsyncImage(
                model = url,
                contentDescription = "Sticker",
                contentScale = ContentScale.Fit,
                modifier = Modifier.fillMaxSize(),
                onState = { loaded = it is AsyncImagePainter.State.Success },
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
private fun UnavailableMediaRow(label: String, isVideo: Boolean, onOutgoing: Boolean) {
    val content = LocalContentColor.current
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
        modifier = Modifier.padding(vertical = 2.dp),
    ) {
        Icon(
            if (isVideo) Icons.Outlined.Videocam else Icons.Outlined.BrokenImage,
            contentDescription = null,
            tint = content.copy(alpha = 0.72f),
            modifier = Modifier.size(20.dp),
        )
        Text(
            label,
            style = MaterialTheme.typography.bodyMedium,
            color = content.copy(alpha = 0.80f),
        )
    }
}

@Composable
private fun MediaFrame(
    url: String,
    aspect: Float,
    edgeToEdge: Boolean = false,
    fallbackUrl: String? = null,
    onClick: () -> Unit,
) {
    var activeUrl by remember(url, fallbackUrl) { mutableStateOf(url) }
    val frameMod = if (edgeToEdge) {
        val frameHeight = (280.dp / aspect).coerceIn(140.dp, 340.dp)
        Modifier
            .widthIn(max = 280.dp)
            .fillMaxWidth()
            .height(frameHeight)
    } else {
        Modifier
            .widthIn(max = 260.dp)
            .fillMaxWidth()
            .aspectRatio(aspect)
            .heightIn(max = 360.dp)
            .clip(RoundedCornerShape(12.dp))
    }
    // Placeholder under the photo (no subcomposition per bubble): the loading one, or "unavailable" if it fails.
    // Hidden once the photo is in.
    var loadState by remember(activeUrl) { mutableStateOf<AsyncImagePainter.State>(AsyncImagePainter.State.Empty) }
    Box(frameMod.clickable(onClick = onClick)) {
        when (loadState) {
            is AsyncImagePainter.State.Success -> {}
            is AsyncImagePainter.State.Error -> MediaUnavailablePlaceholder()
            else -> MediaPlaceholder()
        }
        AsyncImage(
            model = activeUrl,
            contentDescription = "Photo",
            contentScale = ContentScale.Crop,
            onState = { state ->
                loadState = state
                if (state is AsyncImagePainter.State.Error && fallbackUrl != null && activeUrl != fallbackUrl) {
                    activeUrl = fallbackUrl
                }
            },
            modifier = Modifier.matchParentSize(),
        )
    }
}

@Composable
private fun MediaPlaceholder() {
    Box(
        Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surfaceContainerHighest),
        contentAlignment = Alignment.Center,
    ) { Icon(Icons.Outlined.Image, null, tint = MaterialTheme.colorScheme.onSurfaceVariant) }
}

@Composable
private fun MediaUnavailablePlaceholder() {
    Column(
        Modifier.fillMaxSize().background(MaterialTheme.colorScheme.surfaceContainerHighest).padding(16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.Center,
    ) {
        Icon(
            Icons.Outlined.BrokenImage,
            contentDescription = null,
            tint = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.75f),
            modifier = Modifier.size(28.dp),
        )
        Spacer(Modifier.height(6.dp))
        Text(
            "Media unavailable",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.8f),
        )
    }
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
        call?.video == true && missed -> Icons.AutoMirrored.Outlined.MissedVideoCall
        call?.video == true -> Icons.Outlined.Videocam
        missed -> Icons.AutoMirrored.Outlined.PhoneMissed
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

private val OTP_KEYWORD_RE = Regex(
    """\b(?:otp|ma\s*(?:xac\s*(?:thuc|nhan)|bao\s*mat|dang\s*nhap|kich\s*hoat|xac\s*minh|giao\s*dich|pin)|verification\s*code|security\s*code|auth(?:entication)?\s*code|confirm(?:ation)?\s*code|passcode|one[- ]time(?:\s*(?:code|password|pin))?)\b""",
    RegexOption.IGNORE_CASE,
)
private val OTP_AFTER_RE = Regex(
    """\b(?:otp|ma\s*(?:xac\s*(?:thuc|nhan)|bao\s*mat|dang\s*nhap|kich\s*hoat|xac\s*minh|giao\s*dich|pin)|verification\s*code|security\s*code|auth(?:entication)?\s*code|confirm(?:ation)?\s*code|passcode|one[- ]time(?:\s*(?:code|password|pin))?)\b[^\d\n]{0,40}?(?<!\d)(\d{4,8})(?!\d)""",
    RegexOption.IGNORE_CASE,
)
private val OTP_BEFORE_RE = Regex(
    """(?<!\d)(\d{4,8})(?!\d)[^\d\n]{0,24}\b(?:la\s+)?(?:otp|ma\s*(?:xac\s*(?:thuc|nhan)|bao\s*mat|dang\s*nhap|kich\s*hoat|xac\s*minh|giao\s*dich)|verification\s*code|security\s*code|passcode)\b""",
    RegexOption.IGNORE_CASE,
)

/**
 * Extracts a 4–8 digit OTP / verification code when accompanied by an OTP keyword (Vietnamese or English).
 * Ignores 9+ digit phone numbers or account IDs (e.g. `0378795621`).
 */
internal fun extractOtpCode(text: String): String? {
    if (text.isBlank()) return null
    val folded = foldForSearch(text)
    if (!OTP_KEYWORD_RE.containsMatchIn(folded)) return null
    OTP_AFTER_RE.find(folded)?.groupValues?.getOrNull(1)?.let { return it }
    OTP_BEFORE_RE.find(folded)?.groupValues?.getOrNull(1)?.let { return it }
    return null
}

/** One-tap chip that copies a detected OTP code to the clipboard. */
@Composable
fun OtpCopyChip(otp: String, modifier: Modifier = Modifier) {
    val context = LocalContext.current
    FilledTonalButton(
        onClick = {
            val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
            cm?.setPrimaryClip(ClipData.newPlainText("OTP", otp))
            Toast.makeText(context, "Copied OTP $otp", Toast.LENGTH_SHORT).show()
        },
        contentPadding = PaddingValues(horizontal = 14.dp, vertical = 6.dp),
        modifier = modifier,
    ) {
        Icon(Icons.Outlined.ContentCopy, contentDescription = null, modifier = Modifier.size(16.dp))
        Spacer(Modifier.width(6.dp))
        Text("Copy OTP · $otp", style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold)
    }
}

@Composable
private fun LinkCard(a: Attachment, onOutgoing: Boolean, onClick: (() -> Unit)?) {
    val context = LocalContext.current
    val tint = if (onOutgoing) Color.White.copy(alpha = 0.18f) else MaterialTheme.colorScheme.surface.copy(alpha = 0.6f)
    val isRichCard = a.actionLabel != null || a.otp != null || a.remoteHref == null
    val otp = remember(a.otp, a.title, a.description) {
        a.otp ?: extractOtpCode(listOfNotNull(a.title, a.description).joinToString("\n"))
    }
    Column(
        Modifier
            .widthIn(max = 280.dp)
            .clip(RoundedCornerShape(12.dp))
            .background(tint)
            .then(if (onClick != null && a.actionLabel == null) Modifier.clickable(onClick = onClick) else Modifier),
    ) {
        a.remoteThumb?.let {
            AsyncImage(
                model = it,
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxWidth().height(140.dp),
            )
        }
        Column(Modifier.padding(12.dp)) {
            val host = a.remoteHref?.let { runCatching { Uri.parse(it).host }.getOrNull() }?.removePrefix("www.")
            if (host != null && !isRichCard) {
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Outlined.Link, null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(4.dp))
                    Text(host, style = MaterialTheme.typography.labelMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
                Spacer(Modifier.height(2.dp))
            }
            a.title?.let {
                Text(
                    it,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = if (isRichCard) 3 else 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            a.description?.let {
                if (a.title != null) Spacer(Modifier.height(4.dp))
                Text(
                    it,
                    style = if (isRichCard) MaterialTheme.typography.bodyMedium else MaterialTheme.typography.bodySmall,
                    maxLines = if (isRichCard) 10 else 3,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            if (otp != null) {
                Spacer(Modifier.height(10.dp))
                OtpCopyChip(otp = otp, modifier = Modifier.fillMaxWidth())
            }
            val ctaLabel = a.actionLabel
            if (ctaLabel != null && (onClick != null || otp != null)) {
                Spacer(Modifier.height(10.dp))
                Button(
                    onClick = {
                        if (onClick != null) {
                            onClick()
                        } else if (otp != null) {
                            val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as? ClipboardManager
                            cm?.setPrimaryClip(ClipData.newPlainText("OTP", otp))
                            Toast.makeText(context, "Copied OTP $otp", Toast.LENGTH_SHORT).show()
                        }
                    },
                    modifier = Modifier.fillMaxWidth(),
                    contentPadding = PaddingValues(horizontal = 14.dp, vertical = 10.dp),
                ) {
                    Text(ctaLabel, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
        }
    }
}

// ---- Voice messages ----

/** Live state of the single active voice clip. */
data class VoicePlaybackState(
    val url: String? = null,
    val isPlaying: Boolean = false,
    val positionMs: Int = 0,
    val durationMs: Int = 0,
    val speed: Float = 1f,
)

/** Deterministic speech-like waveform bar heights in `0.22f..1.0f` derived from [seed]. */
internal fun voiceWaveformBars(seed: String, count: Int = 28): List<Float> {
    if (count <= 0) return emptyList()
    var state = -3750763034362895579L // FNV-1a 64-bit offset basis
    for (ch in seed) {
        state = (state xor ch.code.toLong()) * 1099511628211L
    }
    fun nextFloat(): Float {
        state += -7046029254386353131L // SplitMix64 increment
        var z = state
        z = (z xor (z ushr 30)) * -4658895280553007687L
        z = (z xor (z ushr 27)) * -7723592293110705685L
        z = z xor (z ushr 31)
        return ((z ushr 40).toInt() and 0xFFFFFF) / 16777215f
    }
    val phase1 = nextFloat() * (PI * 2.0)
    val phase2 = nextFloat() * (PI * 2.0)
    val freq1 = 2.0 + nextFloat() * 1.5
    val freq2 = 4.0 + nextFloat() * 2.5
    return List(count) { i ->
        val t = if (count == 1) 0.5 else i.toDouble() / (count - 1)
        val edge = 0.55 + 0.45 * sin(PI * t)
        val phrase = 0.5 + 0.3 * sin(PI * freq1 * t + phase1) + 0.2 * sin(PI * freq2 * t + phase2)
        val detail = nextFloat()
        val raw = (edge * (0.55 * phrase + 0.45 * detail)).toFloat()
        (0.22f + raw.coerceIn(0f, 1f) * 0.78f).coerceIn(0.22f, 1.0f)
    }
}

/** Cycles voice playback speed: 1× -> 1.5× -> 2× -> 1×. */
internal fun nextPlaybackSpeed(current: Float): Float = when {
    current < 1.25f -> 1.5f
    current < 1.75f -> 2f
    else -> 1f
}

/** Formats a voice playback speed multiplier as `"1×"`, `"1.5×"`, or `"2×"`. */
internal fun formatSpeed(speed: Float): String = when {
    abs(speed - 1.5f) < 0.05f -> "1.5×"
    abs(speed - 2f) < 0.05f -> "2×"
    else -> "1×"
}

/** Plays one voice message at a time, streamed from Zalo's CDN. */
object VoicePlayback {
    private var player: MediaPlayer? = null
    private var prepared = false
    private var pendingSeekFraction: Float? = null

    private val _playing = MutableStateFlow<String?>(null)
    /** URL of the clip playing (or loading), if any. */
    val playing: StateFlow<String?> = _playing.asStateFlow()

    private val _state = MutableStateFlow(VoicePlaybackState())
    /** Full playback state including live position, duration, and speed. */
    val state: StateFlow<VoicePlaybackState> = _state.asStateFlow()

    fun toggle(context: Context, url: String) {
        val current = _state.value
        val p = player
        if (current.url == url && p != null) {
            if (!prepared) {
                stop()
            } else if (current.isPlaying) {
                runCatching { p.pause() }
                val pos = runCatching { p.currentPosition }.getOrDefault(current.positionMs).coerceAtLeast(0)
                _playing.value = null
                _state.value = current.copy(isPlaying = false, positionMs = pos)
            } else {
                applySpeed(p, current.speed)
                runCatching { p.start() }
                _playing.value = url
                _state.value = current.copy(isPlaying = true)
            }
        } else {
            play(context, url)
        }
    }

    /** Refreshes [state] with the underlying player's live position and duration. */
    fun refreshPosition() {
        val p = player ?: return
        if (!prepared) return
        val cur = _state.value
        val pos = runCatching { p.currentPosition }.getOrNull()?.coerceAtLeast(0) ?: return
        val dur = runCatching { p.duration }.getOrNull()?.takeIf { it > 0 } ?: cur.durationMs
        val isPlayingNow = runCatching { p.isPlaying }.getOrDefault(cur.isPlaying)
        if (pos != cur.positionMs || dur != cur.durationMs || isPlayingNow != cur.isPlaying) {
            _state.value = cur.copy(isPlaying = isPlayingNow, positionMs = pos, durationMs = dur)
        }
    }

    /** Seeks [url] to [fraction] (`0f..1f`) when it is the active clip. */
    fun seekTo(url: String, fraction: Float) {
        val clamped = fraction.coerceIn(0f, 1f)
        val p = player ?: return
        val cur = _state.value
        if (cur.url != url) return
        if (!prepared) {
            pendingSeekFraction = clamped
            return
        }
        val dur = runCatching { p.duration }.getOrDefault(cur.durationMs).coerceAtLeast(0)
        if (dur > 0) {
            val targetMs = (dur * clamped).toInt().coerceIn(0, dur)
            runCatching { p.seekTo(targetMs) }
            _state.value = cur.copy(positionMs = targetMs, durationMs = dur)
        }
    }

    /** Advances playback speed (`1× -> 1.5× -> 2× -> 1×`) and applies it to the active player. */
    fun cycleSpeed(): Float {
        val next = nextPlaybackSpeed(_state.value.speed)
        _state.value = _state.value.copy(speed = next)
        player?.takeIf { prepared }?.let { applySpeed(it, next) }
        return next
    }

    private fun applySpeed(p: MediaPlayer, speed: Float) {
        runCatching {
            val wasPlaying = p.isPlaying
            p.playbackParams = p.playbackParams.setSpeed(speed)
            // Some Android implementations start a paused player when playbackParams is updated.
            if (!wasPlaying && p.isPlaying) p.pause()
        }
    }

    private fun play(context: Context, url: String) {
        val keepSpeed = if (_state.value.url != null) _state.value.speed else 1f
        stop()
        val uri = runCatching { Uri.parse(url.trim()) }.getOrNull()
        val scheme = uri?.scheme?.lowercase()
        if (uri == null || scheme !in setOf("http", "https") || uri.host.isNullOrBlank()) {
            return
        }
        val p = MediaPlayer()
        player = p
        prepared = false
        pendingSeekFraction = null
        _playing.value = url
        _state.value = VoicePlaybackState(url = url, isPlaying = true, speed = keepSpeed)
        try {
            p.setAudioAttributes(
                AudioAttributes.Builder().setUsage(AudioAttributes.USAGE_MEDIA).setContentType(AudioAttributes.CONTENT_TYPE_SPEECH).build(),
            )
            p.setDataSource(uri.toString())
            p.setOnPreparedListener { mp ->
                if (player === mp) {
                    prepared = true
                    val dur = runCatching { mp.duration }.getOrDefault(0).coerceAtLeast(0)
                    pendingSeekFraction?.let { frac ->
                        if (dur > 0) runCatching { mp.seekTo((dur * frac).toInt().coerceIn(0, dur)) }
                        pendingSeekFraction = null
                    }
                    applySpeed(mp, _state.value.speed)
                    runCatching { mp.start() }
                    val pos = runCatching { mp.currentPosition }.getOrDefault(0).coerceAtLeast(0)
                    _state.value = _state.value.copy(isPlaying = true, positionMs = pos, durationMs = dur)
                }
            }
            p.setOnCompletionListener { if (player === it) stop() }
            p.setOnErrorListener { mp, _, _ ->
                if (player === mp) {
                    stop()
                    Toast.makeText(context.applicationContext, "Couldn't play this voice message", Toast.LENGTH_SHORT).show()
                }
                true
            }
            p.prepareAsync()
        } catch (_: Exception) {
            stop()
            Toast.makeText(context.applicationContext, "Couldn't play this voice message", Toast.LENGTH_SHORT).show()
        }
    }

    fun stop() {
        player?.let { runCatching { it.release() } }
        player = null
        prepared = false
        pendingSeekFraction = null
        _playing.value = null
        _state.value = VoicePlaybackState()
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun VoiceClip(a: Attachment, onOutgoing: Boolean) {
    val context = LocalContext.current
    val playback by VoicePlayback.state.collectAsStateWithLifecycle()
    val href = a.href
    val isActiveClip = href != null && playback.url == href
    val isPlaying = isActiveClip && playback.isPlaying

    LaunchedEffect(isPlaying) {
        while (isPlaying) {
            VoicePlayback.refreshPosition()
            delay(60L)
        }
    }

    val bars = remember(href, a.duration) { voiceWaveformBars(href ?: a.title.orEmpty(), count = 28) }
    var dragProgress by remember(href) { mutableStateOf<Float?>(null) }

    val totalMs = when {
        isActiveClip && playback.durationMs > 0 -> playback.durationMs.toLong()
        a.duration != null -> a.duration
        else -> 0L
    }
    val liveProgress = if (isActiveClip && totalMs > 0L) {
        (playback.positionMs.toFloat() / totalMs.toFloat()).coerceIn(0f, 1f)
    } else {
        0f
    }
    val displayProgress = dragProgress ?: liveProgress
    val displayPosMs = when {
        dragProgress != null && totalMs > 0L -> (totalMs * (dragProgress ?: 0f)).toLong()
        isActiveClip -> playback.positionMs.toLong().coerceAtLeast(0L)
        else -> 0L
    }

    // Morph smoothly between a circle (paused) and a 12.dp rounded squircle (playing).
    val playCorner by animateDpAsState(
        targetValue = if (isPlaying) 12.dp else 22.dp,
        animationSpec = MaterialTheme.motionScheme.fastSpatialSpec(),
        label = "voicePlayCorner",
    )
    val playedColor = if (onOutgoing) Color.White else MaterialTheme.colorScheme.primary
    val unplayedColor = playedColor.copy(alpha = 0.35f)
    val timerColor = LocalContentColor.current.copy(alpha = 0.8f)

    Row(
        modifier = Modifier.widthIn(min = 216.dp, max = 264.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Surface(
            shape = RoundedCornerShape(playCorner),
            color = if (onOutgoing) Color.White.copy(alpha = 0.22f) else MaterialTheme.colorScheme.primary,
            contentColor = if (onOutgoing) Color.White else MaterialTheme.colorScheme.onPrimary,
            modifier = Modifier
                .size(44.dp)
                .clip(RoundedCornerShape(playCorner))
                .clickable(enabled = href != null) { href?.let { VoicePlayback.toggle(context, it) } },
        ) {
            Box(contentAlignment = Alignment.Center) {
                Icon(
                    if (isPlaying) Icons.Filled.Pause else Icons.Filled.PlayArrow,
                    contentDescription = if (isPlaying) "Pause" else "Play voice message",
                    modifier = Modifier.size(24.dp),
                )
            }
        }
        Spacer(Modifier.width(10.dp))
        Column(modifier = Modifier.weight(1f)) {
            val currentIsActive by rememberUpdatedState(isActiveClip)
            val scrubModifier = if (href != null) {
                Modifier
                    .pointerInput(href) {
                        detectTapGestures { tap ->
                            val frac = if (size.width > 0) (tap.x / size.width.toFloat()).coerceIn(0f, 1f) else 0f
                            if (!currentIsActive) VoicePlayback.toggle(context, href)
                            VoicePlayback.seekTo(href, frac)
                        }
                    }
                    .pointerInput(href) {
                        detectHorizontalDragGestures(
                            onDragStart = { start ->
                                dragProgress = if (size.width > 0) (start.x / size.width.toFloat()).coerceIn(0f, 1f) else 0f
                            },
                            onHorizontalDrag = { change, _ ->
                                change.consume()
                                dragProgress = if (size.width > 0) (change.position.x / size.width.toFloat()).coerceIn(0f, 1f) else 0f
                            },
                            onDragEnd = {
                                val target = dragProgress
                                dragProgress = null
                                if (target != null) {
                                    if (!currentIsActive) VoicePlayback.toggle(context, href)
                                    VoicePlayback.seekTo(href, target)
                                }
                            },
                            onDragCancel = { dragProgress = null },
                        )
                    }
            } else {
                Modifier
            }

            Canvas(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(28.dp)
                    .then(scrubModifier),
            ) {
                if (bars.isEmpty() || size.width <= 0f || size.height <= 0f) return@Canvas
                val step = size.width / bars.size
                val barWidth = (step * 0.56f).coerceIn(2.dp.toPx(), 3.5.dp.toPx())
                val radius = CornerRadius(barWidth / 2f, barWidth / 2f)
                for (i in bars.indices) {
                    val barHeight = (size.height * bars[i]).coerceAtLeast(barWidth)
                    val left = i * step + (step - barWidth) / 2f
                    val top = (size.height - barHeight) / 2f
                    val barFraction = (i + 0.5f) / bars.size.toFloat()
                    val isBarPlayed = displayProgress > 0f && (barFraction <= displayProgress || (i == 0 && displayProgress > 0.01f))
                    drawRoundRect(
                        color = if (isBarPlayed) playedColor else unplayedColor,
                        topLeft = Offset(left, top),
                        size = Size(barWidth, barHeight),
                        cornerRadius = radius,
                    )
                }
            }

            Spacer(Modifier.height(4.dp))
            Row(
                modifier = Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                val timerText = when {
                    (isActiveClip || dragProgress != null) && totalMs > 0L ->
                        "${formatDuration(displayPosMs)} / ${formatDuration(totalMs)}"
                    isActiveClip -> formatDuration(displayPosMs)
                    totalMs > 0L -> formatDuration(totalMs)
                    else -> "Voice message"
                }
                Text(
                    timerText,
                    style = MaterialTheme.typography.labelSmall.copy(fontFeatureSettings = "tnum"),
                    color = timerColor,
                    maxLines = 1,
                )
                AnimatedVisibility(visible = isActiveClip) {
                    Surface(
                        shape = CircleShape,
                        color = if (onOutgoing) Color.White.copy(alpha = 0.22f) else MaterialTheme.colorScheme.primaryContainer,
                        contentColor = if (onOutgoing) Color.White else MaterialTheme.colorScheme.onPrimaryContainer,
                        modifier = Modifier
                            .clip(CircleShape)
                            .clickable { VoicePlayback.cycleSpeed() },
                    ) {
                        Text(
                            formatSpeed(playback.speed),
                            style = MaterialTheme.typography.labelSmall.copy(
                                fontWeight = FontWeight.SemiBold,
                                fontFeatureSettings = "tnum",
                            ),
                            modifier = Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
                        )
                    }
                }
            }
        }
    }
}

// ---- Photo viewer ----

private const val PHOTO_SHARED_MAX_AGE_MS = 60 * 60_000L

private fun clampPhotoOffset(offset: Offset, scale: Float, viewport: IntSize): Offset {
    if (scale <= 1f || viewport.width <= 0 || viewport.height <= 0) return Offset.Zero
    val maxX = ((viewport.width * (scale - 1f)) / 2f).coerceAtLeast(0f)
    val maxY = ((viewport.height * (scale - 1f)) / 2f).coerceAtLeast(0f)
    return Offset(offset.x.coerceIn(-maxX, maxX), offset.y.coerceIn(-maxY, maxY))
}

private suspend fun loadPhotoBitmap(context: Context, url: String): Bitmap? = withContext(Dispatchers.IO) {
    val uri = runCatching { Uri.parse(url.trim()) }.getOrNull()
    val scheme = uri?.scheme?.lowercase()
    if (uri == null || scheme !in setOf("http", "https") || uri.host.isNullOrBlank()) {
        return@withContext null
    }
    runCatching {
        val request = ImageRequest.Builder(context)
            .data(uri.toString())
            .allowHardware(false)
            .build()
        val result = Coil.imageLoader(context).execute(request)
        if (result is SuccessResult) result.drawable.toBitmap() else null
    }.getOrNull()
}

private suspend fun savePhotoToGallery(context: Context, bitmap: Bitmap): Boolean = withContext(Dispatchers.IO) {
    val isPng = bitmap.hasAlpha()
    val format = if (isPng) Bitmap.CompressFormat.PNG else Bitmap.CompressFormat.JPEG
    val quality = if (isPng) 100 else 95
    val ext = if (isPng) "png" else "jpg"
    val mime = if (isPng) "image/png" else "image/jpeg"
    val name = "zalo-photo-${System.currentTimeMillis()}.$ext"
    runCatching {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            val resolver = context.contentResolver
            val values = ContentValues().apply {
                put(MediaStore.Images.Media.DISPLAY_NAME, name)
                put(MediaStore.Images.Media.MIME_TYPE, mime)
                put(MediaStore.Images.Media.RELATIVE_PATH, "${Environment.DIRECTORY_PICTURES}/Zalỏ")
            }
            val uri = resolver.insert(MediaStore.Images.Media.EXTERNAL_CONTENT_URI, values) ?: return@runCatching false
            try {
                val out = resolver.openOutputStream(uri)
                if (out == null) {
                    runCatching { resolver.delete(uri, null, null) }
                    return@runCatching false
                }
                val ok = out.use { bitmap.compress(format, quality, it) }
                if (!ok) {
                    runCatching { resolver.delete(uri, null, null) }
                    return@runCatching false
                }
            } catch (e: Exception) {
                runCatching { resolver.delete(uri, null, null) }
                throw e
            }
            true
        } else {
            @Suppress("DEPRECATION")
            val dir = File(Environment.getExternalStoragePublicDirectory(Environment.DIRECTORY_PICTURES), "Zalỏ")
            dir.mkdirs()
            val file = File(dir, name)
            val ok = file.outputStream().use { bitmap.compress(format, quality, it) }
            if (!ok) {
                file.delete()
                return@runCatching false
            }
            @Suppress("DEPRECATION")
            context.sendBroadcast(Intent(Intent.ACTION_MEDIA_SCANNER_SCAN_FILE, Uri.fromFile(file)))
            true
        }
    }.getOrDefault(false)
}

private suspend fun sharePhoto(context: Context, bitmap: Bitmap): Boolean {
    val isPng = bitmap.hasAlpha()
    val format = if (isPng) Bitmap.CompressFormat.PNG else Bitmap.CompressFormat.JPEG
    val quality = if (isPng) 100 else 95
    val ext = if (isPng) "png" else "jpg"
    val mime = if (isPng) "image/png" else "image/jpeg"
    val uri = withContext(Dispatchers.IO) {
        runCatching {
            val dir = File(context.cacheDir, "shared").apply { mkdirs() }
            val cutoff = System.currentTimeMillis() - PHOTO_SHARED_MAX_AGE_MS
            dir.listFiles()?.forEach { f -> if (f.isFile && f.lastModified() < cutoff) f.delete() }
            val file = File(dir, "zalo-photo.$ext")
            val ok = file.outputStream().use { bitmap.compress(format, quality, it) }
            if (!ok) return@runCatching null
            FileProvider.getUriForFile(context, "${context.packageName}.files", file)
        }.getOrNull()
    } ?: return false
    val send = Intent(Intent.ACTION_SEND).apply {
        type = mime
        putExtra(Intent.EXTRA_STREAM, uri)
        addFlags(Intent.FLAG_GRANT_READ_URI_PERMISSION)
    }
    return withContext(Dispatchers.Main) {
        runCatching {
            context.startActivity(Intent.createChooser(send, "Share photo").addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
            true
        }.getOrDefault(false)
    }
}

/** Single-photo overload for [PhotoViewer]. */
@Composable
fun PhotoViewer(url: String, onDismiss: () -> Unit, subtitle: String? = null) =
    PhotoViewer(photos = listOf(url), initialIndex = 0, onDismiss = onDismiss, subtitle = subtitle)

/** Full-screen swipeable, zoomable photo gallery with pinch/double-tap zoom, swipe-to-dismiss, and M3 Expressive toolbar. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun PhotoViewer(
    photos: List<String>,
    initialIndex: Int = 0,
    onDismiss: () -> Unit,
    subtitle: String? = null,
) {
    if (photos.isEmpty()) return

    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val density = LocalDensity.current
    val currentOnDismiss by rememberUpdatedState(onDismiss)

    val spatialSpec = MaterialTheme.motionScheme.defaultSpatialSpec<Float>()
    val effectsSpec = MaterialTheme.motionScheme.fastEffectsSpec<Float>()
    val dismissThresholdPx = with(density) { 140.dp.toPx() }

    val pagerState = rememberPagerState(
        initialPage = initialIndex.coerceIn(0, (photos.size - 1).coerceAtLeast(0)),
    ) { photos.size }

    var scale by remember { mutableFloatStateOf(1f) }
    var offset by remember { mutableStateOf(Offset.Zero) }
    var dismissOffsetY by remember { mutableFloatStateOf(0f) }
    var viewportSize by remember { mutableStateOf(IntSize.Zero) }
    var showChrome by remember { mutableStateOf(true) }
    var busyAction by remember { mutableStateOf(false) }
    var animJob by remember { mutableStateOf<Job?>(null) }

    LaunchedEffect(pagerState.currentPage) {
        animJob?.cancel()
        scale = 1f
        offset = Offset.Zero
        dismissOffsetY = 0f
    }

    val scrimAlpha = (1f - abs(dismissOffsetY) / (dismissThresholdPx * 2.5f)).coerceIn(0.15f, 1f)
    val dismissScale = (1f - abs(dismissOffsetY) / (dismissThresholdPx * 10f)).coerceIn(0.88f, 1f)
    val isSwiping = abs(dismissOffsetY) > 1f
    val currentUrl = photos.getOrElse(pagerState.currentPage) { photos.first() }

    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false),
    ) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .background(Color.Black.copy(alpha = scrimAlpha))
                .onSizeChanged { viewportSize = it },
        ) {
            HorizontalPager(
                state = pagerState,
                userScrollEnabled = scale <= 1.01f && !isSwiping,
                modifier = Modifier.fillMaxSize(),
            ) { page ->
                val pageUrl = photos.getOrNull(page) ?: return@HorizontalPager
                val isCurrentPage = page == pagerState.currentPage

                Box(
                    modifier = Modifier
                        .fillMaxSize()
                        .pointerInput(page, spatialSpec, dismissThresholdPx) {
                            awaitEachGesture {
                                var zoomAcc = 1f
                                var panAcc = Offset.Zero
                                var pastTouchSlop = false
                                val touchSlop = viewConfiguration.touchSlop
                                var swipingToDismiss = false
                                var wasCanceled = false

                                awaitFirstDown(requireUnconsumed = false)
                                do {
                                    val event = awaitPointerEvent()
                                    val canceled = event.changes.any { it.isConsumed }
                                    if (canceled) {
                                        wasCanceled = true
                                    } else {
                                        val zoomChange = event.calculateZoom()
                                        val panChange = event.calculatePan()
                                        val pointerCount = event.changes.count { it.pressed }

                                        if (!pastTouchSlop) {
                                            zoomAcc *= zoomChange
                                            panAcc += panChange
                                            val centroidSize = event.calculateCentroidSize(useCurrent = false)
                                            val zoomMotion = abs(1f - zoomAcc) * centroidSize
                                            val panMotion = panAcc.getDistance()
                                            if (pointerCount >= 2 || zoomMotion > touchSlop || panMotion > touchSlop) {
                                                pastTouchSlop = true
                                                if (pointerCount < 2 && scale <= 1.01f && abs(panAcc.y) > abs(panAcc.x)) {
                                                    swipingToDismiss = true
                                                    animJob?.cancel()
                                                } else if (pointerCount >= 2 || scale > 1.01f) {
                                                    animJob?.cancel()
                                                }
                                            }
                                        }

                                        if (pastTouchSlop) {
                                            if (pointerCount >= 2 || (scale > 1.01f && !swipingToDismiss)) {
                                                swipingToDismiss = false
                                                if (dismissOffsetY != 0f) dismissOffsetY = 0f
                                                val newScale = (scale * zoomChange).coerceIn(1f, 4f)
                                                val centroid = event.calculateCentroid(useCurrent = false)
                                                val center = Offset(viewportSize.width / 2f, viewportSize.height / 2f)
                                                val scaleRatio = if (scale > 0f) newScale / scale else 1f
                                                val focusOffset = (centroid - center - offset) * (scaleRatio - 1f)
                                                val candidateOffset = if (newScale <= 1.01f) {
                                                    Offset.Zero
                                                } else {
                                                    offset + panChange - focusOffset
                                                }
                                                scale = newScale
                                                offset = clampPhotoOffset(candidateOffset, newScale, viewportSize)
                                                event.changes.forEach { if (it.positionChanged()) it.consume() }
                                            } else if (swipingToDismiss && scale <= 1.01f) {
                                                dismissOffsetY += panChange.y
                                                event.changes.forEach { if (it.positionChanged()) it.consume() }
                                            }
                                            // Horizontal drags at 1x zoom (scale <= 1.01f) are left unconsumed so HorizontalPager advances smoothly.
                                        }
                                    }
                                } while (!wasCanceled && event.changes.any { it.pressed })

                                if (swipingToDismiss) {
                                    if (!wasCanceled && abs(dismissOffsetY) >= dismissThresholdPx) {
                                        currentOnDismiss()
                                    } else if (dismissOffsetY != 0f) {
                                        val startY = dismissOffsetY
                                        animJob?.cancel()
                                        animJob = scope.launch {
                                            animate(
                                                initialValue = startY,
                                                targetValue = 0f,
                                                animationSpec = spatialSpec,
                                            ) { value, _ -> dismissOffsetY = value }
                                        }
                                    }
                                } else if (scale < 1.02f && (scale != 1f || offset != Offset.Zero)) {
                                    val startScale = scale
                                    val startOffset = offset
                                    animJob?.cancel()
                                    animJob = scope.launch {
                                        animate(
                                            initialValue = 0f,
                                            targetValue = 1f,
                                            animationSpec = spatialSpec,
                                        ) { t, _ ->
                                            val nextScale = (startScale + (1f - startScale) * t).coerceIn(1f, 4f)
                                            scale = nextScale
                                            offset = clampPhotoOffset(
                                                Offset(startOffset.x * (1f - t), startOffset.y * (1f - t)),
                                                nextScale,
                                                viewportSize,
                                            )
                                        }
                                    }
                                }
                            }
                        }
                        .pointerInput(page, spatialSpec) {
                            detectTapGestures(
                                onTap = { showChrome = !showChrome },
                                onDoubleTap = { tap ->
                                    animJob?.cancel()
                                    val startScale = scale
                                    val startOffset = offset
                                    val targetScale = if (startScale > 1.05f) 1f else 2.5f
                                    val targetOffset = if (targetScale <= 1f) {
                                        Offset.Zero
                                    } else {
                                        val center = Offset(viewportSize.width / 2f, viewportSize.height / 2f)
                                        val rel = tap - center
                                        clampPhotoOffset(-rel * (targetScale - 1f), targetScale, viewportSize)
                                    }
                                    animJob = scope.launch {
                                        animate(
                                            initialValue = 0f,
                                            targetValue = 1f,
                                            animationSpec = spatialSpec,
                                        ) { t, _ ->
                                            val nextScale = (startScale + (targetScale - startScale) * t).coerceIn(1f, 4f)
                                            scale = nextScale
                                            offset = clampPhotoOffset(
                                                Offset(
                                                    startOffset.x + (targetOffset.x - startOffset.x) * t,
                                                    startOffset.y + (targetOffset.y - startOffset.y) * t,
                                                ),
                                                nextScale,
                                                viewportSize,
                                            )
                                        }
                                    }
                                },
                            )
                        },
                ) {
                    // The zoom/dismiss transform is on the wrapper, so the loading and error states move with it too.
                    var pageState by remember(pageUrl) { mutableStateOf<AsyncImagePainter.State>(AsyncImagePainter.State.Empty) }
                    Box(
                        Modifier
                            .fillMaxSize()
                            .graphicsLayer {
                                if (isCurrentPage) {
                                    scaleX = scale * dismissScale
                                    scaleY = scale * dismissScale
                                    translationX = offset.x
                                    translationY = offset.y + dismissOffsetY
                                }
                            },
                    ) {
                        AsyncImage(
                            model = pageUrl,
                            contentDescription = "Photo",
                            contentScale = ContentScale.Fit,
                            modifier = Modifier.fillMaxSize(),
                            onState = { pageState = it },
                        )
                        when (pageState) {
                            is AsyncImagePainter.State.Success -> {}
                            is AsyncImagePainter.State.Error -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                                Icon(
                                    Icons.Outlined.Image,
                                    contentDescription = "Couldn't load photo",
                                    tint = Color.White.copy(alpha = 0.6f),
                                    modifier = Modifier.size(48.dp),
                                )
                            }
                            else -> Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                                LoadingIndicator(color = Color.White)
                            }
                        }
                    }
                }
            }

            // Top translucent gradient bar with optional subtitle, gallery pill counter, and Close button.
            AnimatedVisibility(
                visible = showChrome && !isSwiping,
                enter = fadeIn(animationSpec = effectsSpec),
                exit = fadeOut(animationSpec = effectsSpec),
                modifier = Modifier.align(Alignment.TopCenter).fillMaxWidth(),
            ) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .background(
                            Brush.verticalGradient(
                                listOf(Color.Black.copy(alpha = 0.65f), Color.Transparent),
                            ),
                        )
                        .statusBarsPadding()
                        .padding(horizontal = 8.dp, vertical = 8.dp),
                ) {
                    if (!subtitle.isNullOrBlank()) {
                        Text(
                            subtitle,
                            color = Color.White,
                            style = MaterialTheme.typography.titleMedium,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier
                                .align(Alignment.CenterStart)
                                .padding(start = 8.dp, end = if (photos.size > 1) 128.dp else 56.dp),
                        )
                    }
                    if (photos.size > 1) {
                        Surface(
                            shape = CircleShape,
                            color = Color(0x991E1F22),
                            contentColor = Color.White,
                            modifier = Modifier.align(Alignment.Center),
                        ) {
                            Text(
                                text = "${pagerState.currentPage + 1} / ${photos.size}",
                                style = MaterialTheme.typography.labelLarge.copy(
                                    fontWeight = FontWeight.Medium,
                                    fontFeatureSettings = "tnum",
                                ),
                                modifier = Modifier.padding(horizontal = 12.dp, vertical = 5.dp),
                            )
                        }
                    }
                    IconButton(
                        onClick = onDismiss,
                        modifier = Modifier.align(Alignment.CenterEnd),
                    ) {
                        Icon(Icons.Outlined.Close, contentDescription = "Close", tint = Color.White)
                    }
                }
            }

            // Bottom M3 Expressive floating toolbar: Save to gallery, Share, Open externally.
            AnimatedVisibility(
                visible = showChrome && !isSwiping,
                enter = fadeIn(animationSpec = effectsSpec),
                exit = fadeOut(animationSpec = effectsSpec),
                modifier = Modifier
                    .align(Alignment.BottomCenter)
                    .navigationBarsPadding()
                    .padding(bottom = 16.dp),
            ) {
                HorizontalFloatingToolbar(
                    expanded = true,
                    colors = FloatingToolbarDefaults.standardFloatingToolbarColors(
                        toolbarContainerColor = Color(0xCC1E1F22),
                        toolbarContentColor = Color.White,
                    ),
                ) {
                    IconButton(
                        enabled = !busyAction,
                        onClick = {
                            val targetUrl = currentUrl
                            busyAction = true
                            scope.launch {
                                val ok = try {
                                    val bmp = loadPhotoBitmap(context, targetUrl)
                                    if (bmp != null) savePhotoToGallery(context, bmp) else false
                                } finally {
                                    busyAction = false
                                }
                                Toast.makeText(
                                    context,
                                    if (ok) "Saved to gallery" else "Couldn't save photo",
                                    Toast.LENGTH_SHORT,
                                ).show()
                            }
                        },
                    ) {
                        Icon(Icons.Outlined.Download, contentDescription = "Save to gallery")
                    }
                    IconButton(
                        enabled = !busyAction,
                        onClick = {
                            val targetUrl = currentUrl
                            busyAction = true
                            scope.launch {
                                val ok = try {
                                    val bmp = loadPhotoBitmap(context, targetUrl)
                                    if (bmp != null) sharePhoto(context, bmp) else false
                                } finally {
                                    busyAction = false
                                }
                                if (!ok) {
                                    Toast.makeText(context, "Couldn't share photo", Toast.LENGTH_SHORT).show()
                                }
                            }
                        },
                    ) {
                        Icon(Icons.Outlined.Share, contentDescription = "Share")
                    }
                    IconButton(onClick = { openExternally(context, currentUrl, "image/*") }) {
                        Icon(Icons.AutoMirrored.Outlined.OpenInNew, contentDescription = "Open in another app")
                    }
                }
            }
        }
    }
}
