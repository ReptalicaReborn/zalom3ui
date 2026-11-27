package com.zalom3ui.app.ui

import android.app.Application
import android.net.Uri
import androidx.lifecycle.AndroidViewModel
import androidx.lifecycle.viewModelScope
import com.zalom3ui.app.data.ApiClient
import com.zalom3ui.app.data.ApiException
import com.zalom3ui.app.data.Appearance
import com.zalom3ui.app.data.BoardItem
import com.zalom3ui.app.data.ChatAccent
import com.zalom3ui.app.data.ChatMessage
import com.zalom3ui.app.data.CreatePollRequest
import com.zalom3ui.app.data.FoundUser
import com.zalom3ui.app.data.FriendRequest
import com.zalom3ui.app.data.GroupLinkInfo
import com.zalom3ui.app.data.GroupMember
import com.zalom3ui.app.data.HistorySyncEvent
import com.zalom3ui.app.data.HistorySyncStatus
import com.zalom3ui.app.data.Mention
import com.zalom3ui.app.data.MessageSearchHit
import com.zalom3ui.app.data.MuteOption
import com.zalom3ui.app.data.PendingMessage
import com.zalom3ui.app.data.PendingShare
import com.zalom3ui.app.data.Person
import com.zalom3ui.app.data.PickedFile
import com.zalom3ui.app.data.PollDetail
import com.zalom3ui.app.data.Quote
import com.zalom3ui.app.data.ServerEvent
import com.zalom3ui.app.data.ServerSettings
import com.zalom3ui.app.data.SessionStatus
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.data.StickerInfo
import com.zalom3ui.app.data.ThreadRef
import com.zalom3ui.app.data.ThreadSummary
import com.zalom3ui.app.data.UserThreadInfo
import com.zalom3ui.app.data.readPickedFile
import com.zalom3ui.app.server.ConversationShortcuts
import com.zalom3ui.app.server.LocalServer
import com.zalom3ui.app.server.StayConnectedCoordinator
import com.zalom3ui.app.server.ZaloCore
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.NonCancellable
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.MutableSharedFlow
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.SharedFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import java.io.File
import java.util.concurrent.atomic.AtomicLong

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
    /** Optimistic outgoing messages waiting for server confirmation or user retry. */
    val pending: List<PendingMessage> = emptyList(),
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
    /** Requests I sent that are still waiting for an answer. */
    val sentRequests: List<FriendRequest>? = null,
    val stickers: StickerSearch = StickerSearch(),
    /** A chat to open, asked for by a tapped notification. */
    val openRequest: ThreadRef? = null,
    /** While the on-phone server starts: when it was asked for, and the latest reason it can't be reached yet. */
    val startingSince: Long = 0,
    val startupError: String? = null,
    /** "Sync messages from phone": the server's progress, and whether its dialog is open (ui/HistorySync.kt). */
    val historySync: HistorySyncStatus = HistorySyncStatus(),
    val historySyncShown: Boolean = false,
    /** Per-chat draft text, keyed by [ThreadRef.key]. */
    val drafts: Map<String, String> = emptyMap(),
    /** Per-chat accent color overrides, keyed by [ThreadRef.key]. */
    val chatAccents: Map<String, ChatAccent> = emptyMap(),
    /** Content shared into Zalỏ from another app, waiting for the user to pick a target chat. */
    val pendingShare: PendingShare? = null,
    /** Global message search query, hits, and loading state. */
    val messageSearchQuery: String = "",
    val messageSearchResults: List<MessageSearchHit> = emptyList(),
    val messageSearchBusy: Boolean = false,
    /** Message id to scroll to after opening a chat from search results. */
    val scrollToMsgId: String? = null,
    /** Multiple messages queued for batch forwarding. */
    val forwardingMany: List<ChatMessage> = emptyList(),
)

data class StickerSearch(
    val query: String = "",
    val results: List<StickerInfo> = emptyList(),
    val loading: Boolean = false,
)

const val TYPING_TIMEOUT_MS = 6_000L

class AppViewModel(app: Application) : AndroidViewModel(app) {
    private val settingsStore = SettingsStore(app)
    private val _state = MutableStateFlow(
        UiState(
            settings = settingsStore.load(),
            appearance = settingsStore.loadAppearance(),
            drafts = settingsStore.loadDrafts(),
            chatAccents = settingsStore.loadChatAccents(),
        ),
    )
    val state: StateFlow<UiState> = _state.asStateFlow()

    /**
     * Drafts as last typed, by [ThreadRef.key]. A keystroke only changes this map: the disk and [UiState.drafts] (the chat
     * list's "Draft:") follow once typing pauses ([draftSaves]), the chat closes or the app goes to the background.
     */
    private val latestDrafts = HashMap<String, String>(_state.value.drafts)
    private val draftSaves = HashMap<String, Job>()

    private val _hideOfficialAccounts = MutableStateFlow(settingsStore.hideOfficialAccounts)
    /** "Hide official accounts": the chat list hides OA chats behind an "Official accounts (N)" row. */
    val hideOfficialAccounts: StateFlow<Boolean> = _hideOfficialAccounts.asStateFlow()

    private val _reducedMotion = MutableStateFlow(settingsStore.reducedMotion)
    /** User preference for reduced motion (combined with system setting at the theme root). */
    val reducedMotion: StateFlow<Boolean> = _reducedMotion.asStateFlow()

    private val _pollEvents = MutableSharedFlow<Long>(extraBufferCapacity = 32)
    /** Ids of polls that changed (voted on, closed); polls on screen reload themselves. */
    val pollEvents: SharedFlow<Long> = _pollEvents

    private val outbox = OfflineOutbox()
    /** Queued text messages held locally while offline and drained FIFO on reconnect. */
    val outboxEntries: StateFlow<List<OutboxEntry>> = outbox.entries

    private val _disappearingTtl = MutableStateFlow<Map<String, Long?>>(emptyMap())
    /** Per-thread disappearing message TTL in ms, keyed by [ThreadRef.key]. */
    val disappearingTtl: StateFlow<Map<String, Long?>> = _disappearingTtl.asStateFlow()

    private val _groupBoards = MutableStateFlow<Map<String, List<BoardNoteItem>>>(emptyMap())
    /** Pinned notes / board items per group ID. */
    val groupBoards: StateFlow<Map<String, List<BoardNoteItem>>> = _groupBoards.asStateFlow()

    private val _blockedUsers = MutableStateFlow<Set<String>>(emptySet())
    /** Users blocked during this session (for instant Block/Unblock UI toggle). */
    val blockedUsers: StateFlow<Set<String>> = _blockedUsers.asStateFlow()

    private val _groupDeputies = MutableStateFlow<Map<String, Set<String>>>(emptyMap())
    /** Session-tracked deputy user IDs per group ID. */
    val groupDeputies: StateFlow<Map<String, Set<String>>> = _groupDeputies.asStateFlow()

    /** Live upload bytes per chat, from the core's `upload_progress` events. Only touched on the main thread. */
    private var uploads: UploadsByChat = emptyMap()
    private val _uploadProgress = MutableStateFlow<Map<String, Float>>(emptyMap())
    /**
     * Overall upload progress (0..1) by [ThreadRef.key]. Deliberately NOT part of [UiState]: it ticks ~10 times a second
     * and only the "Sending…" note reads it, so the chat does not recompose per tick.
     */
    val uploadProgress: StateFlow<Map<String, Float>> = _uploadProgress.asStateFlow()

    private fun setUploads(next: UploadsByChat) {
        if (next === uploads) return
        uploads = next
        val progress = uploadProgressByChat(next)
        if (progress != _uploadProgress.value) _uploadProgress.value = progress
    }

    private fun onUploadProgress(event: ServerEvent.UploadProgress) {
        // Uploads start from the open chat; ignore leftovers for any other one.
        if (_state.value.chat?.ref != event.thread) return
        setUploads(applyUploadProgress(uploads, event))
    }

    /** Once a chat has nothing uploading, drop any progress the core never closed with `done`. */
    private fun clearUploadsIfIdle(ref: ThreadRef) {
        val chat = _state.value.chat?.takeIf { it.ref == ref }
        if (chat == null || chat.uploading == 0) setUploads(clearUploads(uploads, ref.key))
    }

    private fun pollApi(): ApiClient = client ?: throw ApiException("Not connected to the server")

    suspend fun loadPoll(pollId: Long): PollDetail = pollApi().poll(pollId)

    suspend fun votePoll(pollId: Long, optionIds: List<Long>): PollDetail = pollApi().votePoll(pollId, optionIds)

    suspend fun addPollOptions(pollId: Long, options: List<String>): PollDetail = pollApi().addPollOptions(pollId, options)

    suspend fun lockPoll(pollId: Long): PollDetail = pollApi().lockPoll(pollId)

    /** Creates a poll in the open group chat; the poll message then arrives like any other. */
    suspend fun createPoll(request: CreatePollRequest): PollDetail {
        val chat = _state.value.chat?.takeIf { it.kind == "group" } ?: throw ApiException("Polls can only be created in groups")
        return pollApi().createPoll(chat.id, request)
    }

    private var client: ApiClient? = null
    private var liveJob: Job? = null
    private var presenceJob: Job? = null
    private var refreshThreadsJob: Job? = null
    private var refreshThreadsPending = false
    private var restartJob: Job? = null
    private var messageSearchJob: Job? = null
    private var visible = false
    private var lastTypingSent = 0L
    /** When the live loop last (re)started the on-phone server; errors are expected while it boots. */
    private var localStartedAt = 0L
    private val pendingSeq = AtomicLong(0L)

    init {
        rebuildClient()
    }

    fun setAppearance(appearance: Appearance) {
        settingsStore.saveAppearance(appearance)
        _state.update { it.copy(appearance = appearance) }
    }

    fun setHideOfficialAccounts(hide: Boolean) {
        settingsStore.hideOfficialAccounts = hide
        _hideOfficialAccounts.value = hide
    }

    fun setReducedMotion(enabled: Boolean) {
        settingsStore.reducedMotion = enabled
        _reducedMotion.value = enabled
    }

    /** Called on every keystroke, so it only notes the text; saving it waits for typing to pause. */
    fun setDraft(ref: ThreadRef, text: String) {
        val key = ref.key
        val current = latestDrafts[key].orEmpty()
        if (current == text || (current.isEmpty() && text.isBlank())) return
        if (text.isBlank()) {
            latestDrafts.remove(key)
        } else {
            latestDrafts[key] = text
        }
        draftSaves.remove(key)?.cancel()
        draftSaves[key] = viewModelScope.launch {
            delay(DRAFT_SAVE_DEBOUNCE_MS)
            draftSaves.remove(key)
            commitDraft(key)
        }
    }

    /** The chat's draft as last typed, including text not saved yet. */
    fun draft(ref: ThreadRef): String = latestDrafts[ref.key].orEmpty()

    /** Empties the chat's draft at once (its message was sent), so a pending save can't bring the text back. */
    private fun clearDraft(ref: ThreadRef) {
        val key = ref.key
        draftSaves.remove(key)?.cancel()
        latestDrafts.remove(key)
        commitDraft(key)
    }

    /** Saves every draft still waiting for typing to pause. */
    private fun flushDrafts() {
        if (draftSaves.isEmpty()) return
        val keys = draftSaves.keys.toList()
        draftSaves.values.forEach { it.cancel() }
        draftSaves.clear()
        keys.forEach { commitDraft(it) }
    }

    /** Writes the chat's latest draft to disk and [UiState.drafts], unless they already have it. */
    private fun commitDraft(key: String) {
        val text = latestDrafts[key].orEmpty()
        if (_state.value.drafts[key].orEmpty() == text) return
        settingsStore.saveDraft(key, text)
        _state.update { it.copy(drafts = withDraft(it.drafts, key, text)) }
    }

    fun setChatAccent(ref: ThreadRef, accent: ChatAccent?) {
        val key = ref.key
        settingsStore.saveChatAccent(key, accent)
        _state.update { s ->
            val updated = if (accent == null) s.chatAccents - key else s.chatAccents + (key to accent)
            s.copy(chatAccents = updated)
        }
    }

    fun setPendingShare(share: PendingShare?) {
        val normalized = share?.takeIf { !it.text.isNullOrBlank() || it.uris.isNotEmpty() }
        _state.update { it.copy(pendingShare = normalized) }
    }

    fun deliverPendingShare(ref: ThreadRef, name: String, onOpened: () -> Unit) {
        val share = _state.value.pendingShare ?: return
        _state.update { it.copy(pendingShare = null) }
        val existing = _state.value.threads.firstOrNull { it.ref == ref }
        openChat(existing ?: ThreadSummary(id = ref.id, kind = ref.kind, name = name))
        val text = share.text?.trim()
        if (!text.isNullOrEmpty()) {
            send(text)
        }
        if (share.uris.isNotEmpty()) {
            sendFiles(share.uris)
        }
        onOpened()
    }

    fun searchAllMessages(query: String) {
        messageSearchJob?.cancel()
        messageSearchJob = null
        val trimmed = query.trim()
        if (trimmed.isEmpty()) {
            _state.update {
                it.copy(
                    messageSearchQuery = query,
                    messageSearchResults = emptyList(),
                    messageSearchBusy = false,
                )
            }
            return
        }
        _state.update { it.copy(messageSearchQuery = query, messageSearchBusy = true) }
        messageSearchJob = viewModelScope.launch {
            delay(MESSAGE_SEARCH_DEBOUNCE_MS)
            val api = client
            if (api == null) {
                _state.update { it.copy(messageSearchBusy = false) }
                return@launch
            }
            try {
                val results = api.searchMessages(trimmed)
                _state.update { s ->
                    if (s.messageSearchQuery.trim() == trimmed) {
                        s.copy(messageSearchResults = results, messageSearchBusy = false)
                    } else {
                        s
                    }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { s ->
                    if (s.messageSearchQuery.trim() == trimmed) {
                        s.copy(messageSearchBusy = false, error = e.message ?: "Search failed")
                    } else {
                        s
                    }
                }
            }
        }
    }

    fun openChatAtMessage(ref: ThreadRef, name: String, msgId: String) {
        val existing = _state.value.threads.firstOrNull { it.ref == ref }
        _state.update { it.copy(scrollToMsgId = msgId) }
        openChat(existing ?: ThreadSummary(id = ref.id, kind = ref.kind, name = name))
    }

    fun clearScrollToMsgId() {
        _state.update { it.copy(scrollToMsgId = null) }
    }

    fun saveSettings(settings: ServerSettings) {
        val wasOnWatch = _state.value.settings.onDevice
        restartJob?.cancel()
        restartJob = null
        settingsStore.save(settings)
        if (wasOnWatch && !settings.onDevice) {
            // Leaving the on-phone core: stop its service and the core, so it stops talking to Zalo.
            LocalServer.stop(getApplication())
            viewModelScope.launch(Dispatchers.IO) { ZaloCore.shutdown() }
        }
        localStartedAt = 0L
        // "Stay connected" follows whether the app uses the phone's core at all.
        StayConnectedCoordinator.refreshFromSettings(getApplication())
        _state.value = UiState(
            settings = settingsStore.load(),
            appearance = _state.value.appearance,
            drafts = _state.value.drafts,
            chatAccents = _state.value.chatAccents,
            pendingShare = _state.value.pendingShare,
        )
        rebuildClient()
        if (visible) startLiveUpdates()
    }

    /** Restarts the on-phone service and core, e.g. if it got stuck. */
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
            // Same process: stop the core (flushing the store) and start a fresh one, then go live again.
            withContext(Dispatchers.IO) { ZaloCore.restart(getApplication()) }
            if (visible) startLiveUpdates()
        }
    }

    fun logout() {
        VoicePlayback.stop()
        outbox.clear()
        _disappearingTtl.value = emptyMap()
        _groupBoards.value = emptyMap()
        _blockedUsers.value = emptySet()
        _groupDeputies.value = emptyMap()
        ConversationShortcuts.clearAll(getApplication())
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
            // Only "Stay connected: Always" runs the foreground service; the other modes connect while the app is open.
            val refused = if (settingsStore.loadStay().mode.keepsForegroundService) LocalServer.start(getApplication()) else null
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
        // Going to the background: save drafts now rather than after the typing pause, in case the app is killed.
        flushDrafts()
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

    override fun onCleared() {
        // The scope's pending saves are cancelled by now; save their drafts directly.
        flushDrafts()
        super.onCleared()
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
        // On a tablet this leaves the chat beside the list without closing it: its draft shows in the list at once.
        flushDrafts()
        _state.update { it.copy(chat = ChatState(thread.kind, thread.id, thread.name)) }
        pushShortcutForRef(thread.ref, fallbackName = thread.name, fallbackAvatar = thread.avatar)
        if (visible) viewModelScope.launch { runCatching { client?.setPresence(thread.ref) } }
        launchApi(onDone = { updateChat(thread.ref) { it.copy(loading = false) } }) { api ->
            val res = api.messages(thread.kind, thread.id, PAGE_SIZE)
            val unread = _state.value.threads.firstOrNull { it.ref == thread.ref }?.unread ?: thread.unread
            val firstUnread = if (unread > 0) res.messages.filter { !it.isSelf }.takeLast(unread).firstOrNull()?.id else null
            updateChat(thread.ref) {
                it.copy(
                    name = it.name.ifBlank { res.thread?.name.orEmpty() },
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
        flushDrafts()
        val closing = _state.value.chat?.ref
        _state.update { if (ref == null || it.chat?.ref == ref) it.copy(chat = null, scrollToMsgId = null) else it }
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

    /** Sets (or clears, when null/<=0) the disappearing-message TTL in ms for [ref]. */
    fun setDisappearingTtl(ref: ThreadRef, ttlMs: Long?) {
        _disappearingTtl.update { current ->
            if (ttlMs == null || ttlMs <= 0L) current - ref.key else current + (ref.key to ttlMs)
        }
    }

    fun cancelOutboxEntry(localId: String) {
        outbox.remove(localId)
    }

    fun retryOutbox() {
        flushOutbox()
    }

    fun flushOutbox() {
        val api = client ?: return
        if (_state.value.status?.state != "ready") return
        if (outbox.pendingCount() == 0) return
        viewModelScope.launch {
            val sent = outbox.flushAll { entry ->
                runCatching {
                    sendTextOrLink(api, entry.thread, entry.text, entry.quoteId, entry.mentions, entry.ttl)
                    pushShortcutForRef(entry.thread)
                }
            }
            if (sent > 0) {
                _state.value.chat?.let { chat ->
                    val res = runCatching { api.messages(chat.kind, chat.id, PAGE_SIZE) }.getOrNull()
                    if (res != null) {
                        updateChat(chat.ref) { it.copy(messages = merge(it.messages, res.messages)) }
                    }
                }
            }
        }
    }

    /**
     * Sends [text] in the open chat. When offline (session state != `"ready"`), queues directly into [OfflineOutbox]
     * so it drains FIFO on reconnect. Otherwise shows an optimistic bubble and falls back to [OfflineOutbox] on
     * transient/offline errors.
     */
    fun send(text: String, quote: ChatMessage? = null, mentioned: List<GroupMember> = emptyList(), onSent: () -> Unit = {}) {
        val chat = _state.value.chat ?: return
        val body = text.trim()
        if (body.isEmpty()) return
        val mentions = if (chat.kind == "group") findMentions(body, mentioned) else emptyList()
        val ttl = _disappearingTtl.value[chat.ref.key]
        val now = System.currentTimeMillis()
        if (_state.value.status?.state != "ready" || client == null) {
            outbox.enqueue(
                thread = chat.ref,
                text = body,
                quoteId = quote?.id,
                mentions = mentions,
                ttl = ttl,
                localId = "outbox-$now-${pendingSeq.incrementAndGet()}",
                createdAt = now,
            )
            clearDraft(chat.ref)
            onSent()
            return
        }
        val item = PendingMessage(
            localId = "pending-$now-${pendingSeq.incrementAndGet()}",
            text = body,
            ts = now,
            quote = quote?.let { Quote(msgId = it.id, fromId = it.fromId, fromName = it.fromName, text = it.text) },
            quoteId = quote?.id,
            mentions = mentions,
        )
        updateChat(chat.ref) { it.copy(pending = it.pending + item) }
        clearDraft(chat.ref)
        onSent()
        sendPending(chat.ref, item, ttl)
    }

    fun retryPending(localId: String) {
        val chat = _state.value.chat ?: return
        val item = chat.pending.firstOrNull { it.localId == localId } ?: return
        val ttl = _disappearingTtl.value[chat.ref.key]
        val retrying = item.copy(failed = false, error = null, ts = System.currentTimeMillis())
        updateChat(chat.ref) { c ->
            c.copy(pending = c.pending.map { if (it.localId == localId) retrying else it })
        }
        sendPending(chat.ref, retrying, ttl)
    }

    fun dismissPending(localId: String) {
        val chat = _state.value.chat ?: return
        updateChat(chat.ref) { c ->
            c.copy(pending = c.pending.filterNot { it.localId == localId })
        }
    }

    private fun sendPending(ref: ThreadRef, item: PendingMessage, ttl: Long? = null) {
        val api = client
        if (api == null || _state.value.status?.state != "ready") {
            movePendingToOutbox(ref, item, ttl)
            return
        }
        viewModelScope.launch {
            try {
                sendTextOrLink(api, ref, item.text, item.quoteId, item.mentions, ttl)
                pushShortcutForRef(ref)
                val res = runCatching { api.messages(ref.kind, ref.id, PAGE_SIZE) }.getOrNull()
                updateChat(ref) { chat ->
                    val updatedMessages = if (res != null) merge(chat.messages, res.messages) else chat.messages
                    chat.copy(
                        messages = updatedMessages,
                        pending = chat.pending.filterNot { it.localId == item.localId },
                    )
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (isLikelyOfflineError(e)) {
                    movePendingToOutbox(ref, item, ttl)
                } else {
                    markPendingFailed(ref, item.localId, e.message ?: "Failed to send")
                }
            }
        }
    }

    private suspend fun sendTextOrLink(
        api: ApiClient,
        ref: ThreadRef,
        text: String,
        quoteId: String?,
        mentions: List<Mention>,
        ttl: Long?,
    ) {
        if (quoteId == null && mentions.isEmpty() && isStandaloneHttpUrl(text)) {
            try {
                api.sendLink(ref.kind, ref.id, text, ttl = ttl)
                return
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                if (isLikelyOfflineError(e)) throw e
                // Fall back to regular text message if rich link parsing fails on the server.
            }
        }
        api.send(ref.kind, ref.id, text, quoteId = quoteId, mentions = mentions, ttl = ttl)
    }

    private fun movePendingToOutbox(ref: ThreadRef, item: PendingMessage, ttl: Long?) {
        updateChat(ref) { chat ->
            chat.copy(pending = chat.pending.filterNot { it.localId == item.localId })
        }
        outbox.enqueue(
            thread = ref,
            text = item.text,
            quoteId = item.quoteId,
            mentions = item.mentions,
            ttl = ttl,
            localId = item.localId,
            createdAt = item.ts,
        )
    }

    private fun markPendingFailed(ref: ThreadRef, localId: String, error: String) {
        updateChat(ref) { chat ->
            chat.copy(
                pending = chat.pending.map { p ->
                    if (p.localId == localId) p.copy(failed = true, error = error) else p
                },
            )
        }
    }

    /** Sends a contact card into [ref], honoring any active disappearing timer. */
    fun sendCard(ref: ThreadRef, userId: String, phone: String? = null) {
        val ttl = _disappearingTtl.value[ref.key]
        launchApi { api ->
            api.sendCard(ref.kind, ref.id, userId, phone = phone, ttl = ttl)
            pushShortcutForRef(ref)
            val res = runCatching { api.messages(ref.kind, ref.id, PAGE_SIZE) }.getOrNull()
            if (res != null) {
                updateChat(ref) { it.copy(messages = merge(it.messages, res.messages)) }
            }
            _state.update { it.copy(notice = "Contact shared") }
        }
    }

    /** Sends a rich link preview into [ref], honoring any active disappearing timer. */
    fun sendLink(ref: ThreadRef, link: String, message: String? = null) {
        val ttl = _disappearingTtl.value[ref.key]
        launchApi { api ->
            api.sendLink(ref.kind, ref.id, link, message = message, ttl = ttl)
            pushShortcutForRef(ref)
            val res = runCatching { api.messages(ref.kind, ref.id, PAGE_SIZE) }.getOrNull()
            if (res != null) {
                updateChat(ref) { it.copy(messages = merge(it.messages, res.messages)) }
            }
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

    // ---- Sync messages from the phone (ui/HistorySync.kt) ----

    fun startHistorySync() = launchApi { api ->
        _state.update { it.copy(historySyncShown = true) }
        val status = api.startHistorySync()
        _state.update { it.copy(historySync = status) }
    }

    fun cancelHistorySync() = launchApi { api ->
        _state.update { it.copy(historySyncShown = false) }
        val status = api.cancelHistorySync()
        _state.update { it.copy(historySync = status) }
    }

    fun hideHistorySync() {
        _state.update { it.copy(historySyncShown = false) }
    }

    /** Hands the server's sync log to [onReady], to share when a sync fails. */
    fun shareHistorySyncLog(onReady: (String) -> Unit) = launchApi { api ->
        val status = api.historySyncStatus()
        val lines = listOf("state=${status.state} error=${status.error.orEmpty()}") + status.log
        onReady(lines.joinToString("\n"))
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
                        pushShortcutForRef(chat.ref)
                    } catch (e: CancellationException) {
                        throw e
                    } catch (e: Exception) {
                        _state.update { it.copy(error = e.message ?: "Couldn't send the file") }
                    } finally {
                        file?.let { f -> withContext(NonCancellable + Dispatchers.IO) { runCatching { f.cleanup() } } }
                        remaining--
                        updateChat(chat.ref) { it.copy(uploading = (it.uploading - 1).coerceAtLeast(0)) }
                        clearUploadsIfIdle(chat.ref)
                    }
                }
            } finally {
                if (remaining > 0) {
                    updateChat(chat.ref) { it.copy(uploading = (it.uploading - remaining).coerceAtLeast(0)) }
                    clearUploadsIfIdle(chat.ref)
                }
            }
        }
    }

    /** Uploads a recorded voice message to the open chat, refreshes messages, and deletes the temp file. */
    fun sendVoice(file: File) {
        val chat = _state.value.chat
        if (chat == null) {
            viewModelScope.launch(Dispatchers.IO) { runCatching { file.delete() } }
            return
        }
        updateChat(chat.ref) { it.copy(uploading = it.uploading + 1) }
        viewModelScope.launch {
            try {
                val api = client ?: throw ApiException("Not connected to the server")
                api.sendVoice(chat.ref, file)
                pushShortcutForRef(chat.ref)
                val res = runCatching { api.messages(chat.kind, chat.id, PAGE_SIZE) }.getOrNull()
                if (res != null) {
                    updateChat(chat.ref) { it.copy(messages = merge(it.messages, res.messages)) }
                }
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                _state.update { it.copy(error = e.message ?: "Couldn't send voice message") }
            } finally {
                withContext(NonCancellable + Dispatchers.IO) { runCatching { file.delete() } }
                updateChat(chat.ref) { it.copy(uploading = (it.uploading - 1).coerceAtLeast(0)) }
                clearUploadsIfIdle(chat.ref)
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
        launchApi { api ->
            api.sendSticker(chat.kind, chat.id, sticker.id)
            pushShortcutForRef(chat.ref)
        }
    }

    // ---- Friends ----

    fun loadFriends() = launchApi { api ->
        val friends = api.friends()
        _state.update { it.copy(friends = friends) }
    }

    fun loadFriendRequests() = launchApi { api ->
        val requests = api.friendRequests()
        _state.update { it.copy(friendRequests = requests) }
        // The list of sent requests is a nicety; don't fail the screen when it can't load.
        val sent = runCatching { api.sentFriendRequests() }.getOrNull()
        if (sent != null) _state.update { it.copy(sentRequests = sent) }
    }

    /** Takes back a request I sent, from the contacts list. */
    fun cancelSentRequest(request: FriendRequest) = launchApi { api ->
        api.undoFriendRequest(request.id)
        _state.update { s -> s.copy(sentRequests = s.sentRequests?.filterNot { it.id == request.id }, notice = "Request to ${request.name} taken back") }
    }

    /** Opens the 1:1 chat with a friend, whether or not you have talked yet. */
    fun openFriendChat(person: Person) {
        val existing = _state.value.threads.firstOrNull { it.kind == "user" && it.id == person.id }
        openChat(existing ?: ThreadSummary(id = person.id, kind = "user", name = person.name, avatar = person.avatar))
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

    /** Loads 1:1 user presence (`lastOnline`) and `commonGroups`. */
    suspend fun loadUserInfo(userId: String): UserThreadInfo {
        val info = pollApi().userInfo(userId)
        if (info.lastOnline != null) {
            updateChat(ThreadRef("user", userId)) { it.copy(lastOnline = info.lastOnline) }
        }
        return info
    }

    fun changeFriendAlias(userId: String, alias: String) {
        val trimmed = alias.trim()
        if (trimmed.isEmpty()) return
        launchApi { api ->
            api.changeFriendAlias(userId, trimmed)
            val ref = ThreadRef("user", userId)
            updateChat(ref) { it.copy(name = trimmed) }
            refreshThreads(api)
            _state.update { it.copy(notice = "Nickname updated") }
        }
    }

    fun removeFriendAlias(userId: String) = launchApi { api ->
        api.removeFriendAlias(userId)
        refreshThreads(api)
        val ref = ThreadRef("user", userId)
        val refreshedName = _state.value.threads.firstOrNull { it.ref == ref }?.name
        if (!refreshedName.isNullOrBlank()) {
            updateChat(ref) { it.copy(name = refreshedName) }
        }
        _state.update { it.copy(notice = "Nickname reset") }
    }

    fun removeFriend(userId: String, onDone: () -> Unit = {}) = launchApi { api ->
        api.removeFriend(userId)
        refreshThreads(api)
        _state.update { s ->
            s.copy(
                friends = s.friends.filterNot { it.id == userId },
                notice = "Removed from friends",
            )
        }
        onDone()
    }

    fun blockUser(userId: String, onDone: () -> Unit = {}) = launchApi { api ->
        api.blockUser(userId)
        _blockedUsers.update { it + userId }
        _state.update { it.copy(notice = "User blocked") }
        onDone()
    }

    fun unblockUser(userId: String, onDone: () -> Unit = {}) = launchApi { api ->
        api.unblockUser(userId)
        _blockedUsers.update { it - userId }
        _state.update { it.copy(notice = "User unblocked") }
        onDone()
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
            ConversationShortcuts.removeShortcut(getApplication(), chat.ref)
            _state.update { it.copy(notice = "You left ${chat.name}") }
            onLeft()
            refreshThreads(api)
        }
    }

    suspend fun loadGroupLink(groupId: String): GroupLinkInfo = pollApi().groupLink(groupId)

    suspend fun enableGroupLink(groupId: String): GroupLinkInfo = pollApi().enableGroupLink(groupId)

    suspend fun disableGroupLink(groupId: String) {
        pollApi().disableGroupLink(groupId)
    }

    fun joinGroupLink(link: String, onJoined: (ThreadSummary?) -> Unit = {}) = launchApi { api ->
        val resp = api.joinGroupLink(link)
        refreshThreads(api)
        val groupId = resp.groupId?.takeIf { it.isNotBlank() }
        val thread = if (groupId != null) {
            _state.value.threads.firstOrNull { it.kind == "group" && it.id == groupId }
                ?: ThreadSummary(id = groupId, kind = "group", name = "Group")
        } else {
            null
        }
        _state.update { it.copy(notice = "Joined group") }
        onJoined(thread)
    }

    fun addGroupDeputy(groupId: String, userId: String, memberName: String = "") = launchApi { api ->
        api.addGroupDeputy(groupId, userId)
        _groupDeputies.update { current ->
            current + (groupId to (current[groupId].orEmpty() + userId))
        }
        val label = memberName.ifBlank { "Member" }
        _state.update { it.copy(notice = "$label is now a deputy") }
    }

    fun removeGroupDeputy(groupId: String, userId: String, memberName: String = "") = launchApi { api ->
        api.removeGroupDeputy(groupId, userId)
        _groupDeputies.update { current ->
            current + (groupId to (current[groupId].orEmpty() - userId))
        }
        val label = memberName.ifBlank { "Member" }
        _state.update { it.copy(notice = "Removed $label as deputy") }
    }

    fun loadGroupBoard(groupId: String) {
        val api = client ?: return
        viewModelScope.launch {
            runCatching { api.groupBoard(groupId) }.onSuccess { resp ->
                val items = resp.items.map { it.toBoardNoteItem() }
                _groupBoards.update { it + (groupId to items) }
            }
        }
    }

    fun createGroupNote(groupId: String, title: String, pin: Boolean = true, onDone: () -> Unit = {}) {
        val trimmed = title.trim()
        if (trimmed.isEmpty()) return
        launchApi(onDone = onDone) { api ->
            val created = api.createGroupNote(groupId, trimmed, pin).toBoardNoteItem()
            _groupBoards.update { current ->
                val existing = current[groupId].orEmpty().filterNot { it.id == created.id }
                current + (groupId to (listOf(created) + existing))
            }
            _state.update { it.copy(notice = if (pin) "Note pinned" else "Note created") }
            runCatching { api.groupBoard(groupId) }.onSuccess { resp ->
                _groupBoards.update { it + (groupId to resp.items.map { item -> item.toBoardNoteItem() }) }
            }
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
        _state.update { it.copy(selected = message, forwardingMany = emptyList()) }
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

    fun deleteMessages(messages: List<ChatMessage>) {
        if (messages.isEmpty()) return
        launchApi { api ->
            var firstError: Exception? = null
            val deletedByThread = mutableMapOf<ThreadRef, MutableSet<String>>()
            for (msg in messages) {
                try {
                    api.deleteForMe(msg.threadKind, msg.threadId, msg.id)
                    deletedByThread.getOrPut(ThreadRef(msg.threadKind, msg.threadId)) { mutableSetOf() }.add(msg.id)
                } catch (e: CancellationException) {
                    throw e
                } catch (e: Exception) {
                    if (firstError == null) firstError = e
                }
            }
            for ((ref, ids) in deletedByThread) {
                updateChat(ref) { chat -> chat.copy(messages = chat.messages.filterNot { it.id in ids }) }
            }
            if (firstError != null) throw firstError
        }
    }

    fun selectManyForForward(messages: List<ChatMessage>) {
        _state.update { it.copy(forwardingMany = messages) }
    }

    fun forwardSelectedMany(targets: List<ThreadRef>, onDone: () -> Unit) {
        val messages = _state.value.forwardingMany.filterNot { it.recalled }
        if (messages.isEmpty() || targets.isEmpty()) {
            _state.update { it.copy(forwardingMany = emptyList()) }
            return
        }
        _state.update { it.copy(forwardingMany = emptyList()) }
        launchApi { api ->
            for (msg in messages.sortedBy { it.ts }) {
                api.forward(msg.threadKind, msg.threadId, msg.id, targets)
            }
            onDone()
        }
    }

    fun forward(message: ChatMessage, target: ThreadSummary, onDone: () -> Unit) = launchApi { api ->
        if (message.recalled) return@launchApi
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
        ConversationShortcuts.removeShortcut(getApplication(), ref)
        updateChat(ref) { it.copy(messages = emptyList(), hasMore = false) }
    }

    // ---- Internals ----

    private fun rebuildClient() {
        val settings = _state.value.settings
        client = ApiClient.forSettings(getApplication(), settings)
    }

    private suspend fun refreshAll(api: ApiClient) {
        val status = api.status()
        val threads = if (status.isLoggedIn) api.threads() else emptyList()
        val sync = if (status.isLoggedIn) runCatching { api.historySyncStatus() }.getOrNull() else null
        _state.update {
            it.copy(
                status = status,
                threads = keepUnchanged(it.threads, threads),
                connectionError = null,
                startupError = null,
                historySync = sync ?: it.historySync,
            )
        }
        if (threads.isNotEmpty()) syncShortcutsAsync(threads)
        if (status.state == "ready") flushOutbox()
        _state.value.chat?.let { chat ->
            val res = api.messages(chat.kind, chat.id, PAGE_SIZE)
            updateChat(chat.ref) { it.copy(messages = merge(it.messages, res.messages), loading = false) }
        }
    }

    private suspend fun onEvent(api: ApiClient, event: ServerEvent) {
        when (event) {
            is ServerEvent.Status -> {
                val wasLoggedIn = _state.value.status?.isLoggedIn == true
                val wasReady = _state.value.status?.state == "ready"
                _state.update { it.copy(status = event.status, connectionError = null) }
                if (event.status.isLoggedIn && !wasLoggedIn) scheduleRefreshThreads(api)
                if (event.status.state == "ready" && (!wasReady || outbox.pendingCount() > 0)) {
                    flushOutbox()
                }
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
            is ServerEvent.PollChanged -> _pollEvents.tryEmit(event.pollId)
            is ServerEvent.UploadProgress -> onUploadProgress(event)
            is HistorySyncEvent -> {
                val sync = event.status
                _state.update { it.copy(historySync = sync, historySyncShown = it.historySyncShown || sync.isFinished) }
                // Imported chats and last messages: reload the chat list.
                if (sync.state == "done") scheduleRefreshThreads(api)
            }
            // Notifications are the service's business (MessageNotifier).
            is ServerEvent.Read, is ServerEvent.CallIncoming, is ServerEvent.CallConnected, is ServerEvent.CallPeer, is ServerEvent.CallEnded, is ServerEvent.CallRinging -> Unit
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
        _state.update { it.copy(threads = keepUnchanged(it.threads, threads)) }
        if (threads.isNotEmpty()) syncShortcutsAsync(threads)
    }

    private fun syncShortcutsAsync(threads: List<ThreadSummary>) {
        val app = getApplication<Application>()
        viewModelScope.launch(Dispatchers.IO) {
            runCatching { ConversationShortcuts.syncRecentShortcuts(app, threads) }
        }
    }

    private fun pushShortcutForRef(ref: ThreadRef, fallbackName: String = "", fallbackAvatar: String? = null) {
        val thread = _state.value.threads.firstOrNull { it.ref == ref }
        val openChat = _state.value.chat?.takeIf { it.ref == ref }
        val title = thread?.name?.ifBlank { null }
            ?: openChat?.name?.ifBlank { null }
            ?: fallbackName.ifBlank { null }
            ?: return
        val avatar = thread?.avatar?.takeIf { it.isNotBlank() } ?: fallbackAvatar?.takeIf { it.isNotBlank() }
        val app = getApplication<Application>()
        viewModelScope.launch(Dispatchers.IO) {
            runCatching {
                ConversationShortcuts.pushShortcut(
                    context = app,
                    ref = ref,
                    title = title,
                    avatarUrl = avatar,
                    isGroup = ref.kind == "group",
                )
            }
        }
    }

    private suspend fun markRead(api: ApiClient, ref: ThreadRef) {
        try {
            api.markRead(ref.kind, ref.id)
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
        }
        _state.update { s -> s.copy(threads = s.threads.map { if (it.ref == ref && it.unread != 0) it.copy(unread = 0) else it }) }
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
        private const val MESSAGE_SEARCH_DEBOUNCE_MS = 250L
        /** How long typing must pause before a draft is saved. */
        private const val DRAFT_SAVE_DEBOUNCE_MS = 500L
        private const val PAGE_SIZE = 30
        private const val MAX_CHAT_MESSAGES = 300
        private val STANDALONE_HTTP_URL = Regex("^https?://\\S+$", RegexOption.IGNORE_CASE)

        /** True when [text] is a single standalone `http://` or `https://` URL with no surrounding spaces. */
        internal fun isStandaloneHttpUrl(text: String): Boolean = STANDALONE_HTTP_URL.matches(text.trim())

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

        /** [drafts] with [key]'s draft set to [text] (removed when blank); the same map when that changes nothing. */
        fun withDraft(drafts: Map<String, String>, key: String, text: String): Map<String, String> = when {
            text.isBlank() -> if (key in drafts) drafts - key else drafts
            drafts[key] == text -> drafts
            else -> drafts + (key to text)
        }

        /**
         * [fresh] (just loaded from the server) with each thread that didn't change replaced by its instance in [current],
         * and [current] itself when nothing changed, so the chat list redraws only the rows that did.
         */
        fun keepUnchanged(current: List<ThreadSummary>, fresh: List<ThreadSummary>): List<ThreadSummary> {
            if (current.isEmpty() || fresh.isEmpty()) return fresh
            val byKey = current.associateBy { it.key }
            val merged = fresh.map { t -> byKey[t.key]?.takeIf { it == t } ?: t }
            return if (merged.size == current.size && merged.indices.all { merged[it] === current[it] }) current else merged
        }

        fun sortThreads(threads: List<ThreadSummary>): List<ThreadSummary> =
            threads.sortedWith(compareByDescending<ThreadSummary> { it.pinned }.thenByDescending { it.lastMessage?.ts ?: 0L })
    }
}

val ThreadSummary.ref: ThreadRef get() = ThreadRef(kind, id)

private fun BoardItem.toBoardNoteItem(): BoardNoteItem = BoardNoteItem(
    id = id,
    kind = kind,
    title = title,
    creatorId = creatorId,
    createTime = createTime,
    pinned = pinned,
)
