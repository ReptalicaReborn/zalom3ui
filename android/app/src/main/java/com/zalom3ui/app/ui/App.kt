package com.zalom3ui.app.ui

import android.widget.Toast
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
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.GroupAdd
import androidx.compose.material.icons.outlined.People
import androidx.compose.material.icons.outlined.PersonAdd
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.navigation.NavHostController
import androidx.navigation.compose.NavHost
import androidx.navigation.compose.composable
import androidx.navigation.compose.rememberNavController

private object Routes {
    const val HOME = "home"
    const val NEW_CHAT = "new_chat"
    const val NEW_GROUP = "new_group"
    const val ADD_MEMBERS = "add_members"
    const val MEMBERS = "members"
    const val FRIEND_REQUESTS = "friend_requests"
    const val ADD_FRIEND = "add_friend"
    const val CHAT = "chat"
    const val FORWARD = "forward"
    const val SETTINGS = "settings"
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
    // A tapped notification: open its chat once the chat list knows it.
    val openState by vm.state.collectAsStateWithLifecycle()
    LaunchedEffect(openState.openRequest, openState.threads) {
        if (openState.openRequest == null) return@LaunchedEffect
        val thread = vm.consumeOpenRequest() ?: return@LaunchedEffect
        vm.openChat(thread)
        nav.popBackStack(Routes.HOME, inclusive = false)
        nav.navigate(Routes.CHAT)
    }

    NavHost(
        navController = nav,
        startDestination = Routes.HOME,
        enterTransition = { slideInHorizontally { it / 4 } + fadeIn() },
        exitTransition = { fadeOut() },
        popEnterTransition = { fadeIn() },
        popExitTransition = { slideOutHorizontally { it / 4 } + fadeOut() },
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
                status == null -> LoadingScreen(if (state.settings.onDevice) "Starting Zalo…" else "Connecting…")
                !status.isLoggedIn -> LoginScreen(
                    status = status,
                    onLogin = vm::login,
                    onRetry = vm::retryLogin,
                    onSettings = { nav.navigate(Routes.SETTINGS) },
                )
                else -> ThreadListScreen(
                    state = state,
                    actions = ThreadListActions(
                        onOpen = { thread ->
                            vm.openChat(thread)
                            nav.navigate(Routes.CHAT)
                        },
                        onNewChat = { nav.navigate(Routes.NEW_CHAT) },
                        onReconnect = vm::reconnectZalo,
                        onSettings = { nav.navigate(Routes.SETTINGS) },
                        onMarkAllRead = vm::markAllRead,
                        onPin = { thread, pinned -> vm.setPinned(thread.ref, pinned) },
                        onMute = { thread, option -> vm.setMuted(thread.ref, option) },
                        onMarkRead = { vm.markThreadRead(it.ref) },
                        onMarkUnread = { vm.markUnread(it.ref) },
                        onDelete = { vm.deleteConversation(it.ref) },
                    ),
                )
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
                    nav.popBackStack()
                    nav.navigate(Routes.CHAT)
                },
                actions = listOf(
                    PickerAction("New group", Icons.Outlined.GroupAdd) { nav.navigate(Routes.NEW_GROUP) },
                    PickerAction("Add friend", Icons.Outlined.PersonAdd) { nav.navigate(Routes.ADD_FRIEND) },
                    PickerAction("Friend requests", Icons.Outlined.People, badge = state.friendRequests?.size ?: 0) {
                        nav.navigate(Routes.FRIEND_REQUESTS)
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
                    vm.createGroup(name, ids) { thread ->
                        busy = false
                        vm.openChat(thread)
                        nav.popBackStack(Routes.HOME, inclusive = false)
                        nav.navigate(Routes.CHAT)
                    }
                },
            )
            // A failed create shows its error and leaves the picks as they were.
            LaunchedEffect(state.error) { if (state.error != null) busy = false }
        }
        composable(Routes.FRIEND_REQUESTS) {
            val state by vm.state.collectAsStateWithLifecycle()
            FriendRequestsScreen(
                requests = state.friendRequests,
                onLoad = vm::loadFriendRequests,
                onBack = { nav.popBackStack() },
                onAccept = vm::acceptFriend,
                onReject = vm::rejectFriend,
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
                    nav.popBackStack(Routes.HOME, inclusive = false)
                    nav.navigate(Routes.CHAT)
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
                    nav.popBackStack(Routes.HOME, inclusive = false)
                    nav.navigate(Routes.CHAT)
                },
                onRemove = vm::removeMember,
            )
        }
        composable(Routes.ADD_MEMBERS) {
            val state by vm.state.collectAsStateWithLifecycle()
            var busy by remember { mutableStateOf(false) }
            LaunchedEffect(Unit) { vm.loadMembers() }
            LaunchedEffect(state.error) { if (state.error != null) busy = false }
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
                    vm.addMembers(ids) {
                        busy = false
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
                    if (event == Lifecycle.Event.ON_DESTROY) vm.closeChat(ref)
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
                    actions = ChatActions(
                        onBack = { nav.popBackStack() },
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
                            nav.popBackStack(Routes.HOME, inclusive = false)
                        },
                        onDeleteConversation = { vm.deleteConversation(chat.ref) },
                        onSendFiles = vm::sendFiles,
                        onSearchStickers = vm::searchStickers,
                        onSendSticker = vm::sendSticker,
                        onShowMembers = { nav.navigate(Routes.MEMBERS) },
                        onAddMembers = { nav.navigate(Routes.ADD_MEMBERS) },
                        onRenameGroup = vm::renameGroup,
                        onLeaveGroup = { silent ->
                            vm.leaveGroup(silent) {
                                vm.closeChat(chat.ref)
                                nav.popBackStack(Routes.HOME, inclusive = false)
                            }
                        },
                    ),
                )
            }
        }
        composable(Routes.FORWARD) {
            val state by vm.state.collectAsStateWithLifecycle()
            val msg = state.selected
            ContactsScreen(
                title = "Forward to",
                threads = state.threads,
                onBack = { nav.popBackStack() },
                onPick = { target ->
                    if (msg != null) {
                        vm.forward(msg, target, onDone = {
                            Toast.makeText(context, "Forwarded to ${target.name}", Toast.LENGTH_SHORT).show()
                        })
                    }
                    nav.popBackToChat()
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
                onShareCallLog = { vm.shareCallLog { log -> QrActions.shareCallLog(context, log) } },
                onClearCallLog = vm::clearCallLog,
            )
        }
    }
}

private fun NavHostController.popBackToChat() {
    if (!popBackStack(Routes.CHAT, inclusive = false)) popBackStack()
}
