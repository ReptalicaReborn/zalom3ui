package com.zalom3ui.app.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.Chat
import androidx.compose.material.icons.outlined.PersonRemove
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.zalom3ui.app.data.FoundUser
import com.zalom3ui.app.data.FriendRequest
import com.zalom3ui.app.data.GroupMember
import com.zalom3ui.app.data.Person

/** Friends whose name contains [query] (diacritics optional), by name. */
fun filterPeople(people: List<Person>, query: String): List<Person> {
    val q = foldForSearch(query.trim())
    return people.filter { q.isEmpty() || foldForSearch(it.name).contains(q) }.sortedBy { foldForSearch(it.name) }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PeopleScaffold(title: String, onBack: () -> Unit, actions: @Composable () -> Unit = {}, content: @Composable (Modifier) -> Unit) {
    Scaffold(
        topBar = {
            TopAppBar(
                title = { Text(title, maxLines = 1, overflow = TextOverflow.Ellipsis) },
                navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") } },
                actions = { actions() },
            )
        },
    ) { padding -> content(Modifier.padding(padding)) }
}

/**
 * Pick several friends: for a new group ([askName] on, at least 2 people) or to add to a group ([exclude] = its
 * members, shown as already in).
 */
@Composable
fun FriendPickerScreen(
    title: String,
    confirmLabel: String,
    friends: List<Person>,
    exclude: Set<String>,
    askName: Boolean,
    minPicks: Int,
    busy: Boolean,
    onLoad: () -> Unit,
    onBack: () -> Unit,
    onConfirm: (name: String, ids: List<String>) -> Unit,
) {
    LaunchedEffect(Unit) { onLoad() }
    var query by rememberSaveable { mutableStateOf("") }
    var name by rememberSaveable { mutableStateOf("") }
    val picked = remember { mutableStateListOf<String>() }
    val shown = remember(friends, query) { filterPeople(friends, query) }
    PeopleScaffold(
        title = title,
        onBack = onBack,
        actions = {
            TextButton(onClick = { onConfirm(name, picked.toList()) }, enabled = !busy && picked.size >= minPicks) {
                if (busy) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp) else Text(confirmLabel)
            }
        },
    ) { modifier ->
        LazyColumn(modifier.fillMaxSize()) {
            if (askName) {
                item(key = "name") {
                    OutlinedTextField(
                        value = name,
                        onValueChange = { name = it.take(100) },
                        label = { Text("Group name (optional)") },
                        singleLine = true,
                        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
                    )
                }
            }
            item(key = "search") {
                SearchPill(query = query, onQuery = { query = it }, placeholder = "Search friends", modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp))
            }
            item(key = "count") {
                Text(
                    when {
                        picked.isEmpty() && minPicks > 1 -> "Pick at least $minPicks friends"
                        picked.isEmpty() -> "Pick friends to add"
                        else -> "${picked.size} picked"
                    },
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(horizontal = 16.dp, vertical = 4.dp),
                )
            }
            if (friends.isEmpty()) {
                item(key = "empty") { Text("Your friends are still loading…", modifier = Modifier.padding(24.dp)) }
            }
            items(shown, key = { it.id }) { person ->
                val member = person.id in exclude
                val checked = member || person.id in picked
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clickable(enabled = !member && !busy) { if (person.id in picked) picked.remove(person.id) else picked.add(person.id) }
                        .padding(horizontal = 16.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Checkbox(checked = checked, onCheckedChange = null, enabled = !member)
                    Spacer(Modifier.width(8.dp))
                    Avatar(person.name, person.avatar, key = "user:${person.id}", isGroup = false, size = 40.dp)
                    Spacer(Modifier.width(12.dp))
                    Column {
                        Text(person.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        if (member) Text("Already in the group", style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
        }
    }
}

/** Friend requests other people sent: accept or decline. */
@Composable
fun FriendRequestsScreen(
    requests: List<FriendRequest>?,
    onLoad: () -> Unit,
    onBack: () -> Unit,
    onAccept: (FriendRequest) -> Unit,
    onReject: (FriendRequest) -> Unit,
) {
    LaunchedEffect(Unit) { onLoad() }
    PeopleScaffold(title = "Friend requests", onBack = onBack) { modifier ->
        when {
            requests == null -> LoadingScreen()
            requests.isEmpty() -> Text(
                "No friend requests",
                style = MaterialTheme.typography.bodyLarge,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = modifier.padding(24.dp),
            )
            else -> LazyColumn(modifier.fillMaxSize()) {
                items(requests, key = { it.id }) { r ->
                    Row(Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 10.dp), verticalAlignment = Alignment.CenterVertically) {
                        Avatar(r.name, r.avatar, key = "user:${r.id}", isGroup = false, size = 48.dp)
                        Spacer(Modifier.width(12.dp))
                        Column(Modifier.weight(1f)) {
                            Text(r.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
                            if (r.message.isNotBlank()) {
                                Text(r.message, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, maxLines = 2)
                            }
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), modifier = Modifier.padding(top = 6.dp)) {
                                Button(onClick = { onAccept(r) }) { Text("Accept") }
                                OutlinedButton(onClick = { onReject(r) }) { Text("Decline") }
                            }
                        }
                    }
                }
            }
        }
    }
}

/** "Add friend": look someone up by phone number, then ask them or message them. */
@Composable
fun AddFriendScreen(
    onBack: () -> Unit,
    onFind: (phone: String, onResult: (FoundUser?) -> Unit) -> Unit,
    onRequest: (FoundUser, message: String, onSent: () -> Unit) -> Unit,
    onUndo: (FoundUser, onDone: () -> Unit) -> Unit,
    onMessage: (FoundUser) -> Unit,
) {
    var phone by rememberSaveable { mutableStateOf("") }
    var searching by remember { mutableStateOf(false) }
    var found by remember { mutableStateOf<FoundUser?>(null) }
    var message by rememberSaveable { mutableStateOf("") }
    val find = {
        if (phone.isNotBlank() && !searching) {
            searching = true
            found = null
            onFind(phone) { result ->
                searching = false
                found = result
                message = ""
            }
        }
    }
    PeopleScaffold(title = "Add friend", onBack = onBack) { modifier ->
        Column(modifier.fillMaxSize().padding(16.dp)) {
            OutlinedTextField(
                value = phone,
                onValueChange = { phone = it.filter { c -> c.isDigit() || c == '+' || c == ' ' } },
                label = { Text("Phone number") },
                singleLine = true,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Phone, imeAction = ImeAction.Search),
                keyboardActions = KeyboardActions(onSearch = { find() }),
                modifier = Modifier.fillMaxWidth(),
            )
            Spacer(Modifier.height(12.dp))
            Button(onClick = find, enabled = phone.isNotBlank() && !searching) {
                if (searching) CircularProgressIndicator(Modifier.size(18.dp), strokeWidth = 2.dp) else Text("Find")
            }
            found?.let { user ->
                Spacer(Modifier.height(24.dp))
                Row(verticalAlignment = Alignment.CenterVertically) {
                    Avatar(user.name, user.avatar, key = "user:${user.id}", isGroup = false, size = 56.dp)
                    Spacer(Modifier.width(16.dp))
                    Column(Modifier.weight(1f)) {
                        Text(user.name, style = MaterialTheme.typography.titleMedium)
                        Text(
                            when {
                                user.isFriend -> "Already your friend"
                                user.requested -> "Friend request sent"
                                else -> "Not your friend yet"
                            },
                            style = MaterialTheme.typography.bodySmall,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                    IconButton(onClick = { onMessage(user) }) { Icon(Icons.AutoMirrored.Outlined.Chat, contentDescription = "Message ${user.name}") }
                }
                if (!user.isFriend && !user.requested) {
                    Spacer(Modifier.height(16.dp))
                    OutlinedTextField(
                        value = message,
                        onValueChange = { message = it.take(150) },
                        label = { Text("Message (optional)") },
                        modifier = Modifier.fillMaxWidth(),
                    )
                    Spacer(Modifier.height(12.dp))
                    Button(onClick = { onRequest(user, message) { found = user.copy(requested = true) } }) { Text("Send friend request") }
                } else if (user.requested) {
                    Spacer(Modifier.height(12.dp))
                    OutlinedButton(onClick = { onUndo(user) { found = user.copy(requested = false) } }) { Text("Take back request") }
                }
            }
        }
    }
}

/** A group's members; owners and deputies can remove people (Zalo refuses others, with a message). */
@Composable
fun GroupMembersScreen(
    groupName: String,
    members: List<GroupMember>?,
    onLoad: () -> Unit,
    onBack: () -> Unit,
    onAdd: () -> Unit,
    onOpen: (GroupMember) -> Unit,
    onRemove: (GroupMember) -> Unit,
) {
    LaunchedEffect(Unit) { onLoad() }
    var removing by remember { mutableStateOf<GroupMember?>(null) }
    PeopleScaffold(
        title = groupName,
        onBack = onBack,
        actions = { TextButton(onClick = onAdd) { Text("Add") } },
    ) { modifier ->
        if (members == null) {
            LoadingScreen()
            return@PeopleScaffold
        }
        LazyColumn(modifier.fillMaxSize()) {
            item(key = "count") { SectionHeader("${members.size + 1} members (you included)") }
            items(members, key = { it.id }) { m ->
                Row(
                    Modifier.fillMaxWidth().clickable { onOpen(m) }.padding(horizontal = 16.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Avatar(m.name, "", key = m.id, isGroup = false, size = 40.dp)
                    Spacer(Modifier.width(12.dp))
                    Text(m.name, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
                    IconButton(onClick = { removing = m }) { Icon(Icons.Outlined.PersonRemove, contentDescription = "Remove ${m.name}") }
                }
            }
        }
    }
    removing?.let { m ->
        AlertDialog(
            onDismissRequest = { removing = null },
            title = { Text("Remove ${m.name}?") },
            text = { Text("Only the group's owner and deputies can remove members.") },
            confirmButton = {
                TextButton(onClick = {
                    removing = null
                    onRemove(m)
                }) { Text("Remove", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { removing = null }) { Text("Cancel") } },
        )
    }
}
