package com.zalom3ui.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.Backspace
import androidx.compose.material.icons.outlined.EmojiEmotions
import androidx.compose.material.icons.outlined.FavoriteBorder
import androidx.compose.material.icons.outlined.LocalCafe
import androidx.compose.material.icons.outlined.Mood
import androidx.compose.material.icons.outlined.ThumbUp
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.ToggleButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.zalom3ui.app.data.StickerInfo
import java.text.BreakIterator

private data class EmojiCategory(
    val label: String,
    val icon: ImageVector,
    val emojis: List<String>,
)

private fun splitEmoji(raw: String): List<String> = raw.trim().split(Regex("\\s+"))

/** Common emoji grouped by category for quick tab switching. */
private val EMOJI_CATEGORIES = listOf(
    EmojiCategory(
        label = "Smileys",
        icon = Icons.Outlined.EmojiEmotions,
        emojis = splitEmoji(
            """
            😀 😃 😄 😁 😆 😅 🤣 😂 🙂 🙃 😉 😊 😇 🥰 😍 🤩 😘 😗 😚 😙 😋 😛 😜 🤪 😝 🤑 🤗 🤭 🤫 🤔
            🤐 🤨 😐 😑 😶 😏 😒 🙄 😬 😌 😔 😪 🤤 😴 😷 🤒 🤕 🤢 🤮 🥵 🥶 🥴 😵 🤯 🤠 🥳 😎 🤓 🧐 😕
            😟 🙁 😮 😯 😲 😳 🥺 😦 😧 😨 😰 😥 😢 😭 😱 😖 😣 😞 😓 😩 😫 🥱 😤 😡 😠 🤬 😈 👿 💀 💩
            🤡 👻 👽 🤖 🙈 🙉 🙊
            """,
        ),
    ),
    EmojiCategory(
        label = "Gestures",
        icon = Icons.Outlined.ThumbUp,
        emojis = splitEmoji(
            """
            👋 🤚 ✋ 🖖 👌 🤏 ✌️ 🤞 🤟 🤘 🤙 👈 👉 👆 👇 ☝️ 👍 👎 ✊ 👊 🤛 🤜 👏 🙌 👐 🤲 🙏 💪 👀
            🤝 ✍️ 🫶 🫡 🫰
            """,
        ),
    ),
    EmojiCategory(
        label = "Hearts",
        icon = Icons.Outlined.FavoriteBorder,
        emojis = splitEmoji(
            """
            ❤️ 🧡 💛 💚 💙 💜 🖤 🤍 🤎 💔 ❣️ 💕 💞 💓 💗 💖 💘 💝 🩷 🩵 🩶 🔥 ✨ ⭐ 🌟 💯 🎉 🎊 🎁 🎂
            💐 🌹 🌸 🌺 🌻 🌈 ☀️ 🌙 ⚡ 💤 💬 🎵
            """,
        ),
    ),
    EmojiCategory(
        label = "Objects & Food",
        icon = Icons.Outlined.LocalCafe,
        emojis = splitEmoji(
            """
            ☕ 🍵 🍜 🍚 🍲 🍕 🍔 🍟 🍰 🍩 🍦 🍻 🥂 🧋 🐶 🐱 🐼 🐷 🐸 🐵 ✅ ❌ ❓ ❗ 💰 📌 📷 ⚽ 🏆 🚗
            ✈️ 🏠 ⏰ 📞 💻 📱 📚 🎧 💡 🔑
            """,
        ),
    ),
)

/** Unified keyboard-replacement drawer with an M3 Expressive Emoji | Stickers switcher. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun EmojiStickerDrawer(
    stickers: StickerSearch,
    onEmoji: (String) -> Unit,
    onBackspace: () -> Unit,
    onSearchStickers: (String) -> Unit,
    onSendSticker: (StickerInfo) -> Unit,
    modifier: Modifier = Modifier,
) {
    var showStickers by rememberSaveable { mutableStateOf(false) }
    Surface(color = MaterialTheme.colorScheme.surfaceContainerLow, modifier = modifier) {
        Column(Modifier.fillMaxWidth().height(320.dp)) {
            Row(
                Modifier.fillMaxWidth().padding(start = 12.dp, end = 4.dp, top = 6.dp, bottom = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Row(
                    horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween),
                    modifier = Modifier.weight(1f),
                ) {
                    ToggleButton(
                        checked = !showStickers,
                        onCheckedChange = { showStickers = false },
                        shapes = ButtonGroupDefaults.connectedLeadingButtonShapes(),
                        contentPadding = PaddingValues(horizontal = 14.dp, vertical = 6.dp),
                    ) {
                        Icon(Icons.Outlined.EmojiEmotions, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Emoji")
                    }
                    ToggleButton(
                        checked = showStickers,
                        onCheckedChange = { showStickers = true },
                        shapes = ButtonGroupDefaults.connectedTrailingButtonShapes(),
                        contentPadding = PaddingValues(horizontal = 14.dp, vertical = 6.dp),
                    ) {
                        Icon(Icons.Outlined.Mood, contentDescription = null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(6.dp))
                        Text("Stickers")
                    }
                }
                if (!showStickers) {
                    IconButton(onClick = onBackspace) {
                        Icon(Icons.AutoMirrored.Outlined.Backspace, contentDescription = "Delete")
                    }
                }
            }
            if (showStickers) {
                StickerPickerContent(
                    search = stickers,
                    onSearch = onSearchStickers,
                    onPick = onSendSticker,
                    modifier = Modifier.fillMaxWidth().weight(1f),
                )
            } else {
                EmojiCategoryGrid(onEmoji = onEmoji, modifier = Modifier.fillMaxWidth().weight(1f))
            }
        }
    }
}

/** Emoji grid with category icon tabs shown in place of the keyboard. */
@Composable
fun EmojiPanel(onEmoji: (String) -> Unit, onBackspace: () -> Unit, modifier: Modifier = Modifier) {
    Surface(color = MaterialTheme.colorScheme.surfaceContainerLow, modifier = modifier) {
        Column(Modifier.fillMaxWidth().height(288.dp)) {
            Row(Modifier.fillMaxWidth().padding(start = 16.dp, end = 4.dp), verticalAlignment = Alignment.CenterVertically) {
                Text(
                    "Emoji",
                    style = MaterialTheme.typography.titleSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.weight(1f),
                )
                IconButton(onClick = onBackspace) { Icon(Icons.AutoMirrored.Outlined.Backspace, contentDescription = "Delete") }
            }
            EmojiCategoryGrid(onEmoji = onEmoji, modifier = Modifier.fillMaxWidth().weight(1f))
        }
    }
}

@Composable
private fun EmojiCategoryGrid(onEmoji: (String) -> Unit, modifier: Modifier = Modifier) {
    var tab by rememberSaveable { mutableIntStateOf(0) }
    val selected = EMOJI_CATEGORIES[tab.coerceIn(0, EMOJI_CATEGORIES.lastIndex)]
    Column(modifier) {
        Row(
            Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 2.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            EMOJI_CATEGORIES.forEachIndexed { index, category ->
                val active = index == tab
                Box(
                    Modifier
                        .weight(1f)
                        .height(34.dp)
                        .clip(CircleShape)
                        .background(if (active) MaterialTheme.colorScheme.secondaryContainer else Color.Transparent)
                        .clickable { tab = index },
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        category.icon,
                        contentDescription = category.label,
                        tint = if (active) MaterialTheme.colorScheme.onSecondaryContainer else MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.size(20.dp),
                    )
                }
            }
        }
        LazyVerticalGrid(
            columns = GridCells.Adaptive(48.dp),
            contentPadding = PaddingValues(horizontal = 8.dp, vertical = 4.dp),
            modifier = Modifier.weight(1f),
        ) {
            items(selected.emojis) { emoji ->
                Box(
                    Modifier.aspectRatio(1f).clip(CircleShape).clickable { onEmoji(emoji) },
                    contentAlignment = Alignment.Center,
                ) { Text(emoji, fontSize = 26.sp) }
            }
        }
    }
}

/** Index where the character (grapheme, so a whole emoji) before [cursor] starts. */
internal fun previousCharacterStart(text: String, cursor: Int): Int {
    if (cursor <= 0) return 0
    val it = BreakIterator.getCharacterInstance()
    it.setText(text)
    val start = it.preceding(cursor.coerceAtMost(text.length))
    return if (start == BreakIterator.DONE) cursor - 1 else start
}
