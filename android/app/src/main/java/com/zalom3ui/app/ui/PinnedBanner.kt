package com.zalom3ui.app.ui

import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateContentSize
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material.icons.outlined.Timer
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.FilledTonalButton
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import kotlinx.serialization.Serializable

/** One pinned note or announcement on a group's board. */
@Serializable
data class BoardNoteItem(
    val id: String,
    val kind: String,
    val title: String,
    val creatorId: String = "",
    val createTime: Long = 0L,
    val pinned: Boolean = false,
)

/**
 * Compact collapsible Material 3 tonal banner for the top of a group chat, showing the top
 * pinned note/announcement and expanding on tap to reveal all notes and an optional action button.
 */
@Composable
fun PinnedMessageBanner(
    items: List<BoardNoteItem>,
    onCreateNote: (() -> Unit)? = null,
    modifier: Modifier = Modifier,
) {
    if (items.isEmpty()) return

    val ordered = remember(items) { items.sortedByDescending { it.pinned } }
    val top = ordered.first()
    var expanded by rememberSaveable { mutableStateOf(false) }

    Surface(
        modifier = modifier
            .fillMaxWidth()
            .animateContentSize(),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        tonalElevation = 1.dp,
    ) {
        Column(Modifier.fillMaxWidth()) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable { expanded = !expanded }
                    .padding(horizontal = 14.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Box(
                    modifier = Modifier
                        .size(28.dp)
                        .clip(CircleShape),
                    contentAlignment = Alignment.Center,
                ) {
                    Surface(
                        color = MaterialTheme.colorScheme.primaryContainer,
                        shape = CircleShape,
                        modifier = Modifier.size(28.dp),
                    ) {
                        Box(contentAlignment = Alignment.Center) {
                            Icon(
                                imageVector = Icons.Outlined.PushPin,
                                contentDescription = "Pinned",
                                tint = MaterialTheme.colorScheme.onPrimaryContainer,
                                modifier = Modifier.size(16.dp),
                            )
                        }
                    }
                }
                Column(Modifier.weight(1f)) {
                    if (expanded) {
                        Text(
                            text = if (ordered.size == 1) "Pinned note" else "Pinned notes",
                            style = MaterialTheme.typography.labelLarge,
                            fontWeight = FontWeight.SemiBold,
                            color = MaterialTheme.colorScheme.primary,
                            maxLines = 1,
                        )
                    } else {
                        Text(
                            text = top.title,
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurface,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                if (ordered.size > 1) {
                    Surface(
                        shape = RoundedCornerShape(10.dp),
                        color = MaterialTheme.colorScheme.primaryContainer,
                    ) {
                        Text(
                            text = "1/${ordered.size}",
                            style = MaterialTheme.typography.labelSmall,
                            fontWeight = FontWeight.SemiBold,
                            color = MaterialTheme.colorScheme.onPrimaryContainer,
                            modifier = Modifier.padding(horizontal = 7.dp, vertical = 2.dp),
                        )
                    }
                }
                Icon(
                    imageVector = if (expanded) Icons.Filled.KeyboardArrowUp else Icons.Filled.KeyboardArrowDown,
                    contentDescription = if (expanded) "Collapse pinned notes" else "Expand pinned notes",
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(20.dp),
                )
            }

            AnimatedVisibility(visible = expanded) {
                Column(Modifier.fillMaxWidth()) {
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f))
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .heightIn(max = 240.dp)
                            .verticalScroll(rememberScrollState())
                            .padding(horizontal = 14.dp, vertical = 10.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        ordered.forEach { item ->
                            Surface(
                                shape = RoundedCornerShape(14.dp),
                                color = MaterialTheme.colorScheme.surfaceContainerLow,
                                modifier = Modifier.fillMaxWidth(),
                            ) {
                                Row(
                                    modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
                                    verticalAlignment = Alignment.Top,
                                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                                ) {
                                    Icon(
                                        imageVector = Icons.Outlined.PushPin,
                                        contentDescription = null,
                                        tint = MaterialTheme.colorScheme.primary,
                                        modifier = Modifier
                                            .padding(top = 2.dp)
                                            .size(16.dp),
                                    )
                                    Column(Modifier.weight(1f)) {
                                        Text(
                                            text = item.title,
                                            style = MaterialTheme.typography.bodyMedium,
                                            color = MaterialTheme.colorScheme.onSurface,
                                        )
                                        val meta = buildList {
                                            if (item.kind.isNotBlank()) add(item.kind.replaceFirstChar { it.uppercase() })
                                            if (item.createTime > 0L) add(formatListTime(item.createTime))
                                        }.joinToString(" · ")
                                        if (meta.isNotEmpty()) {
                                            Spacer(Modifier.height(2.dp))
                                            Text(
                                                text = meta,
                                                style = MaterialTheme.typography.labelSmall,
                                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                                            )
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if (onCreateNote != null) {
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .padding(start = 14.dp, end = 14.dp, bottom = 10.dp),
                            horizontalArrangement = Arrangement.End,
                        ) {
                            FilledTonalButton(onClick = onCreateNote) {
                                Icon(
                                    imageVector = Icons.Outlined.PushPin,
                                    contentDescription = null,
                                    modifier = Modifier.size(16.dp),
                                )
                                Spacer(Modifier.width(6.dp))
                                Text("Pin a note")
                            }
                        }
                    }
                }
            }

            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f))
        }
    }
}

private val ZALO_GROUP_LINK_REGEX =
    Regex("^(https?://)?(www\\.)?zalo\\.me/g/[A-Za-z0-9_\\-]+/?(\\?.*)?$", RegexOption.IGNORE_CASE)

/** Returns true if [raw] is a non-blank Zalo group invite link (`https://zalo.me/g/...`). */
fun isValidZaloGroupLink(raw: String): Boolean = ZALO_GROUP_LINK_REGEX.matches(raw.trim())

/** Normalizes a validated Zalo group invite link to start with `https://` if no scheme was entered. */
fun normalizeZaloGroupLink(raw: String): String {
    val trimmed = raw.trim()
    return if (trimmed.startsWith("http://", ignoreCase = true) || trimmed.startsWith("https://", ignoreCase = true)) {
        trimmed
    } else {
        "https://$trimmed"
    }
}

/** Material 3 dialog for joining a Zalo group via its `https://zalo.me/g/...` invite link. */
@Composable
fun JoinGroupLinkDialog(
    onDismiss: () -> Unit,
    onJoin: (String) -> Unit,
) {
    var link by rememberSaveable { mutableStateOf("") }
    val trimmed = link.trim()
    val valid = isValidZaloGroupLink(trimmed)
    val showError = trimmed.isNotEmpty() && !valid

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Join group by link") },
        text = {
            Column {
                OutlinedTextField(
                    value = link,
                    onValueChange = { link = it },
                    singleLine = true,
                    isError = showError,
                    label = { Text("Group invite link") },
                    placeholder = { Text("https://zalo.me/g/...") },
                    supportingText = if (showError) {
                        { Text("Enter a valid https://zalo.me/g/... link") }
                    } else {
                        null
                    },
                    trailingIcon = if (link.isNotEmpty()) {
                        {
                            IconButton(onClick = { link = "" }) {
                                Icon(Icons.Outlined.Close, contentDescription = "Clear")
                            }
                        }
                    } else {
                        null
                    },
                    keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onJoin(normalizeZaloGroupLink(trimmed)) },
                enabled = valid,
            ) {
                Text("Join")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}

/** Material 3 dialog for setting or resetting a friend's custom nickname/alias. */
@Composable
fun FriendAliasDialog(
    currentName: String,
    onDismiss: () -> Unit,
    onSave: (String) -> Unit,
    onReset: () -> Unit,
) {
    var alias by rememberSaveable(currentName) { mutableStateOf(currentName) }
    val trimmed = alias.trim()

    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Set nickname") },
        text = {
            Column {
                Text(
                    text = "Only you see this nickname for this friend.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(12.dp))
                OutlinedTextField(
                    value = alias,
                    onValueChange = { alias = it.take(80) },
                    singleLine = true,
                    label = { Text("Nickname") },
                    trailingIcon = if (alias.isNotEmpty()) {
                        {
                            IconButton(onClick = { alias = "" }) {
                                Icon(Icons.Outlined.Close, contentDescription = "Clear")
                            }
                        }
                    } else {
                        null
                    },
                    modifier = Modifier.fillMaxWidth(),
                )
            }
        },
        confirmButton = {
            TextButton(
                onClick = { onSave(trimmed) },
                enabled = trimmed.isNotEmpty() && trimmed != currentName.trim(),
            ) {
                Text("Save")
            }
        },
        dismissButton = {
            Row(horizontalArrangement = Arrangement.spacedBy(4.dp)) {
                TextButton(onClick = onReset) {
                    Text("Reset", color = MaterialTheme.colorScheme.error)
                }
                TextButton(onClick = onDismiss) {
                    Text("Cancel")
                }
            }
        },
    )
}

private data class DisappearingTimerOption(val label: String, val ttlMs: Long?)

private val DISAPPEARING_TIMER_OPTIONS = listOf(
    DisappearingTimerOption("Off", null),
    DisappearingTimerOption("1 minute", 60_000L),
    DisappearingTimerOption("1 hour", 3_600_000L),
    DisappearingTimerOption("24 hours", 86_400_000L),
    DisappearingTimerOption("7 days", 604_800_000L),
    DisappearingTimerOption("30 days", 2_592_000_000L),
)

/** Compact label for a disappearing message TTL (e.g. `"1m"`, `"1h"`, `"24h"`, `"7d"`, `"30d"`). */
fun formatDisappearingTimer(ttlMs: Long?): String = when {
    ttlMs == null || ttlMs <= 0L -> "Off"
    ttlMs == 60_000L -> "1m"
    ttlMs == 3_600_000L -> "1h"
    ttlMs == 86_400_000L -> "24h"
    ttlMs == 604_800_000L -> "7d"
    ttlMs == 2_592_000_000L -> "30d"
    ttlMs < 60_000L -> "${(ttlMs / 1_000L).coerceAtLeast(1L)}s"
    ttlMs < 3_600_000L -> "${ttlMs / 60_000L}m"
    ttlMs < 86_400_000L -> "${ttlMs / 3_600_000L}h"
    else -> "${ttlMs / 86_400_000L}d"
}

/** Material 3 dialog offering disappearing message durations (or `null` for Off). */
@Composable
fun DisappearingTimerDialog(
    currentTtlMs: Long?,
    onDismiss: () -> Unit,
    onSelect: (Long?) -> Unit,
) {
    var selected by rememberSaveable(currentTtlMs) { mutableStateOf(currentTtlMs) }

    AlertDialog(
        onDismissRequest = onDismiss,
        icon = {
            Icon(
                imageVector = Icons.Outlined.Timer,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
            )
        },
        title = { Text("Disappearing messages") },
        text = {
            Column {
                Text(
                    text = "New messages you send will automatically disappear after the chosen time.",
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(10.dp))
                DISAPPEARING_TIMER_OPTIONS.forEach { option ->
                    val isSelected = selected == option.ttlMs
                    Row(
                        modifier = Modifier
                            .fillMaxWidth()
                            .clip(RoundedCornerShape(10.dp))
                            .selectable(
                                selected = isSelected,
                                role = Role.RadioButton,
                                onClick = { selected = option.ttlMs },
                            )
                            .padding(vertical = 6.dp, horizontal = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        RadioButton(selected = isSelected, onClick = null)
                        Spacer(Modifier.width(12.dp))
                        Text(option.label, style = MaterialTheme.typography.bodyLarge)
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onSelect(selected) }) {
                Text("Apply")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) {
                Text("Cancel")
            }
        },
    )
}

/** Compact Material 3 pill badge shown in the chat composer when a disappearing message timer is active. */
@Composable
fun DisappearingTimerChip(
    ttlMs: Long,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    Surface(
        onClick = onClick,
        modifier = modifier,
        shape = RoundedCornerShape(14.dp),
        color = MaterialTheme.colorScheme.primaryContainer,
        contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
    ) {
        Row(
            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
        ) {
            Icon(
                imageVector = Icons.Outlined.Timer,
                contentDescription = "Disappearing timer",
                modifier = Modifier.size(14.dp),
            )
            Text(
                text = formatDisappearingTimer(ttlMs),
                style = MaterialTheme.typography.labelSmall,
                fontWeight = FontWeight.SemiBold,
            )
        }
    }
}
