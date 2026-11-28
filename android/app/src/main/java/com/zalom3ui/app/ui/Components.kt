package com.zalom3ui.app.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.CloudOff
import androidx.compose.material.icons.outlined.Groups
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ContainedLoadingIndicator
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TextField
import androidx.compose.material3.TextFieldDefaults
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.draw.rotate
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Path
import androidx.compose.ui.graphics.StrokeCap
import androidx.compose.ui.graphics.StrokeJoin
import androidx.compose.ui.graphics.drawscope.DrawScope
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.AsyncImage
import coil.compose.AsyncImagePainter
import com.zalom3ui.app.data.ChatMessage
import com.zalom3ui.app.data.MuteOption

/**
 * Person or group avatar: the Zalo picture when there is one, else the first letter on a tinted surface.
 * People are clipped to a circle while groups use a soft expressive squircle.
 * [key] picks the tint, so a person keeps the same color everywhere.
 */
@Composable
fun Avatar(name: String, url: String, key: String, isGroup: Boolean, size: Dp = 48.dp, modifier: Modifier = Modifier) {
    val shape = if (isGroup) RoundedCornerShape((size.value * 0.30f).dp) else CircleShape
    val fallback = @Composable {
        val colors = avatarColors(key)
        Box(
            Modifier.size(size).clip(shape).background(colors.container),
            contentAlignment = Alignment.Center,
        ) {
            if (isGroup && name.isBlank()) {
                Icon(Icons.Outlined.Groups, null, tint = colors.content, modifier = Modifier.size(size * 0.5f))
            } else {
                Text(
                    initial(name),
                    color = colors.content,
                    fontWeight = FontWeight.SemiBold,
                    fontSize = (size.value * 0.4f).sp,
                )
            }
        }
    }
    if (url.isBlank()) {
        Box(modifier) { fallback() }
    } else {
        // Plain AsyncImage over the initials (no subcomposition per chat row). The initials hide in the draw phase
        // once the picture is in, so transparent pictures don't show them through and RAM cache hits don't recompose.
        val loaded = remember(url) { mutableStateOf(false) }
        Box(modifier.size(size).clip(shape)) {
            Box(
                Modifier
                    .matchParentSize()
                    .clearAndSetSemantics {}
                    .drawWithContent { if (!loaded.value) drawContent() },
            ) {
                fallback()
            }
            AsyncImage(
                model = url,
                contentDescription = null,
                contentScale = ContentScale.Crop,
                modifier = Modifier.matchParentSize(),
                onState = { loaded.value = it is AsyncImagePainter.State.Success },
            )
        }
    }
}

/** Delivery state of one of our own messages, shown as check marks in circles. */
enum class Delivery { SENT, DELIVERED, READ }

fun deliveryState(msg: ChatMessage): Delivery? = when {
    !msg.isSelf || msg.recalled -> null
    msg.status == "seen" || (msg.seenCount ?: 0) > 0 -> Delivery.READ
    msg.status == "delivered" -> Delivery.DELIVERED
    msg.status == "sent" -> Delivery.SENT
    else -> null
}

/**
 * One outlined circle with a check when sent, two when delivered, two filled when read.
 * [background] is what the icon sits on, used to cut the front circle out of the back one.
 */
@Composable
fun DeliveryIcon(state: Delivery?, tint: Color, background: Color, modifier: Modifier = Modifier) {
    if (state == null) return
    val double = state != Delivery.SENT
    val stroke = with(LocalDensity.current) { 1.2.dp.toPx() }
    val ringStroke = remember(stroke) { Stroke(stroke) }
    val checkStroke = remember(stroke) { Stroke(width = stroke, cap = StrokeCap.Round, join = StrokeJoin.Round) }
    val checkPath = remember { Path() }
    Canvas(modifier.size(width = if (double) 18.dp else 12.dp, height = 12.dp)) {
        val outer = size.height / 2
        val r = outer - stroke / 2
        val cy = size.height / 2
        if (double) {
            drawDeliveryCircle(Offset(outer, cy), outer, r, stroke, state, tint, background, ringStroke, checkStroke, checkPath)
            drawDeliveryCircle(Offset(size.width - outer, cy), outer, r, stroke, state, tint, background, ringStroke, checkStroke, checkPath)
        } else {
            drawDeliveryCircle(Offset(size.width / 2, cy), outer, r, stroke, state, tint, background, ringStroke, checkStroke, checkPath)
        }
    }
}

private fun DrawScope.drawDeliveryCircle(
    center: Offset,
    outer: Float,
    r: Float,
    stroke: Float,
    state: Delivery,
    tint: Color,
    background: Color,
    ringStroke: Stroke,
    checkStroke: Stroke,
    checkPath: Path,
) {
    if (state == Delivery.READ) {
        drawCircle(background, radius = outer + stroke, center = center)
        drawCircle(tint, radius = outer, center = center)
        drawCheck(center, r, background, checkStroke, checkPath)
    } else {
        drawCircle(background, radius = outer + stroke * 0.5f, center = center)
        drawCircle(tint, radius = r, center = center, style = ringStroke)
        drawCheck(center, r, tint, checkStroke, checkPath)
    }
}

private fun DrawScope.drawCheck(center: Offset, r: Float, color: Color, checkStroke: Stroke, path: Path) {
    path.rewind()
    path.moveTo(center.x - r * 0.45f, center.y + r * 0.02f)
    path.lineTo(center.x - r * 0.12f, center.y + r * 0.36f)
    path.lineTo(center.x + r * 0.46f, center.y - r * 0.30f)
    drawPath(path, color, style = checkStroke)
}

/** Pill with a number (or "@"), as on unread chats. */
@Composable
fun CountBadge(text: String, color: Color, contentColor: Color, modifier: Modifier = Modifier) {
    Box(
        modifier
            .defaultMinSize(minWidth = 22.dp, minHeight = 22.dp)
            .background(color, CircleShape)
            .padding(horizontal = 6.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(text, color = contentColor, style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.Bold)
    }
}

/** Rounded search field used on the chat list and contact pickers. */
@Composable
fun SearchPill(
    query: String,
    onQuery: (String) -> Unit,
    placeholder: String,
    modifier: Modifier = Modifier,
    leading: @Composable (() -> Unit)? = { Icon(Icons.Outlined.Search, null) },
) {
    TextField(
        value = query,
        onValueChange = onQuery,
        placeholder = { Text(placeholder) },
        singleLine = true,
        leadingIcon = leading,
        trailingIcon = {
            if (query.isNotEmpty()) {
                IconButton(onClick = { onQuery("") }) { Icon(Icons.Outlined.Close, contentDescription = "Clear") }
            }
        },
        keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
        shape = RoundedCornerShape(28.dp),
        colors = TextFieldDefaults.colors(
            focusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
            unfocusedContainerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
            focusedIndicatorColor = Color.Transparent,
            unfocusedIndicatorColor = Color.Transparent,
        ),
        modifier = modifier.fillMaxWidth(),
    )
}

@Composable
fun SectionHeader(text: String, modifier: Modifier = Modifier) {
    Text(
        text,
        style = MaterialTheme.typography.titleSmall,
        color = MaterialTheme.colorScheme.onSurface,
        fontWeight = FontWeight.SemiBold,
        modifier = modifier.padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 8.dp),
    )
}

/**
 * Corner shape for an item inside a Material 3 Expressive grouped settings section:
 * 24.dp outer corners and 6.dp inner stacked corners.
 */
fun expressiveGroupItemShape(index: Int, count: Int): RoundedCornerShape = when {
    count <= 1 -> RoundedCornerShape(24.dp)
    index == 0 -> RoundedCornerShape(topStart = 24.dp, topEnd = 24.dp, bottomStart = 6.dp, bottomEnd = 6.dp)
    index == count - 1 -> RoundedCornerShape(topStart = 6.dp, topEnd = 6.dp, bottomStart = 24.dp, bottomEnd = 24.dp)
    else -> RoundedCornerShape(6.dp)
}

/**
 * Material 3 Expressive grouped card container for stacked settings items (24.dp outer corners,
 * 6.dp inner stacked corners, 4.dp gaps, surfaceContainerLow).
 */
@Composable
fun ExpressiveSettingsGroup(
    items: List<@Composable () -> Unit>,
    modifier: Modifier = Modifier,
) {
    if (items.isEmpty()) return
    Column(
        modifier = modifier.fillMaxWidth().padding(horizontal = 16.dp),
        verticalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        items.forEachIndexed { index, item ->
            Surface(
                shape = expressiveGroupItemShape(index, items.size),
                color = MaterialTheme.colorScheme.surfaceContainerLow,
                modifier = Modifier.fillMaxWidth(),
            ) {
                item()
            }
        }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun LoadingScreen(label: String? = null) {
    Column(
        Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        ContainedLoadingIndicator()
        if (label != null) {
            Spacer(Modifier.height(16.dp))
            Text(label, style = MaterialTheme.typography.bodyLarge)
        }
    }
}

/**
 * "Starting Zalo…" for the on-phone server. If it takes long, says what's in the way and offers a restart and the
 * startup log (phones that block background services leave it here).
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun StartingScreen(since: Long, error: String?, onRestart: () -> Unit, onShareLog: () -> Unit) {
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    LaunchedEffect(since) {
        while (true) {
            now = System.currentTimeMillis()
            kotlinx.coroutines.delay(1_000)
        }
    }
    val slow = since > 0 && now - since > SLOW_START_MS
    Column(
        Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        ContainedLoadingIndicator()
        Spacer(Modifier.height(16.dp))
        Text("Starting Zalo…", style = MaterialTheme.typography.bodyLarge)
        if (slow) {
            Spacer(Modifier.height(16.dp))
            Text(
                "This is taking longer than it should." + (error?.let { "\n\n$it" } ?: ""),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(8.dp))
            Text(
                "Some phones (Xiaomi, Oppo, Vivo, Huawei…) block background services for new apps: in the phone's app " +
                    "settings for Zalỏ, allow Autostart and set Battery saver to No restrictions, then restart below.",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
            )
            Spacer(Modifier.height(16.dp))
            Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                OutlinedButton(onClick = onShareLog) { Text("Share startup log") }
                Button(onClick = onRestart) { Text("Restart") }
            }
        }
    }
}

private const val SLOW_START_MS = 12_000L

@Composable
fun ConnectionErrorScreen(error: String, onRetry: () -> Unit, onSettings: () -> Unit) {
    Column(
        Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(Icons.Outlined.CloudOff, null, Modifier.size(48.dp), tint = MaterialTheme.colorScheme.error)
        Spacer(Modifier.height(16.dp))
        Text("Can't reach the server", style = MaterialTheme.typography.titleLarge)
        Spacer(Modifier.height(8.dp))
        Text(
            error,
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
        )
        Spacer(Modifier.height(24.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            OutlinedButton(onClick = onSettings) { Text("Settings") }
            Button(onClick = onRetry) { Text("Try again") }
        }
    }
}

@Composable
fun MuteDialog(onDismiss: () -> Unit, onPick: (MuteOption) -> Unit) {
    var choice by rememberSaveable { mutableStateOf(MuteOption.ONE_HOUR) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Mute notifications") },
        text = {
            Column {
                MuteOption.entries.forEach { option ->
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .selectable(selected = choice == option, onClick = { choice = option })
                            .padding(vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(selected = choice == option, onClick = null)
                        Spacer(Modifier.width(12.dp))
                        Text(option.label, style = MaterialTheme.typography.bodyLarge)
                    }
                }
            }
        },
        confirmButton = { TextButton(onClick = { onPick(choice) }) { Text("Mute") } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** "Delete conversation?" confirmation, shared by the chat list and the chat screen. */
@Composable
fun DeleteConversationDialog(onDismiss: () -> Unit, onConfirm: () -> Unit) {
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Delete conversation?") },
        text = { Text("The chat history is deleted for you. Others in the chat still see it.") },
        confirmButton = { TextButton(onClick = onConfirm) { Text("Delete", color = MaterialTheme.colorScheme.error) } },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** Salmon-pink Material 3 Expressive starburst sticker for the "4x faster!" Easter egg. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun FourXBadge(
    modifier: Modifier = Modifier,
    size: Dp = 84.dp,
    compact: Boolean = false,
) {
    Surface(
        shape = MaterialShapes.SoftBurst.toShape(),
        color = Color(0xFFFF9EA8),
        contentColor = Color(0xFF1F1114),
        shadowElevation = if (compact) 0.dp else 4.dp,
        modifier = modifier
            .size(size)
            .rotate(-8f),
    ) {
        Box(contentAlignment = Alignment.Center) {
            Text(
                text = if (compact) "4x!" else "4x\nfaster!",
                style = if (compact) MaterialTheme.typography.labelSmall else MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.ExtraBold,
                textAlign = TextAlign.Center,
                lineHeight = if (compact) MaterialTheme.typography.labelSmall.lineHeight else MaterialTheme.typography.titleMedium.lineHeight,
            )
        }
    }
}
