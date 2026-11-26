package com.zalom3ui.app.ui

import android.widget.Toast
import androidx.compose.animation.core.Spring
import androidx.compose.animation.core.spring
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.slideInHorizontally
import androidx.compose.animation.slideOutHorizontally
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
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.GroupAdd
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
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController
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

    /** What the chat screen can do, for the phone screen and the tablet pane alike. */
    fun chatActions(chat: ChatState, onBack: () -> Unit) =
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
            onPin = { vm.setPinned(chat.ref, it) },
            onMute = { vm.setMuted(chat.ref, it) },
            onMarkUnread = {
                vm.markUnread(chat.ref)
                if (wideNow) vm.closeChat(chat.ref) else nav.popBackStack(Routes.HOME, inclusive = false)
            },
            onDeleteConversation = { vm.deleteConversation(chat.ref) },
            onSendFiles = vm::sendFiles,
            onSearchStickers = vm::searchStickers,
            onSendSticker = vm::sendSticker,
            onShowMembers = { nav.navigate(Routes.MEMBERS) },
            onAddMembers = { nav.navigate(Routes.ADD_MEMBERS) },
            onOpenInfo = { nav.navigate(Routes.CHAT_INFO) },
            onRenameGroup = vm::renameGroup,
            onLeaveGroup = { silent ->
                vm.leaveGroup(silent) {
                    vm.closeChat(chat.ref)
                    nav.popBackStack(Routes.HOME, inclusive = false)
                }
            },
            onDraftChange = { vm.setDraft(chat.ref, it) },
            onSendVoice = vm::sendVoice,
            onRetryPending = vm::retryPending,
            onDismissPending = vm::dismissPending,
            onDeleteMany = vm::deleteMessages,
            onForwardMany = { msgs ->
                vm.selectManyForForward(msgs)
                nav.navigate(Routes.FORWARD)
            },
        )
    // A tapped notification: open its chat once the chat list knows it.
    val openState by vm.state.collectAsStateWithLifecycle()
    LaunchedEffect(openState.openRequest, openState.threads) {
        if (openState.openRequest == null) return@LaunchedEffect
        val thread = vm.consumeOpenRequest() ?: return@LaunchedEffect
        vm.openChat(thread)
        goToChat()
    }
    // Content shared into Zalỏ from another app: pick a target chat once ready.
    LaunchedEffect(openState.pendingShare, openState.status?.state) {
        if (openState.pendingShare != null && openState.status?.state == "ready") {
            if (nav.currentDestination?.route != Routes.SHARE) {
                nav.navigate(Routes.SHARE) { launchSingleTop = true }
            }
        }
    }
    val signedIn = openState.status?.isLoggedIn == true
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

    CompositionLocalProvider(LocalPolls provides polls) {
    NavHost(
        navController = nav,
        startDestination = Routes.HOME,
        enterTransition = { slideInHorizontally(spring(dampingRatio = 0.8f, stiffness = Spring.StiffnessMediumLow)) { it / 4 } + fadeIn() },
        exitTransition = { fadeOut() },
        popEnterTransition = { fadeIn() },
        popExitTransition = { slideOutHorizontally(spring(stiffness = Spring.StiffnessMedium)) { it / 4 } + fadeOut() },
    ) {
        // Each destination collects state itself so it never renders a stale capture.
        composable(Routes.HOME) {
            val state by vm.state.collectAsStateWithLifecycle()
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
                        ChatScreen(
                            chat = chat,
                            thread = state.threads.firstOrNull { it.ref == chat.ref },
                            typing = state.typing[chat.ref.key],
                            stickers = state.stickers,
                            actions = chatActions(chat, onBack = { vm.closeChat(chat.ref) }),
                            initialSearch = pendingSearch,
                            onSearchConsumed = { pendingSearch = false },
                            initialDraft = state.drafts[chat.ref.key].orEmpty(),
                            chatAccentOverride = state.chatAccents[chat.ref.key],
                            appearance = state.appearance,
                            scrollToMsgId = state.scrollToMsgId,
                            onScrolledToMsg = vm::clearScrollToMsgId,
                        )
                    },
                ) { selectedKey -> ThreadListScreen(
                    state = state,
                    selectedKey = selectedKey,
                    actions = ThreadListActions(
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
                    ),
                ) }
            }
        }
        composable(Routes.NEW_CHAT) {
            val state by vm.state.collectAsStateWithLifecycle()
            LaunchedEffect(Unit) { vm.loadFriendRequests() }
            ContactsScreen(
                title = "New message",
                threads = state.threads,
                onBack = { nav.popBackStack() },
                onPick = { thread ->
                    vm.openChat(thread)
                    goToChat()
                },
                actions = listOf(
                    PickerAction("New group", Icons.Outlined.GroupAdd) { nav.navigate(Routes.NEW_GROUP) },
                    PickerAction("Add friend", Icons.Outlined.PersonAdd) { nav.navigate(Routes.ADD_FRIEND) },
                    PickerAction("Contacts and requests", Icons.Outlined.People, badge = state.friendRequests?.size ?: 0) {
                        nav.navigate(Routes.CONTACTS)
                    },
                ),
            )
        }
        composable(Routes.NEW_GROUP) {
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
                onBack = { nav.popBackStack() },
                onConfirm = { name, ids ->
                    busy = true
                    vm.createGroup(name, ids, onDone = { busy = false }) { thread ->
                        vm.openChat(thread)
                        goToChat()
                    }
                },
            )
        }
        composable(Routes.CONTACTS) {
            val state by vm.state.collectAsStateWithLifecycle()
            FriendsScreen(
                friends = state.friends,
                received = state.friendRequests,
                sent = state.sentRequests,
                onLoad = {
                    vm.loadFriends()
                    vm.loadFriendRequests()
                },
                onBack = { nav.popBackStack() },
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
        composable(Routes.ADD_FRIEND) {
            AddFriendScreen(
                onBack = { nav.popBackStack() },
                onFind = vm::findUser,
                onRequest = vm::sendFriendRequest,
                onUndo = vm::undoFriendRequest,
                onMessage = { user ->
                    vm.openUserChat(user)
                    goToChat()
                },
            )
        }
        composable(Routes.MEMBERS) {
            val state by vm.state.collectAsStateWithLifecycle()
            val chat = state.chat
            GroupMembersScreen(
                groupName = chat?.name.orEmpty(),
                members = chat?.members,
                onLoad = vm::loadMembers,
                onBack = { nav.popBackStack() },
                onAdd = { nav.navigate(Routes.ADD_MEMBERS) },
                onOpen = { member ->
                    val existing = state.threads.firstOrNull { it.kind == "user" && it.id == member.id }
                    vm.openChat(existing ?: com.zalom3ui.app.data.ThreadSummary(id = member.id, kind = "user", name = member.name))
                    goToChat()
                },
                onRemove = vm::removeMember,
            )
        }
        composable(Routes.ADD_MEMBERS) {
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
                onBack = { nav.popBackStack() },
                onConfirm = { _, ids ->
                    busy = true
                    vm.addMembers(ids, onFinished = { busy = false }) {
                        nav.popBackStack()
                    }
                },
            )
        }
        composable(Routes.CHAT) { entry ->
            val state by vm.state.collectAsStateWithLifecycle()
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
                ChatScreen(
                    chat = chat,
                    thread = state.threads.firstOrNull { it.ref == chat.ref },
                    typing = state.typing[chat.ref.key],
                    stickers = state.stickers,
                    actions = chatActions(chat, onBack = { nav.popBackStack() }),
                    initialSearch = pendingSearch,
                    onSearchConsumed = { pendingSearch = false },
                    initialDraft = state.drafts[chat.ref.key].orEmpty(),
                    chatAccentOverride = state.chatAccents[chat.ref.key],
                    appearance = state.appearance,
                    scrollToMsgId = state.scrollToMsgId,
                    onScrolledToMsg = vm::clearScrollToMsgId,
                )
            }
        }
        composable(Routes.CHAT_INFO) {
            val state by vm.state.collectAsStateWithLifecycle()
            val chat = state.chat
            if (chat == null) {
                LaunchedEffect(Unit) { nav.popBackStack(Routes.HOME, inclusive = false) }
            } else {
                ChatInfoScreen(
                    chat = chat,
                    thread = state.threads.firstOrNull { it.ref == chat.ref },
                    onBack = { nav.popBackStack() },
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
                        vm.openChat(existing ?: com.zalom3ui.app.data.ThreadSummary(id = member.id, kind = "user", name = member.name))
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
                )
            }
        }
        composable(Routes.FORWARD) { entry ->
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
                    nav.popBackStack()
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
        composable(Routes.SHARE) { entry ->
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
                    nav.popBackStack()
                },
                onPick = { thread ->
                    vm.deliverPendingShare(thread.ref, thread.name) { goToChat() }
                },
            )
        }
        composable(Routes.SETTINGS) {
            val state by vm.state.collectAsStateWithLifecycle()
            SettingsScreen(
                current = state.settings,
                appearance = state.appearance,
                profileName = state.status?.ownName,
                profileId = state.status?.ownId,
                profileAvatar = state.status?.ownAvatar,
                loggedIn = state.status?.isLoggedIn == true,
                canGoBack = true,
                onBack = { nav.popBackStack() },
                onSave = {
                    vm.saveSettings(it)
                    nav.popBackStack()
                },
                onAppearance = vm::setAppearance,
                onRestartServer = {
                    vm.restartLocalServer()
                    nav.popBackStack()
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

private fun NavHostController.popBackToChat() {
    if (!popBackStack(Routes.CHAT, inclusive = false)) popBackStack()
}
