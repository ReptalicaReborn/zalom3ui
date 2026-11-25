package com.zalom3ui.app.ui

import android.app.Application
import android.net.Uri
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.zalom3ui.app.data.ApiClient
import com.zalom3ui.app.data.Appearance
import com.zalom3ui.app.data.ChatMessage
import com.zalom3ui.app.data.FoundUser
import com.zalom3ui.app.data.FriendRequest
import com.zalom3ui.app.data.GroupMember
import com.zalom3ui.app.data.Mention
import com.zalom3ui.app.data.MuteOption
import com.zalom3ui.app.data.Person
import com.zalom3ui.app.data.PickedFile
import com.zalom3ui.app.data.StickerInfo
import com.zalom3ui.app.data.readPickedFile
import com.zalom3ui.app.data.ServerEvent
import com.zalom3ui.app.data.ServerSettings
import com.zalom3ui.app.data.SessionStatus
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.data.ThreadRef
import com.zalom3ui.app.data.ThreadSummary
import com.zalom3ui.app.server.LocalServer
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

data class ChatState(
    val kind: String,
    val id: String,
    val name: String,
    val messages: List<ChatMessage> = emptyList(),
    val loading: Boolean = true,
    /** The server may hold older messages than the ones shown. */
    val hasMore: Boolean = false,
    val loadingMore: Boolean = false,
    val sending: Boolean = false,
    /** 1:1 chats: epoch ms the other person was last online, if they share it. */
    val lastOnline: Long? = null,
    /** Group chats: members, loaded on demand for @mentions. */
    val members: List<GroupMember>? = null,
    /** First message that was unread when the chat was opened, for the "unread messages" divider. */
    val firstUnreadId: String? = null,
    /** Photos and files still uploading. */
    val uploading: Int = 0,
) {
    val ref: ThreadRef get() = ThreadRef(kind, id)
}

data class TypingInfo(val name: String, val at: Long)

data class UiState(
    val settings: ServerSettings,
    val appearance: Appearance = Appearance(),
    val status: SessionStatus? = null,
    /** Set while the server cannot be reached; the UI keeps showing cached data. */
    val connectionError: String? = null,
    val threads: List<ThreadSummary> = emptyList(),
    val chat: ChatState? = null,
    /** Message the message-actions screen is showing. */
    val selected: ChatMessage? = null,
    /** Who is typing, by thread key. Entries expire after [TYPING_TIMEOUT_MS]. */
    val typing: Map<String, TypingInfo> = emptyMap(),
    /** One-off error to show as a toast-like message. */
    val error: String? = null,
    /** One-off confirmation ("Friend request sent"), shown the same way. */
    val notice: String? = null,
    val friends: List<Person> = emptyList(),
    val friendRequests: List<FriendRequest>? = null,
    val stickers: StickerSearch = StickerSearch(),
    /** A chat to open, asked for by a tapped notification. */
    val openRequest: ThreadRef? = null,
    /** While the on-phone server starts: when it was asked for, and the latest reason it can't be reached yet. */
    val startingSince: Long = 0,
    val startupError: String? = null,
)

data class StickerSearch(
    val query: String = "",
    val results: List<StickerInfo> = emptyList(),
    val loading: Boolean = false,
)

const val TYPING_TIMEOUT_MS = 6_000L

class AppViewModel(app: Application) : AndroidViewModel(app) {
    private val settingsStore = SettingsStore(app)
    private val _state = MutableStateFlow(UiState(settings = settingsStore.load(), appearance = settingsStore.loadAppearance()))
    val state: StateFlow<UiState> = _state.asStateFlow()

    private var client: ApiClient? = null
    private var liveJob: Job? = null
    private var presenceJob: Job? = null
    private var refreshThreadsJob: Job? = null
    private var refreshThreadsPending = false
    private var restartJob: Job? = null
    private var visible = false
    private var lastTypingSent = 0L
    /** When the live loop last (re)started the on-phone server; errors are expected while it boots. */
    private var localStartedAt = 0L

    init {
        rebuildClient()
    }

    fun setAppearance(appearance: Appearance) {
        settingsStore.saveAppearance(appearance)
        _state.update { it.copy(appearance = appearance) }
    }

    fun saveSettings(settings: ServerSettings) {
        val wasOnWatch = _state.value.settings.onDevice
        restartJob?.cancel()
        restartJob = null
        settingsStore.save(settings)
        if (wasOnWatch && !settings.onDevice) LocalServer.stop(getApplication())
        localStartedAt = 0L
        _state.value = UiState(settings = settingsStore.load(), appearance = _state.value.appearance)
        rebuildClient()
        if (visible) startLiveUpdates()
    }

    /** Restarts the on-phone server, e.g. if it got stuck. */
    fun restartLocalServer() {
        if (!_state.value.settings.onDevice) return
        liveJob?.cancel()
        liveJob = null
        presenceJob?.cancel()
        presenceJob = null
        refreshThreadsJob?.cancel()
        refreshThreadsJob = null
        refreshThreadsPending = false
        localStartedAt = 0L
        LocalServer.stop(getApplication())
        _state.update { it.copy(status = null, connectionError = null, startingSince = 0L, startupError = null) }
        restartJob?.cancel()
        restartJob = viewModelScope.launch {
            // Let the old process exit before starting a new one.
            delay(1_500)
            if (visible) startLiveUpdates()
        }
    }

    fun logout() {
        VoicePlayback.stop()
        launchApi { api ->
            val status = api.logout()
            _state.update { it.copy(status = status, threads = emptyList(), chat = null) }
        }
    }

    /** Hold the WebSocket only while the app is visible, to save battery. */
    fun startLiveUpdates() {
        visible = true
        liveJob?.cancel()
        liveJob = null
        presenceJob?.cancel()
        presenceJob = null
        refreshThreadsJob?.cancel()
        refreshThreadsJob = null
        refreshThreadsPending = false
        val api = client ?: return
        val onDevice = _state.value.settings.onDevice
        if (onDevice) {
            val refused = LocalServer.start(getApplication())
            if (localStartedAt == 0L) localStartedAt = System.currentTimeMillis()
            _state.update {
                it.copy(
                    startingSince = if (it.status == null && it.startingSince == 0L) localStartedAt else it.startingSince,
                    startupError = refused?.let { e -> "Android didn't start the Zalo service: ${e.message ?: e.javaClass.simpleName}" },
                )
            }
        }
        // Repeated while visible: the server forgets a chat nobody confirms is still open.
        presenceJob = viewModelScope.launch {
            while (true) {
                runCatching { api.setPresence(_state.value.chat?.ref) }
                delay(PRESENCE_REFRESH_MS)
            }
        }
        liveJob = viewModelScope.launch {
            while (true) {
                val booting = onDevice && System.currentTimeMillis() - localStartedAt < LOCAL_BOOT_GRACE_MS
                try {
                    refreshAll(api)
                    api.events().collect { onEvent(api, it) }
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    // While the on-phone server boots, keep showing "starting" rather than an error; the starting screen
                    // shows the reason after a while (startupError).
                    val error = if (booting) null else e.message ?: "Cannot reach server"
                    _state.update { it.copy(connectionError = error, startupError = e.message ?: e.javaClass.simpleName) }
                }
                delay(if (booting) LOCAL_BOOT_RETRY_MS else RECONNECT_DELAY_MS)
            }
        }
    }

    fun stopLiveUpdates() {
        visible = false
        liveJob?.cancel()
        liveJob = null
        presenceJob?.cancel()
        presenceJob = null
        refreshThreadsJob?.cancel()
        refreshThreadsJob = null
        refreshThreadsPending = false
        // Out of sight: messages of the open chat notify again.
        val api = client ?: return
        viewModelScope.launch { withContext(NonCancellable) { runCatching { api.setPresence(null) } } }
    }

    fun login() = launchApi { api ->
        val status = api.login()
        _state.update { it.copy(status = status) }
    }

    /** Retry a saved session that couldn't reach Zalo. */
    fun retryLogin() = launchApi { api ->
        val status = api.retry()
        _state.update { it.copy(status = status) }
    }

    fun reconnectZalo() = launchApi { api ->
        val status = api.reconnect()
        _state.update { it.copy(status = status) }
    }

    fun clearError() {
        _state.update { it.copy(error = null) }
    }

    fun clearNotice() {
        _state.update { it.copy(notice = null) }
    }

    /** A tapped notification asks for this chat; the navigation opens it once the chat list is there. */
    fun requestOpen(ref: ThreadRef) {
        _state.update { it.copy(openRequest = ref) }
    }

    fun consumeOpenRequest(): ThreadSummary? {
        val current = _state.value
        val ref = current.openRequest ?: return null
        if (current.threads.isEmpty()) return null
        _state.update { it.copy(openRequest = null) }
        return current.threads.firstOrNull { it.ref == ref }
    }

    // ---- Chat ----

    fun openChat(thread: ThreadSummary) {
        _state.update { it.copy(chat = ChatState(thread.kind, thread.id, thread.name)) }
        if (visible) viewModelScope.launch { runCatching { client?.setPresence(thread.ref) } }
        launchApi(onDone = { updateChat(thread.ref) { it.copy(loading = false) } }) { api ->
            val res = api.messages(thread.kind, thread.id, PAGE_SIZE)
            val unread = _state.value.threads.firstOrNull { it.ref == thread.ref }?.unread ?: thread.unread
            val firstUnread = if (unread > 0) res.messages.filter { !it.isSelf }.takeLast(unread).firstOrNull()?.id else null
            updateChat(thread.ref) {
                it.copy(
                    messages = merge(it.messages, res.messages),
                    loading = false,
                    hasMore = res.messages.size >= PAGE_SIZE,
                    firstUnreadId = firstUnread,
                )
            }
            markRead(api, thread.ref)
            val details = runCatching { api.details(thread.kind, thread.id) }.getOrNull()
            if (details?.lastOnline != null) updateChat(thread.ref) { it.copy(lastOnline = details.lastOnline) }
            if (details?.members != null) updateChat(thread.ref) { it.copy(members = details.members) }
        }
    }

    /** Closes the open chat, unless another one was opened since [ref] was. */
    fun closeChat(ref: ThreadRef? = null) {
        val closing = _state.value.chat?.ref
        _state.update { if (ref == null || it.chat?.ref == ref) it.copy(chat = null) else it }
        if (closing != null && _state.value.chat == null) viewModelScope.launch { runCatching { client?.setPresence(null) } }
    }

    fun loadEarlier() {
        val chat = _state.value.chat ?: return
        val oldest = chat.messages.firstOrNull() ?: return
        if (chat.loadingMore) return
        updateChat(chat.ref) { it.copy(loadingMore = true) }
        launchApi(onDone = { updateChat(chat.ref) { it.copy(loadingMore = false) } }) { api ->
            val res = api.messages(chat.kind, chat.id, PAGE_SIZE, before = oldest.ts)
            updateChat(chat.ref) {
                it.copy(messages = merge(it.messages, res.messages, cap = false), hasMore = res.messages.size >= PAGE_SIZE)
            }
        }
    }

    /**
     * Sends [text], optionally as a reply to [quote]. Each of [mentioned] whose "@Name" still appears in
     * the text becomes an @mention (groups only). [onSent] runs once the server accepted it, so the
     * input keeps the draft if sending fails.
     */
    fun send(text: String, quote: ChatMessage? = null, mentioned: List<GroupMember> = emptyList(), onSent: () -> Unit = {}) {
        val chat = _state.value.chat ?: return
        val body = text.trim()
        if (body.isEmpty()) return
        val mentions = if (chat.kind == "group") findMentions(body, mentioned) else emptyList()

        updateChat(chat.ref) { it.copy(sending = true) }
        launchApi(onDone = { updateChat(chat.ref) { it.copy(sending = false) } }) { api ->
            // The sent message comes back over the WebSocket like any other.
            api.send(chat.kind, chat.id, body, quoteId = quote?.id, mentions = mentions)
            onSent()
        }
    }

    /** Called when the reply input opens, so the other side sees "typing…". */
    fun notifyTyping() {
        val chat = _state.value.chat ?: return
        val now = System.currentTimeMillis()
        if (now - lastTypingSent < 3_000) return
        lastTypingSent = now
        viewModelScope.launch { runCatching { client?.typing(chat.kind, chat.id) } }
    }

    // ---- Call research ----

    /** Shares the server's call log (for docs/calls.md); says so when there is nothing in it yet. */
    fun shareCallLog(onReady: (String) -> Unit) = launchApi { api ->
        val log = api.callLog()
        if (log.isBlank()) {
            _state.update { it.copy(notice = "The call log is empty: make or receive a call first") }
        } else {
            onReady(log)
        }
    }

    fun clearCallLog() = launchApi { api ->
        api.clearCallLog()
        _state.update { it.copy(notice = "Call log cleared") }
    }

    // ---- Photos, files and stickers ----

    /** Sends picked photos / files to the open chat, one by one, in the order picked. */
    fun sendFiles(uris: List<Uri>) {
        val chat = _state.value.chat ?: return
        if (uris.isEmpty()) return
        val app = getApplication<Application>()
        updateChat(chat.ref) { it.copy(uploading = it.uploading + uris.size) }
        viewModelScope.launch {
            var remaining = uris.size
            try {
                for (uri in uris) {
                    var file: PickedFile? = null
                    try {
                        val api = client ?: break
                        file = withContext(Dispatchers.IO) { readPickedFile(app.contentResolver, uri, app.cacheDir) }
                        api.sendAttachment(chat.kind, chat.id, file.name, file.mimeType, file.length, file.width, file.height, file.open)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        _state.update { it.copy(error = e.message ?: "Couldn't send the file") }
                    } finally {
                        file?.let { f -> withContext(NonCancellable + Dispatchers.IO) { runCatching { f.cleanup() } } }
                        remaining--
                        updateChat(chat.ref) { it.copy(uploading = (it.uploading - 1).coerceAtLeast(0)) }
                    }
                }
            } finally {
                if (remaining > 0) {
                    updateChat(chat.ref) { it.copy(uploading = (it.uploading - remaining).coerceAtLeast(0)) }
                }
            }
        }
    }

    fun searchStickers(query: String) {
        val q = query.trim()
        _state.update { it.copy(stickers = it.stickers.copy(query = q, loading = q.isNotEmpty())) }
        if (q.isEmpty()) return
        launchApi(onDone = { _state.update { s -> if (s.stickers.query == q) s.copy(stickers = s.stickers.copy(loading = false)) else s } }) { api ->
            val results = api.stickers(q)
            _state.update { s -> if (s.stickers.query == q) s.copy(stickers = s.stickers.copy(results = results)) else s }
        }
    }

    fun sendSticker(sticker: StickerInfo) {
        val chat = _state.value.chat ?: return
        launchApi { api -> api.sendSticker(chat.kind, chat.id, sticker.id) }
    }

    // ---- Friends ----

    fun loadFriends() = launchApi { api ->
        val friends = api.friends()
        _state.update { it.copy(friends = friends) }
    }

    fun loadFriendRequests() = launchApi { api ->
        val requests = api.friendRequests()
        _state.update { it.copy(friendRequests = requests) }
    }

    fun acceptFriend(request: FriendRequest) = launchApi { api ->
        api.acceptFriend(request.id)
        _state.update { s -> s.copy(friendRequests = s.friendRequests?.filterNot { it.id == request.id }, notice = "You and ${request.name} are now friends") }
        refreshThreads(api)
        _state.update { it.copy(friends = runCatching { api.friends() }.getOrDefault(it.friends)) }
    }

    fun rejectFriend(request: FriendRequest) = launchApi { api ->
        api.rejectFriend(request.id)
        _state.update { s -> s.copy(friendRequests = s.friendRequests?.filterNot { it.id == request.id }) }
    }

    /** Looks someone up by phone; [onResult] gets null when nobody uses that number (the error says so). */
    fun findUser(phone: String, onResult: (FoundUser?) -> Unit) {
        var delivered = false
        launchApi(onDone = { if (!delivered) onResult(null) }) { api ->
            val user = api.findUser(phone)
            delivered = true
            onResult(user)
        }
    }

    fun sendFriendRequest(user: FoundUser, message: String, onSent: () -> Unit) = launchApi { api ->
        api.sendFriendRequest(user.id, message)
        _state.update { it.copy(notice = "Friend request sent to ${user.name}") }
        onSent()
    }

    fun undoFriendRequest(user: FoundUser, onDone: () -> Unit) = launchApi { api ->
        api.undoFriendRequest(user.id)
        onDone()
    }

    /** Opens the 1:1 chat with someone found by phone (it exists once a message is sent). */
    fun openUserChat(user: FoundUser) {
        val existing = _state.value.threads.firstOrNull { it.kind == "user" && it.id == user.id }
        openChat(existing ?: ThreadSummary(id = user.id, kind = "user", name = user.name, avatar = user.avatar))
    }

    // ---- Groups ----

    fun createGroup(name: String, members: List<String>, onDone: () -> Unit = {}, onCreated: (ThreadSummary) -> Unit) =
        launchApi(onDone = onDone) { api ->
            val created = api.createGroup(name, members)
            refreshThreads(api)
            val thread = _state.value.threads.firstOrNull { it.kind == "group" && it.id == created.groupId }
                ?: ThreadSummary(id = created.groupId, kind = "group", name = name.ifBlank { "New group" })
            if (created.failed.isNotEmpty()) {
                _state.update { it.copy(notice = "${notAddedNames(created.failed)} couldn't be added") }
            }
            onCreated(thread)
        }

    fun addMembers(members: List<String>, onFinished: () -> Unit = {}, onDone: () -> Unit) {
        val chat = _state.value.chat
        if (chat == null || chat.kind != "group" || members.isEmpty()) {
            onFinished()
            return
        }
        launchApi(onDone = onFinished) { api ->
            val result = api.addGroupMembers(chat.id, members)
            val added = members.size - result.failed.size
            _state.update {
                it.copy(notice = if (result.failed.isEmpty()) "Added $added ${if (added == 1) "member" else "members"}" else "${notAddedNames(result.failed)} couldn't be added")
            }
            reloadMembers(api, chat.ref)
            onDone()
        }
    }

    fun removeMember(member: GroupMember) {
        val chat = _state.value.chat ?: return
        launchApi { api ->
            api.removeGroupMember(chat.id, member.id)
            updateChat(chat.ref) { it.copy(members = it.members?.filterNot { m -> m.id == member.id }) }
            _state.update { it.copy(notice = "Removed ${member.name}") }
        }
    }

    fun renameGroup(name: String) {
        val chat = _state.value.chat ?: return
        val trimmed = name.trim()
        if (chat.kind != "group" || trimmed.isEmpty()) return
        launchApi { api ->
            api.renameGroup(chat.id, trimmed)
            updateChat(chat.ref) { it.copy(name = trimmed) }
        }
    }

    /** Leaves the open group; [onLeft] closes the chat (the server forgets it). */
    fun leaveGroup(silent: Boolean, onLeft: () -> Unit) {
        val chat = _state.value.chat ?: return
        if (chat.kind != "group") return
        launchApi { api ->
            api.leaveGroup(chat.id, silent)
            _state.update { it.copy(notice = "You left ${chat.name}") }
            onLeft()
            refreshThreads(api)
        }
    }

    private fun notAddedNames(ids: List<String>): String {
        val names = ids.map { id -> _state.value.friends.firstOrNull { it.id == id }?.name ?: _state.value.threads.firstOrNull { it.kind == "user" && it.id == id }?.name ?: id }
        return if (names.size <= 3) names.joinToString(", ") else "${names.take(3).joinToString(", ")} and ${names.size - 3} more"
    }

    private suspend fun reloadMembers(api: ApiClient, ref: ThreadRef) {
        val members = runCatching { api.details(ref.kind, ref.id).members }.getOrNull() ?: return
        updateChat(ref) { it.copy(members = members) }
    }

    fun loadMembers() {
        val chat = _state.value.chat ?: return
        if (chat.kind != "group" || chat.members != null) return
        launchApi { api ->
            val members = api.details(chat.kind, chat.id).members.orEmpty()
            updateChat(chat.ref) { it.copy(members = members) }
        }
    }

    // ---- Message actions ----

    fun select(message: ChatMessage) {
        _state.update { it.copy(selected = message) }
    }

    /** Tapping the reaction we already gave removes it, as in the official app. */
    fun toggleReaction(message: ChatMessage, key: String) {
        val mine = message.reactions.any { it.mine && it.key == key }
        launchApi { api -> api.react(message.threadKind, message.threadId, message.id, if (mine) null else key) }
    }

    fun recall(message: ChatMessage) = launchApi { api ->
        api.recall(message.threadKind, message.threadId, message.id)
    }

    fun deleteForMe(message: ChatMessage) = launchApi { api ->
        api.deleteForMe(message.threadKind, message.threadId, message.id)
    }

    fun forward(message: ChatMessage, target: ThreadSummary, onDone: () -> Unit) = launchApi { api ->
        api.forward(message.threadKind, message.threadId, message.id, listOf(target.ref))
        onDone()
    }

    // ---- Conversation actions ----

    // These work on any conversation (chat list long-press) as well as the open one;
    // the server pushes the resulting thread-list changes.

    fun setPinned(ref: ThreadRef, pinned: Boolean) = launchApi { api -> api.setPinned(ref.kind, ref.id, pinned) }

    fun setMuted(ref: ThreadRef, option: MuteOption?) = launchApi { api -> api.setMuted(ref.kind, ref.id, option) }

    fun markUnread(ref: ThreadRef) = launchApi { api -> api.markUnread(ref.kind, ref.id) }

    fun markThreadRead(ref: ThreadRef) = launchApi { api -> markRead(api, ref) }

    fun markAllRead() = launchApi { api ->
        _state.value.threads.filter { it.unread > 0 }.forEach { markRead(api, it.ref) }
    }

    fun deleteConversation(ref: ThreadRef) = launchApi { api ->
        api.deleteConversation(ref.kind, ref.id)
        updateChat(ref) { it.copy(messages = emptyList(), hasMore = false) }
    }

    // ---- Internals ----

    private fun rebuildClient() {
        val settings = _state.value.settings
        client = when {
            settings.onDevice -> ApiClient(LocalServer.url(getApplication()), settingsStore.localToken())
            settings.isConfigured -> ApiClient(settings.serverUrl, settings.token.ifBlank { null })
            else -> null
        }
    }

    private suspend fun refreshAll(api: ApiClient) {
        val status = api.status()
        val threads = if (status.isLoggedIn) api.threads() else emptyList()
        _state.update { it.copy(status = status, threads = threads, connectionError = null, startupError = null) }
        _state.value.chat?.let { chat ->
            val res = api.messages(chat.kind, chat.id, PAGE_SIZE)
            updateChat(chat.ref) { it.copy(messages = merge(it.messages, res.messages), loading = false) }
        }
    }

    private suspend fun onEvent(api: ApiClient, event: ServerEvent) {
        when (event) {
            is ServerEvent.Status -> {
                val wasLoggedIn = _state.value.status?.isLoggedIn == true
                _state.update { it.copy(status = event.status, connectionError = null) }
                if (event.status.isLoggedIn && !wasLoggedIn) scheduleRefreshThreads(api)
            }
            is ServerEvent.ThreadsChanged -> scheduleRefreshThreads(api)
            is ServerEvent.Message -> onMessage(api, event.message)
            is ServerEvent.MessageUpdated -> onMessageUpdated(event.message)
            is ServerEvent.MessageDeleted -> {
                updateChat(event.thread) { it.copy(messages = it.messages.filterNot { m -> m.id == event.id }) }
                if (_state.value.selected?.id == event.id) _state.update { it.copy(selected = null) }
                scheduleRefreshThreads(api)
            }
            is ServerEvent.Typing -> onTyping(event)
            // Notifications are the service's business (MessageNotifier).
            is ServerEvent.Read, is ServerEvent.CallIncoming, is ServerEvent.CallConnected, is ServerEvent.CallPeer, is ServerEvent.CallEnded -> Unit
        }
    }

    private suspend fun onMessage(api: ApiClient, msg: ChatMessage) {
        val ref = ThreadRef(msg.threadKind, msg.threadId)
        val inOpenChat = _state.value.chat?.ref == ref
        updateChat(ref) { it.copy(messages = merge(it.messages, listOf(msg))) }
        // A new message from someone ends their typing indicator.
        if (!msg.isSelf) _state.update { it.copy(typing = it.typing - ref.key) }

        val known = _state.value.threads.any { it.ref == ref }
        if (!known) {
            scheduleRefreshThreads(api)
        } else {
            _state.update { s ->
                val threads = s.threads.map { t ->
                    if (t.ref != ref) return@map t
                    val unread = if (msg.isSelf || inOpenChat) 0 else t.unread + 1
                    t.copy(lastMessage = msg, unread = unread)
                }
                s.copy(threads = sortThreads(threads))
            }
        }
        if (inOpenChat && !msg.isSelf) markRead(api, ref)
    }

    private fun onMessageUpdated(msg: ChatMessage) {
        val ref = ThreadRef(msg.threadKind, msg.threadId)
        updateChat(ref) { chat -> chat.copy(messages = chat.messages.map { if (it.id == msg.id) msg else it }) }
        _state.update { s ->
            s.copy(
                selected = if (s.selected?.id == msg.id) msg else s.selected,
                threads = s.threads.map { t -> if (t.lastMessage?.id == msg.id) t.copy(lastMessage = msg) else t },
            )
        }
    }

    private fun onTyping(event: ServerEvent.Typing) {
        val at = System.currentTimeMillis()
        val key = event.thread.key
        _state.update { it.copy(typing = it.typing + (key to TypingInfo(event.name, at))) }
        viewModelScope.launch {
            delay(TYPING_TIMEOUT_MS)
            _state.update { s -> if (s.typing[key]?.at == at) s.copy(typing = s.typing - key) else s }
        }
    }

    /** Coalesces WebSocket-driven thread refreshes off the event collector so transient errors don't drop the stream. */
    private fun scheduleRefreshThreads(api: ApiClient) {
        if (refreshThreadsJob?.isActive == true) {
            refreshThreadsPending = true
            return
        }
        refreshThreadsPending = false
        refreshThreadsJob = viewModelScope.launch {
            do {
                refreshThreadsPending = false
                try {
                    refreshThreads(api)
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    // Best-effort refresh from WS events; keep the live collector running.
                }
            } while (refreshThreadsPending)
        }
    }

    private suspend fun refreshThreads(api: ApiClient) {
        val threads = api.threads()
        _state.update { it.copy(threads = threads) }
    }

    private suspend fun markRead(api: ApiClient, ref: ThreadRef) {
        try {
            api.markRead(ref.kind, ref.id)
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
        }
        _state.update { s -> s.copy(threads = s.threads.map { if (it.ref == ref) it.copy(unread = 0) else it }) }
    }

    private fun updateChat(ref: ThreadRef, fn: (ChatState) -> ChatState) {
        _state.update { s ->
            val chat = s.chat
            if (chat != null && chat.ref == ref) s.copy(chat = fn(chat)) else s
        }
    }

    private fun launchApi(onDone: () -> Unit = {}, onError: () -> Unit = {}, block: suspend (ApiClient) -> Unit) {
        val api = client
        if (api == null) {
            onError()
            onDone()
            return
        }
        viewModelScope.launch {
            try {
                block(api)
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(error = e.message ?: "Something went wrong") }
                onError()
            } finally {
                onDone()
            }
        }
    }

    companion object {
        private const val RECONNECT_DELAY_MS = 5_000L
        private const val PRESENCE_REFRESH_MS = 30_000L
        private const val LOCAL_BOOT_GRACE_MS = 45_000L
        private const val LOCAL_BOOT_RETRY_MS = 1_000L
        private const val PAGE_SIZE = 30
        private const val MAX_CHAT_MESSAGES = 300

        /** Newer copies of a message (reactions, recalls, receipts) replace older ones. */
        private fun merge(current: List<ChatMessage>, incoming: List<ChatMessage>, cap: Boolean = true): List<ChatMessage> {
            val merged = (incoming + current).distinctBy { it.id }.sortedBy { it.ts }
            return if (cap) merged.takeLast(maxOf(MAX_CHAT_MESSAGES, current.size)) else merged
        }

        /** Positions of "@Name" for each picked member, in text order, without overlaps. */
        fun findMentions(text: String, members: List<GroupMember>): List<Mention> {
            val found = mutableListOf<Mention>()
            for (m in members.distinctBy { it.id }.sortedByDescending { it.name.length }) {
                val tag = "@${m.name}"
                var from = 0
                while (true) {
                    val pos = text.indexOf(tag, from)
                    if (pos < 0) break
                    from = pos + tag.length
                    if (found.none { pos < it.pos + it.len && it.pos < pos + tag.length }) {
                        found += Mention(uid = m.id, pos = pos, len = tag.length)
                    }
                }
            }
            return found.sortedBy { it.pos }
        }

        fun sortThreads(threads: List<ThreadSummary>): List<ThreadSummary> =
            threads.sortedWith(compareByDescending<ThreadSummary> { it.pinned }.thenByDescending { it.lastMessage?.ts ?: 0L })
    }
}

val ThreadSummary.ref: ThreadRef get() = ThreadRef(kind, id)
