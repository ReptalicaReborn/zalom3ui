package com.zalom3ui.app.ui

import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import com.zalom3ui.app.data.ChatMessage
import java.text.Normalizer
import java.util.Locale

private val VIETNAMESE_COMBINING_MARKS = Regex("\\p{Mn}+")

/**
 * Folds Vietnamese text for diacritic- and case-insensitive matching: maps `đ`/`Đ` to `d`,
 * decomposes via NFD, strips combining marks, and lowercases with [Locale.ROOT].
 */
fun foldVietnamese(s: String): String =
    Normalizer.normalize(s.replace('Đ', 'd').replace('đ', 'd'), Normalizer.Form.NFD)
        .replace(VIETNAMESE_COMBINING_MARKS, "")
        .lowercase(Locale.ROOT)

/**
 * Returns IDs of non-recalled [messages] matching [query] in chronological order (`ts` ascending).
 * Matches against message text, attachment title/description, and sender name using [foldVietnamese].
 */
fun findInChatMatches(messages: List<ChatMessage>, query: String): List<String> {
    val needle = foldVietnamese(query.trim())
    if (needle.isEmpty()) return emptyList()
    return messages
        .asSequence()
        .filter { !it.recalled }
        .sortedBy { it.ts }
        .filter { msg ->
            foldVietnamese(msg.text).contains(needle) ||
                msg.attachment?.title?.let { foldVietnamese(it).contains(needle) } == true ||
                msg.attachment?.description?.let { foldVietnamese(it).contains(needle) } == true ||
                foldVietnamese(msg.fromName).contains(needle)
        }
        .map { it.id }
        .toList()
}

/** Formats the `"3 / 12"` (or `"0 / 0"`) counter badge label, or `null` when [query] is blank. */
fun formatInChatMatchCounter(query: String, matchIndex: Int, matchCount: Int): String? {
    if (query.isBlank()) return null
    if (matchCount <= 0) return "0 / 0"
    val displayIndex = matchIndex.coerceIn(0, matchCount - 1) + 1
    return "$displayIndex / $matchCount"
}

/**
 * Material 3 docked search bar for stepping through matches inside a conversation.
 *
 * [matchIndex] is the 0-based index of the currently highlighted match.
 */
@Composable
fun InChatSearchBar(
    query: String,
    onQueryChange: (String) -> Unit,
    matchIndex: Int,
    matchCount: Int,
    onPrev: () -> Unit,
    onNext: () -> Unit,
    onClose: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val focusRequester = remember { FocusRequester() }
    LaunchedEffect(Unit) {
        focusRequester.requestFocus()
    }
    val counterLabel = formatInChatMatchCounter(query, matchIndex, matchCount)
    val hasMatches = matchCount > 0

    Surface(
        modifier = modifier.fillMaxWidth(),
        color = MaterialTheme.colorScheme.surfaceContainer,
        tonalElevation = 2.dp,
    ) {
        Column {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(start = 12.dp, end = 4.dp, top = 6.dp, bottom = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(2.dp),
            ) {
                Surface(
                    modifier = Modifier
                        .weight(1f)
                        .heightIn(min = 42.dp),
                    shape = RoundedCornerShape(24.dp),
                    color = MaterialTheme.colorScheme.surfaceContainerHigh,
                ) {
                    Row(
                        modifier = Modifier.padding(horizontal = 12.dp, vertical = 8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Icon(
                            imageVector = Icons.Outlined.Search,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.size(18.dp),
                        )
                        Spacer(Modifier.width(8.dp))
                        Box(
                            modifier = Modifier.weight(1f),
                            contentAlignment = Alignment.CenterStart,
                        ) {
                            if (query.isEmpty()) {
                                Text(
                                    text = "Search in chat",
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    maxLines = 1,
                                )
                            }
                            BasicTextField(
                                value = query,
                                onValueChange = onQueryChange,
                                singleLine = true,
                                textStyle = MaterialTheme.typography.bodyMedium.copy(
                                    color = MaterialTheme.colorScheme.onSurface,
                                ),
                                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                                keyboardActions = KeyboardActions(
                                    onSearch = { if (hasMatches) onNext() },
                                ),
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .focusRequester(focusRequester),
                            )
                        }
                        if (counterLabel != null) {
                            Spacer(Modifier.width(8.dp))
                            Surface(
                                shape = RoundedCornerShape(10.dp),
                                color = if (hasMatches) {
                                    MaterialTheme.colorScheme.primaryContainer
                                } else {
                                    MaterialTheme.colorScheme.surfaceContainerHighest
                                },
                            ) {
                                Text(
                                    text = counterLabel,
                                    style = MaterialTheme.typography.labelMedium,
                                    fontWeight = FontWeight.SemiBold,
                                    color = if (hasMatches) {
                                        MaterialTheme.colorScheme.onPrimaryContainer
                                    } else {
                                        MaterialTheme.colorScheme.onSurfaceVariant
                                    },
                                    modifier = Modifier.padding(horizontal = 8.dp, vertical = 2.dp),
                                )
                            }
                        }
                    }
                }
                IconButton(onClick = onPrev, enabled = hasMatches) {
                    Icon(
                        imageVector = Icons.Filled.KeyboardArrowUp,
                        contentDescription = "Previous match",
                    )
                }
                IconButton(onClick = onNext, enabled = hasMatches) {
                    Icon(
                        imageVector = Icons.Filled.KeyboardArrowDown,
                        contentDescription = "Next match",
                    )
                }
                IconButton(onClick = onClose) {
                    Icon(
                        imageVector = Icons.Outlined.Close,
                        contentDescription = "Close search",
                    )
                }
            }
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f))
        }
    }
}
