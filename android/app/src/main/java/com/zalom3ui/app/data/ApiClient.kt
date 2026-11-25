package com.zalom3ui.app.data

import kotlinx.coroutines.channels.awaitClose
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.callbackFlow
import kotlinx.coroutines.suspendCancellableCoroutine
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
import okhttp3.Call
import okhttp3.Callback
import okhttp3.HttpUrl
import okhttp3.HttpUrl.Companion.toHttpUrlOrNull
import okhttp3.MediaType
import okhttp3.MediaType.Companion.toMediaType
import okhttp3.MediaType.Companion.toMediaTypeOrNull
import okhttp3.OkHttpClient
import okhttp3.Request
import okhttp3.RequestBody
import okhttp3.RequestBody.Companion.toRequestBody
import okhttp3.Response
import okhttp3.WebSocket
import okhttp3.WebSocketListener
import okio.BufferedSink
import okio.source
import java.io.IOException
import java.io.InputStream
import java.util.concurrent.TimeUnit
import kotlin.coroutines.resume
import kotlin.coroutines.resumeWithException

class ApiException(message: String, val code: Int = 0) : IOException(message)

/** Talks to the Zalỏ server's REST API and WebSocket. */
class ApiClient(serverUrl: String, private val token: String?) {
    private val baseUrl: HttpUrl = normalizeServerUrl(serverUrl)
        ?: throw IllegalArgumentException("Invalid server URL: $serverUrl")

    private val json = Json { ignoreUnknownKeys = true }

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

    suspend fun send(kind: String, id: String, text: String, quoteId: String? = null, mentions: List<Mention> = emptyList()) {
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
        }
        post<Unit>("threads", kind, id, "messages", body = body.toString())
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
        val body = StreamBody(mimeType?.toMediaTypeOrNull() ?: OCTET_STREAM, length, open)
        execute<Unit>(Request.Builder().url(url("threads", kind, id, "attachments", query = query)).post(body), UPLOAD_HTTP)
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
        val (code, body) = executeRaw(Request.Builder().url(url("debug", "call-log")).get())
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

    // ---- Conversations ----

    suspend fun details(kind: String, id: String): ThreadDetails = get("threads", kind, id, "info")

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

    internal fun eventsRequest(): Request {
        val wsUrl = baseUrl.newBuilder()
            .addPathSegment("ws")
            .build()
        return Request.Builder()
            .url(wsUrl.toString().replaceFirst("http", "ws"))
            .apply { token?.let { header("Authorization", "Bearer $it") } }
            .build()
    }

    /** Live server events. Completes or throws when the socket closes; callers reconnect. */
    fun events(): Flow<ServerEvent> = callbackFlow {
        val request = eventsRequest()

        val socket = http.newWebSocket(request, object : WebSocketListener() {
            override fun onMessage(webSocket: WebSocket, text: String) {
                runCatching { parseEvent(text) }.getOrNull()?.let { trySend(it) }
            }

            override fun onClosed(webSocket: WebSocket, code: Int, reason: String) {
                close()
            }

            override fun onFailure(webSocket: WebSocket, t: Throwable, response: Response?) {
                close(if (response?.code == 401) ApiException("Wrong access token", 401) else t)
            }
        })
        awaitClose { socket.close(1000, null) }
    }

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
            "poll" -> obj["pollId"]?.jsonPrimitive?.longOrNull?.let { ServerEvent.PollChanged(it) }
            "history_sync" -> HistorySyncEvent(json.decodeFromJsonElement(HistorySyncStatus.serializer(), obj))
            else -> null
        }
    }

    private suspend inline fun <reified T> get(
        vararg segments: String,
        query: List<Pair<String, String>> = emptyList(),
    ): T = execute(Request.Builder().url(url(*segments, query = query)).get())

    private suspend inline fun <reified T> post(
        vararg segments: String,
        body: String = "",
    ): T = execute(Request.Builder().url(url(*segments)).post(body.toRequestBody(JSON_MEDIA_TYPE)))

    private suspend fun httpPut(vararg segments: String, body: String) {
        execute<Unit>(Request.Builder().url(url(*segments)).put(body.toRequestBody(JSON_MEDIA_TYPE)))
    }

    private suspend fun httpDelete(vararg segments: String) {
        execute<Unit>(Request.Builder().url(url(*segments)).delete())
    }

    private fun JsonObject.string(key: String): String = this[key]?.jsonPrimitive?.content.orEmpty()

    private fun JsonObject.threadRef(): ThreadRef = ThreadRef(string("threadKind"), string("threadId"))

    private suspend fun executeRaw(builder: Request.Builder, client: OkHttpClient = http): Pair<Int, String> =
        suspendCancellableCoroutine { cont ->
            token?.let { builder.header("Authorization", "Bearer $it") }
            val call = client.newCall(builder.build())
            cont.invokeOnCancellation { call.cancel() }
            call.enqueue(object : Callback {
                override fun onFailure(call: Call, e: IOException) {
                    if (cont.isActive) cont.resumeWithException(e)
                }

                override fun onResponse(call: Call, response: Response) {
                    try {
                        response.use { res ->
                            val body = res.body?.string().orEmpty()
                            if (cont.isActive) cont.resume(res.code to body)
                        }
                    } catch (e: Throwable) {
                        if (cont.isActive) cont.resumeWithException(e)
                    }
                }
            })
        }

    private suspend inline fun <reified T> execute(builder: Request.Builder, client: OkHttpClient = http): T {
        val (code, body) = executeRaw(builder, client)
        if (code !in 200..299) {
            val message = runCatching { json.parseToJsonElement(body).jsonObject["error"]?.jsonPrimitive?.content }
                .getOrNull()
            throw ApiException(message ?: "HTTP $code", code)
        }
        return if (T::class == Unit::class) Unit as T else json.decodeFromString<T>(body)
    }

    internal fun url(vararg segments: String, query: List<Pair<String, String>> = emptyList()): HttpUrl =
        baseUrl.newBuilder()
            .addPathSegment("api")
            .apply { segments.forEach { addPathSegment(it) } }
            .apply { query.forEach { (k, v) -> addQueryParameter(k, v) } }
            .build()

    /** Streams a picked photo or file from the content resolver instead of loading it into memory. */
    private class StreamBody(private val type: MediaType, private val length: Long, private val open: () -> InputStream) : RequestBody() {
        override fun contentType(): MediaType = type

        override fun contentLength(): Long = length

        override fun writeTo(sink: BufferedSink) {
            open().source().use { sink.writeAll(it) }
        }
    }

    companion object {
        private val JSON_MEDIA_TYPE = "application/json".toMediaType()
        private val OCTET_STREAM = "application/octet-stream".toMediaType()

        // Shared so the connection pool survives client rebuilds after settings changes.
        private val http = OkHttpClient.Builder()
            .connectTimeout(10, TimeUnit.SECONDS)
            .readTimeout(30, TimeUnit.SECONDS)
            .pingInterval(30, TimeUnit.SECONDS)
            .build()

        /** Uploads go to Zalo before the server answers: give them minutes, not seconds. */
        private val UPLOAD_HTTP = http.newBuilder()
            .writeTimeout(5, TimeUnit.MINUTES)
            .readTimeout(5, TimeUnit.MINUTES)
            .build()

        /** Accepts "192.168.1.5:3000" or "https://zalo.example.com/" and returns a base URL. */
        fun normalizeServerUrl(input: String): HttpUrl? {
            val trimmed = input.trim().trimEnd('/')
            if (trimmed.isEmpty()) return null
            val withScheme = if ("://" in trimmed) trimmed else "http://$trimmed"
            return withScheme.toHttpUrlOrNull()
        }
    }
}
