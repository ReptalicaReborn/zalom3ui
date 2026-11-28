package com.zalom3ui.app.ui

import android.Manifest
import android.content.pm.PackageManager
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
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.gestures.awaitEachGesture
import androidx.compose.foundation.gestures.awaitFirstDown
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.systemGestureExclusion
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
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
import androidx.compose.foundation.lazy.LazyListState
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
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
import androidx.compose.material.icons.automirrored.outlined.MissedVideoCall
import androidx.compose.material.icons.automirrored.outlined.PhoneMissed
import androidx.compose.material.icons.automirrored.outlined.Reply
import androidx.compose.material.icons.automirrored.outlined.Undo
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.KeyboardArrowDown
import androidx.compose.material.icons.filled.KeyboardArrowUp
import androidx.compose.material.icons.filled.PushPin
import androidx.compose.material.icons.outlined.AttachFile
import androidx.compose.material.icons.outlined.Call
import androidx.compose.material.icons.outlined.CheckCircle
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.CloudOff
import androidx.compose.material.icons.outlined.ContactPage
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.EmojiEmotions
import androidx.compose.material.icons.outlined.ErrorOutline
import androidx.compose.material.icons.outlined.Group
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.Keyboard
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.outlined.MarkChatUnread
import androidx.compose.material.icons.outlined.Mic
import androidx.compose.material.icons.outlined.Mood
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.Notifications
import androidx.compose.material.icons.outlined.NotificationsOff
import androidx.compose.material.icons.outlined.PersonAdd
import androidx.compose.material.icons.outlined.PhotoCamera
import androidx.compose.material.icons.outlined.PhotoLibrary
import androidx.compose.material.icons.outlined.Poll
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material.icons.outlined.RadioButtonUnchecked
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.Timer
import androidx.compose.material.icons.outlined.Videocam
import androidx.compose.material.icons.rounded.Add
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
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.LinearWavyProgressIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.SmallFloatingActionButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
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
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.isAltPressed
import androidx.compose.ui.input.key.isCtrlPressed
import androidx.compose.ui.input.key.isShiftPressed
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.layout.Layout
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalConfiguration
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
import androidx.compose.ui.text.style.Hyphens
import androidx.compose.ui.text.style.LineBreak
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Constraints
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.IntOffset
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import androidx.core.content.FileProvider
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import coil.compose.AsyncImage
import com.zalom3ui.app.data.Appearance
import com.zalom3ui.app.data.ChatAccent
import com.zalom3ui.app.data.ChatMessage
import com.zalom3ui.app.data.GroupMember
import com.zalom3ui.app.data.MuteOption
import com.zalom3ui.app.data.PendingMessage
import com.zalom3ui.app.data.Person
import com.zalom3ui.app.data.QUICK_REACTIONS
import com.zalom3ui.app.data.Quote
import com.zalom3ui.app.data.StickerInfo
import com.zalom3ui.app.data.ThreadSummary
import com.zalom3ui.app.ui.call.CallActivity
import java.io.File
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
    val onDraftChange: (String) -> Unit = {},
    val onSendVoice: (File) -> Unit = {},
    val onRetryPending: (String) -> Unit = {},
    val onDismissPending: (String) -> Unit = {},
    val onDeleteMany: (List<ChatMessage>) -> Unit = {},
    val onForwardMany: (List<ChatMessage>) -> Unit = {},
    val onLoadGroupBoard: () -> Unit = {},
    val onCreateGroupNote: (String) -> Unit = {},
    val onSetDisappearingTtl: (Long?) -> Unit = {},
    val onLoadFriends: () -> Unit = {},
    val onSendCard: (Person) -> Unit = {},
    /** Contact cards: open (or start) the 1:1 chat with the shared person. */
    val onOpenUserChat: (Person) -> Unit = {},
    /** Contact cards: send the shared person a friend request. */
    val onAddFriend: (Person) -> Unit = {},
    val onCancelOutbox: (String) -> Unit = {},
    val onRetryOutbox: () -> Unit = {},
)

/** Photos and videos picked at once from the system photo picker. */
private const val MAX_PICKED_MEDIA = 20

private enum class MessageAction { REPLY, COPY, FORWARD, PIN_NOTE, SELECT, RECALL, DELETE }

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
    initialDraft: String = "",
    chatAccentOverride: ChatAccent? = null,
    appearance: Appearance? = null,
    scrollToMsgId: String? = null,
    onScrolledToMsg: () -> Unit = {},
    boardItems: List<BoardNoteItem> = emptyList(),
    disappearingTtlMs: Long? = null,
    outboxEntries: List<OutboxEntry> = emptyList(),
    friends: List<Person> = emptyList(),
) {
    val contactCardActions = remember(friends, actions) {
        ContactCardActions(
            isFriend = { id -> friends.any { it.id == id } },
            onMessage = actions.onOpenUserChat,
            onAddFriend = actions.onAddFriend,
        )
    }
    CompositionLocalProvider(LocalContactCardActions provides contactCardActions) {
    if (chatAccentOverride != null && appearance != null) {
        AppTheme(
            appearance = appearance.copy(accent = chatAccentOverride, dynamicColor = false),
            reducedMotion = LocalReducedMotion.current,
        ) {
            ChatScreenContent(
                chat = chat,
                thread = thread,
                typing = typing,
                stickers = stickers,
                actions = actions,
                initialSearch = initialSearch,
                onSearchConsumed = onSearchConsumed,
                initialDraft = initialDraft,
                scrollToMsgId = scrollToMsgId,
                onScrolledToMsg = onScrolledToMsg,
                boardItems = boardItems,
                disappearingTtlMs = disappearingTtlMs,
                outboxEntries = outboxEntries,
                friends = friends,
            )
        }
    } else {
        ChatScreenContent(
            chat = chat,
            thread = thread,
            typing = typing,
            stickers = stickers,
            actions = actions,
            initialSearch = initialSearch,
            onSearchConsumed = onSearchConsumed,
            initialDraft = initialDraft,
            scrollToMsgId = scrollToMsgId,
            onScrolledToMsg = onScrolledToMsg,
            boardItems = boardItems,
            disappearingTtlMs = disappearingTtlMs,
            outboxEntries = outboxEntries,
            friends = friends,
        )
    }
    }
}

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun ChatScreenContent(
    chat: ChatState,
    thread: ThreadSummary?,
    typing: TypingInfo?,
    stickers: StickerSearch,
    actions: ChatActions,
    initialSearch: Boolean,
    onSearchConsumed: () -> Unit,
    initialDraft: String,
    scrollToMsgId: String?,
    onScrolledToMsg: () -> Unit,
    boardItems: List<BoardNoteItem>,
    disappearingTtlMs: Long?,
    outboxEntries: List<OutboxEntry>,
    friends: List<Person>,
) {
    val context = LocalContext.current
    val clipboard = LocalClipboardManager.current
    val colors = LocalChatColors.current
    val isGroup = chat.kind == "group"
    var menuFor by remember(chat.ref) { mutableStateOf<String?>(null) }
    var reactionsFor by remember(chat.ref) { mutableStateOf<String?>(null) }
    var replyTo by remember(chat.ref) { mutableStateOf<ChatMessage?>(null) }
    var confirm by remember(chat.ref) { mutableStateOf<Pair<MessageAction, ChatMessage>?>(null) }
    var dialog by rememberSaveable { mutableStateOf<String?>(null) }
    var viewingUrl by rememberSaveable { mutableStateOf<String?>(null) }
    var viewingMsgId by rememberSaveable { mutableStateOf<String?>(null) }
    var showStickers by rememberSaveable { mutableStateOf(false) }
    var showPinNoteDialog by rememberSaveable { mutableStateOf(false) }
    var showDisappearingDialog by rememberSaveable { mutableStateOf(false) }
    var showShareContactSheet by rememberSaveable { mutableStateOf(false) }
    var pendingCameraUriStr by rememberSaveable { mutableStateOf<String?>(null) }
    var previewMediaUris by remember(chat.ref) { mutableStateOf<List<Uri>>(emptyList()) }
    var selectedIds by remember(chat.ref) { mutableStateOf(emptySet<String>()) }

    var searching by rememberSaveable { mutableStateOf(initialSearch) }
    var searchQuery by rememberSaveable { mutableStateOf("") }
    var searchIndex by rememberSaveable { mutableIntStateOf(0) }
    var jumpNonce by remember { mutableIntStateOf(0) }
    var jumpTarget by remember { mutableStateOf<Pair<String, Int>?>(null) }

    LaunchedEffect(chat.ref, isGroup) {
        if (isGroup) actions.onLoadGroupBoard()
    }

    LaunchedEffect(initialSearch) {
        if (initialSearch) {
            searching = true
            onSearchConsumed()
        }
    }

    val searchMatches = remember(chat.messages, searchQuery) {
        findInChatMatches(chat.messages, searchQuery)
    }
    LaunchedEffect(searchQuery, searchMatches.size) {
        searchIndex = if (searchMatches.isEmpty()) 0 else searchMatches.lastIndex
    }
    val activeSearchIndex = if (searchMatches.isEmpty()) 0 else searchIndex.coerceIn(0, searchMatches.lastIndex)
    val activeSearchMatchId = searchMatches.getOrNull(activeSearchIndex)

    val threadOutbox = remember(outboxEntries, chat.ref) {
        outboxEntries.filter { it.thread == chat.ref }
    }

    fun jumpToMessage(id: String) {
        jumpNonce++
        jumpTarget = id to jumpNonce
    }

    LaunchedEffect(searching, activeSearchMatchId, activeSearchIndex) {
        if (searching && activeSearchMatchId != null) {
            jumpToMessage(activeSearchMatchId)
        }
    }

    LaunchedEffect(scrollToMsgId, chat.messages, chat.loading) {
        val target = scrollToMsgId ?: return@LaunchedEffect
        if (chat.messages.any { it.id == target }) {
            jumpToMessage(target)
            onScrolledToMsg()
        } else if (!chat.loading) {
            onScrolledToMsg()
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
    BackHandler(enabled = selectedIds.isNotEmpty() && selectedMsg == null) { selectedIds = emptySet() }
    BackHandler(enabled = searching && selectedMsg == null && selectedIds.isEmpty()) { closeSearch() }

    DisposableEffect(chat.ref) { onDispose { VoicePlayback.stop() } }

    val pickMedia = rememberLauncherForActivityResult(ActivityResultContracts.PickMultipleVisualMedia(MAX_PICKED_MEDIA)) { uris ->
        if (uris.isNotEmpty()) {
            val allImages = uris.all { uri ->
                val mime = context.contentResolver.getType(uri)
                mime == null || mime.startsWith("image/")
            }
            if (allImages) {
                previewMediaUris = uris
            } else {
                actions.onSendFiles(uris)
            }
        }
    }
    val pickFiles = rememberLauncherForActivityResult(ActivityResultContracts.OpenMultipleDocuments()) {
        if (it.isNotEmpty()) actions.onSendFiles(it)
    }
    val takePicture = rememberLauncherForActivityResult(ActivityResultContracts.TakePicture()) { success ->
        val uriStr = pendingCameraUriStr
        pendingCameraUriStr = null
        if (success && uriStr != null) {
            previewMediaUris = listOf(Uri.parse(uriStr))
        }
    }
    fun launchCamera() {
        val dir = File(context.cacheDir, "camera").apply { mkdirs() }
        val photoFile = File(dir, "photo_${System.currentTimeMillis()}.jpg")
        val uri = runCatching {
            FileProvider.getUriForFile(context, "${context.packageName}.files", photoFile)
        }.getOrNull()
        if (uri == null) {
            Toast.makeText(context, "Couldn't open camera", Toast.LENGTH_SHORT).show()
            return
        }
        pendingCameraUriStr = uri.toString()
        runCatching { takePicture.launch(uri) }.onFailure {
            pendingCameraUriStr = null
            Toast.makeText(context, "No camera app available", Toast.LENGTH_SHORT).show()
        }
    }
    val cameraPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (granted) {
            launchCamera()
        } else {
            Toast.makeText(context, "Camera permission is needed to take photos", Toast.LENGTH_SHORT).show()
        }
    }
    val onTakePhoto: () -> Unit = {
        if (ContextCompat.checkSelfPermission(context, Manifest.permission.CAMERA) == PackageManager.PERMISSION_GRANTED) {
            launchCamera()
        } else {
            cameraPermission.launch(Manifest.permission.CAMERA)
        }
    }

    val chatPhotoEntries = remember(chat.messages) { extractChatPhotoEntries(chat.messages) }
    val chatPhotos = remember(chatPhotoEntries) { chatPhotoEntries.map { it.second } }

    fun toggleSelect(msg: ChatMessage) {
        selectedIds = if (msg.id in selectedIds) selectedIds - msg.id else selectedIds + msg.id
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
            MessageAction.PIN_NOTE -> {
                val noteTitle = msg.text.trim()
                if (isGroup && noteTitle.isNotEmpty()) {
                    actions.onCreateGroupNote(noteTitle)
                }
            }
            MessageAction.SELECT -> {
                if (msg.id !in selectedIds) selectedIds = selectedIds + msg.id
            }
            MessageAction.RECALL, MessageAction.DELETE -> confirm = action to msg
        }
    }

    val onOpenUrl: (String) -> Unit = { url -> openExternally(context, url) }

    val reducedMotion = LocalReducedMotion.current
    val blurRadius = if (reducedMotion) {
        0.dp
    } else {
        animateDpAsState(
            targetValue = if (selectedMsg != null) 12.dp else 0.dp,
            animationSpec = MaterialTheme.motionScheme.fastEffectsSpec(),
            label = "chatOverlayBlur",
        ).value
    }
    val scaffoldModifier = if (!reducedMotion && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S && blurRadius > 0.dp) {
        Modifier.fillMaxSize().blur(blurRadius)
    } else {
        Modifier.fillMaxSize()
    }

    Box(Modifier.fillMaxSize()) {
        Scaffold(
            modifier = scaffoldModifier,
            containerColor = colors.background,
            topBar = {
                if (selectedIds.isNotEmpty()) {
                    val selectedMessages = chat.messages.filter { it.id in selectedIds }
                    val forwardable = selectedMessages.filter { !it.recalled }
                    SelectionTopBar(
                        count = selectedIds.size,
                        canForward = forwardable.isNotEmpty(),
                        onClose = { selectedIds = emptySet() },
                        onForward = {
                            selectedIds = emptySet()
                            if (forwardable.isNotEmpty()) actions.onForwardMany(forwardable)
                        },
                        onDelete = {
                            selectedIds = emptySet()
                            if (selectedMessages.isNotEmpty()) actions.onDeleteMany(selectedMessages)
                        },
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
                                Column(Modifier.weight(1f)) {
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
                                            overflow = TextOverflow.Ellipsis,
                                        )
                                    }
                                }
                            }
                        },
                        actions = {
                            if (!isGroup) {
                                IconButton(onClick = { context.startActivity(CallActivity.outgoingIntent(context, chat.id, chat.name, thread?.avatar, video = false)) }) {
                                    Icon(Icons.Outlined.Call, contentDescription = "Voice call")
                                }
                                IconButton(onClick = { context.startActivity(CallActivity.outgoingIntent(context, chat.id, chat.name, thread?.avatar, video = true)) }) {
                                    Icon(Icons.Outlined.Videocam, contentDescription = "Video call")
                                }
                            }
                            IconButton(onClick = {
                                if (searching) closeSearch() else searching = true
                            }) {
                                Icon(Icons.Outlined.Search, contentDescription = "Search in chat")
                            }
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
                                onDisappearing = { showDisappearingDialog = true },
                                onDelete = { dialog = "delete" },
                            )
                        },
                    )
                }
            },
            bottomBar = {
                Composer(
                    threadKey = chat.ref.key,
                    initialDraft = initialDraft,
                    onDraftChange = actions.onDraftChange,
                    isGroup = isGroup,
                    members = chat.members,
                    sending = chat.sending,
                    uploading = chat.uploading,
                    stickers = stickers,
                    disappearingTtlMs = disappearingTtlMs,
                    onOpenDisappearingTimer = { showDisappearingDialog = true },
                    onOpenShareContact = { showShareContactSheet = true },
                    onTakePhoto = onTakePhoto,
                    onPickMedia = { pickMedia.launch(PickVisualMediaRequest(ActivityResultContracts.PickVisualMedia.ImageAndVideo)) },
                    onPickFiles = { pickFiles.launch(arrayOf("*/*")) },
                    onPickSticker = { showStickers = true },
                    onSearchStickers = actions.onSearchStickers,
                    onSendSticker = actions.onSendSticker,
                    onSendVoice = actions.onSendVoice,
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
            Column(Modifier.fillMaxSize().padding(padding)) {
                AnimatedVisibility(visible = searching) {
                    InChatSearchBar(
                        query = searchQuery,
                        onQueryChange = { searchQuery = it },
                        matchIndex = activeSearchIndex,
                        matchCount = searchMatches.size,
                        onPrev = {
                            if (searchMatches.isNotEmpty()) {
                                searchIndex = (activeSearchIndex - 1 + searchMatches.size) % searchMatches.size
                            }
                        },
                        onNext = {
                            if (searchMatches.isNotEmpty()) {
                                searchIndex = (activeSearchIndex + 1) % searchMatches.size
                            }
                        },
                        onClose = closeSearch,
                    )
                }
                if (isGroup && boardItems.isNotEmpty()) {
                    PinnedMessageBanner(
                        items = boardItems,
                        onCreateNote = { showPinNoteDialog = true },
                    )
                }
                Box(Modifier.weight(1f).fillMaxWidth()) {
                    MessageList(
                        chat = chat,
                        typing = typing,
                        outboxEntries = threadOutbox,
                        jumpTarget = jumpTarget,
                        searchFocusedId = if (searching) activeSearchMatchId else null,
                        selectedIds = selectedIds,
                        onToggleSelect = ::toggleSelect,
                        onOpenMenu = { menuFor = it.id },
                        onOpenReactions = { reactionsFor = it.id },
                        onQuoteClick = { quote ->
                            findQuotedMessageId(chat.messages, quote)?.let(::jumpToMessage)
                        },
                        onSwipeReply = { replyTo = it },
                        onLoadEarlier = actions.onLoadEarlier,
                        onOpenImage = { msgId, url ->
                            viewingMsgId = msgId
                            viewingUrl = url
                        },
                        onOpenUrl = onOpenUrl,
                        onRetryPending = actions.onRetryPending,
                        onDismissPending = actions.onDismissPending,
                        onCancelOutbox = actions.onCancelOutbox,
                        onRetryOutbox = actions.onRetryOutbox,
                    )
                    if (chat.loading && chat.messages.isEmpty()) {
                        if (reducedMotion) {
                            LinearProgressIndicator(Modifier.fillMaxWidth())
                        } else {
                            LinearWavyProgressIndicator(Modifier.fillMaxWidth())
                        }
                    }
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

    if (showPinNoteDialog) {
        PinNoteDialog(
            onDismiss = { showPinNoteDialog = false },
            onCreate = { noteText ->
                showPinNoteDialog = false
                actions.onCreateGroupNote(noteText)
            },
        )
    }

    if (showDisappearingDialog) {
        DisappearingTimerDialog(
            currentTtlMs = disappearingTtlMs,
            onDismiss = { showDisappearingDialog = false },
            onSelect = { ttl ->
                showDisappearingDialog = false
                actions.onSetDisappearingTtl(ttl)
            },
        )
    }

    if (showShareContactSheet) {
        ShareContactSheet(
            friends = friends,
            onLoad = actions.onLoadFriends,
            onDismiss = { showShareContactSheet = false },
            onPick = { friend ->
                showShareContactSheet = false
                actions.onSendCard(friend)
            },
        )
    }

    if (previewMediaUris.isNotEmpty()) {
        MediaPreviewSheet(
            uris = previewMediaUris,
            onRemove = { uri -> previewMediaUris = previewMediaUris - uri },
            onDismiss = { previewMediaUris = emptyList() },
            onSend = { uris, caption ->
                previewMediaUris = emptyList()
                if (uris.isNotEmpty()) actions.onSendFiles(uris)
                val trimmed = caption.trim()
                if (trimmed.isNotEmpty()) {
                    actions.onSend(trimmed, null, emptyList()) {}
                }
            },
        )
    }

    viewingUrl?.let { url ->
        val idx = findChatPhotoIndex(chatPhotoEntries, viewingMsgId, url)
        val gallery = if (idx >= 0) chatPhotos else listOf(url)
        val initialIndex = if (idx >= 0) idx else 0
        PhotoViewer(
            photos = gallery,
            initialIndex = initialIndex,
            onDismiss = {
                viewingUrl = null
                viewingMsgId = null
            },
            subtitle = chat.name,
        )
    }
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
private fun SelectionTopBar(
    count: Int,
    canForward: Boolean,
    onClose: () -> Unit,
    onForward: () -> Unit,
    onDelete: () -> Unit,
) {
    TopAppBar(
        colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.surfaceContainerHigh),
        navigationIcon = {
            IconButton(onClick = onClose) {
                Icon(Icons.Outlined.Close, contentDescription = "Clear selection")
            }
        },
        title = {
            Text(
                "$count selected",
                style = MaterialTheme.typography.titleMedium,
            )
        },
        actions = {
            IconButton(onClick = onForward, enabled = canForward) {
                Icon(Icons.AutoMirrored.Outlined.Forward, contentDescription = "Forward selected")
            }
            IconButton(onClick = onDelete) {
                Icon(Icons.Outlined.DeleteOutline, contentDescription = "Delete selected")
            }
        },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MediaPreviewSheet(
    uris: List<Uri>,
    onRemove: (Uri) -> Unit,
    onDismiss: () -> Unit,
    onSend: (List<Uri>, String) -> Unit,
) {
    val colors = LocalChatColors.current
    var caption by rememberSaveable { mutableStateOf("") }
    val previewMaxHeight = (LocalConfiguration.current.screenHeightDp * 0.38f).dp
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .imePadding()
                .padding(horizontal = 16.dp, vertical = 4.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Row(
                modifier = Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(
                    if (uris.size == 1) "Send photo" else "Send ${uris.size} photos",
                    style = MaterialTheme.typography.titleMedium,
                    fontWeight = FontWeight.SemiBold,
                )
                IconButton(onClick = onDismiss) {
                    Icon(Icons.Outlined.Close, contentDescription = "Cancel")
                }
            }
            if (uris.size == 1) {
                Box(
                    modifier = Modifier
                        .fillMaxWidth()
                        .heightIn(min = 120.dp, max = previewMaxHeight)
                        .clip(RoundedCornerShape(18.dp))
                        .background(MaterialTheme.colorScheme.surfaceContainerHighest),
                    contentAlignment = Alignment.Center,
                ) {
                    AsyncImage(
                        model = uris.first(),
                        contentDescription = "Selected photo",
                        contentScale = ContentScale.Fit,
                        modifier = Modifier.fillMaxWidth().heightIn(max = previewMaxHeight),
                    )
                }
            } else {
                LazyRow(
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                    contentPadding = PaddingValues(vertical = 2.dp),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    items(uris, key = { it.toString() }) { uri ->
                        Box(
                            modifier = Modifier
                                .size(width = 120.dp, height = 148.dp)
                                .clip(RoundedCornerShape(16.dp))
                                .background(MaterialTheme.colorScheme.surfaceContainerHighest),
                        ) {
                            AsyncImage(
                                model = uri,
                                contentDescription = "Selected photo",
                                contentScale = ContentScale.Crop,
                                modifier = Modifier.fillMaxSize(),
                            )
                            Surface(
                                shape = CircleShape,
                                color = Color.Black.copy(alpha = 0.6f),
                                contentColor = Color.White,
                                modifier = Modifier
                                    .align(Alignment.TopEnd)
                                    .padding(6.dp)
                                    .size(26.dp)
                                    .clip(CircleShape)
                                    .clickable { onRemove(uri) },
                            ) {
                                Box(contentAlignment = Alignment.Center) {
                                    Icon(Icons.Outlined.Close, contentDescription = "Remove photo", modifier = Modifier.size(16.dp))
                                }
                            }
                        }
                    }
                }
            }
            Row(
                modifier = Modifier.fillMaxWidth().padding(bottom = 16.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                OutlinedTextField(
                    value = caption,
                    onValueChange = { caption = it },
                    placeholder = { Text("Add a caption…") },
                    maxLines = 4,
                    shape = RoundedCornerShape(24.dp),
                    modifier = Modifier.weight(1f),
                )
                FilledIconButton(
                    onClick = { onSend(uris, caption) },
                    colors = IconButtonDefaults.filledIconButtonColors(
                        containerColor = colors.outgoing,
                        contentColor = colors.onOutgoing,
                    ),
                    modifier = Modifier.size(50.dp),
                ) {
                    Icon(Icons.AutoMirrored.Filled.Send, contentDescription = "Send")
                }
            }
        }
    }
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
    onDisappearing: () -> Unit,
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
            MenuEntry("Disappearing messages", Icons.Outlined.Timer) {
                open = false
                onDisappearing()
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
    outboxEntries: List<OutboxEntry>,
    jumpTarget: Pair<String, Int>?,
    searchFocusedId: String?,
    selectedIds: Set<String>,
    onToggleSelect: (ChatMessage) -> Unit,
    onOpenMenu: (ChatMessage) -> Unit,
    onOpenReactions: (ChatMessage) -> Unit,
    onQuoteClick: (Quote) -> Unit,
    onSwipeReply: (ChatMessage) -> Unit,
    onLoadEarlier: () -> Unit,
    onOpenImage: (String, String) -> Unit,
    onOpenUrl: (String) -> Unit,
    onRetryPending: (String) -> Unit,
    onDismissPending: (String) -> Unit,
    onCancelOutbox: (String) -> Unit,
    onRetryOutbox: () -> Unit,
) {
    val listState = rememberLazyListState()
    val scope = rememberCoroutineScope()
    // Newest at the bottom: reverseLayout keeps the view pinned to the latest message.
    val items = remember(chat.messages, chat.firstUnreadId) { buildChatItems(chat.messages, chat.firstUnreadId) }
    val reversedItems = remember(items) { items.asReversed() }
    // The list reads each row's content type on every measure pass, so a row is classified once per list.
    val rowTypes = remember(reversedItems) { arrayOfNulls<String>(reversedItems.size) }
    val reversedPending = remember(chat.pending) { chat.pending.asReversed() }
    val reversedOutbox = remember(outboxEntries) { outboxEntries.asReversed() }
    val bottomOffset = (if (typing != null) 1 else 0) + reversedOutbox.size + reversedPending.size
    val isGroup = chat.kind == "group"
    val selectionMode = selectedIds.isNotEmpty()
    val unreadCount = remember(chat.messages, chat.firstUnreadId) {
        val start = chat.messages.indexOfFirst { it.id == chat.firstUnreadId }
        if (start < 0) 0 else chat.messages.drop(start).count { !it.isSelf }
    }

    val reducedMotion = LocalReducedMotion.current
    var highlightedMsgId by remember { mutableStateOf<String?>(null) }
    var highlightNonce by remember { mutableIntStateOf(0) }

    LaunchedEffect(jumpTarget) {
        val (id, nonce) = jumpTarget ?: return@LaunchedEffect
        val idx = reversedItems.indexOfFirst { it.containsId(id) }
        if (idx >= 0) {
            val listIndex = bottomOffset + idx
            if (reducedMotion) {
                listState.scrollToItem(listIndex)
            } else {
                listState.animateScrollToItem(listIndex)
            }
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
    // Follow new messages, queued outbox bubbles, optimistic pending bubbles, and "typing…" when already at the bottom.
    LaunchedEffect(chat.messages.lastOrNull()?.id, chat.pending.size, outboxEntries.size, typing != null) {
        if (listState.firstVisibleItemIndex <= 1) {
            if (reducedMotion) listState.scrollToItem(0) else listState.animateScrollToItem(0)
        }
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

    BoxWithConstraints(Modifier.fillMaxSize()) {
        val maxBubble = maxWidth * 0.78f
        LazyColumn(
            state = listState,
            reverseLayout = true,
            contentPadding = PaddingValues(horizontal = 8.dp, vertical = 12.dp),
            modifier = Modifier.fillMaxSize(),
        ) {
            if (typing != null) {
                item(key = "typing", contentType = "typing") { TypingBubble(isGroup, typing) }
            }
            items(
                reversedOutbox.size,
                key = { "outbox:${reversedOutbox[it].localId}" },
                contentType = { "outbox" },
            ) { i ->
                val entry = reversedOutbox[i]
                OutboxMessageRow(
                    entry = entry,
                    maxBubble = maxBubble,
                    onCancel = { onCancelOutbox(entry.localId) },
                    onRetry = onRetryOutbox,
                )
            }
            items(
                reversedPending.size,
                key = { "pending:${reversedPending[it].localId}" },
                contentType = { "pending" },
            ) { i ->
                val pending = reversedPending[i]
                PendingMessageRow(
                    pending = pending,
                    maxBubble = maxBubble,
                    onRetry = { onRetryPending(pending.localId) },
                    onDismiss = { onDismissPending(pending.localId) },
                )
            }
            items(
                reversedItems.size,
                key = { reversedItems[it].key },
                contentType = { i -> rowTypes[i] ?: chatItemContentType(reversedItems[i]).also { rowTypes[i] = it } },
            ) { i ->
                val item = reversedItems[i]
                val olderItem = reversedItems.getOrNull(i + 1)
                val newerItem = reversedItems.getOrNull(i - 1)
                Column {
                    if (olderItem == null || olderItem.newestDayKey != item.oldestDayKey) DayHeader(item.oldestTs)
                    if (item.firstUnreadAnchor && unreadCount > 0) UnreadDivider(unreadCount)
                    when (item) {
                        is ChatItem.CallGroup -> CallGroupRow(
                            calls = item.calls,
                            highlightedId = highlightedMsgId ?: searchFocusedId,
                            selectedIds = selectedIds,
                            selectionMode = selectionMode,
                            onToggleSelect = onToggleSelect,
                            onLongPressCall = onOpenMenu,
                        )
                        is ChatItem.Single -> {
                            val msg = item.msg
                            val isSelected = msg.id in selectedIds
                            val isSearchFocused = searchFocusedId == msg.id
                            when {
                                item.isCallLog -> CallSystemRow(
                                    msg = msg,
                                    highlighted = highlightedMsgId == msg.id || isSearchFocused,
                                    selected = isSelected,
                                    selectionMode = selectionMode,
                                    onClick = { if (selectionMode) onToggleSelect(msg) },
                                    onLongPress = { if (selectionMode) onToggleSelect(msg) else onOpenMenu(msg) },
                                )
                                item.systemLabel != null -> {
                                    // A poll update points at the poll: tapping the line opens it, as the bubble did.
                                    val pollId = msg.attachment?.poll?.takeIf { it.event != null }?.pollId
                                    var pollOpen by remember { mutableStateOf(false) }
                                    SystemInfoRow(
                                        label = item.systemLabel,
                                        ts = msg.ts,
                                        highlighted = highlightedMsgId == msg.id || isSearchFocused,
                                        selected = isSelected,
                                        selectionMode = selectionMode,
                                        icon = if (pollId != null) Icons.Outlined.Poll else Icons.Outlined.Info,
                                        onClick = {
                                            if (selectionMode) onToggleSelect(msg) else if (pollId != null) pollOpen = true
                                        },
                                        onLongPress = { if (selectionMode) onToggleSelect(msg) else onOpenMenu(msg) },
                                    )
                                    if (pollOpen && pollId != null) PollSheet(pollId, onDismiss = { pollOpen = false })
                                }
                                else -> {
                                    val olderSingle = (olderItem as? ChatItem.Single)?.takeIf { !it.isSystemLike }
                                    val newerSingle = (newerItem as? ChatItem.Single)?.takeIf { !it.isSystemLike }
                                    val older = olderSingle?.msg
                                    val newer = newerSingle?.msg
                                    val firstOfRun = older == null || older.fromId != msg.fromId || olderSingle.oldestDayKey != item.oldestDayKey ||
                                        msg.ts - older.ts > GROUP_GAP_MS || msg.id == chat.firstUnreadId
                                    val lastOfRun = newer == null || newer.fromId != msg.fromId || newerSingle.oldestDayKey != item.oldestDayKey ||
                                        newer.ts - msg.ts > GROUP_GAP_MS || newer.id == chat.firstUnreadId
                                    MessageRow(
                                        msg = msg,
                                        isGroup = isGroup,
                                        firstOfRun = firstOfRun,
                                        lastOfRun = lastOfRun,
                                        maxBubble = maxBubble,
                                        highlighted = highlightedMsgId == msg.id,
                                        searchFocused = isSearchFocused,
                                        selectionMode = selectionMode,
                                        selected = isSelected,
                                        onToggleSelect = { onToggleSelect(msg) },
                                        onLongPress = { if (selectionMode) onToggleSelect(msg) else onOpenMenu(msg) },
                                        onOpenReactions = { if (selectionMode) onToggleSelect(msg) else onOpenReactions(msg) },
                                        onQuoteClick = { quote ->
                                            if (selectionMode) onToggleSelect(msg) else onQuoteClick(quote)
                                        },
                                        onSwipeReply = { onSwipeReply(msg) },
                                        onOpenImage = { url ->
                                            if (selectionMode) onToggleSelect(msg) else onOpenImage(msg.id, url)
                                        },
                                        onOpenUrl = { url ->
                                            if (selectionMode) onToggleSelect(msg) else onOpenUrl(url)
                                        },
                                    )
                                }
                            }
                        }
                    }
                }
            }
            if (chat.loadingMore) {
                item(key = "loading", contentType = "loading") {
                    val loadingMod = Modifier.fillMaxWidth().padding(vertical = 8.dp)
                    if (reducedMotion) {
                        LinearProgressIndicator(loadingMod)
                    } else {
                        LinearWavyProgressIndicator(loadingMod)
                    }
                }
            }
        }

        FloatingDateOverlay(
            listState = listState,
            reversedItems = reversedItems,
            bottomOffset = bottomOffset,
            modifier = Modifier.align(Alignment.TopCenter).padding(top = 10.dp),
        )

        if (LocalUltraExpressive.current) {
            FourXBadge(
                size = 86.dp,
                modifier = Modifier.align(Alignment.TopEnd).padding(top = 10.dp, end = 12.dp),
            )
        }

        val ultra = LocalUltraExpressive.current
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
                        scope.launch {
                            if (reducedMotion) listState.scrollToItem(0) else listState.animateScrollToItem(0)
                        }
                    },
                    containerColor = MaterialTheme.colorScheme.surfaceContainerHighest,
                    contentColor = MaterialTheme.colorScheme.onSurface,
                    shape = CircleShape,
                    modifier = if (ultra) Modifier.size(76.dp) else Modifier,
                ) {
                    Icon(
                        Icons.Filled.KeyboardArrowDown,
                        contentDescription = "Jump to latest",
                        modifier = if (ultra) Modifier.size(40.dp) else Modifier,
                    )
                }
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

/**
 * Sticky date pill shown while scrolling through message history. Scoped to its own composable and keyed by
 * calendar [dayKey] so scrolling within the same day never recomposes [MessageList] or this pill.
 */
@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun FloatingDateOverlay(
    listState: LazyListState,
    reversedItems: List<ChatItem>,
    bottomOffset: Int,
    modifier: Modifier = Modifier,
) {
    val floatingDayKey by remember(listState, reversedItems, bottomOffset) {
        derivedStateOf {
            if (!listState.canScrollForward && !listState.canScrollBackward) return@derivedStateOf null
            val layout = listState.layoutInfo
            val topItem = layout.visibleItemsInfo
                .lastOrNull { it.index >= bottomOffset && (it.index - bottomOffset) < reversedItems.size }
                ?: return@derivedStateOf null
            val topIndex = topItem.index - bottomOffset
            val item = reversedItems.getOrNull(topIndex) ?: return@derivedStateOf null
            val older = reversedItems.getOrNull(topIndex + 1)
            val hasInlineDayHeader = older == null || older.newestDayKey != item.oldestDayKey
            // If this day's inline header is already visible at the top of the viewport, don't duplicate it with a pill.
            val inlineHeaderVisible = hasInlineDayHeader && (topItem.offset + topItem.size) <= layout.viewportEndOffset
            if (inlineHeaderVisible) null else item.oldestDayKey
        }
    }
    var showDatePill by remember { mutableStateOf(false) }
    var lastTopDayKey by remember { mutableStateOf<Long?>(null) }
    LaunchedEffect(floatingDayKey) {
        if (floatingDayKey != null) lastTopDayKey = floatingDayKey
    }
    LaunchedEffect(listState.isScrollInProgress, floatingDayKey) {
        if (listState.isScrollInProgress && floatingDayKey != null) {
            showDatePill = true
        } else if (showDatePill) {
            delay(800)
            showDatePill = false
        }
    }
    val displayDayKey = floatingDayKey ?: lastTopDayKey
    AnimatedVisibility(
        visible = showDatePill && floatingDayKey != null && displayDayKey != null,
        enter = fadeIn(MaterialTheme.motionScheme.fastEffectsSpec()),
        exit = fadeOut(MaterialTheme.motionScheme.fastEffectsSpec()),
        modifier = modifier,
    ) {
        displayDayKey?.let { key ->
            Surface(
                shape = CircleShape,
                color = MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = 0.94f),
                contentColor = MaterialTheme.colorScheme.onSurface,
                shadowElevation = 3.dp,
            ) {
                Text(
                    formatDayKey(key),
                    style = MaterialTheme.typography.labelMedium,
                    fontWeight = FontWeight.SemiBold,
                    modifier = Modifier.padding(horizontal = 14.dp, vertical = 6.dp),
                )
            }
        }
    }
}

/** Optimistic outgoing bubble while in-flight (clock icon) or failed (error tint + tap-to-retry + dismiss). */
@Composable
private fun PendingMessageRow(
    pending: PendingMessage,
    maxBubble: Dp,
    onRetry: () -> Unit,
    onDismiss: () -> Unit,
) {
    val colors = LocalChatColors.current
    val shape = RoundedCornerShape(20.dp)
    val failed = pending.failed
    val container = if (failed) {
        MaterialTheme.colorScheme.errorContainer
    } else {
        colors.outgoing.copy(alpha = 0.72f)
    }
    val content = if (failed) {
        MaterialTheme.colorScheme.onErrorContainer
    } else {
        colors.onOutgoing.copy(alpha = 0.92f)
    }
    val meta = content.copy(alpha = 0.75f)
    val border = if (failed) BorderStroke(1.dp, MaterialTheme.colorScheme.error.copy(alpha = 0.55f)) else null

    Column(
        modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
        horizontalAlignment = Alignment.End,
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.End,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (failed) {
                IconButton(
                    onClick = onDismiss,
                    modifier = Modifier.size(32.dp).padding(end = 4.dp),
                ) {
                    Icon(
                        Icons.Outlined.Close,
                        contentDescription = "Dismiss failed message",
                        tint = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.size(18.dp),
                    )
                }
            }
            Surface(
                shape = shape,
                color = container,
                contentColor = content,
                border = border,
                modifier = Modifier
                    .widthIn(max = maxBubble)
                    .clip(shape)
                    .then(if (failed) Modifier.clickable(onClick = onRetry) else Modifier),
            ) {
                Column(Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
                    pending.quote?.let { q ->
                        QuoteBlock(q.fromName, q.text, onOutgoing = !failed, onClick = {})
                    }
                    TextWithFooter(
                        text = AnnotatedString(pending.text),
                        style = MaterialTheme.typography.bodyLarge,
                        color = content,
                        footer = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(formatClock(pending.ts), style = MaterialTheme.typography.labelSmall, color = meta)
                                Spacer(Modifier.width(4.dp))
                                Icon(
                                    imageVector = if (failed) Icons.Outlined.ErrorOutline else Icons.Outlined.Schedule,
                                    contentDescription = if (failed) "Failed to send" else "Sending",
                                    tint = if (failed) MaterialTheme.colorScheme.error else meta,
                                    modifier = Modifier.size(14.dp),
                                )
                            }
                        },
                    )
                }
            }
        }
        if (failed) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(4.dp),
                modifier = Modifier
                    .padding(top = 3.dp, end = 6.dp)
                    .clip(RoundedCornerShape(8.dp))
                    .clickable(onClick = onRetry)
                    .padding(horizontal = 6.dp, vertical = 2.dp),
            ) {
                Icon(
                    Icons.Outlined.ErrorOutline,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.error,
                    modifier = Modifier.size(13.dp),
                )
                Text(
                    text = "Tap to retry",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.error,
                    fontWeight = FontWeight.Medium,
                )
            }
        }
    }
}

/** Outgoing bubble queued in [OfflineOutbox] while offline, with tap-to-cancel and retry. */
@Composable
private fun OutboxMessageRow(
    entry: OutboxEntry,
    maxBubble: Dp,
    onCancel: () -> Unit,
    onRetry: () -> Unit,
) {
    val colors = LocalChatColors.current
    val shape = RoundedCornerShape(20.dp)
    val container = colors.outgoing.copy(alpha = 0.62f)
    val content = colors.onOutgoing.copy(alpha = 0.90f)
    val meta = content.copy(alpha = 0.78f)
    val hasError = !entry.lastError.isNullOrBlank()

    Column(
        modifier = Modifier.fillMaxWidth().padding(top = 4.dp),
        horizontalAlignment = Alignment.End,
    ) {
        Row(
            modifier = Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.End,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            IconButton(
                onClick = onCancel,
                modifier = Modifier.size(32.dp).padding(end = 4.dp),
            ) {
                Icon(
                    Icons.Outlined.Close,
                    contentDescription = "Cancel queued message",
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(18.dp),
                )
            }
            Surface(
                shape = shape,
                color = container,
                contentColor = content,
                border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.6f)),
                modifier = Modifier
                    .widthIn(max = maxBubble)
                    .clip(shape)
                    .clickable(onClick = onCancel),
            ) {
                Column(Modifier.padding(horizontal = 12.dp, vertical = 8.dp)) {
                    TextWithFooter(
                        text = AnnotatedString(entry.text),
                        style = MaterialTheme.typography.bodyLarge,
                        color = content,
                        footer = {
                            Row(verticalAlignment = Alignment.CenterVertically) {
                                Text(formatClock(entry.createdAt), style = MaterialTheme.typography.labelSmall, color = meta)
                                Spacer(Modifier.width(4.dp))
                                Icon(
                                    imageVector = Icons.Outlined.CloudOff,
                                    contentDescription = "Queued offline",
                                    tint = meta,
                                    modifier = Modifier.size(14.dp),
                                )
                            }
                        },
                    )
                }
            }
        }
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(4.dp),
            modifier = Modifier
                .padding(top = 3.dp, end = 6.dp)
                .clip(RoundedCornerShape(8.dp))
                .clickable(onClick = if (hasError) onRetry else onCancel)
                .padding(horizontal = 6.dp, vertical = 2.dp),
        ) {
            Icon(
                imageVector = if (hasError) Icons.Outlined.ErrorOutline else Icons.Outlined.Schedule,
                contentDescription = null,
                tint = if (hasError) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.size(13.dp),
            )
            Text(
                text = if (hasError) "Queued · tap to retry" else "Queued · sends when online",
                style = MaterialTheme.typography.labelSmall,
                color = if (hasError) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                fontWeight = FontWeight.Medium,
            )
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

internal const val CALL_GROUP_WINDOW_MS = 5 * 60_000L

internal sealed interface ChatItem {
    val key: String
    val oldestTs: Long
    val newestTs: Long
    val oldestDayKey: Long
    val newestDayKey: Long
    val firstUnreadAnchor: Boolean
    fun containsId(id: String): Boolean

    data class Single(
        val msg: ChatMessage,
        override val firstUnreadAnchor: Boolean,
        val isCallLog: Boolean = isCallLogMessage(msg),
        val systemLabel: String? = if (isCallLog) null else systemMessageLabel(msg),
        override val oldestDayKey: Long = dayKey(msg.ts),
    ) : ChatItem {
        override val key: String get() = msg.id
        override val oldestTs: Long get() = msg.ts
        override val newestTs: Long get() = msg.ts
        override val newestDayKey: Long get() = oldestDayKey
        val isSystemLike: Boolean get() = isCallLog || systemLabel != null
        override fun containsId(id: String): Boolean = msg.id == id
    }

    /** Two or more consecutive call log entries within [CALL_GROUP_WINDOW_MS] on the same day. */
    data class CallGroup(
        val calls: List<ChatMessage>,
        override val firstUnreadAnchor: Boolean,
        override val oldestDayKey: Long = dayKey(calls.first().ts),
        override val newestDayKey: Long = dayKey(calls.last().ts),
    ) : ChatItem {
        override val key: String get() = "call-group:${calls.first().id}"
        override val oldestTs: Long get() = calls.first().ts
        override val newestTs: Long get() = calls.last().ts
        override fun containsId(id: String): Boolean = calls.any { it.id == id }
    }
}

private fun isCallLogMessage(msg: ChatMessage): Boolean = !msg.recalled && msg.attachment?.kind == "call"

/**
 * Lazy-list content type of a chat row. It follows the same branches the row renders (call group, call log row,
 * system info row, message bubble), so a recycled row is only reused for a row of the same kind.
 */
private fun chatItemContentType(item: ChatItem): String = when (item) {
    is ChatItem.CallGroup -> "call-group"
    is ChatItem.Single -> when {
        item.isCallLog -> "call"
        item.systemLabel != null -> "system"
        else -> "message"
    }
}

/** Groups consecutive same-day call log entries within [windowMs] without crossing [firstUnreadId]. */
internal fun buildChatItems(
    messages: List<ChatMessage>,
    firstUnreadId: String?,
    windowMs: Long = CALL_GROUP_WINDOW_MS,
): List<ChatItem> {
    if (messages.isEmpty()) return emptyList()
    val out = ArrayList<ChatItem>(messages.size)
    var i = 0
    while (i < messages.size) {
        val m = messages[i]
        val mDayKey = dayKey(m.ts)
        if (!isCallLogMessage(m)) {
            out += ChatItem.Single(
                msg = m,
                firstUnreadAnchor = m.id == firstUnreadId,
                isCallLog = false,
                systemLabel = systemMessageLabel(m),
                oldestDayKey = mDayKey,
            )
            i++
            continue
        }
        val group = mutableListOf(m)
        var j = i + 1
        while (j < messages.size) {
            val next = messages[j]
            if (!isCallLogMessage(next)) break
            if (next.id == firstUnreadId) break
            if (dayKey(next.ts) != mDayKey) break
            if (next.ts - group.last().ts > windowMs) break
            group += next
            j++
        }
        val anchor = m.id == firstUnreadId
        out += if (group.size >= 2) {
            ChatItem.CallGroup(group, firstUnreadAnchor = anchor, oldestDayKey = mDayKey, newestDayKey = mDayKey)
        } else {
            ChatItem.Single(
                msg = m,
                firstUnreadAnchor = anchor,
                isCallLog = true,
                systemLabel = null,
                oldestDayKey = mDayKey,
            )
        }
        i = j
    }
    return out
}

/** Compact, centered single-line system row for a call log event (no nested card or message bubble). */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun CallSystemRow(
    msg: ChatMessage,
    highlighted: Boolean,
    selected: Boolean = false,
    selectionMode: Boolean = false,
    onClick: () -> Unit = {},
    onLongPress: () -> Unit,
) {
    val a = msg.attachment ?: return
    val call = a.call
    val missed = call?.missed == true
    val icon = when {
        call?.video == true && missed -> Icons.AutoMirrored.Outlined.MissedVideoCall
        call?.video == true -> Icons.Outlined.Videocam
        missed -> Icons.AutoMirrored.Outlined.PhoneMissed
        else -> Icons.Outlined.Call
    }
    val tint = if (missed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant
    val label = a.title ?: if (call?.video == true) "Video call" else "Voice call"
    val duration = a.duration?.takeIf { it > 0 }?.let(::formatDuration)
    val summary = buildList {
        add(label)
        if (duration != null) add(duration)
        add(formatClock(msg.ts))
    }.joinToString(" · ")
    val highlightColor by animateColorAsState(
        targetValue = when {
            selected -> MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.45f)
            highlighted -> MaterialTheme.colorScheme.primary.copy(alpha = 0.22f)
            else -> Color.Transparent
        },
        label = "callHighlight",
    )
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .background(highlightColor, RoundedCornerShape(12.dp))
            .then(if (selectionMode) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(vertical = 4.dp, horizontal = 16.dp),
        contentAlignment = Alignment.Center,
    ) {
        if (selectionMode) {
            SelectionCheckIndicator(
                selected = selected,
                modifier = Modifier.align(Alignment.CenterStart),
            )
        }
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
            modifier = Modifier
                .clip(CircleShape)
                .combinedClickable(onClick = onClick, onLongClick = onLongPress)
                .padding(horizontal = 12.dp, vertical = 4.dp),
        ) {
            Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(16.dp))
            Text(
                text = summary,
                style = MaterialTheme.typography.labelMedium,
                color = tint,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

/** Compact centered summary row for consecutive call events within 5 minutes; expands on tap. */
@Composable
private fun CallGroupRow(
    calls: List<ChatMessage>,
    highlightedId: String?,
    selectedIds: Set<String> = emptySet(),
    selectionMode: Boolean = false,
    onToggleSelect: (ChatMessage) -> Unit = {},
    onLongPressCall: (ChatMessage) -> Unit,
) {
    val containsHighlight = highlightedId != null && calls.any { it.id == highlightedId }
    val containsSelected = selectedIds.isNotEmpty() && calls.any { it.id in selectedIds }
    var expanded by rememberSaveable(calls.first().id) { mutableStateOf(false) }
    LaunchedEffect(containsHighlight, containsSelected) {
        if (containsHighlight || containsSelected) expanded = true
    }
    val anyMissed = calls.any { it.attachment?.call?.missed == true }
    val allVideo = calls.all { it.attachment?.call?.video == true }
    val icon = when {
        allVideo && anyMissed -> Icons.AutoMirrored.Outlined.MissedVideoCall
        allVideo -> Icons.Outlined.Videocam
        anyMissed -> Icons.AutoMirrored.Outlined.PhoneMissed
        else -> Icons.Outlined.Call
    }
    val tint = if (anyMissed) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant
    val timeRange = formatCallRange(calls.first().ts, calls.last().ts)
    Column(
        modifier = Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp, horizontal = 16.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Surface(
            shape = CircleShape,
            color = MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = 0.65f),
            modifier = Modifier
                .clip(CircleShape)
                .clickable { expanded = !expanded },
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 5.dp),
            ) {
                Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(16.dp))
                Text(
                    text = "${calls.size} calls · $timeRange",
                    style = MaterialTheme.typography.labelMedium,
                    fontWeight = FontWeight.Medium,
                    color = tint,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                Icon(
                    if (expanded) Icons.Filled.KeyboardArrowUp else Icons.Filled.KeyboardArrowDown,
                    contentDescription = if (expanded) "Collapse calls" else "Expand calls",
                    tint = tint,
                    modifier = Modifier.size(16.dp),
                )
            }
        }
        AnimatedVisibility(visible = expanded) {
            Column(
                modifier = Modifier.fillMaxWidth().padding(top = 2.dp),
                horizontalAlignment = Alignment.CenterHorizontally,
            ) {
                calls.forEach { callMsg ->
                    CallSystemRow(
                        msg = callMsg,
                        highlighted = highlightedId == callMsg.id,
                        selected = callMsg.id in selectedIds,
                        selectionMode = selectionMode,
                        onClick = { if (selectionMode) onToggleSelect(callMsg) },
                        onLongPress = { if (selectionMode) onToggleSelect(callMsg) else onLongPressCall(callMsg) },
                    )
                }
            }
        }
    }
}

/** Distinct centered system/service info row (e.g. post-call audio feedback, connection notices). */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun SystemInfoRow(
    label: String,
    ts: Long,
    highlighted: Boolean,
    selected: Boolean = false,
    selectionMode: Boolean = false,
    icon: ImageVector = Icons.Outlined.Info,
    onClick: () -> Unit = {},
    onLongPress: () -> Unit,
) {
    val highlightColor by animateColorAsState(
        targetValue = when {
            selected -> MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.45f)
            highlighted -> MaterialTheme.colorScheme.primary.copy(alpha = 0.22f)
            else -> Color.Transparent
        },
        label = "systemHighlight",
    )
    Box(
        modifier = Modifier
            .fillMaxWidth()
            .background(highlightColor, RoundedCornerShape(12.dp))
            .then(if (selectionMode) Modifier.clickable(onClick = onClick) else Modifier)
            .padding(vertical = 4.dp, horizontal = 20.dp),
        contentAlignment = Alignment.Center,
    ) {
        if (selectionMode) {
            SelectionCheckIndicator(
                selected = selected,
                modifier = Modifier.align(Alignment.CenterStart),
            )
        }
        Surface(
            shape = CircleShape,
            color = MaterialTheme.colorScheme.surfaceContainerHigh.copy(alpha = 0.55f),
            modifier = Modifier
                .clip(CircleShape)
                .combinedClickable(onClick = onClick, onLongClick = onLongPress),
        ) {
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                modifier = Modifier.padding(horizontal = 12.dp, vertical = 5.dp),
            ) {
                Icon(
                    icon,
                    contentDescription = null,
                    tint = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.size(14.dp),
                )
                Text(
                    text = "$label · ${formatClock(ts)}",
                    style = MaterialTheme.typography.labelMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@Composable
private fun SelectionCheckIndicator(selected: Boolean, modifier: Modifier = Modifier) {
    Icon(
        imageVector = if (selected) Icons.Filled.CheckCircle else Icons.Outlined.RadioButtonUnchecked,
        contentDescription = if (selected) "Selected" else "Not selected",
        tint = if (selected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.outline,
        modifier = modifier.size(22.dp),
    )
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
    searchFocused: Boolean = false,
    selectionMode: Boolean = false,
    selected: Boolean = false,
    onToggleSelect: () -> Unit = {},
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
        targetValue = when {
            selected -> MaterialTheme.colorScheme.primaryContainer.copy(alpha = 0.45f)
            searchFocused -> MaterialTheme.colorScheme.primary.copy(alpha = 0.18f)
            highlighted -> MaterialTheme.colorScheme.primary.copy(alpha = 0.24f)
            else -> Color.Transparent
        },
        animationSpec = tween(durationMillis = if (highlighted || selected || searchFocused) 160 else 640),
        label = "bubbleHighlight",
    )

    Box(
        Modifier
            .fillMaxWidth()
            .padding(top = if (firstOfRun) 8.dp else 2.dp)
            .clip(RoundedCornerShape(20.dp))
            .background(highlightColor)
            .then(
                if (searchFocused) {
                    Modifier.border(
                        width = 1.5.dp,
                        color = MaterialTheme.colorScheme.primary.copy(alpha = 0.65f),
                        shape = RoundedCornerShape(20.dp),
                    )
                } else {
                    Modifier
                },
            )
            .then(if (selectionMode) Modifier.clickable(onClick = onToggleSelect) else Modifier),
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
                .pointerInput(msg.id, msg.recalled, selectionMode) {
                    if (msg.recalled || selectionMode) {
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
            verticalAlignment = Alignment.CenterVertically,
        ) {
            if (selectionMode) {
                SelectionCheckIndicator(
                    selected = selected,
                    modifier = Modifier.padding(start = 4.dp, end = 8.dp),
                )
            }
            Row(
                modifier = Modifier.weight(1f),
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
                        StickerBubble(
                            msg = msg,
                            isGroup = isGroup,
                            firstOfRun = firstOfRun,
                            onClick = { if (selectionMode) onToggleSelect() },
                            onLongPress = onLongPress,
                        )
                    } else {
                        Bubble(
                            msg = msg,
                            isGroup = isGroup,
                            firstOfRun = firstOfRun,
                            lastOfRun = lastOfRun,
                            onClick = { if (selectionMode) onToggleSelect() },
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
}

@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun Bubble(
    msg: ChatMessage,
    isGroup: Boolean,
    firstOfRun: Boolean,
    lastOfRun: Boolean,
    onClick: () -> Unit = {},
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
    val isFramelessMedia = attachment != null && attachment.hasRenderableMedia()

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
        modifier = Modifier.clip(shape).combinedClickable(onClick = onClick, onLongClick = onLongPress),
    ) {
        if (isFramelessMedia && attachment != null) {
            val caption = attachment.caption()
            val showHeader = (isGroup && !self && firstOfRun && msg.fromName.isNotBlank()) || msg.quote != null
            Column(Modifier.widthIn(max = 280.dp)) {
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
                    if (!self) {
                        val otp = remember(msg.otp, msg.text) { msg.otp ?: extractOtpCode(msg.text) }
                        if (otp != null) {
                            Spacer(Modifier.height(8.dp))
                            OtpCopyChip(otp = otp)
                        }
                    }
                }
            }
        }
    }
}

/** A sticker stands on its own, without a bubble; the time sits under it. */
@OptIn(ExperimentalFoundationApi::class)
@Composable
private fun StickerBubble(
    msg: ChatMessage,
    isGroup: Boolean,
    firstOfRun: Boolean,
    onClick: () -> Unit = {},
    onLongPress: () -> Unit,
) {
    Column(
        horizontalAlignment = if (msg.isSelf) Alignment.End else Alignment.Start,
        modifier = Modifier.clip(RoundedCornerShape(12.dp)).combinedClickable(onClick = onClick, onLongClick = onLongPress),
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
    val currentOnOpenUrl by rememberUpdatedState(onOpenUrl)
    val text = bubbleText(msg)
    val attachmentLabel = msg.attachment?.let { a -> a.title?.takeIf { it.isNotBlank() } ?: a.href ?: "Attachment" }
    return remember(text, msg.mentions, attachmentLabel, linkColor) {
        val urls = extractUrls(text)
        val appendAttachment = attachmentLabel != null && text != attachmentLabel && !text.endsWith(attachmentLabel)
        if (msg.mentions.isEmpty() && urls.isEmpty() && !appendAttachment) {
            AnnotatedString(text)
        } else {
            buildAnnotatedString {
                append(text)
                for (m in msg.mentions) {
                    val start = m.pos.coerceIn(0, text.length)
                    val end = (m.pos + m.len).coerceIn(start, text.length)
                    if (end > start) addStyle(SpanStyle(fontWeight = FontWeight.Bold), start, end)
                }
                if (urls.isNotEmpty()) {
                    val linkStyle = TextLinkStyles(SpanStyle(color = linkColor, textDecoration = TextDecoration.Underline))
                    for (range in urls) {
                        val url = text.substring(range)
                        addLink(
                            LinkAnnotation.Url(
                                url = url,
                                styles = linkStyle,
                                linkInteractionListener = LinkInteractionListener { currentOnOpenUrl(url) },
                            ),
                            start = range.first,
                            end = range.last + 1,
                        )
                    }
                }
                if (appendAttachment) {
                    if (text.isNotBlank()) append("\n")
                    append("📎 $attachmentLabel")
                }
            }
        }
    }
}

@Composable
private fun linkifiedAnnotatedString(text: String, onOutgoing: Boolean, onOpenUrl: (String) -> Unit): AnnotatedString {
    if (text.isEmpty()) return AnnotatedString("")
    val linkColor = if (onOutgoing) LocalChatColors.current.onOutgoing else MaterialTheme.colorScheme.primary
    val currentOnOpenUrl by rememberUpdatedState(onOpenUrl)
    return remember(text, linkColor) {
        val urls = extractUrls(text)
        if (urls.isEmpty()) {
            AnnotatedString(text)
        } else {
            buildAnnotatedString {
                append(text)
                val linkStyle = TextLinkStyles(SpanStyle(color = linkColor, textDecoration = TextDecoration.Underline))
                for (range in urls) {
                    val url = text.substring(range)
                    addLink(
                        LinkAnnotation.Url(
                            url = url,
                            styles = linkStyle,
                            linkInteractionListener = LinkInteractionListener { currentOnOpenUrl(url) },
                        ),
                        start = range.first,
                        end = range.last + 1,
                    )
                }
            }
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
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Column(Modifier.fillMaxWidth().navigationBarsPadding().padding(bottom = 24.dp)) {
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
                    ToolbarActionItem("Forward", Icons.AutoMirrored.Outlined.Forward) { onAction(MessageAction.FORWARD) }
                    if (isGroup && msg.text.isNotBlank()) {
                        ToolbarActionItem("Pin note", Icons.Outlined.PushPin) { onAction(MessageAction.PIN_NOTE) }
                    }
                }
                ToolbarActionItem("Select", Icons.Outlined.CheckCircle) { onAction(MessageAction.SELECT) }
                if (!msg.recalled && msg.isSelf) {
                    ToolbarActionItem("Recall", Icons.AutoMirrored.Outlined.Undo, destructive = true) { onAction(MessageAction.RECALL) }
                }
                ToolbarActionItem("Delete for me", Icons.Outlined.DeleteOutline, destructive = true) { onAction(MessageAction.DELETE) }
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
            .padding(horizontal = 8.dp, vertical = 6.dp),
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

/** Message input: rounded field, emoji/sticker drawer, attachment sheet, hold-to-record voice, and keyboard shortcuts. */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun Composer(
    threadKey: String,
    initialDraft: String,
    onDraftChange: (String) -> Unit,
    isGroup: Boolean,
    members: List<GroupMember>?,
    sending: Boolean,
    uploading: Int,
    stickers: StickerSearch,
    disappearingTtlMs: Long?,
    onOpenDisappearingTimer: () -> Unit,
    onOpenShareContact: () -> Unit,
    onTakePhoto: () -> Unit,
    onPickMedia: () -> Unit,
    onPickFiles: () -> Unit,
    onPickSticker: () -> Unit,
    onSearchStickers: (String) -> Unit,
    onSendSticker: (StickerInfo) -> Unit,
    onSendVoice: (File) -> Unit,
    replyTo: ChatMessage?,
    onCancelReply: () -> Unit,
    onStartTyping: () -> Unit,
    onLoadMembers: () -> Unit,
    onSend: (String, List<GroupMember>, onSent: () -> Unit) -> Unit,
) {
    val context = LocalContext.current
    val colors = LocalChatColors.current
    val haptics = LocalHapticFeedback.current
    val recorderScope = rememberCoroutineScope()
    var value by remember(threadKey) {
        mutableStateOf(TextFieldValue(initialDraft, TextRange(initialDraft.length)))
    }
    val mentioned = remember(threadKey) { mutableStateListOf<GroupMember>() }
    var showEmoji by rememberSaveable { mutableStateOf(false) }
    var attachSheet by remember { mutableStateOf(false) }
    val keyboard = LocalSoftwareKeyboardController.current
    val focus = remember { FocusRequester() }
    val density = LocalDensity.current
    val ime = WindowInsets.ime
    val imeVisible by remember(ime, density) { derivedStateOf { ime.getBottom(density) > 0 } }

    val voiceRecorder = remember(context) { VoiceRecorder(context) }
    val isRecording by voiceRecorder.isRecording.collectAsStateWithLifecycle()
    val recordingElapsedMs by voiceRecorder.elapsedMs.collectAsStateWithLifecycle()
    val recordingAmplitudes by voiceRecorder.amplitudes.collectAsStateWithLifecycle()
    var recordingLocked by remember(threadKey) { mutableStateOf(false) }
    var slideOffsetX by remember { mutableFloatStateOf(0f) }
    var slideOffsetY by remember { mutableFloatStateOf(0f) }

    DisposableEffect(Unit) {
        onDispose { voiceRecorder.cancel() }
    }
    DisposableEffect(threadKey) {
        onDispose {
            voiceRecorder.cancel()
            recordingLocked = false
        }
    }
    LaunchedEffect(isRecording) {
        if (!isRecording) {
            recordingLocked = false
            slideOffsetX = 0f
            slideOffsetY = 0f
        }
    }
    BackHandler(enabled = isRecording && recordingLocked) {
        voiceRecorder.cancel()
        recordingLocked = false
    }

    val recordAudioPermission = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) { granted ->
        if (!granted) {
            Toast.makeText(context, "Microphone permission is needed for voice messages", Toast.LENGTH_SHORT).show()
        }
    }
    val currentOnSendVoice by rememberUpdatedState(onSendVoice)
    val currentRequestAudioPerm by rememberUpdatedState {
        recordAudioPermission.launch(Manifest.permission.RECORD_AUDIO)
    }

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

    fun updateValue(newValue: TextFieldValue) {
        if (newValue.text != value.text) {
            onDraftChange(newValue.text)
        }
        value = newValue
    }

    fun replaceSelection(insert: String) {
        val start = value.selection.min
        updateValue(TextFieldValue(value.text.replaceRange(start, value.selection.max, insert), TextRange(start + insert.length)))
    }

    fun insertMention(member: GroupMember) {
        val (start, _) = mentionQuery ?: return
        val end = value.selection.start
        val tag = "@${member.name} "
        updateValue(TextFieldValue(value.text.replaceRange(start, end, tag), TextRange(start + tag.length)))
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
        updateValue(TextFieldValue(value.text.removeRange(start, cursor), TextRange(start)))
    }

    fun sendCurrentText() {
        if (sending || value.text.isBlank()) return
        haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
        val sentText = value.text
        onSend(sentText, mentioned.toList()) {
            if (value.text == sentText) {
                value = TextFieldValue("")
                mentioned.clear()
                onDraftChange("")
            }
        }
    }

    var showCreatePoll by remember { mutableStateOf(false) }
    if (showCreatePoll) CreatePollDialog(onDismiss = { showCreatePoll = false })
    val canCreatePoll = isGroup && LocalPolls.current != null
    val cancelThresholdPx = with(density) { 96.dp.toPx() }
    val lockThresholdPx = with(density) { 72.dp.toPx() }

    Column(Modifier.background(colors.background).navigationBarsPadding().imePadding()) {
        if (uploading > 0) UploadingNoteHost(threadKey, uploading)
        if (!isRecording && disappearingTtlMs != null && disappearingTtlMs > 0L) {
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 12.dp, vertical = 2.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                DisappearingTimerChip(
                    ttlMs = disappearingTtlMs,
                    onClick = onOpenDisappearingTimer,
                )
            }
        }
        if (isRecording && !recordingLocked) {
            val lockProgress = (-slideOffsetY / lockThresholdPx).coerceIn(0f, 1f)
            Box(
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(end = 12.dp, bottom = 4.dp),
                contentAlignment = Alignment.CenterEnd,
            ) {
                Surface(
                    shape = CircleShape,
                    color = MaterialTheme.colorScheme.surfaceContainerHigh,
                    shadowElevation = 2.dp,
                    modifier = Modifier.graphicsLayer {
                        translationY = (slideOffsetY * 0.35f).coerceAtLeast(-28f)
                        val s = 1f + 0.12f * lockProgress
                        scaleX = s
                        scaleY = s
                    },
                ) {
                    Column(
                        horizontalAlignment = Alignment.CenterHorizontally,
                        modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
                    ) {
                        Icon(
                            Icons.Outlined.Lock,
                            contentDescription = "Slide up to lock recording",
                            tint = if (lockProgress > 0.7f) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.size(18.dp),
                        )
                        Icon(
                            Icons.Filled.KeyboardArrowUp,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.size(14.dp),
                        )
                    }
                }
            }
        }
        if (!isRecording && suggestions.isNotEmpty()) {
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
        val ultra = LocalUltraExpressive.current
        val buttonSize = if (ultra) 116.dp else 48.dp
        val showSendButton = value.text.isNotBlank() || sending || (isRecording && recordingLocked)

        Row(
            Modifier.padding(start = 8.dp, end = 8.dp, top = 4.dp, bottom = 8.dp),
            verticalAlignment = Alignment.Bottom,
        ) {
            if (isRecording) {
                if (recordingLocked) {
                    IconButton(
                        onClick = {
                            voiceRecorder.cancel()
                            recordingLocked = false
                        },
                        modifier = Modifier.padding(end = 4.dp).size(48.dp),
                    ) {
                        Icon(
                            Icons.Outlined.DeleteOutline,
                            contentDescription = "Cancel recording",
                            tint = MaterialTheme.colorScheme.error,
                        )
                    }
                }
                VoiceRecordingBar(
                    elapsedMs = recordingElapsedMs,
                    amplitudes = recordingAmplitudes,
                    locked = recordingLocked,
                    slideCancelProgress = (-slideOffsetX / cancelThresholdPx).coerceIn(0f, 1f),
                    slideOffsetX = slideOffsetX,
                    modifier = Modifier.weight(1f),
                )
            } else {
                FilledIconButton(
                    onClick = {
                        showEmoji = false
                        keyboard?.hide()
                        attachSheet = true
                    },
                    colors = IconButtonDefaults.filledIconButtonColors(
                        containerColor = colors.outgoing,
                        contentColor = colors.onOutgoing,
                    ),
                    modifier = Modifier.padding(end = 8.dp).size(if (ultra) 88.dp else 48.dp),
                ) {
                    Icon(Icons.Rounded.Add, contentDescription = "Attach", modifier = Modifier.size(if (ultra) 46.dp else 26.dp))
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
                                    updateValue(it)
                                },
                                textStyle = MaterialTheme.typography.bodyLarge.copy(color = MaterialTheme.colorScheme.onSurface),
                                cursorBrush = SolidColor(MaterialTheme.colorScheme.primary),
                                keyboardOptions = KeyboardOptions(capitalization = KeyboardCapitalization.Sentences),
                                maxLines = 6,
                                modifier = Modifier
                                    .weight(1f)
                                    .padding(top = 13.dp, bottom = 13.dp, end = 16.dp)
                                    .focusRequester(focus)
                                    .onPreviewKeyEvent { event ->
                                        val isPlainEnter = event.type == KeyEventType.KeyDown &&
                                            (event.key == Key.Enter || event.key == Key.NumPadEnter) &&
                                            !event.isShiftPressed &&
                                            !event.isAltPressed &&
                                            !event.isCtrlPressed
                                        if (isPlainEnter && !sending && value.text.isNotBlank()) {
                                            sendCurrentText()
                                            true
                                        } else {
                                            false
                                        }
                                    },
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
            }

            if (showSendButton) {
                val isVoiceSend = isRecording && recordingLocked
                FilledIconButton(
                    onClick = {
                        if (isVoiceSend) {
                            haptics.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                            recordingLocked = false
                            val file = voiceRecorder.stop()
                            if (file != null) currentOnSendVoice(file)
                        } else {
                            sendCurrentText()
                        }
                    },
                    enabled = isVoiceSend || (!sending && value.text.isNotBlank()),
                    colors = IconButtonDefaults.filledIconButtonColors(
                        containerColor = if (ultra) Color(0xFFFF9EA8) else colors.outgoing,
                        contentColor = if (ultra) Color(0xFF1F1114) else colors.onOutgoing,
                        disabledContainerColor = if (ultra) Color(0xFFFF9EA8).copy(alpha = 0.72f) else colors.outgoing.copy(alpha = 0.38f),
                        disabledContentColor = if (ultra) Color(0xFF1F1114).copy(alpha = 0.72f) else colors.onOutgoing.copy(alpha = 0.38f),
                    ),
                    modifier = Modifier.padding(start = 8.dp).size(buttonSize),
                ) {
                    Icon(
                        Icons.AutoMirrored.Filled.Send,
                        contentDescription = if (isVoiceSend) "Send voice message" else "Send",
                        modifier = Modifier.size(if (ultra) 56.dp else 24.dp),
                    )
                }
            } else {
                // Keep mounted through the entire press-hold-drag-release gesture so pointerInput is never cancelled.
                val micScale by animateFloatAsState(
                    targetValue = if (isRecording) 1.18f else 1f,
                    animationSpec = MaterialTheme.motionScheme.fastSpatialSpec(),
                    label = "micHoldScale",
                )
                Surface(
                    shape = CircleShape,
                    color = if (isRecording) MaterialTheme.colorScheme.error else if (ultra) Color(0xFFFF9EA8) else colors.outgoing,
                    contentColor = if (isRecording) MaterialTheme.colorScheme.onError else if (ultra) Color(0xFF1F1114) else colors.onOutgoing,
                    shadowElevation = if (isRecording) 6.dp else 0.dp,
                    modifier = Modifier
                        .padding(start = 8.dp)
                        .size(buttonSize)
                        .systemGestureExclusion()
                        .graphicsLayer {
                            scaleX = micScale
                            scaleY = micScale
                            if (isRecording) {
                                translationX = slideOffsetX.coerceIn(-cancelThresholdPx * 0.45f, 0f)
                                translationY = slideOffsetY.coerceIn(-lockThresholdPx * 0.35f, 0f)
                            }
                        }
                        .pointerInput(voiceRecorder) {
                            awaitEachGesture {
                                val down = awaitFirstDown(requireUnconsumed = false)
                                val hasAudio = ContextCompat.checkSelfPermission(
                                    context,
                                    Manifest.permission.RECORD_AUDIO,
                                ) == PackageManager.PERMISSION_GRANTED
                                if (!hasAudio) {
                                    currentRequestAudioPerm()
                                    do {
                                        val ev = awaitPointerEvent()
                                    } while (ev.changes.any { it.pressed })
                                    return@awaitEachGesture
                                }

                                VoicePlayback.stop()
                                val started = voiceRecorder.start(recorderScope)
                                if (!started) {
                                    Toast.makeText(context, "Couldn't start recording", Toast.LENGTH_SHORT).show()
                                    return@awaitEachGesture
                                }
                                haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                                recordingLocked = false
                                slideOffsetX = 0f
                                slideOffsetY = 0f

                                var totalDx = 0f
                                var totalDy = 0f
                                var canceledBySlide = false
                                var lockedBySlide = false

                                while (true) {
                                    val event = awaitPointerEvent()
                                    val change = event.changes.firstOrNull { it.id == down.id } ?: break
                                    if (!change.pressed) {
                                        change.consume()
                                        break
                                    }
                                    val delta = change.position - change.previousPosition
                                    totalDx += delta.x
                                    totalDy += delta.y
                                    slideOffsetX = totalDx.coerceAtMost(0f)
                                    slideOffsetY = totalDy.coerceAtMost(0f)
                                    change.consume()

                                    if (totalDx <= -cancelThresholdPx) {
                                        canceledBySlide = true
                                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                                        voiceRecorder.cancel()
                                        break
                                    }
                                    if (totalDy <= -lockThresholdPx) {
                                        lockedBySlide = true
                                        recordingLocked = true
                                        haptics.performHapticFeedback(HapticFeedbackType.LongPress)
                                        break
                                    }
                                }

                                slideOffsetX = 0f
                                slideOffsetY = 0f

                                if (!canceledBySlide && !lockedBySlide) {
                                    val file = voiceRecorder.stop()
                                    if (file != null) {
                                        currentOnSendVoice(file)
                                    } else {
                                        Toast.makeText(context, "Hold to record a voice message", Toast.LENGTH_SHORT).show()
                                    }
                                }
                            }
                        },
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Icon(
                            Icons.Outlined.Mic,
                            contentDescription = "Hold to record voice message",
                            modifier = Modifier.size(if (ultra) 56.dp else 24.dp),
                        )
                    }
                }
            }
        }
        if (showEmoji && !isRecording) {
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
            onTakePhoto = {
                attachSheet = false
                onTakePhoto()
            },
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
            onShareContact = {
                attachSheet = false
                keyboard?.hide()
                onOpenShareContact()
            },
            onCreatePoll = if (canCreatePoll) {
                {
                    attachSheet = false
                    keyboard?.hide()
                    showCreatePoll = true
                }
            } else {
                null
            },
        )
    }
}

/** Live recording bar overlay shown inside the composer while recording a voice note. */
@Composable
private fun VoiceRecordingBar(
    elapsedMs: Long,
    amplitudes: List<Float>,
    locked: Boolean,
    slideCancelProgress: Float,
    slideOffsetX: Float,
    modifier: Modifier = Modifier,
) {
    val pulseTransition = rememberInfiniteTransition(label = "voiceRecPulse")
    val dotAlpha by pulseTransition.animateFloat(
        initialValue = 0.35f,
        targetValue = 1f,
        animationSpec = infiniteRepeatable(tween(550), RepeatMode.Reverse),
        label = "recDotAlpha",
    )
    val waveColor = MaterialTheme.colorScheme.primary
    val trackColor = MaterialTheme.colorScheme.onSurfaceVariant.copy(alpha = 0.25f)

    Surface(
        shape = RoundedCornerShape(24.dp),
        color = MaterialTheme.colorScheme.surfaceContainerHigh,
        modifier = modifier.height(48.dp),
    ) {
        Row(
            modifier = Modifier.fillMaxSize().padding(horizontal = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Box(
                modifier = Modifier
                    .size(10.dp)
                    .graphicsLayer { alpha = dotAlpha }
                    .background(Color(0xFFE53935), CircleShape),
            )
            Text(
                text = formatRecordingDuration(elapsedMs),
                style = MaterialTheme.typography.labelLarge.copy(
                    fontWeight = FontWeight.SemiBold,
                    fontFeatureSettings = "tnum",
                ),
                color = MaterialTheme.colorScheme.onSurface,
                maxLines = 1,
            )
            Canvas(
                modifier = Modifier
                    .weight(1f)
                    .height(24.dp),
            ) {
                val maxBars = 28
                val step = size.width / maxBars
                val barWidth = (step * 0.56f).coerceIn(2.dp.toPx(), 3.5.dp.toPx())
                val radius = CornerRadius(barWidth / 2f, barWidth / 2f)
                val visible = if (amplitudes.size > maxBars) amplitudes.takeLast(maxBars) else amplitudes
                val startOffset = maxBars - visible.size
                for (i in 0 until maxBars) {
                    val left = i * step + (step - barWidth) / 2f
                    val amp = if (i >= startOffset) visible[i - startOffset] else null
                    val h = if (amp != null) {
                        (size.height * amp.coerceIn(0.12f, 1f)).coerceAtLeast(barWidth)
                    } else {
                        barWidth
                    }
                    val top = (size.height - h) / 2f
                    drawRoundRect(
                        color = if (amp != null) waveColor else trackColor,
                        topLeft = Offset(left, top),
                        size = Size(barWidth, h),
                        cornerRadius = radius,
                    )
                }
            }
            if (!locked) {
                Text(
                    text = "← Slide to cancel",
                    style = MaterialTheme.typography.labelMedium,
                    color = if (slideCancelProgress > 0.65f) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant,
                    maxLines = 1,
                    modifier = Modifier.graphicsLayer {
                        translationX = (slideOffsetX * 0.25f).coerceAtLeast(-28f)
                        alpha = (1f - slideCancelProgress * 0.6f).coerceIn(0.35f, 1f)
                    },
                )
            }
        }
    }
}

/** One action in the attachment grid. */
private class AttachAction(
    val label: String,
    val icon: ImageVector,
    val container: Color,
    val content: Color,
    val onClick: () -> Unit,
)

/** Attachment sheet: a left-aligned 4-column grid of tonal, rounded action tiles. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun AttachmentSheet(
    onDismiss: () -> Unit,
    onTakePhoto: () -> Unit,
    onPickMedia: () -> Unit,
    onPickFiles: () -> Unit,
    onPickSticker: () -> Unit,
    onShareContact: () -> Unit,
    onCreatePoll: (() -> Unit)? = null,
) {
    val cs = MaterialTheme.colorScheme
    // One tonal family for every tile so none of them reads as disabled.
    val tile = cs.secondaryContainer
    val onTile = cs.onSecondaryContainer
    val actions = buildList {
        add(AttachAction("Camera", Icons.Outlined.PhotoCamera, tile, onTile, onTakePhoto))
        add(AttachAction("Gallery", Icons.Outlined.PhotoLibrary, tile, onTile, onPickMedia))
        add(AttachAction("File", Icons.Outlined.AttachFile, tile, onTile, onPickFiles))
        add(AttachAction("Sticker", Icons.Outlined.Mood, tile, onTile, onPickSticker))
        add(AttachAction("Contact", Icons.Outlined.ContactPage, tile, onTile, onShareContact))
        if (onCreatePoll != null) {
            add(AttachAction("Poll", Icons.Outlined.Poll, tile, onTile, onCreatePoll))
        }
    }
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .padding(start = 16.dp, end = 16.dp, bottom = 24.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Text(
                "Attach",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.padding(start = 8.dp, bottom = 8.dp),
            )
            actions.chunked(4).forEach { rowActions ->
                Row(
                    modifier = Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    rowActions.forEach { action ->
                        AttachmentTile(
                            label = action.label,
                            icon = action.icon,
                            container = action.container,
                            content = action.content,
                            onClick = action.onClick,
                            modifier = Modifier.weight(1f),
                        )
                    }
                    repeat(4 - rowActions.size) {
                        Spacer(Modifier.weight(1f))
                    }
                }
            }
        }
    }
}

/** Material 3 dialog for pinning a new note on a group's board. */
@Composable
internal fun PinNoteDialog(
    onDismiss: () -> Unit,
    onCreate: (String) -> Unit,
) {
    var note by rememberSaveable { mutableStateOf("") }
    val trimmed = note.trim()
    AlertDialog(
        onDismissRequest = onDismiss,
        icon = {
            Icon(
                imageVector = Icons.Outlined.PushPin,
                contentDescription = null,
                tint = MaterialTheme.colorScheme.primary,
            )
        },
        title = { Text("Pin a note") },
        text = {
            OutlinedTextField(
                value = note,
                onValueChange = { note = it.take(1000) },
                label = { Text("Note") },
                placeholder = { Text("Announcement or reminder for the group…") },
                minLines = 2,
                maxLines = 5,
                modifier = Modifier.fillMaxWidth(),
            )
        },
        confirmButton = {
            TextButton(
                onClick = { onCreate(trimmed) },
                enabled = trimmed.isNotEmpty(),
            ) {
                Text("Pin")
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss) { Text("Cancel") }
        },
    )
}

/** Bottom sheet letting the user pick a friend to share as a Zalo contact card. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun ShareContactSheet(
    friends: List<Person>,
    onLoad: () -> Unit,
    onDismiss: () -> Unit,
    onPick: (Person) -> Unit,
) {
    LaunchedEffect(Unit) { onLoad() }
    var query by rememberSaveable { mutableStateOf("") }
    val filtered = remember(friends, query) { filterPeople(friends, query) }

    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
    ) {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .navigationBarsPadding()
                .heightIn(min = 240.dp, max = 520.dp)
                .padding(bottom = 16.dp),
        ) {
            Text(
                "Share contact",
                style = MaterialTheme.typography.titleMedium,
                fontWeight = FontWeight.SemiBold,
                modifier = Modifier.padding(horizontal = 20.dp, vertical = 4.dp),
            )
            SearchPill(
                query = query,
                onQuery = { query = it },
                placeholder = "Search friends",
                modifier = Modifier.padding(horizontal = 16.dp, vertical = 8.dp),
            )
            if (friends.isEmpty()) {
                Text(
                    "Loading contacts…",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(24.dp),
                )
            } else if (filtered.isEmpty()) {
                Text(
                    "No matching contacts",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.padding(24.dp),
                )
            } else {
                LazyColumn(Modifier.weight(1f, fill = false)) {
                    items(filtered, key = { it.id }) { person ->
                        Row(
                            modifier = Modifier
                                .fillMaxWidth()
                                .clickable { onPick(person) }
                                .padding(horizontal = 16.dp, vertical = 10.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Avatar(person.name, person.avatar, key = "user:${person.id}", isGroup = false, size = 40.dp)
                            Spacer(Modifier.width(12.dp))
                            Text(
                                person.name,
                                style = MaterialTheme.typography.bodyLarge,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                    }
                }
            }
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
    modifier: Modifier = Modifier,
) {
    Column(
        horizontalAlignment = Alignment.CenterHorizontally,
        modifier = modifier
            .clip(RoundedCornerShape(20.dp))
            .clickable(onClick = onClick)
            .padding(horizontal = 4.dp, vertical = 8.dp),
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
            style = MaterialTheme.typography.labelMedium.copy(
                hyphens = Hyphens.None,
                lineBreak = LineBreak.Heading,
            ),
            color = MaterialTheme.colorScheme.onSurface,
            textAlign = TextAlign.Center,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
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

/** `(messageId, fullUrl)` pairs for all non-recalled photo/GIF messages in [messages], oldest first. */
internal fun extractChatPhotoEntries(messages: List<ChatMessage>): List<Pair<String, String>> =
    messages.mapNotNull { msg ->
        if (msg.recalled) return@mapNotNull null
        val a = msg.attachment ?: return@mapNotNull null
        if (a.kind != "photo" && a.kind != "gif") return@mapNotNull null
        val image = a.image ?: return@mapNotNull null
        val full = (a.href ?: image).takeIf { it.isNotBlank() } ?: return@mapNotNull null
        msg.id to full
    }

/** Full image URLs of all non-recalled photo/GIF messages in [messages], in chronological order. */
internal fun extractChatPhotoUrls(messages: List<ChatMessage>): List<String> =
    extractChatPhotoEntries(messages).map { it.second }

/** Resolves the gallery index for a tapped photo, preferring [msgId] match when duplicate URLs exist. */
internal fun findChatPhotoIndex(entries: List<Pair<String, String>>, msgId: String?, url: String): Int {
    if (msgId != null) {
        val byId = entries.indexOfFirst { it.first == msgId }
        if (byId >= 0) return byId
    }
    return entries.indexOfFirst { it.second == url }
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
    if (text.length < 8 || !text.contains("://")) return emptyList()
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
