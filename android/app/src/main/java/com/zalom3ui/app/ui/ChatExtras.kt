package com.zalom3ui.app.ui

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
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.items
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.unit.dp
import com.zalom3ui.app.data.StickerInfo
import kotlinx.coroutines.delay

/** Words Zalo's sticker search knows well, offered before anything is typed. */
val STICKER_SUGGESTIONS = listOf("hi", "ok", "haha", "love", "cảm ơn", "buồn", "chúc ngủ ngon", "yêu", "giận", "wow")

/** Zalo's sticker search: type a word (or pick one), tap a sticker to send it. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun StickerSheet(search: StickerSearch, onSearch: (String) -> Unit, onPick: (StickerInfo) -> Unit, onDismiss: () -> Unit) {
    var query by rememberSaveable { mutableStateOf(search.query.ifEmpty { STICKER_SUGGESTIONS.first() }) }
    // Debounced, so typing a word doesn't search every letter.
    LaunchedEffect(query) {
        if (query.isNotBlank()) {
            delay(350)
            if (query.trim() != search.query) onSearch(query)
        }
    }
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.padding(bottom = 16.dp)) {
            SearchPill(
                query = query,
                onQuery = { query = it },
                placeholder = "Search stickers",
                modifier = Modifier.padding(horizontal = 16.dp),
            )
            LazyRow(
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                modifier = Modifier.padding(vertical = 8.dp),
                contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 16.dp),
            ) {
                items(STICKER_SUGGESTIONS) { word ->
                    FilterChip(selected = query.trim() == word, onClick = { query = word }, label = { Text(word) })
                }
            }
            Box(Modifier.fillMaxWidth().heightIn(min = 200.dp, max = 420.dp)) {
                when {
                    search.loading && search.results.isEmpty() -> CircularProgressIndicator(Modifier.align(Alignment.Center))
                    search.results.isEmpty() && search.query.isNotEmpty() && !search.loading -> Text(
                        "No stickers for \"${search.query}\"",
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.align(Alignment.Center),
                    )
                    else -> LazyVerticalGrid(
                        columns = GridCells.Adaptive(88.dp),
                        modifier = Modifier.padding(horizontal = 8.dp),
                    ) {
                        items(search.results, key = { it.id }) { sticker ->
                            StickerImage(
                                sticker.url,
                                Modifier
                                    .padding(4.dp)
                                    .clip(RoundedCornerShape(12.dp))
                                    .clickable { onPick(sticker) },
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
fun RenameGroupDialog(current: String, onDismiss: () -> Unit, onRename: (String) -> Unit) {
    var name by rememberSaveable { mutableStateOf(current) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Rename group") },
        text = {
            OutlinedTextField(value = name, onValueChange = { name = it.take(100) }, singleLine = true, label = { Text("Group name") })
        },
        confirmButton = {
            TextButton(onClick = { onRename(name) }, enabled = name.isNotBlank() && name.trim() != current) { Text("Save") }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

/** Official wording: leaving deletes the chat here; "silently" only tells the group's admins. */
@Composable
fun LeaveGroupDialog(name: String, onDismiss: () -> Unit, onLeave: (silent: Boolean) -> Unit) {
    var silent by rememberSaveable { mutableStateOf(false) }
    AlertDialog(
        onDismissRequest = onDismiss,
        title = { Text("Leave $name?") },
        text = {
            Column {
                Text("You won't see this group's messages any more, and the chat is removed from this phone.")
                Spacer(Modifier.height(12.dp))
                Row(
                    Modifier.fillMaxWidth().clip(RoundedCornerShape(8.dp)).clickable { silent = !silent },
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Checkbox(checked = silent, onCheckedChange = { silent = it })
                    Spacer(Modifier.width(4.dp))
                    Column {
                        Text("Leave silently", style = MaterialTheme.typography.bodyLarge)
                        Text(
                            "Only the group's owner and deputies are told",
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        },
        confirmButton = {
            TextButton(onClick = { onLeave(silent) }) { Text("Leave", color = MaterialTheme.colorScheme.error) }
        },
        dismissButton = { TextButton(onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
fun UploadingNote(count: Int) {
    Row(Modifier.padding(horizontal = 16.dp, vertical = 4.dp), verticalAlignment = Alignment.CenterVertically) {
        CircularProgressIndicator(Modifier.size(14.dp), strokeWidth = 2.dp)
        Spacer(Modifier.width(8.dp))
        Text(
            if (count == 1) "Sending 1 file…" else "Sending $count files…",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}
