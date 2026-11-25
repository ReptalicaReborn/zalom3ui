package com.zalom3ui.app.ui

import android.net.Uri
import android.os.Build
import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.PickVisualMediaRequest
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.animateColorAsState
import androidx.compose.animation.core.Animatable
import androidx.compose.animation.core.FiniteAnimationSpec
import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animate
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.animation.core.tween
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.scaleIn
import androidx.compose.animation.scaleOut
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.offset
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.Reply
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.automirrored.outlined.Forward
import androidx.compose.material.icons.automirrored.outlined.Logout
import androidx.compose.material.icons.automirrored.outlined.Reply
import androidx.compose.material.icons.automirrored.outlined.Undo
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.filled.PushPin
import androidx.compose.material.icons.outlined.AddCircleOutline
import androidx.compose.material.icons.outlined.AttachFile
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.EmojiEmotions
import androidx.compose.material.icons.outlined.Group
import androidx.compose.material.icons.outlined.Keyboard
import androidx.compose.material.icons.outlined.MarkChatUnread
import androidx.compose.material.icons.outlined.Mood
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.Notifications
import androidx.compose.material.icons.outlined.NotificationsOff
import androidx.compose.material.icons.outlined.PersonAdd
import androidx.compose.material.icons.outlined.PhotoLibrary
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.FloatingToolbarDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.HorizontalFloatingToolbar
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.LinearWavyProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.runtime.snapshotFlow
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.blur
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.LinkAnnotation
import androidx.compose.ui.text.LinkInteractionListener
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextLayoutResult
import androidx.compose.ui.text.TextLinkStyles
import androidx.compose.ui.text.TextRange
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.KeyboardCapitalization
import androidx.compose.ui.text.input.TextFieldValue
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.zalom3ui.app.data.ChatMessage
import com.zalom3ui.app.data.GroupMember
import com.zalom3ui.app.data.MuteOption
import com.zalom3ui.app.data.QUICK_REACTIONS
import com.zalom3ui.app.data.Quote
import com.zalom3ui.app.data.StickerInfo
import com.zalom3ui.app.data.ThreadSummary
import kotlin.math.ceil
import kotlin.math.roundToInt
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

/** Everything the chat screen can ask for. */
class ChatActions(
    val onBack: () -> Unit,
    val onSend: (text: String, quote: ChatMessage?, mentioned: List<GroupMember>, onSent: () -> Unit) -> Unit,
    val onStartTyping: () -> Unit,
    val onLoadMembers: () -> Unit,
    val onLoadEarlier: () -> Unit,
    val onReact: (ChatMessage, String) -> Unit,
    val onRecall: (ChatMessage) -> Unit,
    val onDelete: (ChatMessage) -> Unit,
    val onForward: (ChatMessage) -> Unit,
    val onPin: (Boolean) -> Unit,
    val onMute: (MuteOption?) -> Unit,
    val onMarkUnread: () -> Unit,
    val onDeleteConversation: () -> Unit,
    val onSendFiles: (List<Uri>) -> Unit,
    val onSearchStickers: (String) -> Unit,
    val onSendSticker: (StickerInfo) -> Unit,
    val onShowMembers: () -> Unit,
    val onAddMembers: () -> Unit,
    val onRenameGroup: (String) -> Unit,
    val onLeaveGroup: (silent: Boolean) -> Unit,
    val onOpenInfo: () -> Unit = {},
)

/** Photos and videos picked at once from the system photo picker. */
private const val MAX_PICKED_MEDIA = 20

private enum class MessageAction { REPLY, COPY, FORWARD, RECALL, DELETE }

/** Messages from the same person this close together are drawn as one group of bubbles. */
private const val GROUP_GAP_MS = 5 * 60_000L

private val URL_REGEX = Regex("""https?://[^\s<>"]+""", RegexOption.IGNORE_CASE)
private val TRAILING_URL_PUNCT = setOf('.', ',', '!', '?', ';', ':', '\'', '"')

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun ChatScreen(
    chat: ChatState,
    thread: ThreadSummary?,
    typing: TypingInfo?,
    stickers: StickerSearch,
    actions: ChatActions,
    initialSearch: Boolean = false,
    onSearchConsumed: () -> Unit = {},
) {
    val context = LocalContext.current
    val clipboard = LocalClipboardManager.current
    val colors = LocalChatColors.current
    var menuFor by remember { mutableStateOf<String?>(null) }
    var reactionsFor by remember { mutableStateOf<String?>(null) }
    var replyTo by remember { mutableStateOf<ChatMessage?>(null) }
    var confirm by remember { mutableStateOf<Pair<MessageAction, ChatMessage>?>(null) }
    var dialog by rememberSaveable { mutableStateOf<String?>(null) }
    var viewing by rememberSaveable { mutableStateOf<String?>(null) }
    var showStickers by rememberSaveable { mutableStateOf(false) }

    var searching by rememberSaveable { mutableStateOf(initialSearch) }
    var searchQuery by rememberSaveable { mutableStateOf("") }
    var searchIndex by rememberSaveable { mutableIntStateOf(0) }
    var jumpNonce by remember { mutableIntStateOf(0) }
    var jumpTarget by remember { mutableStateOf<Pair<String, Int>?>(null) }

    LaunchedEffect(initialSearch) {
        if (initialSearch) {
            searching = true
            onSearchConsumed()
        }
    }

    val searchMatches = remember(chat.messages, searchQuery) {
        findMessageMatches(chat.messages, searchQuery)
    }
    val activeSearchIndex = if (searchMatches.isEmpty()) 0 else searchIndex.coerceIn(0, searchMatches.lastIndex)
    val activeSearchMatchId = searchMatches.getOrNull(activeSearchIndex)

    fun jumpToMessage(id: String) {
        jumpNonce++
        jumpTarget = id to jumpNonce
    }

    LaunchedEffect(searching, activeSearchMatchId, activeSearchIndex) {
        if (searching && activeSearchMatchId != null) {
            jumpToMessage(activeSearchMatchId)
        }
    }

    val closeSearch: () -> Unit = {
        searching = false
        searchQuery = ""
        searchIndex = 0
    }

    val selectedMsg = remember(menuFor, chat.messages) {
        menuFor?.let { id -> chat.messages.firstOrNull { it.id == id } }
    }
    var overlayMsg by remember { mutableStateOf<ChatMessage?>(null) }
    if (selectedMsg != null) overlayMsg = selectedMsg

    BackHandler(enabled = selectedMsg != null) { menuFor = null }
    BackHandler(enabled = searching && selectedMsg == null) { closeSearch() }

    val isGroup = chat.kind == "group"
    DisposableEffect(chat.ref) { onDispose { VoicePlayback.stop() } }
    val pickMedia = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(MAX_PICKED_MEDIA)) {
        if (it.isNotEmpty()) actions.onSendFiles(it)
    }
    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) {
        if (it.isNotEmpty()) actions.onSendFiles(it)
    }

    val onAction: (ChatMessage, MessageAction) -> Unit = { msg, action ->
        menuFor = null
        when (action) {
            MessageAction.REPLY -> replyTo = msg
            MessageAction.COPY -> {
                clipboard.setText(AnnotatedString(msg.text))
                // Android 13+ confirms copies itself.
                if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
                    Toast.makeText(context, "Copied", Toast.LENGTH_SHORT).show()
                }
            }
            MessageAction.FORWARD -> actions.onForward(msg)
            MessageAction.RECALL, MessageAction.DELETE -> confirm = action to msg
        }
    }

    val onOpenUrl: (String) -> Unit = { url -> openExternally(context, url) }

    val blurRadius by animateDpAsState(
        targetValue = if (selectedMsg != null) 12.dp else 0.dp,
        animationSpec = MaterialTheme.motionScheme.fastEffectsSpec(),
        label = "chatOverlayBlur",
    )
    val scaffoldModifier = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && blurRadius > 0.dp) {
        Modifier.fillMaxSize().blur(blurRadius)
    } else {
        Modifier.fillMaxSize()
    }

    Box(Modifier.fillMaxSize()) {
        Scaffold(
            modifier = scaffoldModifier,
            containerColor = colors.background,
            topBar = {
                if (searching) {
                    InChatSearchTopBar(
                        query = searchQuery,
                        onQueryChange = {
                            searchQuery = it
                            searchIndex = 0
                        },
                        matchCount = searchMatches.size,
                        activeIndex = activeSearchIndex,
                        onOlder = {
                            if (searchMatches.isNotEmpty()) {
                                searchIndex = (activeSearchIndex + 1) % searchMatches.size
                            }
                        },
                        onNewer = {
                            if (searchMatches.isNotEmpty()) {
                                searchIndex = (activeSearchIndex - 1 + searchMatches.size) % searchMatches.size
                            }
                        },
                        onClose = closeSearch,
                    )
                } else {
                    TopAppBar(
                        colors = TopAppBarDefaults.topAppBarColors(containerColor = colors.background),
                        navigationIcon = {
                            IconButton(onClick = actions.onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back") }
                        },
                        title = {
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clip(RoundedCornerShape(12.dp))
                                    .clickable(onClick = actions.onOpenInfo)
                                    .padding(vertical = 4.dp, horizontal = 2.dp),
                            ) {
                                Avatar(chat.name, thread?.avatar.orEmpty(), key = chat.ref.key, isGroup = isGroup, size = 36.dp)
                                Spacer(Modifier.width(12.dp))
                                Column {
                                    Text(chat.name, maxLines = 1, overflow = TextOverflow.Ellipsis, style = MaterialTheme.typography.titleMedium)
                                    val subtitle = when {
                                        typing != null -> if (isGroup && typing.name.isNotBlank()) "${typing.name} is typing…" else "typing…"
                                        chat.lastOnline != null -> formatPresence(chat.lastOnline)
                                        isGroup && chat.members != null -> "${chat.members.size + 1} members"
                                        else -> null
                                    }
                                    subtitle?.let {
                                        Text(
                                            it,
                                            style = MaterialTheme.typography.bodySmall,
                                            color = if (typing != null) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                                            maxLines = 1,
                                        )
                                    }
                                }
                            }
                        },
                        actions = {
                            ChatMenu(
                                thread = thread,
                                isGroup = isGroup,
                                onSearch = { searching = true },
                                onMembers = actions.onShowMembers,
                                onAddMembers = actions.onAddMembers,
                                onRename = { dialog = "rename" },
                                onLeave = { dialog = "leave" },
                                onPin = actions.onPin,
                                onMute = { dialog = "mute" },
                                onUnmute = { actions.onMute(null) },
                                onMarkUnread = actions.onMarkUnread,
                                onDelete = { dialog = "delete" },
                            )
                        },
                    )
                }
            },
            bottomBar = {
                Composer(
                    isGroup = isGroup,
                    members = chat.members,
                    sending = chat.sending,
                    uploading = chat.uploading,
                    stickers = stickers,
                    onPickMedia = { pickMedia.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo)) },
                    onPickFiles = { pickFiles.launch(arrayOf("*/*")) },
                    onPickSticker = { showStickers = true },
                    onSearchStickers = actions.onSearchStickers,
                    onSendSticker = actions.onSendSticker,
                    replyTo = replyTo,
                    onCancelReply = { replyTo = null },
                    onStartTyping = actions.onStartTyping,
                    onLoadMembers = actions.onLoadMembers,
                    onSend = { text, mentioned, onSent ->
                        val quote = replyTo
                        actions.onSend(text, quote, mentioned) {
                            if (replyTo?.id == quote?.id) replyTo = null
                            onSent()
                        }
                    },
                )
            },
        ) { padding ->
            Box(Modifier.fillMaxSize().padding(padding)) {
                MessageList(
                    chat = chat,
                    typing = typing,
                    jumpTarget = jumpTarget,
                    onOpenMenu = { menuFor = it.id },
                    onOpenReactions = { reactionsFor = it.id },
                    onQuoteClick = { quote ->
                        findQuotedMessageId(chat.messages, quote)?.let(::jumpToMessage)
                    },
                    onSwipeReply = { replyTo = it },
                    onLoadEarlier = actions.onLoadEarlier,
                    onOpenImage = { viewing = it },
                    onOpenUrl = onOpenUrl,
                )
                if (chat.loading && chat.messages.isEmpty()) {
                    LinearWavyProgressIndicator(Modifier.fillMaxWidth())
                }
            }
        }

        AnimatedVisibility(
            visible = selectedMsg != null,
            enter = fadeIn(MaterialTheme.motionScheme.fastEffectsSpec()),
            exit = fadeOut(MaterialTheme.motionScheme.fastEffectsSpec()),
        ) {
            overlayMsg?.let { msg ->
                FocusedMessageOverlay(
                    msg = msg,
                    isGroup = isGroup,
                    onDismiss = { menuFor = null },
                    onAction = { onAction(msg, it) },
                    onReact = { key ->
                        menuFor = null
                        actions.onReact(msg, key)
                    },
                )
            }
        }
    }

    reactionsFor?.let { msgId ->
        val target = chat.messages.firstOrNull { it.id == msgId }
        if (target == null || target.reactions.isEmpty()) {
            LaunchedEffect(msgId, target?.reactions?.size) { reactionsFor = null }
        } else {
            ReactionsDetailsSheet(
                msg = target,
                onToggle = { key -> actions.onReact(target, key) },
                onDismiss = { reactionsFor = null },
            )
        }
    }

    confirm?.let { (action, msg) ->
        val recall = action == MessageAction.RECALL
        AlertDialog(
            onDismissRequest = { confirm = null },
            title = { Text(if (recall) "Recall this message?" else "Delete this message?") },
            text = { Text(if (recall) "It is removed for everyone in the chat." else "It is removed only for you.") },
            confirmButton = {
                TextButton(onClick = {
                    confirm = null
                    if (recall) actions.onRecall(msg) else actions.onDelete(msg)
                }) { Text(if (recall) "Recall" else "Delete", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { confirm = null }) { Text("Cancel") } },
        )
    }

    when (dialog) {
        "mute" -> MuteDialog(
            onDismiss = { dialog = null },
            onPick = {
                dialog = null
                actions.onMute(it)
            },
        )
        "delete" -> DeleteConversationDialog(
            onDismiss = { dialog = null },
            onConfirm = {
                dialog = null
                actions.onDeleteConversation()
            },
        )
        "rename" -> RenameGroupDialog(
            current = chat.name,
            onDismiss = { dialog = null },
            onRename = {
                dialog = null
                actions.onRenameGroup(it)
            },
        )
        "leave" -> LeaveGroupDialog(
            name = chat.name,
            onDismiss = { dialog = null },
            onLeave = {
                dialog = null
                actions.onLeaveGroup(it)
            },
        )
    }

    viewing?.let { PhotoViewer(it, onDismiss = { viewing = null }) }
    if (showStickers) {
        StickerSheet(
            search = stickers,
            onSearch = actions.onSearchStickers,
            onPick = {
                showStickers = false
                actions.onSendSticker(it)
            },
            onDismiss = { showStickers = false },
        )
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun InChatSearchTopBar(
    query: String,
    onQueryChange: (String) -> Unit,
    matchCount: Int,
    activeIndex: Int,
    onOlder: () -> Unit,
    onNewer: () -> Unit,
    onClose: () -> Unit,
) {
    val colors = LocalChatColors.current
    val focus = remember { FocusRequester() }
    LaunchedEffect(Unit) { runCatching { focus.requestFocus() } }
    TopAppBar(
        colors = TopAppBarDefaults.topAppBarColors(containerColor = colors.background),
        navigationIcon = {
            IconButton(onClick = onClose) {
                Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Close search")
            }
        },
        title = {
            BasicTextField(
                value = query,
                onValueChange = onQueryChange,
                singleLine = true,
                textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                modifier = Modifier.fillMaxWidth().focusRequester(focus),
                decorationBox = { inner ->
                    Box(contentAlignment = Alignment.CenterStart) {
                        if (query.isEmpty()) {
                            Text(
                                "Search in chat",
                                style = MaterialTheme.typography.bodyLarge,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        inner()
                    }
                },
            )
        },
        actions = {
            if (query.isNotBlank()) {
                Text(
                    if (matchCount == 0) "0 / 0" else "${activeIndex + 1} / $matchCount",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(end = 2.dp),
                )
                IconButton(onClick = onOlder, enabled = matchCount > 0) {
                    Icon(Icons.Filled.KeyboardArrowUp, contentDescription = "Older match")
                }
                IconButton(onClick = onNewer, enabled = matchCount > 0) {
                    Icon(Icons.Filled.KeyboardArrowDown, contentDescription = "Newer match")
                }
                IconButton(onClick = { onQueryChange("") }) {
                    Icon(Icons.Outlined.Close, contentDescription = "Clear search")
                }
            }
        },
    )
}

@Composable
private fun ChatMenu(
    thread: ThreadSummary?,
    isGroup: Boolean,
    onSearch: () -> Unit,
    onMembers: () -> Unit,
    onAddMembers: () -> Unit,
    onRename: () -> Unit,
    onLeave: () -> Unit,
    onPin: (Boolean) -> Unit,
    onMute: () -> Unit,
    onUnmute: () -> Unit,
    onMarkUnread: () -> Unit,
    onDelete: () -> Unit,
) {
    var open by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { open = true }) { Icon(Icons.Outlined.MoreVert, contentDescription = "Chat options") }
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            MenuEntry("Search in chat", Icons.Outlined.Search) {
                open = false
                onSearch()
            }
            HorizontalDivider()
            if (isGroup) {
                MenuEntry("Members", Icons.Outlined.Group) {
                    open = false
                    onMembers()
                }
                MenuEntry("Add members", Icons.Outlined.PersonAdd) {
                    open = false
                    onAddMembers()
                }
                MenuEntry("Rename group", Icons.Outlined.Edit) {
                    open = false
                    onRename()
                }
                HorizontalDivider()
            }
            val pinned = thread?.pinned == true
            MenuEntry(if (pinned) "Unpin" else "Pin to top", if (pinned) Icons.Filled.PushPin else Icons.Outlined.PushPin) {
                open = false
                onPin(!pinned)
            }
            if (thread?.isMuted == true) {
                MenuEntry("Unmute", Icons.Outlined.Notifications) {
                    open = false
                    onUnmute()
                }
            } else {
                MenuEntry("Mute…", Icons.Outlined.NotificationsOff) {
                    open = false
                    onMute()
                }
            }
            MenuEntry("Mark as unread", Icons.Outlined.MarkChatUnread) {
                open = false
                onMarkUnread()
            }
            HorizontalDivider()
            MenuEntry("Delete conversation", Icons.Outlined.Delete, destructive = true) {
                open = false
                onDelete()
            }
            if (isGroup) {
                MenuEntry("Leave group", Icons.AutoMirrored.Outlined.Logout, destructive = true) {
                    open = false
                    onLeave()
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun MessageList(
    chat: ChatState,
    typing: TypingInfo?,
    jumpTarget: Pair<String, Int>?,
    onOpenMenu: (ChatMessage) -> Unit,
    onOpenReactions: (ChatMessage) -> Unit,
    onQuoteClick: (Quote) -> Unit,
    onSwipeReply: (ChatMessage) -> Unit,
    onLoadEarlier: () -> Unit,
    onOpenImage: (String) -> Unit,
    onOpenUrl: (String) -> Unit,
) {
    val listState = rememberLazyListState()
    val scope = rememberCoroutineScope()
    // Newest at the bottom: reverseLayout keeps the view pinned to the latest message.
    val reversed = remember(chat.messages) { chat.messages.asReversed() }
    val isGroup = chat.kind == "group"
    val unreadCount = remember(chat.messages, chat.firstUnreadId) {
        val start = chat.messages.indexOfFirst { it.id == chat.firstUnreadId }
        if (start < 0) 0 else chat.messages.drop(start).count { !it.isSelf }
    }

    var highlightedMsgId by remember { mutableStateOf<String?>(null) }
    var highlightNonce by remember { mutableIntStateOf(0) }

    LaunchedEffect(jumpTarget) {
        val (id, nonce) = jumpTarget ?: return@LaunchedEffect
        val idx = reversed.indexOfFirst { it.id == id }
        if (idx >= 0) {
            val listIndex = if (typing != null) idx + 1 else idx
            listState.animateScrollToItem(listIndex)
            highlightedMsgId = id
            highlightNonce = nonce
        }
    }
    LaunchedEffect(highlightedMsgId, highlightNonce) {
        if (highlightedMsgId != null) {
            delay(800)
            highlightedMsgId = null
        }
    }

    val nearTop by remember {
        derivedStateOf {
            val info = listState.layoutInfo
            val last = info.visibleItemsInfo.lastOrNull()?.index ?: 0
            info.totalItemsCount > 0 && last >= info.totalItemsCount - 3
        }
    }
    // Re-checked after each page, so it keeps loading while the top of the chat is in view.
    LaunchedEffect(listState, chat.hasMore, chat.messages.size) {
        snapshotFlow { nearTop }.collect { if (it && chat.hasMore) onLoadEarlier() }
    }
    // Follow new messages (and "typing…") when already at the bottom.
    LaunchedEffect(chat.messages.lastOrNull()?.id, typing != null) {
        if (listState.firstVisibleItemIndex <= 1) listState.animateScrollToItem(0)
    }

    val showJump by remember { derivedStateOf { listState.firstVisibleItemIndex > 2 } }
    var newWhileScrolledUp by remember(chat.ref) { mutableIntStateOf(0) }
    var lastSeenNewestId by remember(chat.ref) { mutableStateOf(chat.messages.lastOrNull()?.id) }
    LaunchedEffect(showJump) {
        if (!showJump) newWhileScrolledUp = 0
    }
    LaunchedEffect(chat.messages.lastOrNull()?.id) {
        val newestId = chat.messages.lastOrNull()?.id
        val prevId = lastSeenNewestId
        if (newestId != null && prevId != null && newestId != prevId && showJump) {
            val prevIdx = chat.messages.indexOfLast { it.id == prevId }
            if (prevIdx >= 0 && prevIdx < chat.messages.lastIndex) {
                newWhileScrolledUp += chat.messages.subList(prevIdx + 1, chat.messages.size).count { !it.isSelf }
            }
        }
        lastSeenNewestId = newestId
    }

    val topVisibleTs by remember(reversed, typing != null) {
        derivedStateOf {
            val offset = if (typing != null) 1 else 0
            val topItem = listState.layoutInfo.visibleItemsInfo
                .lastOrNull { it.index >= offset && (it.index - offset) < reversed.size }
            topItem?.let { reversed.getOrNull(it.index - offset)?.ts }
        }
    }
    var showDatePill by remember { mutableStateOf(false) }
    LaunchedEffect(listState.isScrollInProgress) {
        if (listState.isScrollInProgress) {
            showDatePill = true
        } else if (showDatePill) {
            delay(1200)
            showDatePill = false
        }
    }
    var lastTopTs by remember { mutableStateOf<Long?>(null) }
    topVisibleTs?.let { lastTopTs = it }

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val maxBubble = maxWidth * 0.78f
        LazyColumn(
            state = listState,
            reverseLayout = true,
            contentPadding = PaddingValues(horizontal = 8.dp, vertical = 8.dp),
            modifier = Modifier.fillMaxSize(),
        ) {
            if (typing != null) {
                item(key = "typing") { TypingBubble(isGroup, typing) }
            }
            items(reversed.size, key = { reversed[it].id }) { i ->
                val msg = reversed[i]
                val older = reversed.getOrNull(i + 1)
                val newer = reversed.getOrNull(i - 1)
                val firstOfRun = older == null || older.fromId != msg.fromId || !sameDay(older.ts, msg.ts) ||
                    msg.ts - older.ts > GROUP_GAP_MS || msg.id == chat.firstUnreadId
                val lastOfRun = newer == null || newer.fromId != msg.fromId || !sameDay(newer.ts, msg.ts) ||
                    newer.ts - msg.ts > GROUP_GAP_MS || newer.id == chat.firstUnreadId
                Column {
                    if (older == null || !sameDay(older.ts, msg.ts)) DayHeader(msg.ts)
                    if (msg.id == chat.firstUnreadId && unreadCount > 0) UnreadDivider(unreadCount)
                    MessageRow(
                        msg = msg,
                        isGroup = isGroup,
                        firstOfRun = firstOfRun,
                        lastOfRun = lastOfRun,
                        maxBubble = maxBubble,
                        highlighted = highlightedMsgId == msg.id,
                        onLongPress = { onOpenMenu(msg) },
                        onOpenReactions = { onOpenReactions(msg) },
                        onQuoteClick = onQuoteClick,
                        onSwipeReply = { onSwipeReply(msg) },
                        onOpenImage = onOpenImage,
                        onOpenUrl = onOpenUrl,
                    )
                }
            }
            if (chat.loadingMore) {
                item(key = "loading") { LinearWavyProgressIndicator(Modifier.fillMaxWidth().padding(vertical = 8.dp)) }
            }
        }

        AnimatedVisibility(
            visible = showDatePill && lastTopTs != null,
            enter = fadeIn(MaterialTheme.motionScheme.fastEffectsSpec()),
            exit = fadeOut(MaterialTheme.motionScheme.fastEffectsSpec()),
            modifier = Modifier.align(Alignment.TopCenter).padding(top = 10.dp),
        ) {
            lastTopTs?.let { ts ->
                Surface(
                    shape = CircleShape,
                    color = MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = 0.94f),
                    contentColor = MaterialTheme.colorScheme.onSurface,
                    shadowElevation = 3.dp,
                ) {
                    Text(
                        formatDay(ts),
                        style = MaterialTheme.typography.labelMedium,
                        fontWeight = FontWeight.SemiBold,
                        modifier = Modifier.padding(horizontal = 14.dp, vertical = 6.dp),
                    )
                }
            }
        }

        AnimatedVisibility(
            visible = showJump,
            enter = fadeIn(MaterialTheme.motionScheme.fastEffectsSpec()) + scaleIn(MaterialTheme.motionScheme.fastSpatialSpec()),
            exit = fadeOut(MaterialTheme.motionScheme.fastEffectsSpec()) + scaleOut(MaterialTheme.motionScheme.fastSpatialSpec()),
            modifier = Modifier.align(Alignment.BottomEnd).padding(16.dp),
        ) {
            Box {
                SmallFloatingActionButton(
                    onClick = {
                        newWhileScrolledUp = 0
                        scope.launch { listState.animateScrollToItem(0) }
                    },
                    containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                    contentColor = MaterialTheme.colorScheme.onSurface,
                    shape = CircleShape,
                ) { Icon(Icons.Filled.KeyboardArrowDown, contentDescription = "Jump to latest") }
                if (newWhileScrolledUp > 0) {
                    CountBadge(
                        text = if (newWhileScrolledUp > 99) "99+" else "$newWhileScrolledUp",
                        color = MaterialTheme.colorScheme.primary,
                        contentColor = MaterialTheme.colorScheme.onPrimary,
                        modifier = Modifier.align(Alignment.TopEnd).offset(x = 4.dp, y = (-8).dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun DayHeader(ts: Long) {
    Box(Modifier.fillMaxWidth().padding(top = 16.dp, bottom = 8.dp), contentAlignment = Alignment.Center) {
        Text(
            formatDay(ts),
            style = MaterialTheme.typography.labelMedium,
            fontWeight = FontWeight.SemiBold,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
    }
}

@Composable
private fun UnreadDivider(count: Int) {
    Surface(
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        shape = RoundedCornerShape(8.dp),
        modifier = Modifier.fillMaxWidth().padding(vertical = 8.dp),
    ) {
        Text(
            if (count == 1) "1 unread message" else "$count unread messages",
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            textAlign = TextAlign.Center,
            modifier = Modifier.padding(vertical = 6.dp).fillMaxWidth(),
        )
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun MessageRow(
    msg: ChatMessage,
    isGroup: Boolean,
    firstOfRun: Boolean,
    lastOfRun: Boolean,
    maxBubble: Dp,
    highlighted: Boolean,
    onLongPress: () -> Unit,
    onOpenReactions: () -> Unit,
    onQuoteClick: (Quote) -> Unit,
    onSwipeReply: () -> Unit,
    onOpenImage: (String) -> Unit,
    onOpenUrl: (String) -> Unit,
) {
    val self = msg.isSelf
    val haptics = LocalHapticFeedback.current
    val scope = rememberCoroutineScope()
    var dragOffset by remember { mutableFloatStateOf(0f) }
    var settleJob by remember { mutableStateOf<Job?>(null) }
    val threshold = with(LocalDensity.current) { 64.dp.toPx() }
    val currentOnSwipeReply by rememberUpdatedState(onSwipeReply)
    val settleSpec: FiniteAnimationSpec<Float> = MaterialTheme.motionScheme.fastSpatialSpec()
    val currentSettleSpec by rememberUpdatedState(settleSpec)

    val highlightColor by animateColorAsState(
        targetValue = if (highlighted) MaterialTheme.colorScheme.primary.copy(alpha = 0.24f) else Color.Transparent,
        animationSpec = tween(durationMillis = if (highlighted) 160 else 640),
        label = "bubbleHighlight",
    )

    Box(
        Modifier
            .fillMaxWidth()
            .padding(top = if (firstOfRun) 8.dp else 2.dp)
            .clip(RoundedCornerShape(20.dp))
            .background(highlightColor),
    ) {
        // Revealed while swiping a message to the right to reply to it.
        if (dragOffset > 0f) {
            Icon(
                Icons.AutoMirrored.Filled.Reply,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier
                    .align(Alignment.CenterStart)
                    .padding(start = 8.dp)
                    .graphicsLayer {
                        val progress = (dragOffset / threshold).coerceIn(0f, 1f)
                        alpha = progress
                        scaleX = 0.6f + 0.4f * progress
                        scaleY = 0.6f + 0.4f * progress
                    },
            )
        }
        Row(
            Modifier
                .fillMaxWidth()
                .offset { IntOffset(dragOffset.roundToInt(), 0) }
                .pointerInput(msg.id, msg.recalled) {
                    if (msg.recalled) {
                        dragOffset = 0f
                        return@pointerInput
                    }
                    detectHorizontalDragGestures(
                        onDragStart = {
                            settleJob?.cancel()
                            settleJob = null
                        },
                        onDragEnd = {
                            if (dragOffset >= threshold) currentOnSwipeReply()
                            val start = dragOffset
                            settleJob?.cancel()
                            settleJob = scope.launch {
                                animate(
                                    initialValue = start,
                                    targetValue = 0f,
                                    animationSpec = currentSettleSpec,
                                ) { value, _ -> dragOffset = value }
                            }
                        },
                        onDragCancel = {
                            val start = dragOffset
                            settleJob?.cancel()
                            settleJob = scope.launch {
                                animate(
                                    initialValue = start,
                                    targetValue = 0f,
                                    animationSpec = currentSettleSpec,
                                ) { value, _ -> dragOffset = value }
                            }
                        },
                        onHorizontalDrag = { change, delta ->
                            val prev = dragOffset
                            val next = (prev + delta).coerceIn(0f, threshold * 1.3f)
                            if (prev < threshold && next >= threshold) {
                                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                            }
                            dragOffset = next
                            change.consume()
                        },
                    )
                },
            horizontalArrangement = if (self) Arrangement.End else Arrangement.Start,
            verticalAlignment = Alignment.Bottom,
        ) {
            if (isGroup && !self) {
                Box(Modifier.width(36.dp)) {
                    if (lastOfRun) Avatar(msg.fromName, "", key = msg.fromId, isGroup = false, size = 28.dp)
                }
            }
            Column(
                horizontalAlignment = if (self) Alignment.End else Alignment.Start,
                modifier = Modifier.widthIn(max = maxBubble),
            ) {
                if (msg.attachment?.kind == "sticker" && !msg.recalled) {
                    StickerBubble(msg, isGroup, firstOfRun, onLongPress)
                } else {
                    Bubble(
                        msg = msg,
                        isGroup = isGroup,
                        firstOfRun = firstOfRun,
                        lastOfRun = lastOfRun,
                        onLongPress = onLongPress,
                        onOpenImage = onOpenImage,
                        onQuoteClick = onQuoteClick,
                        onOpenUrl = onOpenUrl,
                    )
                }
                if (msg.reactions.isNotEmpty()) {
                    Reactions(
                        msg = msg,
                        onClick = onOpenReactions,
                        modifier = Modifier.offset(y = (-6).dp).padding(horizontal = 10.dp),
                    )
                }
            }
        }
    }
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Bubble(
    msg: ChatMessage,
    isGroup: Boolean,
    firstOfRun: Boolean,
    lastOfRun: Boolean,
    onLongPress: () -> Unit,
    onOpenImage: (String) -> Unit,
    onQuoteClick: (Quote) -> Unit,
    onOpenUrl: (String) -> Unit,
    shadowElevation: Dp = 0.dp,
) {
    val self = msg.isSelf
    val colors = LocalChatColors.current
    val big = 20.dp
    val small = 4.dp
    val shape = if (self) {
        RoundedCornerShape(topStart = big, topEnd = if (firstOfRun) big else small, bottomEnd = if (lastOfRun) big else small, bottomStart = big)
    } else {
        RoundedCornerShape(topStart = if (firstOfRun) big else small, topEnd = big, bottomEnd = big, bottomStart = if (lastOfRun) big else small)
    }
    val container = if (self) colors.outgoing else colors.incoming
    val content = if (self) colors.onOutgoing else colors.onIncoming
    val meta = content.copy(alpha = 0.75f)
    val attachment = msg.attachment?.takeIf { !msg.recalled && it.isShown() }
    val isFramelessMedia = attachment != null && attachment.kind in setOf("photo", "gif", "video")

    val footer: @Composable () -> Unit = {
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text(formatClock(msg.ts), style = MaterialTheme.typography.labelSmall, color = meta)
            if (self && !msg.recalled) {
                Spacer(Modifier.width(4.dp))
                DeliveryIcon(deliveryState(msg), tint = meta, background = container)
            }
        }
    }

    Surface(
        shape = shape,
        color = container,
        contentColor = content,
        shadowElevation = shadowElevation,
        modifier = Modifier.clip(shape).combinedClickable(onClick = {}, onLongClick = onLongPress),
    ) {
        if (isFramelessMedia && attachment != null) {
            val caption = attachment.caption()
            val showHeader = (isGroup && !self && firstOfRun && msg.fromName.isNotBlank()) || msg.quote != null
            Column {
                if (showHeader) {
                    Column(Modifier.padding(start = 12.dp, end = 12.dp, top = 8.dp, bottom = 4.dp)) {
                        if (isGroup && !self && firstOfRun && msg.fromName.isNotBlank()) {
                            Text(
                                msg.fromName,
                                style = MaterialTheme.typography.labelLarge,
                                fontWeight = FontWeight.SemiBold,
                                color = senderNameColor(msg.fromId),
                                modifier = Modifier.padding(bottom = 2.dp),
                            )
                        }
                        msg.quote?.let { q -> QuoteBlock(q.fromName, q.text, onOutgoing = self, onClick = { onQuoteClick(q) }) }
                    }
                }
                Box {
                    AttachmentContent(attachment, onOutgoing = self, onOpenImage = onOpenImage, edgeToEdge = true)
                    if (caption.isNullOrBlank()) {
                        Surface(
                            shape = CircleShape,
                            color = Color.Black.copy(alpha = 0.50f),
                            contentColor = Color.White,
                            modifier = Modifier.align(Alignment.BottomEnd).padding(8.dp),
                        ) {
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                modifier = Modifier.padding(horizontal = 8.dp, vertical = 3.dp),
                            ) {
                                Text(formatClock(msg.ts), style = MaterialTheme.typography.labelSmall, color = Color.White)
                                if (self && !msg.recalled) {
                                    Spacer(Modifier.width(4.dp))
                                    DeliveryIcon(deliveryState(msg), tint = Color.White, background = Color.Black.copy(alpha = 0.50f))
                                }
                            }
                        }
                    }
                }
                if (!caption.isNullOrBlank()) {
                    Box(Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
                        TextWithFooter(
                            text = linkifiedAnnotatedString(caption, onOutgoing = self, onOpenUrl = onOpenUrl),
                            style = MaterialTheme.typography.bodyLarge,
                            color = content,
                            footer = footer,
                        )
                    }
                }
            }
        } else {
            Column(Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
                if (isGroup && !self && firstOfRun && msg.fromName.isNotBlank()) {
                    Text(
                        msg.fromName,
                        style = MaterialTheme.typography.labelLarge,
                        fontWeight = FontWeight.SemiBold,
                        color = senderNameColor(msg.fromId),
                        modifier = Modifier.padding(bottom = 2.dp),
                    )
                }
                msg.quote?.let { q -> QuoteBlock(q.fromName, q.text, onOutgoing = self, onClick = { onQuoteClick(q) }) }
                if (msg.recalled) {
                    TextWithFooter(
                        text = AnnotatedString("Message recalled"),
                        style = MaterialTheme.typography.bodyLarge.copy(fontStyle = FontStyle.Italic),
                        color = meta,
                        footer = footer,
                    )
                } else if (attachment != null) {
                    AttachmentContent(attachment, onOutgoing = self, onOpenImage = onOpenImage)
                    Spacer(Modifier.height(4.dp))
                    TextWithFooter(
                        text = linkifiedAnnotatedString(attachment.caption().orEmpty(), onOutgoing = self, onOpenUrl = onOpenUrl),
                        style = MaterialTheme.typography.bodyLarge,
                        color = content,
                        footer = footer,
                    )
                } else {
                    TextWithFooter(
                        text = messageText(msg, onOutgoing = self, onOpenUrl = onOpenUrl),
                        style = MaterialTheme.typography.bodyLarge,
                        color = content,
                        footer = footer,
                    )
                }
            }
        }
    }
}

/** A sticker stands on its own, without a bubble; the time sits under it. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun StickerBubble(msg: ChatMessage, isGroup: Boolean, firstOfRun: Boolean, onLongPress: () -> Unit) {
    Column(
        horizontalAlignment = if (msg.isSelf) Alignment.End else Alignment.Start,
        modifier = Modifier.clip(RoundedCornerShape(12.dp)).combinedClickable(onClick = {}, onLongClick = onLongPress),
    ) {
        if (isGroup && !msg.isSelf && firstOfRun && msg.fromName.isNotBlank()) {
            Text(
                msg.fromName,
                style = MaterialTheme.typography.labelLarge,
                fontWeight = FontWeight.SemiBold,
                color = senderNameColor(msg.fromId),
                modifier = Modifier.padding(start = 4.dp, bottom = 2.dp),
            )
        }
        StickerImage(msg.attachment?.thumb)
        Text(
            formatClock(msg.ts),
            style = MaterialTheme.typography.labelSmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.padding(horizontal = 4.dp),
        )
    }
}

/** Message text with @mentions emphasized, auto-linkified clickable URLs, and attachments named. */
@Composable
private fun messageText(msg: ChatMessage, onOutgoing: Boolean, onOpenUrl: (String) -> Unit): AnnotatedString {
    val linkColor = if (onOutgoing) LocalChatColors.current.onOutgoing else MaterialTheme.colorScheme.primary
    return buildAnnotatedString {
        val text = msg.text
        append(text)
        for (m in msg.mentions) {
            val start = m.pos.coerceIn(0, text.length)
            val end = (m.pos + m.len).coerceIn(start, text.length)
            if (end > start) addStyle(SpanStyle(fontWeight = FontWeight.Bold), start, end)
        }
        val linkStyle = TextLinkStyles(SpanStyle(color = linkColor, textDecoration = TextDecoration.Underline))
        for (range in extractUrls(text)) {
            val url = text.substring(range)
            addLink(
                LinkAnnotation.Url(
                    url = url,
                    styles = linkStyle,
                    linkInteractionListener = LinkInteractionListener { onOpenUrl(url) },
                ),
                start = range.first,
                end = range.last + 1,
            )
        }
        msg.attachment?.let { a ->
            val label = a.title?.takeIf { it.isNotBlank() } ?: a.href ?: "Attachment"
            if (text.isNotBlank()) append("\n")
            append("📎 $label")
        }
    }
}

@Composable
private fun linkifiedAnnotatedString(text: String, onOutgoing: Boolean, onOpenUrl: (String) -> Unit): AnnotatedString {
    if (text.isEmpty()) return AnnotatedString("")
    val linkColor = if (onOutgoing) LocalChatColors.current.onOutgoing else MaterialTheme.colorScheme.primary
    return buildAnnotatedString {
        append(text)
        val linkStyle = TextLinkStyles(SpanStyle(color = linkColor, textDecoration = TextDecoration.Underline))
        for (range in extractUrls(text)) {
            val url = text.substring(range)
            addLink(
                LinkAnnotation.Url(
                    url = url,
                    styles = linkStyle,
                    linkInteractionListener = LinkInteractionListener { onOpenUrl(url) },
                ),
                start = range.first,
                end = range.last + 1,
            )
        }
    }
}

/**
 * Text followed by the time and check marks: on the last line when they fit there, as chat apps do,
 * otherwise on a line of their own.
 */
@Composable
private fun TextWithFooter(text: AnnotatedString, style: TextStyle, color: Color, footer: @Composable () -> Unit) {
    val textLayout = remember { arrayOfNulls<TextLayoutResult>(1) }
    Layout(
        content = {
            Text(text, style = style, color = color, onTextLayout = { textLayout[0] = it })
            footer()
        },
    ) { measurables, constraints ->
        val textPlaceable = measurables[0].measure(constraints.copy(minWidth = 0, minHeight = 0))
        val footerPlaceable = measurables[1].measure(Constraints())
        val gap = 8.dp.roundToPx()
        val result = textLayout[0]
        val lastLineWidth = if (result != null && result.lineCount > 0) {
            val last = result.lineCount - 1
            ceil(result.getLineRight(last) - result.getLineLeft(last)).toInt()
        } else {
            textPlaceable.width
        }
        val empty = text.isEmpty()
        val inline = !empty && lastLineWidth + gap + footerPlaceable.width <= constraints.maxWidth
        val width: Int
        val height: Int
        when {
            empty -> {
                width = footerPlaceable.width
                height = footerPlaceable.height
            }
            inline -> {
                width = maxOf(textPlaceable.width, lastLineWidth + gap + footerPlaceable.width)
                height = maxOf(textPlaceable.height, footerPlaceable.height)
            }
            else -> {
                width = maxOf(textPlaceable.width, footerPlaceable.width)
                height = textPlaceable.height + footerPlaceable.height
            }
        }
        layout(width, height) {
            if (!empty) textPlaceable.place(0, 0)
            footerPlaceable.place(width - footerPlaceable.width, height - footerPlaceable.height)
        }
    }
}

@Composable
private fun QuoteBlock(from: String, text: String, onOutgoing: Boolean, onClick: () -> Unit) {
    val colors = LocalChatColors.current
    val bar = if (onOutgoing) colors.onOutgoing else MaterialTheme.colorScheme.primary
    val background = if (onOutgoing) Color.White.copy(alpha = 0.18f) else colors.background.copy(alpha = 0.65f)
    Row(
        Modifier
            .padding(bottom = 6.dp)
            .clip(RoundedCornerShape(10.dp))
            .background(background)
            .clickable(onClick = onClick)
            .height(IntrinsicSize.Min),
    ) {
        Box(Modifier.width(4.dp).fillMaxHeight().background(bar))
        Column(Modifier.padding(horizontal = 10.dp, vertical = 6.dp)) {
            Text(
                from.ifBlank { "Message" },
                style = MaterialTheme.typography.labelLarge,
                fontWeight = FontWeight.SemiBold,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(text, style = MaterialTheme.typography.bodyMedium, maxLines = 3, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
private fun Reactions(msg: ChatMessage, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val colors = LocalChatColors.current
    Row(
        modifier = modifier
            .clip(CircleShape)
            .clickable(onClick = onClick),
        horizontalArrangement = Arrangement.spacedBy(4.dp),
    ) {
        msg.reactions.forEach { r ->
            Surface(
                shape = CircleShape,
                color = if (r.mine) MaterialTheme.colorScheme.secondaryContainer else colors.incoming,
                contentColor = colors.onIncoming,
                border = BorderStroke(2.dp, colors.background),
            ) {
                Text(
                    if (r.count > 1) "${r.emoji} ${r.count}" else r.emoji,
                    style = MaterialTheme.typography.labelMedium,
                    modifier = Modifier.padding(horizontal = 8.dp, vertical = 3.dp),
                )
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ReactionsDetailsSheet(
    msg: ChatMessage,
    onToggle: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    val sorted = remember(msg.reactions) { msg.reactions.sortedByDescending { it.count } }
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(Modifier.fillMaxWidth().padding(bottom = 24.dp)) {
            Text(
                "Reactions",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.padding(horizontal = 20.dp),
            )
            Text(
                "Tap a reaction to toggle yours",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp),
            )
            Spacer(Modifier.height(8.dp))
            sorted.forEach { r ->
                Row(
                    Modifier
                        .fillMaxWidth()
                        .clickable { onToggle(r.key) }
                        .padding(horizontal = 20.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(r.emoji, fontSize = 26.sp)
                    Spacer(Modifier.width(14.dp))
                    Column(Modifier.weight(1f)) {
                        Text(
                            if (r.count == 1) "1 reaction" else "${r.count} reactions",
                            style = MaterialTheme.typography.bodyLarge,
                            fontWeight = FontWeight.Medium,
                        )
                        if (r.mine) {
                            Text(
                                "Includes yours — tap to remove",
                                style = MaterialTheme.typography.bodySmall,
                                color = MaterialTheme.colorScheme.primary,
                            )
                        }
                    }
                    Surface(
                        shape = CircleShape,
                        color = if (r.mine) MaterialTheme.colorScheme.primaryContainer else MaterialTheme.colorScheme.surfaceContainerHigh,
                    ) {
                        Text(
                            "${r.count}",
                            style = MaterialTheme.typography.labelLarge,
                            fontWeight = FontWeight.SemiBold,
                            color = if (r.mine) MaterialTheme.colorScheme.onPrimaryContainer else MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(horizontal = 12.dp, vertical = 4.dp),
                        )
                    }
                }
            }
        }
    }
}

/** Focused message overlay: floating reaction pill, elevated bubble preview, and M3 Expressive HorizontalFloatingToolbar. */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun FocusedMessageOverlay(
    msg: ChatMessage,
    isGroup: Boolean,
    onDismiss: () -> Unit,
    onAction: (MessageAction) -> Unit,
    onReact: (String) -> Unit,
) {
    val fastSpatial: FiniteAnimationSpec<Float> = MaterialTheme.motionScheme.fastSpatialSpec()
    var appeared by remember(msg.id) { mutableStateOf(false) }
    LaunchedEffect(msg.id) { appeared = true }
    val previewScale by animateFloatAsState(
        targetValue = if (appeared) 1.02f else 0.96f,
        animationSpec = fastSpatial,
        label = "previewScale",
    )

    Box(
        Modifier
            .fillMaxSize()
            .background(Color.Black.copy(alpha = 0.55f))
            .clickable(
                interactionSource = remember { MutableInteractionSource() },
                indication = null,
                onClick = onDismiss,
            )
            .statusBarsPadding()
            .navigationBarsPadding()
            .padding(horizontal = 16.dp),
        contentAlignment = Alignment.Center,
    ) {
        Column(
            horizontalAlignment = if (msg.isSelf) Alignment.End else Alignment.Start,
            verticalArrangement = Arrangement.spacedBy(14.dp),
            modifier = Modifier.fillMaxWidth(),
        ) {
            if (!msg.recalled) {
                Surface(
                    shape = RoundedCornerShape(28.dp),
                    color = MaterialTheme.colorScheme.surfaceContainerHigh,
                    shadowElevation = 6.dp,
                ) {
                    Row(
                        Modifier.padding(horizontal = 8.dp, vertical = 6.dp),
                        horizontalArrangement = Arrangement.spacedBy(2.dp),
                    ) {
                        QUICK_REACTIONS.forEachIndexed { index, (key, emoji) ->
                            val mine = msg.reactions.any { it.mine && it.key == key }
                            val scale = remember(msg.id, key) { Animatable(0f) }
                            LaunchedEffect(msg.id, key) {
                                delay(index * 28L)
                                scale.animateTo(1f, animationSpec = fastSpatial)
                            }
                            Box(
                                Modifier
                                    .size(44.dp)
                                    .graphicsLayer {
                                        scaleX = scale.value
                                        scaleY = scale.value
                                    }
                                    .clip(CircleShape)
                                    .background(if (mine) MaterialTheme.colorScheme.primaryContainer else Color.Transparent)
                                    .clickable { onReact(key) },
                                contentAlignment = Alignment.Center,
                            ) { Text(emoji, fontSize = 24.sp) }
                        }
                    }
                }
            }

            Box(
                Modifier
                    .widthIn(max = 310.dp)
                    .graphicsLayer {
                        scaleX = previewScale
                        scaleY = previewScale
                    },
            ) {
                if (msg.attachment?.kind == "sticker" && !msg.recalled) {
                    StickerBubble(msg, isGroup = isGroup, firstOfRun = true, onLongPress = {})
                } else {
                    Bubble(
                        msg = msg,
                        isGroup = isGroup,
                        firstOfRun = true,
                        lastOfRun = true,
                        onLongPress = {},
                        onOpenImage = {},
                        onQuoteClick = {},
                        onOpenUrl = {},
                        shadowElevation = 8.dp,
                    )
                }
            }

            HorizontalFloatingToolbar(
                expanded = true,
                colors = FloatingToolbarDefaults.standardFloatingToolbarColors(),
            ) {
                if (!msg.recalled) {
                    ToolbarActionItem("Reply", Icons.AutoMirrored.Outlined.Reply) { onAction(MessageAction.REPLY) }
                    if (msg.text.isNotBlank() && msg.attachment == null) {
                        ToolbarActionItem("Copy", Icons.Outlined.ContentCopy) { onAction(MessageAction.COPY) }
                    }
                    if (msg.isText) {
                        ToolbarActionItem("Forward", Icons.AutoMirrored.Outlined.Forward) { onAction(MessageAction.FORWARD) }
                    }
                    if (msg.isSelf) {
                        ToolbarActionItem("Recall", Icons.AutoMirrored.Outlined.Undo, destructive = true) { onAction(MessageAction.RECALL) }
                    }
                }
                ToolbarActionItem("Delete", Icons.Outlined.DeleteOutline, destructive = true) { onAction(MessageAction.DELETE) }
            }
        }
    }
}

@Composable
private fun ToolbarActionItem(
    label: String,
    icon: ImageVector,
    destructive: Boolean = false,
    onClick: () -> Unit,
) {
    val tint = if (destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = Modifier
            .clip(RoundedCornerShape(16.dp))
            .clickable(onClick = onClick)
            .padding(horizontal = 10.dp, vertical = 6.dp),
    ) {
        Icon(icon, contentDescription = label, tint = tint, modifier = Modifier.size(20.dp))
        Spacer(Modifier.height(2.dp))
        Text(label, style = MaterialTheme.typography.labelSmall, color = tint, maxLines = 1)
    }
}

/** "…" bubble while the other side is typing. */
@Composable
private fun TypingBubble(isGroup: Boolean, typing: TypingInfo) {
    val colors = LocalChatColors.current
    val transition = rememberInfiniteTransition(label = "typing")
    Row(Modifier.fillMaxWidth().padding(top = 8.dp), verticalAlignment = Alignment.Bottom) {
        if (isGroup) {
            Box(Modifier.width(36.dp)) { Avatar(typing.name, "", key = typing.name, isGroup = false, size = 28.dp) }
        }
        Surface(shape = RoundedCornerShape(18.dp), color = colors.incoming) {
            Row(
                Modifier.padding(horizontal = 16.dp, vertical = 14.dp),
                horizontalArrangement = Arrangement.spacedBy(5.dp),
            ) {
                repeat(3) { i ->
                    val dotAlpha by transition.animateFloat(
                        initialValue = 0.25f,
                        targetValue = 1f,
                        animationSpec = infiniteRepeatable(tween(durationMillis = 450, delayMillis = i * 150), RepeatMode.Reverse),
                        label = "dot$i",
                    )
                    Box(
                        Modifier
                            .size(8.dp)
                            .graphicsLayer { alpha = dotAlpha }
                            .background(colors.onIncoming.copy(alpha = 0.7f), CircleShape),
                    )
                }
            }
        }
    }
}

/** Message input: a rounded field with a unified emoji/sticker drawer, Signal-style attachment sheet, and spring Send button. */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun Composer(
    isGroup: Boolean,
    members: List<GroupMember>?,
    sending: Boolean,
    uploading: Int,
    stickers: StickerSearch,
    onPickMedia: () -> Unit,
    onPickFiles: () -> Unit,
    onPickSticker: () -> Unit,
    onSearchStickers: (String) -> Unit,
    onSendSticker: (StickerInfo) -> Unit,
    replyTo: ChatMessage?,
    onCancelReply: () -> Unit,
    onStartTyping: () -> Unit,
    onLoadMembers: () -> Unit,
    onSend: (String, List<GroupMember>, onSent: () -> Unit) -> Unit,
) {
    val colors = LocalChatColors.current
    val haptics = LocalHapticFeedback.current
    var value by remember { mutableStateOf(TextFieldValue("")) }
    val mentioned = remember { mutableStateListOf<GroupMember>() }
    var showEmoji by rememberSaveable { mutableStateOf(false) }
    var attachSheet by remember { mutableStateOf(false) }
    val keyboard = LocalSoftwareKeyboardController.current
    val focus = remember { FocusRequester() }
    val density = LocalDensity.current
    val ime = WindowInsets.ime
    val imeVisible by remember(ime, density) { derivedStateOf { ime.getBottom(density) > 0 } }
    // The keyboard and the emoji/sticker drawer take turns.
    LaunchedEffect(imeVisible) { if (imeVisible) showEmoji = false }
    // Starting a reply (swipe or menu) goes straight to typing it.
    LaunchedEffect(replyTo?.id) {
        if (replyTo != null && !showEmoji) {
            runCatching { focus.requestFocus() }
            keyboard?.show()
        }
    }
    BackHandler(enabled = showEmoji) { showEmoji = false }

    // The "@query" being typed right before the cursor, if any.
    val mentionQuery = remember(value, isGroup) { if (isGroup) mentionQueryAt(value) else null }
    LaunchedEffect(mentionQuery != null) { if (mentionQuery != null) onLoadMembers() }
    val suggestions = remember(mentionQuery, members) {
        val q = mentionQuery?.let { foldForSearch(it.second) }
        if (q == null) emptyList() else members.orEmpty().filter { foldForSearch(it.name).contains(q) }.take(6)
    }

    fun replaceSelection(insert: String) {
        val start = value.selection.min
        value = TextFieldValue(value.text.replaceRange(start, value.selection.max, insert), TextRange(start + insert.length))
    }

    fun insertMention(member: GroupMember) {
        val (start, _) = mentionQuery ?: return
        val end = value.selection.start
        val tag = "@${member.name} "
        value = TextFieldValue(value.text.replaceRange(start, end, tag), TextRange(start + tag.length))
        mentioned += member
    }

    fun backspace() {
        if (!value.selection.collapsed) {
            replaceSelection("")
            return
        }
        val cursor = value.selection.start
        if (cursor == 0) return
        val start = previousCharacterStart(value.text, cursor)
        value = TextFieldValue(value.text.removeRange(start, cursor), TextRange(start))
    }

    Column(Modifier.background(colors.background).navigationBarsPadding().imePadding()) {
        if (uploading > 0) UploadingNote(uploading)
        if (suggestions.isNotEmpty()) {
            Surface(
                color = MaterialTheme.colorScheme.surfaceContainerHigh,
                shape = RoundedCornerShape(16.dp),
                modifier = Modifier.fillMaxWidth().padding(horizontal = 8.dp, vertical = 4.dp),
            ) {
                Column(Modifier.padding(vertical = 4.dp)) {
                    suggestions.forEach { m ->
                        Row(
                            Modifier.fillMaxWidth().clickable { insertMention(m) }.padding(horizontal = 12.dp, vertical = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Avatar(m.name, "", key = m.id, isGroup = false, size = 32.dp)
                            Spacer(Modifier.width(12.dp))
                            Text(m.name, style = MaterialTheme.typography.bodyLarge)
                        }
                    }
                }
            }
        }
        Row(
            Modifier.padding(start = 4.dp, end = 8.dp, top = 4.dp, bottom = 8.dp),
            verticalAlignment = Alignment.Bottom,
        ) {
            Box(Modifier.padding(bottom = 4.dp)) {
                IconButton(onClick = { attachSheet = true }) {
                    Icon(Icons.Outlined.AddCircleOutline, contentDescription = "Attach", tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            Surface(
                shape = RoundedCornerShape(24.dp),
                color = MaterialTheme.colorScheme.surfaceContainerHigh,
                modifier = Modifier.weight(1f),
            ) {
                Column {
                    replyTo?.let { ReplyPreview(it, onCancelReply) }
                    Row(verticalAlignment = Alignment.Bottom) {
                        IconButton(onClick = {
                            if (showEmoji) {
                                showEmoji = false
                                runCatching { focus.requestFocus() }
                                keyboard?.show()
                            } else {
                                keyboard?.hide()
                                showEmoji = true
                            }
                        }) {
                            Icon(
                                if (showEmoji) Icons.Outlined.Keyboard else Icons.Outlined.EmojiEmotions,
                                contentDescription = if (showEmoji) "Show keyboard" else "Emoji & stickers",
                                tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                        BasicTextField(
                            value = value,
                            onValueChange = {
                                if (it.text != value.text && it.text.isNotEmpty()) onStartTyping()
                                value = it
                            },
                            textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                            cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                            keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                            maxLines = 6,
                            modifier = Modifier
                                .weight(1f)
                                .padding(top = 13.dp, bottom = 13.dp, end = 16.dp)
                                .focusRequester(focus),
                            decorationBox = { inner ->
                                Box {
                                    if (value.text.isEmpty()) {
                                        Text(
                                            "Message",
                                            style = MaterialTheme.typography.bodyLarge,
                                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                                        )
                                    }
                                    inner()
                                }
                            },
                        )
                    }
                }
            }
            AnimatedVisibility(
                visible = value.text.isNotBlank(),
                enter = fadeIn(MaterialTheme.motionScheme.fastEffectsSpec()) + scaleIn(MaterialTheme.motionScheme.fastSpatialSpec()),
                exit = fadeOut(MaterialTheme.motionScheme.fastEffectsSpec()) + scaleOut(MaterialTheme.motionScheme.fastSpatialSpec()),
            ) {
                FilledIconButton(
                    onClick = {
                        haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                        val sentText = value.text
                        onSend(sentText, mentioned.toList()) {
                            if (value.text == sentText) {
                                value = TextFieldValue("")
                                mentioned.clear()
                            }
                        }
                    },
                    enabled = !sending,
                    colors = IconButtonDefaults.filledIconButtonColors(
                        containerColor = colors.outgoing,
                        contentColor = colors.onOutgoing,
                    ),
                    modifier = Modifier.padding(start = 8.dp).size(48.dp),
                ) { Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "Send") }
            }
        }
        if (showEmoji) {
            EmojiStickerDrawer(
                stickers = stickers,
                onEmoji = { emoji ->
                    replaceSelection(emoji)
                    onStartTyping()
                },
                onBackspace = { backspace() },
                onSearchStickers = onSearchStickers,
                onSendSticker = { sticker ->
                    onSendSticker(sticker)
                },
            )
        }
    }

    if (attachSheet) {
        AttachmentSheet(
            onDismiss = { attachSheet = false },
            onPickMedia = {
                attachSheet = false
                onPickMedia()
            },
            onPickFiles = {
                attachSheet = false
                onPickFiles()
            },
            onPickSticker = {
                attachSheet = false
                keyboard?.hide()
                onPickSticker()
            },
        )
    }
}

/** Signal-style attachment sheet with expressive 60.dp rounded-squircle tinted tiles. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AttachmentSheet(
    onDismiss: () -> Unit,
    onPickMedia: () -> Unit,
    onPickFiles: () -> Unit,
    onPickSticker: () -> Unit,
) {
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Row(
            Modifier
                .fillMaxWidth()
                .padding(start = 24.dp, end = 24.dp, top = 8.dp, bottom = 32.dp),
            horizontalArrangement = Arrangement.SpaceEvenly,
        ) {
            AttachmentTile(
                label = "Photos & videos",
                icon = Icons.Outlined.PhotoLibrary,
                container = MaterialTheme.colorScheme.primaryContainer,
                content = MaterialTheme.colorScheme.onPrimaryContainer,
                onClick = onPickMedia,
            )
            AttachmentTile(
                label = "File",
                icon = Icons.Outlined.AttachFile,
                container = MaterialTheme.colorScheme.secondaryContainer,
                content = MaterialTheme.colorScheme.onSecondaryContainer,
                onClick = onPickFiles,
            )
            AttachmentTile(
                label = "Sticker",
                icon = Icons.Outlined.Mood,
                container = MaterialTheme.colorScheme.tertiaryContainer,
                content = MaterialTheme.colorScheme.onTertiaryContainer,
                onClick = onPickSticker,
            )
        }
    }
}

@Composable
private fun AttachmentTile(
    label: String,
    icon: ImageVector,
    container: Color,
    content: Color,
    onClick: () -> Unit,
) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = Modifier
            .widthIn(min = 84.dp)
            .clip(RoundedCornerShape(16.dp))
            .clickable(onClick = onClick)
            .padding(8.dp),
    ) {
        Surface(
            shape = RoundedCornerShape(20.dp),
            color = container,
            contentColor = content,
            modifier = Modifier.size(60.dp),
        ) {
            Box(contentAlignment = Alignment.Center) {
                Icon(icon, contentDescription = label, modifier = Modifier.size(28.dp))
            }
        }
        Spacer(Modifier.height(8.dp))
        Text(
            label,
            style = MaterialTheme.typography.labelMedium,
            color = MaterialTheme.colorScheme.onSurface,
            textAlign = TextAlign.Center,
            maxLines = 2,
        )
    }
}

@Composable
private fun ReplyPreview(msg: ChatMessage, onCancel: () -> Unit) {
    Row(
        Modifier
            .padding(start = 8.dp, end = 8.dp, top = 8.dp)
            .clip(RoundedCornerShape(14.dp))
            .background(MaterialTheme.colorScheme.surfaceContainerHighest)
            .height(IntrinsicSize.Min),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.width(4.dp).fillMaxHeight().background(MaterialTheme.colorScheme.primary))
        Column(Modifier.weight(1f).padding(horizontal = 10.dp, vertical = 6.dp)) {
            Text(
                if (msg.isSelf) "You" else msg.fromName.ifBlank { "Message" },
                style = MaterialTheme.typography.labelLarge,
                color = MaterialTheme.colorScheme.primary,
            )
            Text(
                if (msg.recalled) "Message recalled" else msg.text,
                style = MaterialTheme.typography.bodySmall,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
        IconButton(onClick = onCancel) { Icon(Icons.Outlined.Close, contentDescription = "Cancel reply") }
    }
}

private fun findQuotedMessageId(messages: List<ChatMessage>, quote: Quote): String? {
    if (quote.msgId.isNotBlank()) {
        messages.firstOrNull { it.id == quote.msgId || it.cliMsgId == quote.msgId }?.let { return it.id }
    }
    if (quote.text.isNotBlank()) {
        messages.lastOrNull {
            !it.recalled && it.text == quote.text && (quote.fromName.isBlank() || it.fromName == quote.fromName)
        }?.let { return it.id }
    }
    return null
}

/** IDs of non-recalled messages matching [query] (case- and diacritic-insensitive), newest first. */
internal fun findMessageMatches(messages: List<ChatMessage>, query: String): List<String> {
    val q = foldForSearch(query.trim())
    if (q.isEmpty()) return emptyList()
    return messages
        .asReversed()
        .filter { msg ->
            !msg.recalled && (
                foldForSearch(msg.text).contains(q) ||
                    msg.attachment?.title?.let { foldForSearch(it).contains(q) } == true
            )
        }
        .sortedByDescending { it.ts }
        .map { it.id }
}

/** Ranges of `http://` / `https://` URLs in [text], with trailing sentence punctuation stripped. */
internal fun extractUrls(text: String): List<IntRange> {
    if (text.isEmpty()) return emptyList()
    val out = mutableListOf<IntRange>()
    for (match in URL_REGEX.findAll(text)) {
        val start = match.range.first
        var end = match.range.last
        while (end >= start) {
            val c = text[end]
            val trim = when {
                c in TRAILING_URL_PUNCT -> true
                c == ')' -> text.substring(start, end + 1).let { s -> s.count { it == ')' } > s.count { it == '(' } }
                c == ']' -> text.substring(start, end + 1).let { s -> s.count { it == ']' } > s.count { it == '[' } }
                else -> false
            }
            if (trim) end-- else break
        }
        val schemeEnd = text.indexOf("://", start) + 3
        if (end >= schemeEnd) out += start..end
    }
    return out
}

/** Start index of "@" and the text typed after it, when the cursor is inside an @mention being typed. */
internal fun mentionQueryAt(value: TextFieldValue): Pair<Int, String>? {
    val cursor = value.selection.start
    if (!value.selection.collapsed || cursor <= 0) return null
    val before = value.text.substring(0, cursor)
    val at = before.lastIndexOf('@')
    if (at < 0) return null
    if (at > 0 && !before[at - 1].isWhitespace()) return null
    val query = before.substring(at + 1)
    if (query.length > 30 || '\n' in query || query.startsWith(" ")) return null
    return at to query
}
