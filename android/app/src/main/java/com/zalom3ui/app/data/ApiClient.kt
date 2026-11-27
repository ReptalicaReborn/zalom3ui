package com.zalom3ui.app.data

import android.content.Context
import com.zalom3ui.app.server.ZaloCore
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.mapNotNull
import kotlinx.coroutines.withContext
import kotlinx.serialization.Serializable
import kotlinx.serialization.decodeFromString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.add
import kotlinx.serialization.json.addJsonObject
import kotlinx.serialization.json.booleanOrNull
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.longOrNull
import kotlinx.serialization.json.put
import kotlinx.serialization.json.putJsonArray
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.OkHttpClient
import java.io.File
import java.io.IOException
import java.io.InputStream

class ApiException(message: String, val code: Int = 0) : IOException(message)

/**
 * How the app reads the server's JSON. A `null` for a field that has a default takes the default (the server sends
 * `connectedAt: null` before a call connects, for example) instead of failing the whole answer.
 */
internal val AppJson = Json {
    ignoreUnknownKeys = true
    coerceInputValues = true
}

/**
 * Talks to the Zalo core's REST API and event stream, over a [Transport]: HTTP to a Zalo server ([ApiClient] with a
 * server address), or straight into the core running in this process ([onDevice]). Everything above the transport
 * (requests, JSON, error mapping, models) is the same for both.
 */
class ApiClient internal constructor(private val transport: Transport) {
    /** A client for the Zalo server at [serverUrl] (the "A server" mode); [token] is its access token. */
    constructor(serverUrl: String, token: String?) : this(HttpTransport(serverUrl, token))

    private val json = AppJson

    suspend fun status(): SessionStatus = get("status")

    suspend fun threads(): List<ThreadSummary> = get("threads")

    suspend fun login(): SessionStatus = post("login")

    /** Sync messages from the phone (docs/history-sync.md). */
    suspend fun historySyncStatus(): HistorySyncStatus = get("history-sync", "status")

    suspend fun startHistorySync(): HistorySyncStatus = post("history-sync", "start")

    suspend fun cancelHistorySync(): HistorySyncStatus = post("history-sync", "cancel")

    suspend fun reconnect(): SessionStatus = post("reconnect")

    /** Retry a saved session that couldn't reach Zalo, without waiting for the next attempt. */
    suspend fun retry(): SessionStatus = post("retry")

    suspend fun logout(): SessionStatus = post("logout")

    // ---- Stay connected: pause / resume the chat listener ----

    /** Stops the chat listener without logging out (no auto-reconnect while paused). No-op when logged out. */
    suspend fun listenerPause(): SessionStatus = post("listener", "pause")

    /** Starts the chat listener again after [listenerPause]; the replay of missed messages follows. */
    suspend fun listenerResume(): SessionStatus = post("listener", "resume")

    /**
     * Opens or extends the window in which replayed messages newer than [triggerTs] (epoch ms) minus 2 minutes notify.
     * Closes by itself after [ttlMs].
     */
    suspend fun setNotifyWindow(triggerTs: Long, ttlMs: Long) {
        httpPut("notify-window", body = buildJsonObject {
            put("triggerTs", triggerTs)
            put("ttlMs", ttlMs)
        }.toString())
    }

    suspend fun clearNotifyWindow() {
        httpDelete("notify-window")
    }

    // ---- Replay test (docs/replay-test.md) ----

    /** What Zalo replayed at each connect, newest first. Ids, counts and timestamps only. */
    suspend fun replayLog(): String {
        val (code, body) = transport.execute("GET", path("debug", "replay-log")).let { it.code to it.body }
        if (code !in 200..299) throw ApiException("HTTP $code", code)
        return body
    }

    suspend fun clearReplayLog() {
        httpDelete("debug", "replay-log")
    }

    /**
     * The socket log, newest first: one structure-only entry per decoded WebSocket frame (cmd, handler, key names, array
     * lengths, message item types and drop reasons). No message text.
     */
    suspend fun socketLog(): String {
        val (code, body) = transport.execute("GET", path("debug", "socket-log")).let { it.code to it.body }
        if (code !in 200..299) throw ApiException("HTTP $code", code)
        return body
    }

    suspend fun clearSocketLog() {
        httpDelete("debug", "socket-log")
    }

    /** `"null"`, `"newest"` or `"page"`: which `lastId` the replay requests carry. */
    suspend fun replayMode(): String {
        val body = transport.execute("GET", path("debug", "replay-mode")).let { it.body }
        return json.parseToJsonElement(body).jsonObject["mode"]?.jsonPrimitive?.content ?: "null"
    }

    suspend fun setReplayMode(mode: String) {
        httpPut("debug", "replay-mode", body = buildJsonObject { put("mode", mode) }.toString())
    }

    // ---- Polls ----

    suspend fun createPoll(groupId: String, request: CreatePollRequest): PollDetail =
        post("groups", groupId, "polls", body = createPollBody(request).toString())

    suspend fun poll(pollId: Long): PollDetail = get("polls", pollId.toString())

    /** [optionIds] is the whole set this account chooses; empty takes the vote back. */
    suspend fun votePoll(pollId: Long, optionIds: List<Long>): PollDetail =
        post("polls", pollId.toString(), "vote", body = buildJsonObject { putJsonArray("optionIds") { optionIds.forEach { add(JsonPrimitive(it)) } } }.toString())

    suspend fun addPollOptions(pollId: Long, options: List<String>): PollDetail =
        post("polls", pollId.toString(), "options", body = buildJsonObject { putJsonArray("options") { options.forEach { add(JsonPrimitive(it)) } } }.toString())

    suspend fun lockPoll(pollId: Long): PollDetail = post("polls", pollId.toString(), "lock")

    internal fun createPollBody(request: CreatePollRequest): JsonObject = buildJsonObject {
        put("question", request.question.trim())
        putJsonArray("options") { request.options.map { it.trim() }.filter { it.isNotEmpty() }.distinct().forEach { add(JsonPrimitive(it)) } }
        put("allowMultiple", request.allowMultiple)
        put("allowAddOptions", request.allowAddOptions)
        if (request.expiresAt > 0) put("expiresAt", request.expiresAt)
    }

    // ---- Messages ----

    suspend fun messages(kind: String, id: String, limit: Int, before: Long? = null): ThreadMessages =
        get(
            "threads", kind, id, "messages",
            query = listOfNotNull("limit" to limit.toString(), before?.let { "before" to it.toString() }),
        )

    suspend fun send(
        kind: String,
        id: String,
        text: String,
        quoteId: String? = null,
        mentions: List<Mention> = emptyList(),
        ttl: Long? = null,
    ) {
        val body = buildJsonObject {
            put("text", text)
            quoteId?.let { put("quoteId", it) }
            if (mentions.isNotEmpty()) {
                putJsonArray("mentions") {
                    mentions.forEach { m ->
                        addJsonObject {
                            put("uid", m.uid)
                            put("pos", m.pos)
                            put("len", m.len)
                        }
                    }
                }
            }
            ttl?.takeIf { it > 0L }?.let { put("ttl", it) }
        }
        post<Unit>("threads", kind, id, "messages", body = body.toString())
    }

    /** Sends a Zalo contact card into a thread (`POST /api/threads/:kind/:id/card`). */
    suspend fun sendCard(
        kind: String,
        id: String,
        userId: String,
        phone: String? = null,
        ttl: Long? = null,
    ) {
        val body = buildJsonObject {
            put("userId", userId)
            phone?.takeIf { it.isNotBlank() }?.let { put("phone", it) }
            ttl?.takeIf { it > 0L }?.let { put("ttl", it) }
        }
        post<Unit>("threads", kind, id, "card", body = body.toString())
    }

    /** Sends a rich link preview into a thread (`POST /api/threads/:kind/:id/link`). */
    suspend fun sendLink(
        kind: String,
        id: String,
        link: String,
        message: String? = null,
        ttl: Long? = null,
    ) {
        val body = buildJsonObject {
            put("link", link)
            message?.takeIf { it.isNotBlank() }?.let { put("message", it) }
            ttl?.takeIf { it > 0L }?.let { put("ttl", it) }
        }
        post<Unit>("threads", kind, id, "link", body = body.toString())
    }

    /** [reaction] is a reaction key such as "heart", or null to remove ours. */
    suspend fun react(kind: String, id: String, msgId: String, reaction: String?) {
        httpPut("threads", kind, id, "messages", msgId, "reaction", body = buildJsonObject { put("reaction", reaction) }.toString())
    }

    suspend fun recall(kind: String, id: String, msgId: String) {
        post<Unit>("threads", kind, id, "messages", msgId, "recall")
    }

    suspend fun deleteForMe(kind: String, id: String, msgId: String) {
        httpDelete("threads", kind, id, "messages", msgId)
    }

    suspend fun forward(kind: String, id: String, msgId: String, targets: List<ThreadRef>) {
        val body = buildJsonObject {
            putJsonArray("to") {
                targets.forEach { t ->
                    addJsonObject {
                        put("kind", t.kind)
                        put("id", t.id)
                    }
                }
            }
        }
        post<Unit>("threads", kind, id, "messages", msgId, "forward", body = body.toString())
    }

    /**
     * Sends one photo or file. [name] carries the extension that decides how Zalo sends it; photos need [width] and
     * [height]. [open] is read once, as the upload streams.
     */
    suspend fun sendAttachment(
        kind: String,
        id: String,
        name: String,
        mimeType: String?,
        length: Long,
        width: Int? = null,
        height: Int? = null,
        open: () -> InputStream,
    ) {
        val query = listOfNotNull("name" to name, width?.let { "width" to it.toString() }, height?.let { "height" to it.toString() })
        InFlightWork.track {
            execute<Unit>("POST", path("threads", kind, id, "attachments", query = query), ApiBody.Stream(mimeType, length, open), upload = true)
        }
    }

    /** Full-text search across messages on the server (`GET /api/search?q=...&limit=...`). */
    suspend fun searchMessages(query: String, limit: Int = 50): List<MessageSearchHit> =
        get<SearchMessagesResponse>("search", query = listOf("q" to query, "limit" to limit.toString())).results

    internal fun parseSearchResults(body: String): List<MessageSearchHit> =
        json.decodeFromString<SearchMessagesResponse>(body).results

    /** Uploads a recorded voice message (`POST /api/threads/:kind/:id/voice`, multipart field `"voice"`). */
    suspend fun sendVoice(ref: ThreadRef, file: File) {
        InFlightWork.track {
            execute<Unit>("POST", path("threads", ref.kind, ref.id, "voice"), ApiBody.VoiceFile(file, AUDIO_MP4), upload = true)
        }
    }

    suspend fun stickers(query: String): List<StickerInfo> = get("stickers", query = listOf("q" to query))

    suspend fun sendSticker(kind: String, id: String, stickerId: Long) {
        post<Unit>("threads", kind, id, "stickers", body = buildJsonObject { put("id", stickerId) }.toString())
    }

    /** Tells the server which chat is on screen (null: none), so it doesn't notify about it. */
    suspend fun setPresence(ref: ThreadRef?) {
        val body = buildJsonObject {
            if (ref != null) {
                put("kind", ref.kind)
                put("id", ref.id)
            }
        }
        httpPut("presence", body = body.toString())
    }

    // ---- Call research ----

    /** Picks up a ringing call for 30 s without sound, recording the media connection in the call log. */
    suspend fun answerCallTest(callId: String) {
        post<Unit>("calls", callId, "answer-test")
    }

    /** Declines a call ringing on this account (the caller sees it declined). */
    suspend fun declineCall(callId: String) {
        post<Unit>("calls", callId, "decline")
    }

    /** Picks up a ringing call; the server joins the media server and sends a `connected` event. */
    suspend fun answerCall(callId: String) {
        post<Unit>("calls", callId, "answer")
    }

    /** Places a 1:1 call; progress comes as `ringing` / `connected` / `ended` events. */
    suspend fun startCall(userId: String, video: Boolean): CurrentCall =
        post("calls", "start", body = buildJsonObject {
            put("userId", userId)
            put("video", video)
        }.toString())

    /** Ends the call in progress: hangs up when connected, cancels while it rings. */
    suspend fun hangUpCall() {
        post<Unit>("calls", "hangup")
    }

    /** Tells the other side of the call we muted / unmuted (the microphone itself is silenced by the app). */
    suspend fun setCallMuted(muted: Boolean) {
        post<Unit>("calls", "mute", body = buildJsonObject { put("muted", muted) }.toString())
    }

    /** Tells the other side our camera is on / off (the app starts or stops sending video itself). */
    suspend fun setCamera(on: Boolean) {
        post<Unit>("calls", "camera", body = buildJsonObject { put("on", on) }.toString())
    }

    /** The call being answered or connected on this account, or null when there is none. */
    suspend fun currentCall(): CurrentCall? = try {
        get<CurrentCall>("calls", "current")
    } catch (e: ApiException) {
        if (e.code == 404) null else throw e
    }

    /** The server's call log (JSON lines of the call events Zalo sent). */
    suspend fun callLog(): String {
        val (code, body) = transport.execute("GET", path("debug", "call-log")).let { it.code to it.body }
        if (code !in 200..299) throw ApiException("HTTP $code", code)
        return body
    }

    suspend fun clearCallLog() {
        httpDelete("debug", "call-log")
    }

    // ---- Friends ----

    suspend fun friends(): List<Person> = get("friends")

    suspend fun friendRequests(): List<FriendRequest> = get("friend-requests")

    /** Requests I sent that are still unanswered. */
    suspend fun sentFriendRequests(): List<FriendRequest> = get("friend-requests", "sent")

    suspend fun acceptFriend(userId: String) {
        post<Unit>("friend-requests", userId, "accept")
    }

    suspend fun rejectFriend(userId: String) {
        post<Unit>("friend-requests", userId, "reject")
    }

    suspend fun findUser(phone: String): FoundUser = get("users", "find", query = listOf("phone" to phone))

    suspend fun sendFriendRequest(userId: String, message: String) {
        post<Unit>(
            "friend-requests",
            body = buildJsonObject {
                put("userId", userId)
                put("message", message)
            }.toString(),
        )
    }

    suspend fun undoFriendRequest(userId: String) {
        httpDelete("friend-requests", userId)
    }

    suspend fun changeFriendAlias(userId: String, alias: String) {
        httpPut("friends", userId, "alias", body = buildJsonObject { put("alias", alias) }.toString())
    }

    suspend fun removeFriendAlias(userId: String) {
        httpDelete("friends", userId, "alias")
    }

    suspend fun removeFriend(userId: String) {
        httpDelete("friends", userId)
    }

    suspend fun blockUser(userId: String) {
        post<Unit>("users", userId, "block")
    }

    suspend fun unblockUser(userId: String) {
        httpDelete("users", userId, "block")
    }

    // ---- Groups ----

    suspend fun createGroup(name: String, members: List<String>): CreatedGroup =
        post(
            "groups",
            body = buildJsonObject {
                put("name", name)
                putJsonArray("members") { members.forEach { add(JsonPrimitive(it)) } }
            }.toString(),
        )

    suspend fun addGroupMembers(groupId: String, members: List<String>): AddMembersResult =
        post(
            "groups", groupId, "members",
            body = buildJsonObject {
                putJsonArray("members") { members.forEach { add(JsonPrimitive(it)) } }
            }.toString(),
        )

    suspend fun removeGroupMember(groupId: String, userId: String) {
        httpDelete("groups", groupId, "members", userId)
    }

    suspend fun renameGroup(groupId: String, name: String) {
        httpPut("groups", groupId, "name", body = buildJsonObject { put("name", name) }.toString())
    }

    suspend fun leaveGroup(groupId: String, silent: Boolean) {
        post<Unit>("groups", groupId, "leave", body = buildJsonObject { put("silent", silent) }.toString())
    }

    suspend fun groupLink(groupId: String): GroupLinkInfo = get("groups", groupId, "link")

    suspend fun enableGroupLink(groupId: String): GroupLinkInfo = post("groups", groupId, "link")

    suspend fun disableGroupLink(groupId: String) {
        httpDelete("groups", groupId, "link")
    }

    suspend fun joinGroupLink(link: String): JoinGroupLinkResponse =
        post("groups", "join-link", body = buildJsonObject { put("link", link) }.toString())

    suspend fun addGroupDeputy(groupId: String, userId: String) {
        post<Unit>("groups", groupId, "deputies", userId)
    }

    suspend fun removeGroupDeputy(groupId: String, userId: String) {
        httpDelete("groups", groupId, "deputies", userId)
    }

    suspend fun groupBoard(groupId: String, page: Int = 1, count: Int = 20): BoardResponse =
        get("groups", groupId, "board", query = listOf("page" to page.toString(), "count" to count.toString()))

    suspend fun createGroupNote(groupId: String, title: String, pin: Boolean = true): BoardItem =
        post(
            "groups", groupId, "notes",
            body = buildJsonObject {
                put("title", title)
                put("pin", pin)
            }.toString(),
        )

    // ---- Conversations ----

    suspend fun details(kind: String, id: String): ThreadDetails = get("threads", kind, id, "info")

    suspend fun userInfo(id: String): UserThreadInfo = get("threads", "user", id, "info")

    suspend fun lastOnline(id: String): Long? = userInfo(id).lastOnline

    suspend fun markRead(kind: String, id: String) {
        post<Unit>("threads", kind, id, "read")
    }

    suspend fun markUnread(kind: String, id: String) {
        post<Unit>("threads", kind, id, "unread")
    }

    suspend fun typing(kind: String, id: String) {
        post<Unit>("threads", kind, id, "typing")
    }

    suspend fun setPinned(kind: String, id: String, pinned: Boolean) {
        httpPut("threads", kind, id, "pin", body = buildJsonObject { put("pinned", pinned) }.toString())
    }

    /** [option] null unmutes. */
    suspend fun setMuted(kind: String, id: String, option: MuteOption?) {
        httpPut("threads", kind, id, "mute", body = buildJsonObject { put("duration", option?.apiValue) }.toString())
    }

    suspend fun deleteConversation(kind: String, id: String) {
        httpDelete("threads", kind, id)
    }

    /** Live server events. Completes or throws when the stream closes; callers reconnect. */
    fun events(): Flow<ServerEvent> = transport.events().mapNotNull { text -> runCatching { parseEvent(text) }.getOrNull() }

    internal fun parseEvent(text: String): ServerEvent? {
        val obj = json.parseToJsonElement(text).jsonObject
        return when (obj["type"]?.jsonPrimitive?.content) {
            "status" -> ServerEvent.Status(json.decodeFromJsonElement(SessionStatus.serializer(), obj.getValue("status")))
            "message" -> ServerEvent.Message(
                json.decodeFromJsonElement(ChatMessage.serializer(), obj.getValue("message")),
                notify = obj["notify"]?.jsonPrimitive?.booleanOrNull == true,
                threadName = obj.string("threadName"),
            )
            "read" -> ServerEvent.Read(obj.threadRef())
            "call" -> {
                val call = obj["call"]?.jsonObject ?: return null
                when (call.string("type")) {
                    "incoming" -> ServerEvent.CallIncoming(json.decodeFromJsonElement(IncomingCall.serializer(), call.getValue("call")))
                    "ended" -> ServerEvent.CallEnded(call.string("callId"), call.string("act"))
                    "ringing" -> ServerEvent.CallRinging(call.string("callId"))
                    "connected" -> ServerEvent.CallConnected(
                        callId = call.string("callId"),
                        callerId = call.string("callerId"),
                        callerName = call.string("callerName"),
                        video = call["video"]?.jsonPrimitive?.booleanOrNull == true,
                        connectedAt = call["connectedAt"]?.jsonPrimitive?.longOrNull ?: 0L,
                    )
                    "peer" -> ServerEvent.CallPeer(
                        callId = call.string("callId"),
                        cameraOn = call["cameraOn"]?.jsonPrimitive?.booleanOrNull == true,
                        muted = call["muted"]?.jsonPrimitive?.booleanOrNull == true,
                    )
                    else -> null
                }
            }
            "message_updated" ->
                ServerEvent.MessageUpdated(json.decodeFromJsonElement(ChatMessage.serializer(), obj.getValue("message")))
            "message_deleted" -> ServerEvent.MessageDeleted(obj.threadRef(), obj.string("id"))
            "typing" -> ServerEvent.Typing(obj.threadRef(), obj.string("uid"), obj.string("name"))
            "threads" -> ServerEvent.ThreadsChanged
            "upload_progress" -> {
                val uploadId = obj.string("uploadId")
                if (uploadId.isEmpty()) {
                    null
                } else {
                    ServerEvent.UploadProgress(
                        thread = obj.threadRef(),
                        uploadId = uploadId,
                        sent = obj["sent"]?.jsonPrimitive?.longOrNull ?: 0L,
                        total = obj["total"]?.jsonPrimitive?.longOrNull ?: 0L,
                        done = obj["done"]?.jsonPrimitive?.booleanOrNull == true,
                    )
                }
            }
            "poll" -> obj["pollId"]?.jsonPrimitive?.longOrNull?.let { ServerEvent.PollChanged(it) }
            "history_sync" -> HistorySyncEvent(json.decodeFromJsonElement(HistorySyncStatus.serializer(), obj))
            else -> null
        }
    }

    private suspend inline fun <reified T> get(
        vararg segments: String,
        query: List<Pair<String, String>> = emptyList(),
    ): T = execute("GET", path(*segments, query = query))

    private suspend inline fun <reified T> post(
        vararg segments: String,
        body: String = "",
    ): T = execute("POST", path(*segments), jsonBody(body))

    private suspend fun httpPut(vararg segments: String, body: String) {
        execute<Unit>("PUT", path(*segments), jsonBody(body))
    }

    private suspend fun httpDelete(vararg segments: String) {
        execute<Unit>("DELETE", path(*segments))
    }

    private fun jsonBody(text: String) = ApiBody.Bytes(JSON, text.toByteArray(Charsets.UTF_8))

    private fun JsonObject.string(key: String): String = this[key]?.jsonPrimitive?.content.orEmpty()

    private fun JsonObject.threadRef(): ThreadRef = ThreadRef(string("threadKind"), string("threadId"))

    private suspend inline fun <reified T> execute(
        method: String,
        path: String,
        body: ApiBody = ApiBody.None,
        upload: Boolean = false,
    ): T {
        val (code, text) = transport.execute(method, path, body, upload).let { it.code to it.body }
        if (code !in 200..299) {
            val message = runCatching { json.parseToJsonElement(text).jsonObject["error"]?.jsonPrimitive?.content }
                .getOrNull()
            throw ApiException(message ?: "HTTP $code", code)
        }
        if (T::class == Unit::class) return Unit as T
        // Off the main thread: callers run on the UI dispatcher, and the chat list alone can be a large document.
        return withContext(Dispatchers.Default) { json.decodeFromString<T>(text) }
    }

    @Serializable
    private data class SearchMessagesResponse(
        val results: List<MessageSearchHit> = emptyList(),
    )

    companion object {
        private const val JSON = "application/json"
        private const val AUDIO_MP4 = "audio/mp4"

        /** Shared OkHttp client (connection pool + dispatcher) reused by HTTP transport and Coil image loading. */
        internal fun sharedHttpClient(): OkHttpClient = HttpTransport.SHARED_HTTP

        /** The client for the Zalo core running on this phone: direct calls, no socket, port or token. */
        fun onDevice(context: Context): ApiClient {
            val app = context.applicationContext ?: context
            return ApiClient(NativeTransport(ZaloCore, File(app.cacheDir, "uploads")) { ZaloCore.ensureStarted(app) })
        }

        /** The client the settings ask for, or null while a server is chosen but no address is set. */
        fun forSettings(context: Context, settings: ServerSettings = SettingsStore(context).load()): ApiClient? = when {
            settings.onDevice -> onDevice(context)
            settings.isConfigured -> ApiClient(settings.serverUrl, settings.token.ifBlank { null })
            else -> null
        }

        /** The percent-encoded request path and query for `/api/<segments>?<query>`; each segment is encoded whole. */
        internal fun path(vararg segments: String, query: List<Pair<String, String>> = emptyList()): String {
            val url = PATH_BASE.newBuilder()
                .addPathSegment("api")
                .apply { segments.forEach { addPathSegment(it) } }
                .apply { query.forEach { (k, v) -> addQueryParameter(k, v) } }
                .build()
            return url.encodedPath + (url.encodedQuery?.let { "?$it" } ?: "")
        }

        private val PATH_BASE: HttpUrl = "http://localhost/".toHttpUrl()

        /** Accepts "192.168.1.5:3000" or "https://zalo.example.com/" and returns a base URL. */
        fun normalizeServerUrl(input: String): HttpUrl? {
            val trimmed = input.trim().trimEnd('/')
            if (trimmed.isEmpty()) return null
            val withScheme = if ("://" in trimmed) trimmed else "http://$trimmed"
            return withScheme.toHttpUrlOrNull()
        }
    }
}
