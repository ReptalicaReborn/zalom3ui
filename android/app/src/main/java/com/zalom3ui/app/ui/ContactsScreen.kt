package com.zalom3ui.app.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MediumFlexibleTopAppBar
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.zalom3ui.app.data.ThreadSummary

private const val MAX_SEARCH_RESULTS = 300

/** An entry above the list ("New group", "Add friend"…), with an optional count badge. */
class PickerAction(val label: String, val icon: ImageVector, val badge: Int = 0, val onClick: () -> Unit)

/** Pick any friend or group: starting a new chat, or choosing where to forward a message. */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun ContactsScreen(
    title: String,
    threads: List<ThreadSummary>,
    onBack: () -> Unit,
    onPick: (ThreadSummary) -> Unit,
    actions: List<PickerAction> = emptyList(),
) {
    var query by rememberSaveable { mutableStateOf("") }
    val scrollBehavior = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    val folded = foldForSearch(query.trim())
    val (contacts, groups) = remember(threads, folded) {
        val matches = threads
            .filter { folded.isEmpty() || foldForSearch(it.name).contains(folded) }
            .sortedBy { foldForSearch(it.name) }
        matches.filter { it.kind != "group" }.take(MAX_SEARCH_RESULTS) to matches.filter { it.kind == "group" }.take(MAX_SEARCH_RESULTS)
    }

    Scaffold(
        modifier = Modifier.nestedScroll(scrollBehavior.nestedScrollConnection),
        topBar = {
            MediumFlexibleTopAppBar(
                title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") }
                },
                scrollBehavior = scrollBehavior,
            )
        },
    ) { padding ->
        LazyColumn(Modifier.fillMaxSize(), contentPadding = padding) {
            items(actions, key = { "action-${it.label}" }, contentType = { "action" }) { action ->
                Row(
                    Modifier.fillMaxWidth().clickable(onClick = action.onClick).padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Icon(action.icon, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.padding(horizontal = 10.dp))
                    Spacer(Modifier.width(16.dp))
                    Text(action.label, style = MaterialTheme.typography.bodyLarge, modifier = Modifier.weight(1f))
                    if (action.badge > 0) {
                        CountBadge(action.badge.toString(), MaterialTheme.colorScheme.primary, MaterialTheme.colorScheme.onPrimary)
                    }
                }
            }
            item(key = "search", contentType = "search") {
                SearchPill(
                    query = query,
                    onQuery = { query = it },
                    placeholder = "Search name",
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
                )
            }
            if (contacts.isEmpty() && groups.isEmpty()) {
                item(key = "empty", contentType = "empty") {
                    Text(
                        if (threads.isEmpty()) "Your contacts are still loading…" else "No matches",
                        style = MaterialTheme.typography.bodyLarge,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(24.dp),
                    )
                }
            }
            if (contacts.isNotEmpty()) {
                item(key = "h-contacts", contentType = "header") { SectionHeader("Contacts") }
                items(contacts, key = { it.key }, contentType = { "contact" }) { ContactRow(it, onClick = { onPick(it) }) }
            }
            if (groups.isNotEmpty()) {
                item(key = "h-groups", contentType = "header") { SectionHeader("Groups") }
                items(groups, key = { it.key }, contentType = { "contact" }) { ContactRow(it, onClick = { onPick(it) }) }
            }
        }
    }
}

/** A friend or group in a picker or in search results. */
@Composable
fun ContactRow(thread: ThreadSummary, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable(onClick = onClick).padding(horizontal = 16.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Avatar(thread.name, thread.avatar, key = thread.key, isGroup = thread.kind == "group", size = 44.dp)
        Spacer(Modifier.width(16.dp))
        Column {
            Text(thread.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            if (thread.kind == "group") {
                Text("Group", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}
