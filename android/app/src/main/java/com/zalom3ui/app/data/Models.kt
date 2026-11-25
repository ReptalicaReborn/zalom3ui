package com.zalom3ui.app.data

import kotlinx.serialization.Serializable

/** Mirrors `SessionStatus` in the server's src/session.ts. */
@Serializable
data class SessionStatus(
    val state: String,
    val qrImage: String? = null,
    val scannedBy: String? = null,
    val ownId: String? = null,
    val ownName: String? = null,
    val error: String? = null,
    /** A saved session couldn't reach Zalo and is being retried; a QR login can replace it. */
    val retrying: Boolean = false,
) {
    val isLoggedIn: Boolean get() = state == "ready" || state == "disconnected"
}

@Serializable
data class CallInfo(
    val video: Boolean = false,
    val missed: Boolean = false,
    val outgoing: Boolean = false,
)

@Serializable
data class StickerRef(
    val id: Long,
    val catId: Long = 0,
    val type: Int = 0,
)

/** Mirrors `Attachment` in the server's src/store.ts. Messages cached by older servers only have title/href/thumb. */
@Serializable
data class Attachment(
    /** "photo", "sticker", "gif", "video", "voice", "file", "link", "contact", "location", "call" or "other". */
    val kind: String? = null,
    val title: String? = null,
    val description: String? = null,
    val href: String? = null,
    val thumb: String? = null,
    val width: Int? = null,
    val height: Int? = null,
    /** Video and voice length in ms. */
    val duration: Long? = null,
    /** File size in bytes. */
    val size: Long? = null,
    val fileExt: String? = null,
    val sticker: StickerRef? = null,
    val userId: String? = null,
    /** Call log entries ("Missed voice call", "Incoming video call"…). */
    val call: CallInfo? = null,
) {
    /** The picture to show for a photo, GIF or sticker (the full image when there is no thumbnail). */
    val image: String? get() = when (kind) {
        "sticker" -> thumb
        "photo", "gif" -> thumb ?: href
        else -> null
    }
}

@Serializable
data class Quote(
    val msgId: String,
    val fromId: String = "",
    val fromName: String = "",
    val text: String = "",
)

@Serializable
data class Mention(
    val uid: String,
    val pos: Int,
    val len: Int,
)

@Serializable
data class ReactionSummary(
    val key: String,
    val emoji: String,
    val count: Int,
    val mine: Boolean = false,
)

/** Mirrors `ChatMessage` in the server's src/store.ts. */
@Serializable
data class ChatMessage(
    val id: String,
    val cliMsgId: String = "",
    val threadId: String,
    val threadKind: String,
    val fromId: String = "",
    val fromName: String = "",
    val isSelf: Boolean = false,
    val ts: Long,
    val msgType: String = "",
    val text: String,
    val attachment: Attachment? = null,
    val quote: Quote? = null,
    val mentions: List<Mention> = emptyList(),
    val recalled: Boolean = false,
    val reactions: List<ReactionSummary> = emptyList(),
    val mentionsMe: Boolean = false,
    /** "sent", "delivered" or "seen"; only on our own messages. */
    val status: String? = null,
    /** Group chats only: how many members have seen it. */
    val seenCount: Int? = null,
) {
    /** Plain text messages ("webchat" in Zalo) are the only kind the server can forward. */
    val isText: Boolean get() = !recalled && attachment == null && msgType == "webchat"
}

/** Mirrors `ThreadSummary` in the server's src/store.ts. */
@Serializable
data class ThreadSummary(
    val id: String,
    val kind: String,
    val name: String,
    val avatar: String = "",
    val lastMessage: ChatMessage? = null,
    val unread: Int = 0,
    val pinned: Boolean = false,
    /** Epoch ms the mute ends, -1 for "until turned off", null when not muted. */
    val mutedUntil: Long? = null,
) {
    val key: String get() = "$kind:$id"
    val isMuted: Boolean get() = mutedUntil != null
}

@Serializable
data class ThreadSummaryInfo(
    val id: String,
    val kind: String,
    val name: String,
)

@Serializable
data class ThreadMessages(
    val thread: ThreadSummaryInfo? = null,
    val messages: List<ChatMessage>,
)

@Serializable
data class GroupMember(
    val id: String,
    val name: String,
)

/** A friend (`GET /friends`). */
@Serializable
data class Person(
    val id: String,
    val name: String,
    val avatar: String = "",
)

/** Someone who asked to be my friend (`GET /friend-requests`). */
@Serializable
data class FriendRequest(
    val id: String,
    val name: String,
    val avatar: String = "",
    val message: String = "",
    val time: Long = 0,
)

/** `GET /users/find?phone=`. */
@Serializable
data class FoundUser(
    val id: String,
    val name: String,
    val avatar: String = "",
    val isFriend: Boolean = false,
    /** I already sent them a friend request. */
    val requested: Boolean = false,
)

@Serializable
data class StickerInfo(
    val id: Long,
    val catId: Long = 0,
    val type: Int = 0,
    val url: String,
)

@Serializable
data class CreatedGroup(
    val groupId: String,
    /** People Zalo did not add (they blocked strangers, or are in too many groups). */
    val failed: List<String> = emptyList(),
)

@Serializable
data class AddMembersResult(
    val failed: List<String> = emptyList(),
)

/** `GET /threads/:kind/:id/info`: last-online for users, members for groups. */
@Serializable
data class ThreadDetails(
    val lastOnline: Long? = null,
    val members: List<GroupMember>? = null,
)

data class ThreadRef(val kind: String, val id: String) {
    val key: String get() = "$kind:$id"
}

/** Zalo's six quick reactions, in the order the official app shows them. */
val QUICK_REACTIONS = listOf(
    "heart" to "❤️",
    "like" to "👍",
    "haha" to "😆",
    "wow" to "😮",
    "cry" to "😭",
    "angry" to "😡",
)

/** Values accepted by `PUT /threads/:kind/:id/mute`. */
enum class MuteOption(val apiValue: String, val label: String) {
    ONE_HOUR("1h", "For 1 hour"),
    FOUR_HOURS("4h", "For 4 hours"),
    UNTIL_8AM("8am", "Until 8 AM"),
    FOREVER("forever", "Until I turn it back on"),
}

/** Someone is calling (`call` event, type "incoming"); answering happens in the official Zalo app. */
@Serializable
data class IncomingCall(
    val callId: String = "",
    val callerId: String = "",
    val video: Boolean = false,
    val group: Boolean = false,
    val groupId: String? = null,
    val ts: Long = 0,
    val callerName: String = "",
    val groupName: String? = null,
)

/** `GET /calls/current`: the call this account is answering or on. [connectedAt] is epoch millis, 0 until connected. */
@Serializable
data class CurrentCall(
    val callId: String = "",
    val callerId: String = "",
    val callerName: String = "",
    val video: Boolean = false,
    /** "connecting" or "connected". */
    val state: String = "",
    val connectedAt: Long = 0,
)

sealed interface ServerEvent {
    data class Status(val status: SessionStatus) : ServerEvent
    /** [notify]: the server thinks a phone should show it (not mine, not muted, not open, not old). */
    data class Message(val message: ChatMessage, val notify: Boolean = false, val threadName: String = "") : ServerEvent
    /** The chat was read (here or in another client): its notification can go. */
    data class Read(val thread: ThreadRef) : ServerEvent
    data class CallIncoming(val call: IncomingCall) : ServerEvent
    /** The call stopped ringing (answered elsewhere, cancelled, ended); [callId] may be empty. */
    data class CallEnded(val callId: String) : ServerEvent
    /** The call was answered and its media is up; [connectedAt] is epoch millis. */
    data class CallConnected(
        val callId: String,
        val callerId: String,
        val callerName: String,
        val video: Boolean,
        val connectedAt: Long,
    ) : ServerEvent
    data class MessageUpdated(val message: ChatMessage) : ServerEvent
    data class MessageDeleted(val thread: ThreadRef, val id: String) : ServerEvent
    data class Typing(val thread: ThreadRef, val uid: String, val name: String) : ServerEvent
    data object ThreadsChanged : ServerEvent
}
