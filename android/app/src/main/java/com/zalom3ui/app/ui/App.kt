package com.zalom3ui.app.ui

import android.widget.Toast
import androidx.compose.animation.AnimatedContentScope
import androidx.compose.animation.EnterTransition
import androidx.compose.animation.ExitTransition
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SnackbarHost
import androidx.compose.material3.SnackbarHostState
import androidx.compose.material3.Surface
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.GroupAdd
import androidx.compose.material.icons.outlined.Link
import androidx.compose.material.icons.outlined.People
import androidx.compose.material.icons.outlined.PersonAdd
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation.NavBackStackEntry
import androidx.navigation.NavController
import androidx.navigation.NavGraphBuilder
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
import com.zalom3ui.app.data.ThreadRef
import com.zalom3ui.app.data.ThreadSummary
import com.zalom3ui.app.server.StartupLog
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private object Routes {
    const val HOME = "home"
    const val NEW_CHAT = "new_chat"
    const val NEW_GROUP = "new_group"
    const val ADD_MEMBERS = "add_members"
    const val MEMBERS = "members"
    const val CONTACTS = "contacts"
    const val ADD_FRIEND = "add_friend"
    const val CHAT = "chat"
    const val FORWARD = "forward"
    const val SHARE = "share"
    const val SETTINGS = "settings"
    const val CHAT_INFO = "chat_info"
}

@Composable
fun App(vm: AppViewModel) {
    val state by vm.state.collectAsStateWithLifecycle()
    val reducedMotionPref by vm.reducedMotion.collectAsStateWithLifecycle()
    val snackbar = remember { SnackbarHostState() }
    LaunchedEffect(state.error) {
        val error = state.error ?: return@LaunchedEffect
        snackbar.showSnackbar(error)
        vm.clearError()
    }
    LaunchedEffect(state.notice) {
        val notice = state.notice ?: return@LaunchedEffect
        snackbar.showSnackbar(notice)
        vm.clearNotice()
    }

    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        Box(Modifier.fillMaxSize()) {
            if (!state.settings.isConfigured) {
                SettingsScreen(
                    current = state.settings,
                    appearance = state.appearance,
                    profileName = null,
                    profileId = null,
                    loggedIn = false,
                    canGoBack = false,
                    onBack = {},
                    onSave = vm::saveSettings,
                    onAppearance = vm::setAppearance,
                    reducedMotion = reducedMotionPref,
                    onReducedMotion = vm::setReducedMotion,
                    onRestartServer = vm::restartLocalServer,
                    onLogout = vm::logout,
                )
            } else {
                AppNav(vm)
            }
            HistorySyncHost(vm)
            SnackbarHost(
                snackbar,
                Modifier.align(Alignment.BottomCenter).navigationBarsPadding().imePadding().padding(bottom = 72.dp),
            )
        }
    }
}

@Composable
private fun AppNav(vm: AppViewModel) {
    val nav = rememberNavController()
    val context = LocalContext.current
    val scope = rememberCoroutineScope()

    val configuration = LocalConfiguration.current
    // Tablets (not phones turned sideways) show the chat list and the open chat side by side, like Telegram.
    val wide = configuration.smallestScreenWidthDp >= 600 && configuration.screenWidthDp >= 700
    val wideNow by rememberUpdatedState(wide)
    // Set by the chat-info screen's "Search" action; the chat screen consumes it and opens its search bar.
    var pendingSearch by remember { mutableStateOf(false) }

    /** Shows the chat the view model just opened: beside the list on a tablet, as its own screen on a phone. */
    fun goToChat() {
        nav.popBackStack(Routes.HOME, inclusive = false)
        if (!wideNow) nav.navigate(Routes.CHAT)
    }

    /**
     * What the chat screen can do, for the phone screen and the tablet pane alike. Callers remember it per chat, so the
     * chat screen can skip recomposing when nothing it shows changed.
     */
    fun chatActions(ref: ThreadRef, onBack: () -> Unit) =
        ChatActions(
            onBack = onBack,
            onSend = { text, quote, mentioned, onSent -> vm.send(text, quote, mentioned, onSent) },
            onStartTyping = vm::notifyTyping,
            onLoadMembers = vm::loadMembers,
            onLoadEarlier = vm::loadEarlier,
            onReact = vm::toggleReaction,
            onRecall = { vm.recall(it) },
            onDelete = { vm.deleteForMe(it) },
            onForward = { msg ->
                vm.select(msg)
                nav.navigate(Routes.FORWARD)
            },
            onPin = { vm.setPinned(ref, it) },
            onMute = { vm.setMuted(ref, it) },
            onMarkUnread = {
                vm.markUnread(ref)
                if (wideNow) vm.closeChat(ref) else nav.popBackStack(Routes.HOME, inclusive = false)
            },
            onDeleteConversation = { vm.deleteConversation(ref) },
            onSendFiles = vm::sendFiles,
            onSearchStickers = vm::searchStickers,
            onSendSticker = vm::sendSticker,
            onShowMembers = { nav.navigate(Routes.MEMBERS) },
            onAddMembers = { nav.navigate(Routes.ADD_MEMBERS) },
            onOpenInfo = { nav.navigate(Routes.CHAT_INFO) },
            onRenameGroup = vm::renameGroup,
            onLeaveGroup = { silent ->
                vm.leaveGroup(silent) {
                    vm.closeChat(ref)
                    nav.popBackStack(Routes.HOME, inclusive = false)
                }
            },
            onDraftChange = { vm.setDraft(ref, it) },
            onSendVoice = vm::sendVoice,
            onRetryPending = vm::retryPending,
            onDismissPending = vm::dismissPending,
            onDeleteMany = vm::deleteMessages,
            onForwardMany = { msgs ->
                vm.selectManyForForward(msgs)
                nav.navigate(Routes.FORWARD)
            },
            onLoadGroupBoard = { if (ref.kind == "group") vm.loadGroupBoard(ref.id) },
            onCreateGroupNote = { title -> if (ref.kind == "group") vm.createGroupNote(ref.id, title) },
            onSetDisappearingTtl = { ttl -> vm.setDisappearingTtl(ref, ttl) },
            onLoadFriends = vm::loadFriends,
            onSendCard = { person -> vm.sendCard(ref, person.id) },
            onCancelOutbox = vm::cancelOutboxEntry,
            onRetryOutbox = vm::retryOutbox,
        )
    // Only these slices are read here, so a state change elsewhere (a message, someone typing) doesn't recompose the NavHost.
    val openState = vm.state.collectAsStateWithLifecycle()
    val openRequest by remember { derivedStateOf { openState.value.openRequest } }
    val hasThreads by remember { derivedStateOf { openState.value.threads.isNotEmpty() } }
    val pendingShare by remember { derivedStateOf { openState.value.pendingShare } }
    val sessionState by remember { derivedStateOf { openState.value.status?.state } }
    val signedIn by remember { derivedStateOf { openState.value.status?.isLoggedIn == true } }
    // A tapped notification: open its chat once the chat list knows it.
    LaunchedEffect(openRequest, hasThreads) {
        if (openRequest == null) return@LaunchedEffect
        val thread = vm.consumeOpenRequest() ?: return@LaunchedEffect
        vm.openChat(thread)
        goToChat()
    }
    // Content shared into Zalỏ from another app: jump straight to Direct Share target or pick a chat once ready.
    LaunchedEffect(pendingShare, sessionState) {
        val share = pendingShare ?: return@LaunchedEffect
        if (sessionState == "ready") {
            val target = share.target
            if (target != null) {
                val existing = vm.state.value.threads.firstOrNull { it.ref == target }
                vm.deliverPendingShare(target, existing?.name.orEmpty()) { goToChat() }
            } else if (nav.currentDestination?.route != Routes.SHARE) {
                nav.navigate(Routes.SHARE) { launchSingleTop = true }
            }
        }
    }
    LaunchedEffect(signedIn) { if (signedIn) vm.loadFriendRequests() }
    // Turning a tablet (or resizing a window) moves an open chat between its own screen and the pane.
    LaunchedEffect(wide) {
        val route = nav.currentDestination?.route
        if (wide && route == Routes.CHAT) {
            nav.popBackStack(Routes.HOME, inclusive = false)
        } else if (!wide && route == Routes.HOME && vm.state.value.chat != null) {
            nav.navigate(Routes.CHAT)
        }
    }

    // Poll cards and the poll sheet load and change polls through this (see Polls.kt).
    val polls = remember(vm) {
        Polls(scope, vm::loadPoll, vm::votePoll, vm::addPollOptions, vm::lockPoll, vm::createPoll)
    }
    LaunchedEffect(polls) { vm.pollEvents.collect { polls.onChanged(it) } }

    val reducedMotion = LocalReducedMotion.current
    CompositionLocalProvider(LocalPolls provides polls, LocalUploadProgress provides vm.uploadProgress) {
    NavHost(
        navController = nav,
        startDestination = Routes.HOME,
        enterTransition = {
            if (reducedMotion) {
                EnterTransition.None
            } else {
                slideInHorizontally(spring(dampingRatio = 0.8f, stiffness = Spring.StiffnessMediumLow)) { it / 4 } + fadeIn()
            }
        },
        exitTransition = { if (reducedMotion) ExitTransition.None else fadeOut() },
        // Back slides the screen off to the right under the finger, revealing the previous one (PredictiveBack.kt).
        popEnterTransition = { if (reducedMotion) EnterTransition.None else backSlideIn() },
        popExitTransition = { if (reducedMotion) ExitTransition.None else backSlideOut() },
    ) {
        // Each destination collects state itself so it never renders a stale capture.
        screen(Routes.HOME, nav) {
            val state by vm.state.collectAsStateWithLifecycle()
            // Remembered: a new instance each recomposition would make the chat list and every row recompose too.
            val listActions = remember(vm, nav) {
                ThreadListActions(
                    onOpen = { thread ->
                        vm.openChat(thread)
                        if (!wideNow) nav.navigate(Routes.CHAT)
                    },
                    onNewChat = { nav.navigate(Routes.NEW_CHAT) },
                    onContacts = { nav.navigate(Routes.CONTACTS) },
                    onReconnect = vm::reconnectZalo,
                    onSettings = { nav.navigate(Routes.SETTINGS) },
                    onMarkAllRead = vm::markAllRead,
                    onPin = { thread, pinned -> vm.setPinned(thread.ref, pinned) },
                    onMute = { thread, option -> vm.setMuted(thread.ref, option) },
                    onMarkRead = { vm.markThreadRead(it.ref) },
                    onMarkUnread = { vm.markUnread(it.ref) },
                    onDelete = { vm.deleteConversation(it.ref) },
                    onSearchMessages = vm::searchAllMessages,
                    onOpenSearchHit = { hit ->
                        vm.openChatAtMessage(hit.ref, hit.threadName, hit.message.id)
                        goToChat()
                    },
                    onJoinGroupLink = { link ->
                        vm.joinGroupLink(link) { thread ->
                            if (thread != null) {
                                vm.openChat(thread)
                                goToChat()
                            }
                        }
                    },
                )
            }
            val status = state.status
            val connectionError = state.connectionError
            when {
                status == null && connectionError != null -> ConnectionErrorScreen(
                    error = connectionError,
                    onRetry = vm::startLiveUpdates,
                    onSettings = { nav.navigate(Routes.SETTINGS) },
                )
                status == null && state.settings.onDevice -> StartingScreen(
                    since = state.startingSince,
                    error = state.startupError,
                    onRestart = vm::restartLocalServer,
                    onShareLog = {
                        scope.launch {
                            val report = withContext(Dispatchers.IO) { StartupLog.report(context) }
                            QrActions.shareText(context, "zaloo-startup-log.txt", "Share startup log", report)
                        }
                    },
                )
                status == null -> LoadingScreen("Connecting…")
                !status.isLoggedIn -> LoginScreen(
                    status = status,
                    onLogin = vm::login,
                    onRetry = vm::retryLogin,
                    onSettings = { nav.navigate(Routes.SETTINGS) },
                )
                else -> TwoPaneHome(
                    wide = wide,
                    chat = state.chat,
                    onCloseChat = { vm.closeChat() },
                    chatPane = { chat ->
                        val outboxEntries by vm.outboxEntries.collectAsStateWithLifecycle()
                        val disappearingTtl by vm.disappearingTtl.collectAsStateWithLifecycle()
                        val groupBoards by vm.groupBoards.collectAsStateWithLifecycle()
                        val actions = remember(chat.ref) { chatActions(chat.ref, onBack = { vm.closeChat(chat.ref) }) }
                        // The composer reads it only when it is created, so read it only then too.
                        val initialDraft = remember(chat.ref) { vm.draft(chat.ref) }
                        ChatScreen(
                            chat = chat,
                            thread = state.threads.firstOrNull { it.ref == chat.ref },
                            typing = state.typing[chat.ref.key],
                            stickers = state.stickers,
                            actions = actions,
                            initialSearch = pendingSearch,
                            onSearchConsumed = { pendingSearch = false },
                            initialDraft = initialDraft,
                            chatAccentOverride = state.chatAccents[chat.ref.key],
                            appearance = state.appearance,
                            scrollToMsgId = state.scrollToMsgId,
                            onScrolledToMsg = vm::clearScrollToMsgId,
                            boardItems = if (chat.kind == "group") groupBoards[chat.id].orEmpty() else emptyList(),
                            disappearingTtlMs = disappearingTtl[chat.ref.key],
                            outboxEntries = outboxEntries,
                            friends = state.friends,
                        )
                    },
                ) { selectedKey ->
                    // The fields go in as they are (no copies), so the list skips when none of them changed.
                    ThreadListScreen(
                        threads = state.threads,
                        drafts = state.drafts,
                        typing = state.typing,
                        status = state.status,
                        connectionError = state.connectionError,
                        pendingRequests = state.friendRequests?.size ?: 0,
                        messageSearchResults = state.messageSearchResults,
                        messageSearchBusy = state.messageSearchBusy,
                        actions = listActions,
                        selectedKey = selectedKey,
                    )
                }
            }
        }
        screen(Routes.NEW_CHAT, nav) {
            val state by vm.state.collectAsStateWithLifecycle()
            var showJoinGroupDialog by remember { mutableStateOf(false) }
            LaunchedEffect(Unit) { vm.loadFriendRequests() }
            ContactsScreen(
                title = "New message",
                threads = state.threads,
                onBack = { nav.safePopBackStack(Routes.NEW_CHAT) },
                onPick = { thread ->
                    vm.openChat(thread)
                    goToChat()
                },
                actions = listOf(
                    PickerAction("New group", Icons.Outlined.GroupAdd) { nav.navigate(Routes.NEW_GROUP) },
                    PickerAction("Join group by link", Icons.Outlined.Link) { showJoinGroupDialog = true },
                    PickerAction("Add friend", Icons.Outlined.PersonAdd) { nav.navigate(Routes.ADD_FRIEND) },
                    PickerAction("Contacts and requests", Icons.Outlined.People, badge = state.friendRequests?.size ?: 0) {
                        nav.navigate(Routes.CONTACTS)
                    },
                ),
            )
            if (showJoinGroupDialog) {
                JoinGroupLinkDialog(
                    onDismiss = { showJoinGroupDialog = false },
                    onJoin = { link ->
                        showJoinGroupDialog = false
                        vm.joinGroupLink(link) { thread ->
                            if (thread != null) {
                                vm.openChat(thread)
                                goToChat()
                            }
                        }
                    },
                )
            }
        }
        screen(Routes.NEW_GROUP, nav) {
            val state by vm.state.collectAsStateWithLifecycle()
            var busy by remember { mutableStateOf(false) }
            FriendPickerScreen(
                title = "New group",
                confirmLabel = "Create",
                friends = state.friends,
                exclude = emptySet(),
                askName = true,
                minPicks = 2,
                busy = busy,
                onLoad = vm::loadFriends,
                onBack = { nav.safePopBackStack(Routes.NEW_GROUP) },
                onConfirm = { name, ids ->
                    busy = true
                    vm.createGroup(name, ids, onDone = { busy = false }) { thread ->
                        vm.openChat(thread)
                        goToChat()
                    }
                },
            )
        }
        screen(Routes.CONTACTS, nav) {
            val state by vm.state.collectAsStateWithLifecycle()
            FriendsScreen(
                friends = state.friends,
                received = state.friendRequests,
                sent = state.sentRequests,
                onLoad = {
                    vm.loadFriends()
                    vm.loadFriendRequests()
                },
                onBack = { nav.safePopBackStack(Routes.CONTACTS) },
                onAccept = vm::acceptFriend,
                onReject = vm::rejectFriend,
                onUndo = vm::cancelSentRequest,
                onAddFriend = { nav.navigate(Routes.ADD_FRIEND) },
                onMessage = { person ->
                    vm.openFriendChat(person)
                    goToChat()
                },
            )
        }
        screen(Routes.ADD_FRIEND, nav) {
            AddFriendScreen(
                onBack = { nav.safePopBackStack(Routes.ADD_FRIEND) },
                onFind = vm::findUser,
                onRequest = vm::sendFriendRequest,
                onUndo = vm::undoFriendRequest,
                onMessage = { user ->
                    vm.openUserChat(user)
                    goToChat()
                },
            )
        }
        screen(Routes.MEMBERS, nav) {
            val state by vm.state.collectAsStateWithLifecycle()
            val deputiesByGroup by vm.groupDeputies.collectAsStateWithLifecycle()
            val chat = state.chat
            GroupMembersScreen(
                groupName = chat?.name.orEmpty(),
                members = chat?.members,
                onLoad = vm::loadMembers,
                onBack = { nav.safePopBackStack(Routes.MEMBERS) },
                onAdd = { nav.navigate(Routes.ADD_MEMBERS) },
                onOpen = { member ->
                    val existing = state.threads.firstOrNull { it.kind == "user" && it.id == member.id }
                    vm.openChat(existing ?: ThreadSummary(id = member.id, kind = "user", name = member.name))
                    goToChat()
                },
                onRemove = vm::removeMember,
                deputyIds = chat?.id?.let { deputiesByGroup[it] }.orEmpty(),
                onAddDeputy = { member ->
                    chat?.id?.let { gid -> vm.addGroupDeputy(gid, member.id, member.name) }
                },
                onRemoveDeputy = { member ->
                    chat?.id?.let { gid -> vm.removeGroupDeputy(gid, member.id, member.name) }
                },
            )
        }
        screen(Routes.ADD_MEMBERS, nav) {
            val state by vm.state.collectAsStateWithLifecycle()
            var busy by remember { mutableStateOf(false) }
            LaunchedEffect(Unit) { vm.loadMembers() }
            FriendPickerScreen(
                title = "Add to ${state.chat?.name.orEmpty()}",
                confirmLabel = "Add",
                friends = state.friends,
                exclude = state.chat?.members.orEmpty().map { it.id }.toSet(),
                askName = false,
                minPicks = 1,
                busy = busy,
                onLoad = vm::loadFriends,
                onBack = { nav.safePopBackStack(Routes.ADD_MEMBERS) },
                onConfirm = { _, ids ->
                    busy = true
                    vm.addMembers(ids, onFinished = { busy = false }) {
                        nav.safePopBackStack(Routes.ADD_MEMBERS)
                    }
                },
            )
        }
        screen(Routes.CHAT, nav) { entry ->
            val state by vm.state.collectAsStateWithLifecycle()
            val outboxEntries by vm.outboxEntries.collectAsStateWithLifecycle()
            val disappearingTtl by vm.disappearingTtl.collectAsStateWithLifecycle()
            val groupBoards by vm.groupBoards.collectAsStateWithLifecycle()
            // Close the chat once this screen leaves the back stack (not when covered by "Forward").
            LaunchedEffect(entry) {
                val ref = vm.state.value.chat?.ref
                entry.lifecycle.addObserver(LifecycleEventObserver { _, event ->
                    if (event == Lifecycle.Event.ON_DESTROY && !wideNow) vm.closeChat(ref)
                })
            }
            val chat = state.chat
            if (chat == null) {
                LaunchedEffect(Unit) { nav.popBackStack(Routes.HOME, inclusive = false) }
            } else {
                val actions = remember(chat.ref) { chatActions(chat.ref, onBack = { nav.safePopBackStack(Routes.CHAT) }) }
                // The composer reads it only when it is created, so read it only then too.
                val initialDraft = remember(chat.ref) { vm.draft(chat.ref) }
                ChatScreen(
                    chat = chat,
                    thread = state.threads.firstOrNull { it.ref == chat.ref },
                    typing = state.typing[chat.ref.key],
                    stickers = state.stickers,
                    actions = actions,
                    initialSearch = pendingSearch,
                    onSearchConsumed = { pendingSearch = false },
                    initialDraft = initialDraft,
                    chatAccentOverride = state.chatAccents[chat.ref.key],
                    appearance = state.appearance,
                    scrollToMsgId = state.scrollToMsgId,
                    onScrolledToMsg = vm::clearScrollToMsgId,
                    boardItems = if (chat.kind == "group") groupBoards[chat.id].orEmpty() else emptyList(),
                    disappearingTtlMs = disappearingTtl[chat.ref.key],
                    outboxEntries = outboxEntries,
                    friends = state.friends,
                )
            }
        }
        screen(Routes.CHAT_INFO, nav) {
            val state by vm.state.collectAsStateWithLifecycle()
            val disappearingTtl by vm.disappearingTtl.collectAsStateWithLifecycle()
            val groupBoards by vm.groupBoards.collectAsStateWithLifecycle()
            val blockedUsers by vm.blockedUsers.collectAsStateWithLifecycle()
            val deputiesByGroup by vm.groupDeputies.collectAsStateWithLifecycle()
            val chat = state.chat
            if (chat == null) {
                LaunchedEffect(Unit) { nav.popBackStack(Routes.HOME, inclusive = false) }
            } else {
                ChatInfoScreen(
                    chat = chat,
                    thread = state.threads.firstOrNull { it.ref == chat.ref },
                    onBack = { nav.safePopBackStack(Routes.CHAT_INFO) },
                    onSearch = {
                        pendingSearch = true
                        nav.popBackToChat()
                    },
                    onPin = { vm.setPinned(chat.ref, it) },
                    onMute = { vm.setMuted(chat.ref, it) },
                    onMarkUnread = {
                        vm.markUnread(chat.ref)
                        if (wideNow) vm.closeChat(chat.ref)
                        nav.popBackStack(Routes.HOME, inclusive = false)
                    },
                    onLoadMembers = vm::loadMembers,
                    onShowMembers = { nav.navigate(Routes.MEMBERS) },
                    onAddMembers = { nav.navigate(Routes.ADD_MEMBERS) },
                    onOpenMember = { member ->
                        val existing = state.threads.firstOrNull { it.kind == "user" && it.id == member.id }
                        vm.openChat(existing ?: ThreadSummary(id = member.id, kind = "user", name = member.name))
                        goToChat()
                    },
                    onRenameGroup = vm::renameGroup,
                    onLeaveGroup = { silent ->
                        vm.leaveGroup(silent) {
                            vm.closeChat(chat.ref)
                            nav.popBackStack(Routes.HOME, inclusive = false)
                        }
                    },
                    onDeleteConversation = {
                        vm.deleteConversation(chat.ref)
                        nav.popBackStack(Routes.HOME, inclusive = false)
                    },
                    chatAccent = state.chatAccents[chat.ref.key],
                    onSetChatAccent = { accent -> vm.setChatAccent(chat.ref, accent) },
                    disappearingTtlMs = disappearingTtl[chat.ref.key],
                    onSetDisappearingTtl = { ttl -> vm.setDisappearingTtl(chat.ref, ttl) },
                    isBlocked = chat.kind == "user" && chat.id in blockedUsers,
                    onLoadUserInfo = vm::loadUserInfo,
                    onOpenCommonGroup = { group ->
                        val existing = state.threads.firstOrNull { it.ref == group.ref }
                        vm.openChat(existing ?: ThreadSummary(id = group.id, kind = group.kind, name = group.name, avatar = group.avatar))
                        goToChat()
                    },
                    onChangeAlias = { alias -> vm.changeFriendAlias(chat.id, alias) },
                    onRemoveAlias = { vm.removeFriendAlias(chat.id) },
                    onBlockUser = { vm.blockUser(chat.id) },
                    onUnblockUser = { vm.unblockUser(chat.id) },
                    onRemoveFriend = {
                        vm.removeFriend(chat.id) {
                            nav.popBackStack(Routes.HOME, inclusive = false)
                        }
                    },
                    boardItems = if (chat.kind == "group") groupBoards[chat.id].orEmpty() else emptyList(),
                    onLoadGroupBoard = { if (chat.kind == "group") vm.loadGroupBoard(chat.id) },
                    onCreateGroupNote = { note -> if (chat.kind == "group") vm.createGroupNote(chat.id, note) },
                    onLoadGroupLink = { vm.loadGroupLink(chat.id) },
                    onEnableGroupLink = { vm.enableGroupLink(chat.id) },
                    onDisableGroupLink = { vm.disableGroupLink(chat.id) },
                    deputyIds = if (chat.kind == "group") deputiesByGroup[chat.id].orEmpty() else emptySet(),
                    onAddDeputy = { member -> vm.addGroupDeputy(chat.id, member.id, member.name) },
                    onRemoveDeputy = { member -> vm.removeGroupDeputy(chat.id, member.id, member.name) },
                )
            }
        }
        screen(Routes.FORWARD, nav) { entry ->
            val state by vm.state.collectAsStateWithLifecycle()
            val many = state.forwardingMany
            val msg = state.selected
            LaunchedEffect(entry) {
                entry.lifecycle.addObserver(LifecycleEventObserver { _, event ->
                    if (event == Lifecycle.Event.ON_DESTROY) vm.selectManyForForward(emptyList())
                })
            }
            ContactsScreen(
                title = if (many.size > 1) "Forward ${many.size} messages to" else "Forward to",
                threads = state.threads,
                onBack = {
                    vm.selectManyForForward(emptyList())
                    nav.safePopBackStack(Routes.FORWARD)
                },
                onPick = { target ->
                    if (many.isNotEmpty()) {
                        val count = many.size
                        vm.forwardSelectedMany(listOf(target.ref)) {
                            val label = if (count == 1) "Forwarded to ${target.name}" else "Forwarded $count messages to ${target.name}"
                            Toast.makeText(context, label, Toast.LENGTH_SHORT).show()
                        }
                    } else if (msg != null) {
                        vm.forward(msg, target, onDone = {
                            Toast.makeText(context, "Forwarded to ${target.name}", Toast.LENGTH_SHORT).show()
                        })
                    }
                    nav.popBackToChat()
                },
            )
        }
        screen(Routes.SHARE, nav) { entry ->
            val state by vm.state.collectAsStateWithLifecycle()
            LaunchedEffect(entry) {
                entry.lifecycle.addObserver(LifecycleEventObserver { _, event ->
                    if (event == Lifecycle.Event.ON_DESTROY) vm.setPendingShare(null)
                })
            }
            ContactsScreen(
                title = "Share to…",
                threads = state.threads,
                onBack = {
                    vm.setPendingShare(null)
                    nav.safePopBackStack(Routes.SHARE)
                },
                onPick = { thread ->
                    vm.deliverPendingShare(thread.ref, thread.name) { goToChat() }
                },
            )
        }
        screen(Routes.SETTINGS, nav) {
            val state by vm.state.collectAsStateWithLifecycle()
            val reducedMotionPref by vm.reducedMotion.collectAsStateWithLifecycle()
            SettingsScreen(
                current = state.settings,
                appearance = state.appearance,
                profileName = state.status?.ownName,
                profileId = state.status?.ownId,
                profileAvatar = state.status?.ownAvatar,
                loggedIn = state.status?.isLoggedIn == true,
                canGoBack = true,
                onBack = { nav.safePopBackStack(Routes.SETTINGS) },
                onSave = {
                    vm.saveSettings(it)
                    nav.safePopBackStack(Routes.SETTINGS)
                },
                onAppearance = vm::setAppearance,
                reducedMotion = reducedMotionPref,
                onReducedMotion = vm::setReducedMotion,
                onRestartServer = {
                    vm.restartLocalServer()
                    nav.safePopBackStack(Routes.SETTINGS)
                },
                onLogout = {
                    vm.logout()
                    nav.popBackStack(Routes.HOME, inclusive = false)
                },
                onSyncHistory = vm::startHistorySync,
                onShareCallLog = { vm.shareCallLog { log -> scope.launch { QrActions.shareCallLog(context, log) } } },
                onClearCallLog = vm::clearCallLog,
            )
        }
    }
    }
}

/** A destination whose screen follows the back gesture and slides away (see [BackSlideFrame]). */
private fun NavGraphBuilder.screen(
    route: String,
    nav: NavController,
    content: @Composable AnimatedContentScope.(NavBackStackEntry) -> Unit,
) = composable(route) { entry -> BackSlideFrame(nav, entry) { content(entry) } }

/**
 * Pops the top destination only while it is still active and another destination sits beneath it.
 * Prevents rapid double-taps on a back arrow from popping the root screen and leaving an empty host.
 */
private fun NavHostController.safePopBackStack(fromRoute: String? = null) {
    if (previousBackStackEntry == null) return
    val current = currentBackStackEntry ?: return
    if (fromRoute != null && current.destination.route != fromRoute) return
    if (current.lifecycle.currentState != Lifecycle.State.RESUMED) return
    popBackStack()
}

private fun NavHostController.popBackToChat() {
    if (!popBackStack(Routes.CHAT, inclusive = false)) safePopBackStack()
}
