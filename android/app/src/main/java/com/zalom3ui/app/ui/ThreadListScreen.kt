package com.zalom3ui.app.ui

import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.SizeTransform
import androidx.compose.animation.core.animate
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.defaultMinSize
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.Chat
import androidx.compose.material.icons.filled.PushPin
import androidx.compose.material.icons.outlined.Campaign
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Drafts
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.MarkChatUnread
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.Notifications
import androidx.compose.material.icons.outlined.NotificationsOff
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.Sync
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material.icons.outlined.WifiOff
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Badge
import androidx.compose.material3.BadgedBox
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FloatingActionButton
import androidx.compose.material.icons.outlined.People
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialShapes
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MediumFlexibleTopAppBar
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.ToggleButton
import androidx.compose.material3.ToggleButtonDefaults
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.material3.toShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.painter.ColorPainter
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import coil.compose.AsyncImage
import com.zalom3ui.app.R
import com.zalom3ui.app.data.MessageSearchHit
import com.zalom3ui.app.data.MuteOption
import com.zalom3ui.app.data.SessionStatus
import com.zalom3ui.app.data.ThreadSummary
import kotlin.math.abs
import kotlin.math.roundToInt
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/** Everything the chat list can ask for. */
class ThreadListActions(
    val onOpen: (ThreadSummary) -> Unit,
    val onNewChat: () -> Unit,
    val onContacts: () -> Unit,
    val onReconnect: () -> Unit,
    val onSettings: () -> Unit,
    val onMarkAllRead: () -> Unit,
    val onPin: (ThreadSummary, Boolean) -> Unit,
    val onMute: (ThreadSummary, MuteOption?) -> Unit,
    val onMarkRead: (ThreadSummary) -> Unit,
    val onMarkUnread: (ThreadSummary) -> Unit,
    /** Turns on "Hide official accounts" (from an OA chat's long-press menu). */
    val onHideOfficialAccounts: () -> Unit = {},
    val onDelete: (ThreadSummary) -> Unit,
    val onSearchMessages: (String) -> Unit = {},
    val onOpenSearchHit: (MessageSearchHit) -> Unit = {},
    val onJoinGroupLink: (String) -> Unit = {},
)

/** Filter modes shown in the chat list's connected pill bar. */
enum class ChatFilter {
    ALL,
    UNREAD,
    GROUPS,
    /** Only Official Accounts; reached from the "Official accounts (N)" row, not from the pill bar. */
    OFFICIAL,
}

/** Pure helper that filters conversation summaries according to [filter], preserving order. */
internal fun filterChats(chats: List<ThreadSummary>, filter: ChatFilter): List<ThreadSummary> = when (filter) {
    ChatFilter.ALL -> chats
    ChatFilter.UNREAD -> chats.filter { it.unread > 0 }
    ChatFilter.GROUPS -> chats.filter { it.kind == "group" }
    ChatFilter.OFFICIAL -> chats.filter { it.isOfficialAccount }
}

/**
 * The chats the list shows for [filter]. With [hideOfficialAccounts] on, Official Accounts are left out of every
 * filter except [ChatFilter.OFFICIAL], which exists to reach them.
 */
internal fun chatsForFilter(chats: List<ThreadSummary>, filter: ChatFilter, hideOfficialAccounts: Boolean): List<ThreadSummary> {
    val base = if (hideOfficialAccounts && filter != ChatFilter.OFFICIAL) chats.filterNot { it.isOfficialAccount } else chats
    return filterChats(base, filter)
}

/** How many Official Account chats the "Official accounts (N)" row stands for; 0 when they are not hidden. */
internal fun hiddenOfficialAccountCount(chats: List<ThreadSummary>, hideOfficialAccounts: Boolean): Int =
    if (hideOfficialAccounts) chats.count { it.isOfficialAccount } else 0

private val WHITESPACE_REGEX = Regex("\\s+")

/** Trims [draft], collapses newlines/whitespace runs to single spaces, and returns null if blank. */
internal fun formatDraftSnippet(draft: String?): String? =
    draft?.trim()?.takeIf { it.isNotEmpty() }?.replace(WHITESPACE_REGEX, " ")

/** Finds non-overlapping character ranges in [text] matching [query] (case- and diacritic-insensitive). */
internal fun findHighlightRanges(text: String, query: String): List<IntRange> {
    val needle = query.trim()
    if (text.isEmpty() || needle.isEmpty()) return emptyList()
    val direct = mutableListOf<IntRange>()
    var start = 0
    while (start < text.length) {
        val idx = text.indexOf(needle, startIndex = start, ignoreCase = true)
        if (idx < 0) break
        direct.add(idx until (idx + needle.length))
        start = idx + needle.length
    }
    if (direct.isNotEmpty()) return direct

    val foldedNeedle = foldForSearch(needle)
    if (foldedNeedle.isEmpty()) return emptyList()

    val foldedBuilder = StringBuilder(text.length)
    val origStarts = ArrayList<Int>(text.length)
    val origEnds = ArrayList<Int>(text.length)
    var i = 0
    while (i < text.length) {
        val chStart = i
        val cp = text.codePointAt(i)
        i += Character.charCount(cp)
        while (i < text.length && Character.getType(text.codePointAt(i)) == Character.NON_SPACING_MARK.toInt()) {
            i += Character.charCount(text.codePointAt(i))
        }
        val folded = foldForSearch(text.substring(chStart, i))
        for (fc in folded) {
            foldedBuilder.append(fc)
            origStarts.add(chStart)
            origEnds.add(i)
        }
    }
    val foldedText = foldedBuilder.toString()
    val ranges = mutableListOf<IntRange>()
    var fStart = 0
    while (fStart < foldedText.length) {
        val fIdx = foldedText.indexOf(foldedNeedle, startIndex = fStart)
        if (fIdx < 0) break
        val fEnd = fIdx + foldedNeedle.length - 1
        ranges.add(origStarts[fIdx] until origEnds[fEnd])
        fStart = fIdx + foldedNeedle.length
    }
    return ranges
}

private enum class RowAction { PIN, UNPIN, MUTE, UNMUTE, READ, UNREAD, DELETE, HIDE_OFFICIAL }

/**
 * The chat list. It takes only the parts of [UiState] it shows, so it skips recomposing when anything else changes;
 * each row gets only its own typing and draft, so a change in one chat redraws only that row.
 */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun ThreadListScreen(
    threads: List<ThreadSummary>,
    /** Draft text by [com.zalom3ui.app.data.ThreadRef.key]. */
    drafts: Map<String, String>,
    /** Who is typing, by thread key. */
    typing: Map<String, TypingInfo>,
    status: SessionStatus?,
    /** The saved session reconnecting in the background: a slim note above the chats (an enum, so it rarely changes). */
    connectionNote: ConnectionNote = ConnectionNote.None,
    connectionError: String?,
    /** Friend requests waiting for an answer, for the badge on Contacts. */
    pendingRequests: Int,
    messageSearchResults: List<MessageSearchHit>,
    messageSearchBusy: Boolean,
    actions: ThreadListActions,
    selectedKey: String? = null,
    /** "Hide official accounts" setting: OA chats move behind an "Official accounts (N)" row. */
    hideOfficialAccounts: Boolean = false,
) {
    var searching by rememberSaveable { mutableStateOf(false) }
    var query by rememberSaveable { mutableStateOf("") }
    var filter by rememberSaveable { mutableStateOf(ChatFilter.ALL) }
    var sheetFor by remember { mutableStateOf<ThreadSummary?>(null) }
    var muteFor by remember { mutableStateOf<ThreadSummary?>(null) }
    var deleteFor by remember { mutableStateOf<ThreadSummary?>(null) }
    var showJoinGroupDialog by rememberSaveable { mutableStateOf(false) }
    val closeSearch: () -> Unit = {
        searching = false
        query = ""
    }
    BackHandler(enabled = searching) { closeSearch() }
    BackHandler(enabled = !searching && filter == ChatFilter.OFFICIAL) { filter = ChatFilter.ALL }

    val scrollBehavior = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    val latestOnSearchMessages by rememberUpdatedState(actions.onSearchMessages)
    LaunchedEffect(searching) {
        if (searching) scrollBehavior.state.heightOffset = 0f
    }
    LaunchedEffect(searching, query) {
        latestOnSearchMessages(if (searching) query else "")
    }
    // The chat list shows conversations; contacts you never talked to are under search and "New chat".
    val chats = remember(threads, drafts) {
        threads.filter { it.lastMessage != null || it.pinned || it.unread > 0 || !drafts[it.ref.key].isNullOrBlank() }
    }
    val unreadChatCount = remember(chats, hideOfficialAccounts) {
        chats.count { it.unread > 0 && !(hideOfficialAccounts && it.isOfficialAccount) }
    }
    val officialCount = remember(chats, hideOfficialAccounts) { hiddenOfficialAccountCount(chats, hideOfficialAccounts) }
    // If the setting is turned off while the official-accounts list is open, fall back to all chats.
    val activeFilter = if (filter == ChatFilter.OFFICIAL && !hideOfficialAccounts) ChatFilter.ALL else filter
    val visibleChats = remember(chats, activeFilter, searching, hideOfficialAccounts) {
        if (searching) chats else chatsForFilter(chats, activeFilter, hideOfficialAccounts)
    }
    val folded = foldForSearch(query.trim())
    val matches = remember(threads, folded) {
        if (folded.isEmpty()) emptyList() else threads.filter { foldForSearch(it.name).contains(folded) }
    }

    val onRowAction: (ThreadSummary, RowAction) -> Unit = { thread, action ->
        sheetFor = null
        when (action) {
            RowAction.PIN -> actions.onPin(thread, true)
            RowAction.UNPIN -> actions.onPin(thread, false)
            RowAction.MUTE -> muteFor = thread
            RowAction.UNMUTE -> actions.onMute(thread, null)
            RowAction.READ -> actions.onMarkRead(thread)
            RowAction.UNREAD -> actions.onMarkUnread(thread)
            RowAction.DELETE -> deleteFor = thread
            RowAction.HIDE_OFFICIAL -> actions.onHideOfficialAccounts()
        }
    }

    Scaffold(
        modifier = Modifier
            .onPreviewKeyEvent { event ->
                if (event.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                when {
                    event.isCtrlPressed && event.key == Key.K -> {
                        searching = true
                        true
                    }
                    searching && event.key == Key.Escape -> {
                        closeSearch()
                        true
                    }
                    else -> false
                }
            }
            .then(if (searching) Modifier else Modifier.nestedScroll(scrollBehavior.nestedScrollConnection)),
        topBar = {
            if (searching) {
                SearchTopBar(query = query, onQuery = { query = it }, onClose = closeSearch)
            } else {
                MediumFlexibleTopAppBar(
                    navigationIcon = {
                        IconButton(onClick = actions.onSettings) {
                            Avatar(
                                name = status?.ownName.orEmpty(),
                                url = status?.ownAvatar.orEmpty(),
                                key = status?.ownId ?: "me",
                                isGroup = false,
                                size = 32.dp,
                            )
                        }
                    },
                    title = {
                        Text(
                            text = stringResource(R.string.app_name),
                            fontWeight = FontWeight.Bold,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    },
                    actions = {
                        IconButton(onClick = actions.onContacts) {
                            BadgedBox(badge = { if (pendingRequests > 0) Badge { Text(pendingRequests.toString()) } }) {
                                Icon(Icons.Outlined.People, contentDescription = "Contacts")
                            }
                        }
                        IconButton(onClick = { searching = true }) {
                            Icon(Icons.Outlined.Search, contentDescription = "Search")
                        }
                        OverflowMenu(
                            hasUnread = unreadChatCount > 0,
                            onMarkAllRead = actions.onMarkAllRead,
                            onJoinGroupLink = { showJoinGroupDialog = true },
                            onSettings = actions.onSettings,
                        )
                    },
                    scrollBehavior = scrollBehavior,
                )
            }
        },
        floatingActionButton = {
            if (!searching) {
                val ultra = LocalUltraExpressive.current
                FloatingActionButton(
                    onClick = actions.onNewChat,
                    shape = if (ultra) MaterialShapes.SoftBurst.toShape() else RoundedCornerShape(20.dp),
                    containerColor = if (ultra) Color(0xFFFF9EA8) else MaterialTheme.colorScheme.primaryContainer,
                    contentColor = if (ultra) Color(0xFF1F1114) else MaterialTheme.colorScheme.onPrimaryContainer,
                    modifier = Modifier.size(if (ultra) 116.dp else 56.dp),
                ) {
                    Icon(
                        Icons.Outlined.Edit,
                        contentDescription = "New chat",
                        modifier = Modifier.size(if (ultra) 54.dp else 24.dp),
                    )
                }
            }
        },
    ) { padding ->
        val ultra = LocalUltraExpressive.current
        Box(Modifier.fillMaxSize()) {
            LazyColumn(
                contentPadding = PaddingValues(
                    top = padding.calculateTopPadding(),
                    bottom = padding.calculateBottomPadding() + if (ultra) 180.dp else 120.dp,
                ),
                modifier = Modifier.fillMaxSize(),
            ) {
            if (searching && query.isNotBlank()) {
                val chatHits = matches.filter { it.lastMessage != null || !drafts[it.ref.key].isNullOrBlank() }
                val contactHits = matches.filter { it.lastMessage == null && drafts[it.ref.key].isNullOrBlank() && it.kind == "user" }.sortedBy { foldForSearch(it.name) }
                val groupHits = matches.filter { it.lastMessage == null && drafts[it.ref.key].isNullOrBlank() && it.kind == "group" }.sortedBy { foldForSearch(it.name) }
                val messageHits = messageSearchResults
                if (matches.isEmpty() && !messageSearchBusy && messageHits.isEmpty()) {
                    item(key = "no-results", contentType = "no-results") {
                        Text(
                            "No results for “${query.trim()}”",
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(24.dp),
                        )
                    }
                }
                if (chatHits.isNotEmpty()) {
                    item(key = "h-chat-hits", contentType = "header") { SectionHeader("Chats") }
                    items(chatHits, key = { it.key }, contentType = { "thread" }) { thread ->
                        ConversationRow(
                            thread = thread,
                            selected = thread.key == selectedKey,
                            typing = typing[thread.key],
                            draft = drafts[thread.ref.key],
                            swipeEnabled = false,
                            onClick = { actions.onOpen(thread) },
                            onLongClick = {},
                            onAction = {},
                        )
                    }
                }
                if (contactHits.isNotEmpty()) {
                    item(key = "h-contact-hits", contentType = "header") { SectionHeader("Contacts") }
                    items(contactHits.take(MAX_RESULTS), key = { it.key }, contentType = { "contact" }) { ContactRow(it, onClick = { actions.onOpen(it) }) }
                }
                if (groupHits.isNotEmpty()) {
                    item(key = "h-group-hits", contentType = "header") { SectionHeader("Groups") }
                    items(groupHits.take(MAX_RESULTS), key = { it.key }, contentType = { "contact" }) { ContactRow(it, onClick = { actions.onOpen(it) }) }
                }
                if (messageSearchBusy || messageHits.isNotEmpty()) {
                    item(key = "h-message-hits", contentType = "message-hits-header") {
                        MessagesSectionHeader(busy = messageSearchBusy)
                    }
                    items(
                        items = messageHits,
                        key = { hit -> "msg:${hit.ref.key}:${hit.message.id}:${hit.message.ts}" },
                        contentType = { "message-hit" },
                    ) { hit ->
                        MessageSearchHitRow(
                            hit = hit,
                            query = query.trim(),
                            onClick = {
                                closeSearch()
                                actions.onOpenSearchHit(hit)
                            },
                        )
                    }
                }
            } else {
                if (connectionNote != ConnectionNote.None) {
                    item(key = "connecting", contentType = "connecting") { ConnectionNoteRow(connectionNote) }
                }
                if (status?.state == "disconnected" || connectionError != null) {
                    item(key = "banner", contentType = "banner") {
                        StatusBanner(
                            text = connectionError ?: status?.error ?: "Disconnected from Zalo",
                            action = if (status?.state == "disconnected") "Reconnect" else null,
                            onAction = actions.onReconnect,
                        )
                    }
                }
                if (!searching && chats.isNotEmpty()) {
                    item(key = "filter-bar", contentType = "filter-bar") {
                        ChatFilterBar(
                            selected = filter,
                            unreadCount = unreadChatCount,
                            onSelect = { filter = it },
                        )
                    }
                }
                if (!searching && officialCount > 0 && activeFilter == ChatFilter.OFFICIAL) {
                    item(key = "official-back", contentType = "official-row") {
                        OfficialAccountsRow(
                            text = "Official accounts ($officialCount)",
                            hint = "Back to all chats",
                            onClick = { filter = ChatFilter.ALL },
                        )
                    }
                }
                if (chats.isEmpty()) {
                    item(key = "empty", contentType = "empty") { EmptyChats() }
                } else if (visibleChats.isEmpty()) {
                    item(key = "empty-filtered", contentType = "empty") {
                        val (title, subtitle) = when (activeFilter) {
                            ChatFilter.OFFICIAL -> "No official accounts" to "News, shops and notification senders appear here."
                            ChatFilter.UNREAD -> "All caught up" to "No unread conversations right now."
                            ChatFilter.GROUPS -> "No group chats" to "Group conversations you join or create appear here."
                            ChatFilter.ALL -> "No chats yet" to "New messages appear here. Tap the pencil to write to a friend or group."
                        }
                        EmptyChats(
                            title = title,
                            subtitle = subtitle,
                            actionLabel = if (activeFilter != ChatFilter.ALL) "View all chats" else null,
                            onAction = if (activeFilter != ChatFilter.ALL) ({ filter = ChatFilter.ALL }) else null,
                        )
                    }
                    if (officialCount > 0 && activeFilter == ChatFilter.ALL) {
                        item(key = "official-row", contentType = "official-row") {
                            OfficialAccountsRow(
                                text = "Official accounts ($officialCount)",
                                hint = "Hidden from your chat list",
                                onClick = { filter = ChatFilter.OFFICIAL },
                            )
                        }
                    }
                } else {
                    val pinned = visibleChats.filter { it.pinned }
                    val others = visibleChats.filterNot { it.pinned }
                    if (pinned.isNotEmpty()) {
                        item(key = "h-pinned", contentType = "header") { SectionHeader("Pinned") }
                        items(pinned, key = { it.key }, contentType = { "thread" }) { thread ->
                            ConversationRow(
                                thread = thread,
                                selected = thread.key == selectedKey,
                                typing = typing[thread.key],
                                draft = drafts[thread.ref.key],
                                swipeEnabled = true,
                                onClick = { actions.onOpen(thread) },
                                onLongClick = { sheetFor = thread },
                                onAction = { onRowAction(thread, it) },
                            )
                        }
                        if (others.isNotEmpty()) item(key = "h-chats", contentType = "header") { SectionHeader("Chats") }
                    }
                    items(others, key = { it.key }, contentType = { "thread" }) { thread ->
                        ConversationRow(
                            thread = thread,
                            selected = thread.key == selectedKey,
                            typing = typing[thread.key],
                            draft = drafts[thread.ref.key],
                            swipeEnabled = true,
                            onClick = { actions.onOpen(thread) },
                            onLongClick = { sheetFor = thread },
                            onAction = { onRowAction(thread, it) },
                        )
                    }
                    if (officialCount > 0 && activeFilter == ChatFilter.ALL) {
                        item(key = "official-row", contentType = "official-row") {
                            OfficialAccountsRow(
                                text = "Official accounts ($officialCount)",
                                hint = "Hidden from your chat list",
                                onClick = { filter = ChatFilter.OFFICIAL },
                            )
                        }
                    }
                }
            }
            }
            if (ultra && !searching) {
                FourXBadge(
                    size = 86.dp,
                    modifier = Modifier
                        .align(Alignment.TopEnd)
                        .padding(top = padding.calculateTopPadding() + 4.dp, end = 12.dp),
                )
            }
        }
    }

    sheetFor?.let { thread ->
        ThreadActionsSheet(
            thread = thread,
            onDismiss = { sheetFor = null },
            onAction = { onRowAction(thread, it) },
        )
    }
    muteFor?.let { thread ->
        MuteDialog(
            onDismiss = { muteFor = null },
            onPick = {
                actions.onMute(thread, it)
                muteFor = null
            },
        )
    }
    deleteFor?.let { thread ->
        DeleteConversationDialog(
            onDismiss = { deleteFor = null },
            onConfirm = {
                actions.onDelete(thread)
                deleteFor = null
            },
        )
    }
    if (showJoinGroupDialog) {
        JoinGroupLinkDialog(
            onDismiss = { showJoinGroupDialog = false },
            onJoin = { link ->
                showJoinGroupDialog = false
                actions.onJoinGroupLink(link)
            },
        )
    }
}

private const val MAX_RESULTS = 100

/** Compact row that stands in for hidden Official Accounts and opens (or leaves) their own list. */
@Composable
private fun OfficialAccountsRow(text: String, hint: String, onClick: () -> Unit) {
    val colors = MaterialTheme.colorScheme
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 20.dp, vertical = 14.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(Icons.Outlined.Campaign, contentDescription = null, tint = colors.onSurfaceVariant)
        Spacer(Modifier.width(16.dp))
        Column(Modifier.weight(1f)) {
            Text(text, style = MaterialTheme.typography.bodyLarge, maxLines = 1, overflow = TextOverflow.Ellipsis)
            Text(
                hint,
                style = MaterialTheme.typography.bodySmall,
                color = colors.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun ChatFilterBar(
    selected: ChatFilter,
    unreadCount: Int,
    onSelect: (ChatFilter) -> Unit,
    modifier: Modifier = Modifier,
) {
    val colors = MaterialTheme.colorScheme
    val filters = listOf(
        ChatFilter.ALL to "All",
        ChatFilter.UNREAD to if (unreadCount > 0) "Unread · $unreadCount" else "Unread",
        ChatFilter.GROUPS to "Groups",
    )
    Row(
        modifier = modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween),
    ) {
        filters.forEachIndexed { index, (filter, label) ->
            val shapes = when (index) {
                0 -> ButtonGroupDefaults.connectedLeadingButtonShapes()
                filters.lastIndex -> ButtonGroupDefaults.connectedTrailingButtonShapes()
                else -> ButtonGroupDefaults.connectedMiddleButtonShapes()
            }
            val isSelected = selected == filter
            val ultra = LocalUltraExpressive.current
            ToggleButton(
                checked = isSelected,
                onCheckedChange = { onSelect(filter) },
                shapes = shapes,
                colors = ToggleButtonDefaults.tonalToggleButtonColors(
                    containerColor = colors.surfaceContainerHigh,
                    contentColor = colors.onSurfaceVariant,
                    checkedContainerColor = colors.primaryContainer,
                    checkedContentColor = colors.onPrimaryContainer,
                ),
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = if (ultra) 18.dp else 6.dp),
                modifier = Modifier
                    .weight(1f)
                    .then(if (ultra) Modifier.height(76.dp) else Modifier)
                    .semantics { role = Role.RadioButton },
            ) {
                Text(
                    text = label,
                    style = if (ultra) MaterialTheme.typography.titleMedium else MaterialTheme.typography.labelLarge,
                    fontWeight = if (isSelected || ultra) FontWeight.Bold else FontWeight.Medium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SearchTopBar(query: String, onQuery: (String) -> Unit, onClose: () -> Unit) {
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
    val colors = MaterialTheme.colorScheme
    TopAppBar(
        navigationIcon = {
            IconButton(onClick = onClose) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Close search")
            }
        },
        title = {
            Surface(
                shape = CircleShape,
                color = colors.surfaceContainerHigh,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(end = 12.dp)
                    .height(44.dp),
            ) {
                Row(
                    modifier = Modifier
                        .fillMaxSize()
                        .padding(start = 16.dp, end = 4.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Box(Modifier.weight(1f), contentAlignment = Alignment.CenterStart) {
                        if (query.isEmpty()) {
                            Text(
                                text = "Search",
                                style = MaterialTheme.typography.bodyLarge,
                                color = colors.onSurfaceVariant,
                                maxLines = 1,
                            )
                        }
                        BasicTextField(
                            value = query,
                            onValueChange = onQuery,
                            singleLine = true,
                            textStyle = MaterialTheme.typography.bodyLarge.copy(color = colors.onSurface),
                            cursorBrush = SolidColor(colors.primary),
                            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                            modifier = Modifier
                                .fillMaxWidth()
                                .focusRequester(focus),
                        )
                    }
                    if (query.isNotEmpty()) {
                        IconButton(onClick = { onQuery("") }) {
                            Icon(
                                Icons.Outlined.Close,
                                contentDescription = "Clear",
                                tint = colors.onSurfaceVariant,
                            )
                        }
                    }
                }
            }
        },
    )
}

@Composable
private fun OverflowMenu(
    hasUnread: Boolean,
    onMarkAllRead: () -> Unit,
    onJoinGroupLink: () -> Unit,
    onSettings: () -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { Icon(Icons.Outlined.MoreVert, contentDescription = "More options") }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            DropdownMenuItem(
                text = { Text("Mark all read") },
                enabled = hasUnread,
                onClick = {
                    open = false
                    onMarkAllRead()
                },
            )
            DropdownMenuItem(
                text = { Text("Join group by link") },
                onClick = {
                    open = false
                    onJoinGroupLink()
                },
            )
            DropdownMenuItem(
                text = { Text("Settings") },
                onClick = {
                    open = false
                    onSettings()
                },
            )
        }
    }
}

@OptIn(ExperimentalFoundationApi::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun ConversationRow(
    thread: ThreadSummary,
    /** The chat open beside the list (tablets). */
    selected: Boolean = false,
    typing: TypingInfo?,
    draft: String? = null,
    swipeEnabled: Boolean = true,
    onClick: () -> Unit,
    onLongClick: () -> Unit,
    onAction: (RowAction) -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val unread = thread.unread > 0
    val last = thread.lastMessage
    val rowBackground = if (selected) colors.surfaceContainerHighest else colors.background

    val scope = rememberCoroutineScope()
    val haptic = LocalHapticFeedback.current
    val density = LocalDensity.current
    val thresholdPx = with(density) { 72.dp.toPx() }
    val maxSwipePx = with(density) { 104.dp.toPx() }
    val resetSpec = MaterialTheme.motionScheme.defaultSpatialSpec<Float>()

    var offsetX by remember(thread.key) { mutableFloatStateOf(0f) }
    var resetJob by remember(thread.key) { mutableStateOf<Job?>(null) }
    val latestThread by rememberUpdatedState(thread)
    val latestOnAction by rememberUpdatedState(onAction)
    val latestResetSpec by rememberUpdatedState(resetSpec)

    fun settleBack() {
        resetJob?.cancel()
        val start = offsetX
        if (start == 0f) return
        resetJob = scope.launch {
            animate(
                initialValue = start,
                targetValue = 0f,
                animationSpec = latestResetSpec,
            ) { value, _ ->
                offsetX = value
            }
        }
    }

    Box(Modifier.fillMaxWidth()) {
        if (offsetX != 0f) {
            val swipingRight = offsetX > 0f
            val progress = (abs(offsetX) / thresholdPx).coerceIn(0f, 1f)
            val pillColor = if (swipingRight) colors.primary else colors.secondaryContainer
            val pillContentColor = if (swipingRight) colors.onPrimary else colors.onSecondaryContainer
            val pillIcon = when {
                swipingRight && unread -> Icons.Outlined.Drafts
                swipingRight -> Icons.Outlined.MarkChatUnread
                thread.pinned -> Icons.Filled.PushPin
                else -> Icons.Outlined.PushPin
            }
            val pillLabel = when {
                swipingRight && unread -> "Read"
                swipingRight -> "Unread"
                thread.pinned -> "Unpin"
                else -> "Pin"
            }
            Box(
                modifier = Modifier
                    .matchParentSize()
                    .padding(horizontal = 12.dp, vertical = 4.dp)
                    .clip(RoundedCornerShape(20.dp))
                    .background(pillColor)
                    .padding(horizontal = 20.dp),
                contentAlignment = if (swipingRight) Alignment.CenterStart else Alignment.CenterEnd,
            ) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    modifier = Modifier.graphicsLayer {
                        val scale = 0.85f + 0.15f * progress
                        scaleX = scale
                        scaleY = scale
                        alpha = progress.coerceAtLeast(0.35f)
                    },
                ) {
                    if (swipingRight) {
                        Icon(pillIcon, contentDescription = null, tint = pillContentColor, modifier = Modifier.size(22.dp))
                        Text(
                            pillLabel,
                            style = MaterialTheme.typography.labelLarge,
                            fontWeight = FontWeight.SemiBold,
                            color = pillContentColor,
                        )
                    } else {
                        Text(
                            pillLabel,
                            style = MaterialTheme.typography.labelLarge,
                            fontWeight = FontWeight.SemiBold,
                            color = pillContentColor,
                        )
                        Icon(pillIcon, contentDescription = null, tint = pillContentColor, modifier = Modifier.size(22.dp))
                    }
                }
            }
        }

        Box(
            modifier = Modifier
                .fillMaxWidth()
                .offset { IntOffset(offsetX.roundToInt(), 0) }
                .background(colors.background)
                .pointerInput(swipeEnabled, thread.key) {
                    if (!swipeEnabled) return@pointerInput
                    var crossed = false
                    detectHorizontalDragGestures(
                        onDragStart = {
                            resetJob?.cancel()
                            crossed = abs(offsetX) >= thresholdPx
                        },
                        onHorizontalDrag = { change, dragAmount ->
                            change.consume()
                            val damped = if (abs(offsetX) > thresholdPx && offsetX * dragAmount > 0f) {
                                dragAmount * 0.35f
                            } else {
                                dragAmount
                            }
                            offsetX = (offsetX + damped).coerceIn(-maxSwipePx, maxSwipePx)
                            val nowCrossed = abs(offsetX) >= thresholdPx
                            if (nowCrossed && !crossed) {
                                crossed = true
                                haptic.performHapticFeedback(HapticFeedbackType.LongPress)
                            } else if (!nowCrossed && crossed) {
                                crossed = false
                            }
                        },
                        onDragEnd = {
                            val current = latestThread
                            when {
                                offsetX >= thresholdPx -> latestOnAction(if (current.unread > 0) RowAction.READ else RowAction.UNREAD)
                                offsetX <= -thresholdPx -> latestOnAction(if (current.pinned) RowAction.UNPIN else RowAction.PIN)
                            }
                            settleBack()
                        },
                        onDragCancel = { settleBack() },
                    )
                }
                .then(if (selected) Modifier.padding(horizontal = 8.dp, vertical = 2.dp) else Modifier),
        ) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .defaultMinSize(minHeight = 72.dp)
                    .then(if (selected) Modifier.clip(RoundedCornerShape(16.dp)) else Modifier)
                    .background(rowBackground)
                    .combinedClickable(onClick = onClick, onLongClick = onLongClick)
                    .padding(horizontal = if (selected) 8.dp else 16.dp, vertical = if (selected) 8.dp else 10.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Avatar(thread.name, thread.avatar, key = thread.key, isGroup = thread.kind == "group", size = 52.dp)
                Spacer(Modifier.width(14.dp))
                Column(Modifier.weight(1f)) {
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Row(Modifier.weight(1f), verticalAlignment = Alignment.CenterVertically) {
                            Text(
                                thread.name,
                                style = MaterialTheme.typography.titleMedium,
                                fontWeight = if (unread) FontWeight.Bold else FontWeight.Medium,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.weight(1f, fill = false),
                            )
                            if (thread.isMuted) {
                                Spacer(Modifier.width(4.dp))
                                Icon(
                                    Icons.Outlined.NotificationsOff,
                                    contentDescription = "Muted",
                                    modifier = Modifier.size(16.dp),
                                    tint = colors.onSurfaceVariant,
                                )
                            }
                        }
                        if (last != null || thread.pinned) {
                            Spacer(Modifier.width(8.dp))
                            if (last != null) {
                                if (typing == null && last.isSelf) {
                                    DeliveryIcon(deliveryState(last), tint = colors.onSurfaceVariant, background = rowBackground)
                                    Spacer(Modifier.width(4.dp))
                                }
                                Text(
                                    formatListTime(last.ts),
                                    style = MaterialTheme.typography.labelMedium,
                                    fontWeight = if (unread) FontWeight.SemiBold else FontWeight.Normal,
                                    color = if (unread) colors.primary else colors.onSurfaceVariant,
                                )
                            }
                            if (thread.pinned) {
                                if (last != null) Spacer(Modifier.width(4.dp))
                                Icon(
                                    Icons.Outlined.PushPin,
                                    contentDescription = "Pinned",
                                    modifier = Modifier.size(14.dp),
                                    tint = colors.onSurfaceVariant.copy(alpha = 0.7f),
                                )
                            }
                        }
                    }
                    Spacer(Modifier.height(2.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        val mentioned = last?.mentionsMe == true && unread
                        val draftSnippet = formatDraftSnippet(draft)
                        val showDraft = typing == null && draftSnippet != null
                        val thumb = if (typing == null && !showDraft && last?.recalled != true) {
                            last?.attachment?.thumb?.takeIf { it.isNotBlank() }
                        } else {
                            null
                        }
                        if (thumb != null) {
                            val thumbPlaceholder = remember(colors.surfaceContainerHigh) { ColorPainter(colors.surfaceContainerHigh) }
                            AsyncImage(
                                model = thumb,
                                contentDescription = null,
                                contentScale = ContentScale.Crop,
                                modifier = Modifier
                                    .size(20.dp)
                                    .clip(RoundedCornerShape(4.dp)),
                                placeholder = thumbPlaceholder,
                                error = thumbPlaceholder,
                            )
                            Spacer(Modifier.width(6.dp))
                        }
                        if (showDraft) {
                            val draftAnnotated = remember(draftSnippet, colors.primary, colors.onSurfaceVariant) {
                                buildAnnotatedString {
                                    withStyle(SpanStyle(color = colors.primary, fontWeight = FontWeight.SemiBold)) {
                                        append("Draft: ")
                                    }
                                    withStyle(SpanStyle(color = colors.onSurfaceVariant, fontWeight = FontWeight.Normal)) {
                                        append(draftSnippet)
                                    }
                                }
                            }
                            Text(
                                text = draftAnnotated,
                                style = MaterialTheme.typography.bodyMedium,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.weight(1f),
                            )
                        } else {
                            Text(
                                text = when {
                                    typing != null -> if (thread.kind == "group" && typing.name.isNotBlank()) "${typing.name} is typing…" else "typing…"
                                    else -> preview(last)
                                },
                                style = MaterialTheme.typography.bodyMedium,
                                fontWeight = if (unread && typing == null) FontWeight.SemiBold else FontWeight.Normal,
                                color = when {
                                    typing != null -> colors.primary
                                    unread -> colors.onSurface
                                    else -> colors.onSurfaceVariant
                                },
                                fontStyle = if (typing == null && last?.recalled == true) FontStyle.Italic else FontStyle.Normal,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.weight(1f),
                            )
                        }
                        if (mentioned) {
                            Spacer(Modifier.width(8.dp))
                            CountBadge("@", colors.primary, colors.onPrimary)
                        }
                        if (unread) {
                            Spacer(Modifier.width(6.dp))
                            CountBadge(
                                text = if (thread.unread > 99) "99+" else thread.unread.toString(),
                                color = if (thread.isMuted) colors.outline else colors.primary,
                                contentColor = if (thread.isMuted) colors.surface else colors.onPrimary,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun MessagesSectionHeader(busy: Boolean) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(start = 16.dp, end = 16.dp, top = 16.dp, bottom = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Text(
            text = "Messages",
            style = MaterialTheme.typography.titleSmall,
            color = MaterialTheme.colorScheme.onSurface,
            fontWeight = FontWeight.SemiBold,
        )
        if (busy) {
            CircularProgressIndicator(
                modifier = Modifier.size(14.dp),
                strokeWidth = 2.dp,
            )
        }
    }
}

private fun windowSnippet(text: String, ranges: List<IntRange>): String {
    val first = ranges.firstOrNull() ?: return text
    if (first.first <= 24) return text
    var cut = (first.first - 16).coerceAtLeast(0)
    val space = text.indexOf(' ', startIndex = cut)
    if (space in cut..(cut + 8) && space < first.first) {
        cut = space + 1
    }
    return "…" + text.substring(cut)
}

@Composable
private fun MessageSearchHitRow(
    hit: MessageSearchHit,
    query: String,
    onClick: () -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val displayName = hit.threadName.ifBlank { hit.message.fromName.ifBlank { hit.threadId } }
    val rawSnippet = remember(hit.message.text, hit.message.attachment?.title) {
        hit.message.text
            .ifBlank { hit.message.attachment?.title.orEmpty() }
            .replace(WHITESPACE_REGEX, " ")
            .trim()
    }
    val annotatedSnippet = remember(rawSnippet, query, colors.primary) {
        val initialRanges = findHighlightRanges(rawSnippet, query)
        val windowed = windowSnippet(rawSnippet, initialRanges)
        val ranges = if (windowed === rawSnippet) initialRanges else findHighlightRanges(windowed, query)
        buildAnnotatedString {
            var cursor = 0
            for (range in ranges) {
                if (range.first > cursor) {
                    append(windowed.substring(cursor, range.first))
                }
                withStyle(SpanStyle(fontWeight = FontWeight.SemiBold, color = colors.primary)) {
                    append(windowed.substring(range.first, range.last + 1))
                }
                cursor = range.last + 1
            }
            if (cursor < windowed.length) {
                append(windowed.substring(cursor))
            }
        }
    }
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .defaultMinSize(minHeight = 72.dp)
            .clickable(onClick = onClick)
            .padding(horizontal = 16.dp, vertical = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Avatar(
            name = displayName,
            url = hit.threadAvatar,
            key = hit.ref.key,
            isGroup = hit.threadKind == "group",
            size = 52.dp,
        )
        Spacer(Modifier.width(14.dp))
        Column(Modifier.weight(1f)) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Text(
                    text = displayName,
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.Medium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                Spacer(Modifier.width(8.dp))
                Text(
                    text = formatListTime(hit.message.ts),
                    style = MaterialTheme.typography.labelMedium,
                    color = colors.onSurfaceVariant,
                )
            }
            Spacer(Modifier.height(2.dp))
            Text(
                text = annotatedSnippet,
                style = MaterialTheme.typography.bodyMedium,
                color = colors.onSurfaceVariant,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ThreadActionsSheet(
    thread: ThreadSummary,
    onDismiss: () -> Unit,
    onAction: (RowAction) -> Unit,
) {
    val colors = MaterialTheme.colorScheme
    val unread = thread.unread > 0
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        containerColor = colors.surfaceContainerLow,
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(bottom = 20.dp),
        ) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 20.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Avatar(thread.name, thread.avatar, key = thread.key, isGroup = thread.kind == "group", size = 44.dp)
                Spacer(Modifier.width(14.dp))
                Column(Modifier.weight(1f)) {
                    Text(
                        text = thread.name,
                        style = MaterialTheme.typography.titleMedium,
                        fontWeight = FontWeight.SemiBold,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    val subtitle = buildList {
                        if (thread.kind == "group") add("Group")
                        if (thread.pinned) add("Pinned")
                        if (thread.isMuted) add("Muted")
                    }.joinToString(" · ")
                    if (subtitle.isNotEmpty()) {
                        Text(
                            text = subtitle,
                            style = MaterialTheme.typography.bodySmall,
                            color = colors.onSurfaceVariant,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }
            HorizontalDivider(
                modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp),
                color = colors.outlineVariant.copy(alpha = 0.6f),
            )
            SheetActionRow(
                label = if (thread.pinned) "Unpin" else "Pin",
                icon = if (thread.pinned) Icons.Filled.PushPin else Icons.Outlined.PushPin,
                onClick = { onAction(if (thread.pinned) RowAction.UNPIN else RowAction.PIN) },
            )
            if (thread.isMuted) {
                SheetActionRow(
                    label = "Unmute",
                    icon = Icons.Outlined.Notifications,
                    onClick = { onAction(RowAction.UNMUTE) },
                )
            } else {
                SheetActionRow(
                    label = "Mute…",
                    icon = Icons.Outlined.NotificationsOff,
                    onClick = { onAction(RowAction.MUTE) },
                )
            }
            if (unread) {
                SheetActionRow(
                    label = "Mark as read",
                    icon = Icons.Outlined.Drafts,
                    onClick = { onAction(RowAction.READ) },
                )
            } else {
                SheetActionRow(
                    label = "Mark as unread",
                    icon = Icons.Outlined.MarkChatUnread,
                    onClick = { onAction(RowAction.UNREAD) },
                )
            }
            if (thread.isOfficialAccount) {
                SheetActionRow(
                    label = "Hide official accounts",
                    icon = Icons.Outlined.VisibilityOff,
                    onClick = { onAction(RowAction.HIDE_OFFICIAL) },
                )
            }
            HorizontalDivider(
                modifier = Modifier.padding(horizontal = 20.dp, vertical = 8.dp),
                color = colors.outlineVariant.copy(alpha = 0.6f),
            )
            SheetActionRow(
                label = "Delete",
                icon = Icons.Outlined.Delete,
                destructive = true,
                onClick = { onAction(RowAction.DELETE) },
            )
        }
    }
}

@Composable
private fun SheetActionRow(
    label: String,
    icon: ImageVector,
    destructive: Boolean = false,
    onClick: () -> Unit,
) {
    val tint = if (destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 12.dp, vertical = 2.dp)
            .clip(RoundedCornerShape(16.dp))
            .clickable(onClick = onClick)
            .defaultMinSize(minHeight = 52.dp)
            .padding(horizontal = 12.dp, vertical = 12.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(24.dp))
        Spacer(Modifier.width(16.dp))
        Text(
            text = label,
            style = MaterialTheme.typography.bodyLarge,
            fontWeight = FontWeight.Medium,
            color = tint,
        )
    }
}

@Composable
fun MenuEntry(
    label: String,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    destructive: Boolean = false,
    onClick: () -> Unit,
) {
    val color = if (destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface
    DropdownMenuItem(
        text = { Text(label, color = color) },
        leadingIcon = { Icon(icon, null, tint = color) },
        onClick = onClick,
    )
}

/** A slim, static line (no animation: it can stay up while offline, and spinning costs battery) under the top bar. */
@Composable
private fun ConnectionNoteRow(note: ConnectionNote) {
    val waiting = note == ConnectionNote.WaitingForConnection
    Row(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Icon(
            imageVector = if (waiting) Icons.Outlined.WifiOff else Icons.Outlined.Sync,
            contentDescription = null,
            modifier = Modifier.size(14.dp),
            tint = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Text(
            text = if (waiting) "Waiting for a connection…" else "Connecting…",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
fun StatusBanner(text: String, action: String?, onAction: () -> Unit) {
    Card(
        colors = CardDefaults.cardColors(
            containerColor = MaterialTheme.colorScheme.errorContainer,
            contentColor = MaterialTheme.colorScheme.onErrorContainer,
        ),
        modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
    ) {
        Row(Modifier.padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 8.dp), verticalAlignment = Alignment.CenterVertically) {
            Icon(Icons.Outlined.WifiOff, null, Modifier.size(20.dp))
            Spacer(Modifier.size(12.dp))
            Text(text, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.weight(1f))
            if (action != null) TextButton(onClick = onAction) { Text(action) }
        }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun EmptyChats(
    title: String = "No chats yet",
    subtitle: String = "New messages appear here. Tap the pencil to write to a friend or group.",
    actionLabel: String? = null,
    onAction: (() -> Unit)? = null,
) {
    Box(Modifier.fillMaxWidth().padding(top = 88.dp), contentAlignment = Alignment.Center) {
        Column(horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Box(
                modifier = Modifier
                    .size(72.dp)
                    .clip(MaterialShapes.SoftBurst.toShape())
                    .background(MaterialTheme.colorScheme.primaryContainer),
                contentAlignment = Alignment.Center,
            ) {
                Icon(
                    Icons.AutoMirrored.Outlined.Chat,
                    contentDescription = null,
                    modifier = Modifier.size(32.dp),
                    tint = MaterialTheme.colorScheme.onPrimaryContainer,
                )
            }
            Spacer(Modifier.height(4.dp))
            Text(title, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            Text(
                subtitle,
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                textAlign = TextAlign.Center,
                modifier = Modifier.padding(horizontal = 32.dp),
            )
            if (actionLabel != null && onAction != null) {
                TextButton(onClick = onAction) {
                    Text(actionLabel)
                }
            }
        }
    }
}
