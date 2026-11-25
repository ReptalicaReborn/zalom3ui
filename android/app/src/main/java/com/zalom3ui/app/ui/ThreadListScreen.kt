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
import androidx.compose.material.icons.outlined.WifiOff
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FloatingActionButton
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
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.IntSize
import androidx.compose.ui.unit.dp
import coil.compose.SubcomposeAsyncImage
import com.zalom3ui.app.R
import com.zalom3ui.app.data.MuteOption
import com.zalom3ui.app.data.ThreadSummary
import kotlin.math.abs
import kotlin.math.roundToInt
import kotlinx.coroutines.Job
import kotlinx.coroutines.launch

/** Everything the chat list can ask for. */
class ThreadListActions(
    val onOpen: (ThreadSummary) -> Unit,
    val onNewChat: () -> Unit,
    val onReconnect: () -> Unit,
    val onSettings: () -> Unit,
    val onMarkAllRead: () -> Unit,
    val onPin: (ThreadSummary, Boolean) -> Unit,
    val onMute: (ThreadSummary, MuteOption?) -> Unit,
    val onMarkRead: (ThreadSummary) -> Unit,
    val onMarkUnread: (ThreadSummary) -> Unit,
    val onDelete: (ThreadSummary) -> Unit,
)

/** Filter modes shown in the chat list's connected pill bar. */
enum class ChatFilter { ALL, UNREAD, GROUPS }

/** Pure helper that filters conversation summaries according to [filter], preserving order. */
internal fun filterChats(chats: List<ThreadSummary>, filter: ChatFilter): List<ThreadSummary> = when (filter) {
    ChatFilter.ALL -> chats
    ChatFilter.UNREAD -> chats.filter { it.unread > 0 }
    ChatFilter.GROUPS -> chats.filter { it.kind == "group" }
}

private enum class RowAction { PIN, UNPIN, MUTE, UNMUTE, READ, UNREAD, DELETE }

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun ThreadListScreen(state: UiState, actions: ThreadListActions, selectedKey: String? = null) {
    var searching by rememberSaveable { mutableStateOf(false) }
    var query by rememberSaveable { mutableStateOf("") }
    var filter by rememberSaveable { mutableStateOf(ChatFilter.ALL) }
    var sheetFor by remember { mutableStateOf<ThreadSummary?>(null) }
    var muteFor by remember { mutableStateOf<ThreadSummary?>(null) }
    var deleteFor by remember { mutableStateOf<ThreadSummary?>(null) }
    val closeSearch: () -> Unit = {
        searching = false
        query = ""
    }
    BackHandler(enabled = searching) { closeSearch() }

    val scrollBehavior = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    // The chat list shows conversations; contacts you never talked to are under search and "New chat".
    val chats = remember(state.threads) { state.threads.filter { it.lastMessage != null || it.pinned || it.unread > 0 } }
    val unreadChatCount = remember(chats) { chats.count { it.unread > 0 } }
    val visibleChats = remember(chats, filter, searching) {
        if (searching) chats else filterChats(chats, filter)
    }
    val folded = foldForSearch(query.trim())
    val matches = remember(state.threads, folded) {
        if (folded.isEmpty()) emptyList() else state.threads.filter { foldForSearch(it.name).contains(folded) }
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
        }
    }

    val topBarEffectsSpec = MaterialTheme.motionScheme.fastEffectsSpec<Float>()
    val topBarSlideSpec = MaterialTheme.motionScheme.fastSpatialSpec<IntOffset>()
    val topBarSizeSpec = MaterialTheme.motionScheme.fastSpatialSpec<IntSize>()

    Scaffold(
        modifier = if (searching) Modifier else Modifier.nestedScroll(scrollBehavior.nestedScrollConnection),
        topBar = {
            AnimatedContent(
                targetState = searching,
                transitionSpec = {
                    (fadeIn(topBarEffectsSpec) + slideInVertically(topBarSlideSpec) { -it / 4 })
                        .togetherWith(fadeOut(topBarEffectsSpec))
                        .using(SizeTransform(clip = false) { _, _ -> topBarSizeSpec })
                },
                label = "TopBarSearchTransition",
            ) { isSearching ->
                if (isSearching) {
                    SearchTopBar(query = query, onQuery = { query = it }, onClose = closeSearch)
                } else {
                    MediumFlexibleTopAppBar(
                        navigationIcon = {
                            IconButton(onClick = actions.onSettings) {
                                Avatar(
                                    name = state.status?.ownName.orEmpty(),
                                    url = "",
                                    key = state.status?.ownId ?: "me",
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
                            IconButton(onClick = { searching = true }) {
                                Icon(Icons.Outlined.Search, contentDescription = "Search")
                            }
                            OverflowMenu(
                                hasUnread = unreadChatCount > 0,
                                onMarkAllRead = actions.onMarkAllRead,
                                onSettings = actions.onSettings,
                            )
                        },
                        scrollBehavior = scrollBehavior,
                    )
                }
            }
        },
        floatingActionButton = {
            if (!searching) {
                FloatingActionButton(
                    onClick = actions.onNewChat,
                    shape = RoundedCornerShape(20.dp),
                    containerColor = MaterialTheme.colorScheme.primaryContainer,
                    contentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                    modifier = Modifier.size(56.dp),
                ) { Icon(Icons.Outlined.Edit, contentDescription = "New chat") }
            }
        },
    ) { padding ->
        LazyColumn(
            contentPadding = PaddingValues(top = padding.calculateTopPadding(), bottom = padding.calculateBottomPadding() + 96.dp),
            modifier = Modifier.fillMaxSize(),
        ) {
            if (searching && folded.isNotEmpty()) {
                val chatHits = matches.filter { it.lastMessage != null }
                val contactHits = matches.filter { it.lastMessage == null && it.kind == "user" }.sortedBy { foldForSearch(it.name) }
                val groupHits = matches.filter { it.lastMessage == null && it.kind == "group" }.sortedBy { foldForSearch(it.name) }
                if (matches.isEmpty()) {
                    item(key = "no-results") {
                        Text(
                            "No results for “${query.trim()}”",
                            style = MaterialTheme.typography.bodyLarge,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(24.dp),
                        )
                    }
                }
                if (chatHits.isNotEmpty()) {
                    item(key = "h-chat-hits") { SectionHeader("Chats") }
                    items(chatHits, key = { it.key }) { thread ->
                        ConversationRow(
                            thread = thread,
                            selected = thread.key == selectedKey,
                            typing = state.typing[thread.key],
                            swipeEnabled = false,
                            onClick = { actions.onOpen(thread) },
                            onLongClick = {},
                            onAction = {},
                        )
                    }
                }
                if (contactHits.isNotEmpty()) {
                    item(key = "h-contact-hits") { SectionHeader("Contacts") }
                    items(contactHits.take(MAX_RESULTS), key = { it.key }) { ContactRow(it, onClick = { actions.onOpen(it) }) }
                }
                if (groupHits.isNotEmpty()) {
                    item(key = "h-group-hits") { SectionHeader("Groups") }
                    items(groupHits.take(MAX_RESULTS), key = { it.key }) { ContactRow(it, onClick = { actions.onOpen(it) }) }
                }
            } else {
                val status = state.status
                if (status?.state == "disconnected" || state.connectionError != null) {
                    item(key = "banner") {
                        StatusBanner(
                            text = state.connectionError ?: status?.error ?: "Disconnected from Zalo",
                            action = if (status?.state == "disconnected") "Reconnect" else null,
                            onAction = actions.onReconnect,
                        )
                    }
                }
                if (!searching && chats.isNotEmpty()) {
                    item(key = "filter-bar") {
                        ChatFilterBar(
                            selected = filter,
                            unreadCount = unreadChatCount,
                            onSelect = { filter = it },
                        )
                    }
                }
                if (chats.isEmpty()) {
                    item(key = "empty") { EmptyChats() }
                } else if (visibleChats.isEmpty()) {
                    item(key = "empty-filtered") {
                        val (title, subtitle) = when (filter) {
                            ChatFilter.UNREAD -> "All caught up" to "No unread conversations right now."
                            ChatFilter.GROUPS -> "No group chats" to "Group conversations you join or create appear here."
                            ChatFilter.ALL -> "No chats yet" to "New messages appear here. Tap the pencil to write to a friend or group."
                        }
                        EmptyChats(title = title, subtitle = subtitle)
                    }
                } else {
                    val pinned = visibleChats.filter { it.pinned }
                    val others = visibleChats.filterNot { it.pinned }
                    if (pinned.isNotEmpty()) {
                        item(key = "h-pinned") { SectionHeader("Pinned") }
                        items(pinned, key = { it.key }) { thread ->
                            ConversationRow(
                                thread = thread,
                                selected = thread.key == selectedKey,
                                typing = state.typing[thread.key],
                                swipeEnabled = true,
                                onClick = { actions.onOpen(thread) },
                                onLongClick = { sheetFor = thread },
                                onAction = { onRowAction(thread, it) },
                            )
                        }
                        if (others.isNotEmpty()) item(key = "h-chats") { SectionHeader("Chats") }
                    }
                    items(others, key = { it.key }) { thread ->
                        ConversationRow(
                            thread = thread,
                            selected = thread.key == selectedKey,
                            typing = state.typing[thread.key],
                            swipeEnabled = true,
                            onClick = { actions.onOpen(thread) },
                            onLongClick = { sheetFor = thread },
                            onAction = { onRowAction(thread, it) },
                        )
                    }
                }
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
}

private const val MAX_RESULTS = 100

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
                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp),
                modifier = Modifier
                    .weight(1f)
                    .semantics { role = Role.RadioButton },
            ) {
                Text(
                    text = label,
                    style = MaterialTheme.typography.labelLarge,
                    fontWeight = if (isSelected) FontWeight.SemiBold else FontWeight.Medium,
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
private fun OverflowMenu(hasUnread: Boolean, onMarkAllRead: () -> Unit, onSettings: () -> Unit) {
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
                        val thumb = if (typing == null && last?.recalled != true) {
                            last?.attachment?.thumb?.takeIf { it.isNotBlank() }
                        } else {
                            null
                        }
                        if (thumb != null) {
                            SubcomposeAsyncImage(
                                model = thumb,
                                contentDescription = null,
                                contentScale = ContentScale.Crop,
                                modifier = Modifier
                                    .size(20.dp)
                                    .clip(RoundedCornerShape(4.dp)),
                                loading = { Box(Modifier.fillMaxSize().background(colors.surfaceContainerHigh)) },
                                error = { Box(Modifier.fillMaxSize().background(colors.surfaceContainerHigh)) },
                            )
                            Spacer(Modifier.width(6.dp))
                        }
                        Text(
                            text = when {
                                typing != null -> if (thread.kind == "group" && typing.name.isNotBlank()) "${typing.name} is typing…" else "typing…"
                                else -> preview(last)
                            },
                            style = MaterialTheme.typography.bodyMedium,
                            color = when {
                                typing != null -> colors.primary
                                unread -> colors.onSurface
                                else -> colors.onSurfaceVariant
                            },
                            fontStyle = if (typing == null && last?.recalled == true) FontStyle.Italic else FontStyle.Normal,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f),
                        )
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
        }
    }
}
