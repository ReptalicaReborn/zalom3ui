package com.zalom3ui.app.server

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Bitmap
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.SystemClock
import android.widget.Toast
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.Person
import androidx.core.app.RemoteInput
import androidx.core.content.FileProvider
import androidx.core.graphics.drawable.IconCompat
import androidx.core.graphics.drawable.toBitmap
import coil.imageLoader
import coil.request.ImageRequest
import coil.request.SuccessResult
import coil.size.Scale
import com.zalom3ui.app.MainActivity
import com.zalom3ui.app.R
import com.zalom3ui.app.call.CallPopup
import com.zalom3ui.app.data.ApiClient
import com.zalom3ui.app.data.ChatMessage
import com.zalom3ui.app.data.IncomingCall
import com.zalom3ui.app.data.ServerEvent
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.data.ThreadRef
import com.zalom3ui.app.ui.call.CallActivity
import java.io.File
import java.util.concurrent.ConcurrentHashMap
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.joinAll
import kotlinx.coroutines.launch
import kotlinx.coroutines.withTimeoutOrNull

/**
 * Shows a notification for each new message the on-phone server flags (`notify`: not mine, not muted, not the chat on
 * screen, not replayed history unless the bridge's notify window is open), one per chat with its recent lines, and
 * removes it once the chat is read anywhere. Runs in [ZaloService] ("Always"), or while Zalỏ is online in the other
 * "Stay connected" modes (see [StayConnectedCoordinator]).
 */
class MessageNotifier(
    private val context: Context,
    /** Called after a message notification for the chat was posted, with the chat's title (the bridge hides Zalo's own). */
    private val onPosted: (ThreadRef, String) -> Unit = { _, _ -> },
) {
    private val stateLock = Any()

    private val history = object : LinkedHashMap<String, ArrayDeque<Line>>(MAX_THREADS, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, ArrayDeque<Line>>?): Boolean =
            size > MAX_THREADS
    }

    private data class Line(
        val id: String,
        val sender: String,
        val senderId: String,
        val text: String,
        val ts: Long,
        val mediaUri: Uri? = null,
        val mediaMime: String? = null,
    )

    /** The latest one-time code in each chat's notification, kept (10 min) until the chat is read, so "Copy" survives newer lines. */
    private val otpByThread = HashMap<String, Pair<String, Long>>()

    /** Monotonic counter per thread so cancelled or superseded async notification jobs never post stale alerts. */
    private val generationByThread = HashMap<String, Long>()

    /** Cached thread avatar URLs keyed by `ThreadRef.key`, refreshed lazily from the on-device core. */
    private val avatarUrls = ConcurrentHashMap<String, String>()
    @Volatile
    private var lastThreadsFetchMs = 0L

    /** Thread keys of Official Accounts, refreshed with the thread list; read when "Hide official accounts" is on. */
    private val officialAccountKeys = ConcurrentHashMap.newKeySet<String>()
    private val settings by lazy { SettingsStore(context) }

    private var scope: CoroutineScope? = null

    /** Listens until [scope] ends; reconnects while the server starts or restarts. */
    fun start(scope: CoroutineScope) {
        this.scope = scope
        ensureChannel(context)
        scope.launch {
            while (isActive) {
                try {
                    val api = ApiClient.onDevice(context)
                    launch(Dispatchers.IO) { refreshAvatarUrls(force = true) }
                    api.events().collect { onEvent(it) }
                } catch (e: CancellationException) {
                    throw e
                } catch (_: Exception) {
                    // The server is still booting, or restarted.
                }
                delay(RECONNECT_MS)
            }
        }
    }

    private fun onEvent(event: ServerEvent) {
        when (event) {
            is ServerEvent.Message -> if (event.notify) showUnlessOfficial(event.message, event.threadName)
            is ServerEvent.Read -> cancel(event.thread)
            is ServerEvent.CallIncoming -> showCall(event.call)
            is ServerEvent.CallEnded -> endCall(event.callId)
            is ServerEvent.ThreadsChanged -> scope?.launch(Dispatchers.IO) { refreshAvatarUrls(force = false) }
            else -> Unit
        }
    }

    /** With "Hide official accounts" on, an Official Account's message posts no notification. */
    private fun showUnlessOfficial(msg: ChatMessage, threadName: String) {
        if (msg.threadKind != "user" || !settings.hideOfficialAccounts) {
            show(msg, threadName)
            return
        }
        val key = ThreadRef(msg.threadKind, msg.threadId).key
        val targetScope = scope ?: CoroutineScope(SupervisorJob() + Dispatchers.IO)
        targetScope.launch(Dispatchers.IO) {
            // The core flags the chat before it announces the message, so a fresh list already knows a new OA.
            if (key !in officialAccountKeys) refreshAvatarUrls(force = true)
            if (key !in officialAccountKeys) show(msg, threadName)
        }
    }

    private suspend fun refreshAvatarUrls(force: Boolean) {
        val now = SystemClock.elapsedRealtime()
        if (!force && now - lastThreadsFetchMs < THREADS_REFRESH_MIN_MS) return
        lastThreadsFetchMs = now
        val threads = runCatching { ApiClient.onDevice(context).threads() }.getOrNull() ?: return
        for (t in threads) {
            if (t.avatar.isNotBlank()) avatarUrls[t.key] = t.avatar
            if (t.isOfficialAccount) officialAccountKeys.add(t.key) else officialAccountKeys.remove(t.key)
        }
    }

    private fun show(msg: ChatMessage, threadName: String) {
        val ref = ThreadRef(msg.threadKind, msg.threadId)
        val isGroup = ref.kind == "group"
        val displayTitle = threadName.ifBlank { msg.fromName.ifBlank { "Chat" } }
        val senderName = msg.fromName.ifBlank { displayTitle }
        val otp = OtpText.messageCode(msg)
        val text = notificationText(msg)
        val lineText = if (otp != null) OtpText.line(otp, text) else text
        val newLine = Line(
            id = msg.id,
            sender = senderName,
            senderId = msg.fromId,
            text = lineText,
            ts = msg.ts,
        )

        val linesSnapshot: List<Line>
        val code: String?
        val gen: Long
        synchronized(stateLock) {
            val lines = history.getOrPut(ref.key) { ArrayDeque() }
            if (otp != null) {
                otpByThread[ref.key] = otp to System.currentTimeMillis()
                OtpNotifications.dedupe.noteNotified(otp, System.currentTimeMillis())
            }
            lines.addLast(newLine)
            while (lines.size > MAX_LINES) lines.removeFirst()
            linesSnapshot = lines.toList()
            code = otpByThread[ref.key]
                ?.takeIf { System.currentTimeMillis() - it.second <= OtpDedupe.RESCUE_MEMORY_MS }
                ?.first
            gen = (generationByThread[ref.key] ?: 0L) + 1L
            generationByThread[ref.key] = gen
        }

        val targetScope = scope ?: CoroutineScope(SupervisorJob() + Dispatchers.IO)
        targetScope.launch(Dispatchers.IO) {
            val photoUrl = if (msg.attachment?.kind == "photo") {
                msg.attachment.remoteThumb ?: msg.attachment.remoteHref
            } else {
                null
            }

            var threadIcon: IconCompat? = null
            var senderIcon: IconCompat? = null
            var shortcutPushed = false
            var photoUri: Uri? = null

            withTimeoutOrNull(FETCH_TIMEOUT_MS) {
                coroutineScope {
                    val avatarJob = launch {
                        if (!avatarUrls.containsKey(ref.key)) {
                            refreshAvatarUrls(force = false)
                        }
                        val threadAvatarUrl = avatarUrls[ref.key]
                        val senderAvatarUrl = if (isGroup && msg.fromId.isNotBlank()) {
                            avatarUrls["user:${msg.fromId}"]
                        } else {
                            threadAvatarUrl
                        }
                        threadIcon = ConversationShortcuts.loadCircularAvatarIcon(context, threadAvatarUrl)
                        senderIcon = if (senderAvatarUrl != null && senderAvatarUrl != threadAvatarUrl) {
                            ConversationShortcuts.loadCircularAvatarIcon(context, senderAvatarUrl)
                        } else {
                            threadIcon
                        }
                        ConversationShortcuts.pushShortcut(
                            context = context,
                            ref = ref,
                            title = displayTitle,
                            avatarUrl = threadAvatarUrl,
                            isGroup = isGroup,
                        )
                        shortcutPushed = true
                    }
                    val mediaJob = launch {
                        if (photoUrl != null) {
                            photoUri = fetchNotificationPhotoPreview(photoUrl, msg.id)
                        }
                    }
                    joinAll(avatarJob, mediaJob)
                }
            }

            // Guarantee the conversation shortcut exists even if remote avatar fetch timed out.
            if (!shortcutPushed) {
                ConversationShortcuts.pushShortcut(
                    context = context,
                    ref = ref,
                    title = displayTitle,
                    avatarUrl = null,
                    isGroup = isGroup,
                )
            }

            val finalLines = if (photoUri != null) {
                synchronized(stateLock) {
                    val current = history[ref.key]
                    if (current != null && current.isNotEmpty()) {
                        val lastIdx = current.indexOfLast { it.id == msg.id && it.ts == msg.ts }
                        if (lastIdx >= 0) {
                            val list = current.toMutableList()
                            list[lastIdx] = list[lastIdx].copy(mediaUri = photoUri, mediaMime = "image/jpeg")
                            current.clear()
                            current.addAll(list)
                        }
                    }
                }
                linesSnapshot.mapIndexed { idx, line ->
                    if (idx == linesSnapshot.lastIndex) line.copy(mediaUri = photoUri, mediaMime = "image/jpeg") else line
                }
            } else {
                linesSnapshot
            }

            val stillCurrent = synchronized(stateLock) { generationByThread[ref.key] == gen }
            if (!stillCurrent) return@launch

            val resolvedThreadIcon = threadIcon
                ?: ConversationShortcuts.createInitialsIcon(displayTitle, ref.key, isGroup)
            val me = Person.Builder().setName("You").build()
            val style = NotificationCompat.MessagingStyle(me)
                .setConversationTitle(if (isGroup) displayTitle else null)
                .setGroupConversation(isGroup)

            for (line in finalLines) {
                val lineName = line.sender.ifBlank { displayTitle }
                val personKey = if (line.senderId.isNotBlank()) "user:${line.senderId}" else "${ref.key}:$lineName"
                val lineIcon = when {
                    !isGroup -> resolvedThreadIcon
                    line.senderId == msg.fromId && senderIcon != null -> senderIcon
                    else -> ConversationShortcuts.createInitialsIcon(lineName, personKey, isGroup = false)
                }
                val person = Person.Builder()
                    .setKey(personKey)
                    .setName(lineName)
                    .apply { if (lineIcon != null) setIcon(lineIcon) }
                    .build()
                val styleMsg = NotificationCompat.MessagingStyle.Message(line.text, line.ts, person)
                if (line.mediaUri != null && line.mediaMime != null) {
                    styleMsg.setData(line.mediaMime, line.mediaUri)
                }
                style.addMessage(styleMsg)
            }

            val requestCode = ref.key.hashCode()
            val open = PendingIntent.getActivity(
                context,
                requestCode,
                MainActivity.openChatIntent(context, ref),
                PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
            )
            val reply = NotificationCompat.Action.Builder(
                R.drawable.ic_launcher_foreground,
                "Reply",
                PendingIntent.getBroadcast(
                    context,
                    requestCode,
                    NotificationActionReceiver.intent(context, NotificationActionReceiver.ACTION_REPLY, ref),
                    // RemoteInput fills the intent in, so it has to stay mutable.
                    PendingIntent.FLAG_MUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                ),
            )
                .addRemoteInput(RemoteInput.Builder(NotificationActionReceiver.KEY_TEXT).setLabel("Message").build())
                .setSemanticAction(NotificationCompat.Action.SEMANTIC_ACTION_REPLY)
                .setShowsUserInterface(false)
                .build()
            val markRead = NotificationCompat.Action.Builder(
                R.drawable.ic_launcher_foreground,
                "Mark as read",
                PendingIntent.getBroadcast(
                    context,
                    requestCode + 1,
                    NotificationActionReceiver.intent(context, NotificationActionReceiver.ACTION_MARK_READ, ref),
                    PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
                ),
            )
                .setSemanticAction(NotificationCompat.Action.SEMANTIC_ACTION_MARK_AS_READ)
                .setShowsUserInterface(false)
                .build()

            val builder = NotificationCompat.Builder(context, CHANNEL_ID)
                .setSmallIcon(R.drawable.ic_launcher_foreground)
                .setStyle(style)
                .setContentTitle(displayTitle)
                .setContentText(finalLines.last().text)
                .setContentIntent(open)
                .setShortcutId(ConversationShortcuts.shortcutId(ref))
                .setAutoCancel(true)
                .setCategory(NotificationCompat.CATEGORY_MESSAGE)
                .setPriority(NotificationCompat.PRIORITY_HIGH)
                .setWhen(msg.ts)
                .addAction(reply)
                .addAction(markRead)
            resolvedThreadIcon?.let { icon ->
                runCatching { builder.setLargeIcon(icon.toIcon(context)) }
            }
            if (code != null) {
                builder.addAction(OtpNotifications.copyAction(context, code))
                OtpNotifications.applyLockScreenPrivacy(context, builder, CHANNEL_ID)
            }
            val notification = builder.build()
            if (canNotify(context)) {
                val posted = runCatching {
                    NotificationManagerCompat.from(context).notify(ref.key, NOTIFICATION_ID, notification)
                }
                if (posted.isSuccess) runCatching { onPosted(ref, displayTitle) }
            }
        }
    }

    /**
     * Downloads a scaled JPEG thumbnail for an incoming photo message into `<cacheDir>/shared/notif_media/`
     * (covered by `file_paths.xml`'s `shared/` cache-path), grants read access to SystemUI, and prunes
     * older previews to keep at most [MAX_MEDIA_FILES].
     */
    private suspend fun fetchNotificationPhotoPreview(url: String, msgId: String): Uri? {
        return try {
            val request = ImageRequest.Builder(context)
                .data(url)
                .size(MAX_PREVIEW_PX, MAX_PREVIEW_PX)
                .scale(Scale.FIT)
                .allowHardware(false)
                .build()
            val result = context.imageLoader.execute(request) as? SuccessResult ?: return null
            val drawable = result.drawable
            val w = drawable.intrinsicWidth.takeIf { it > 0 }?.coerceAtMost(MAX_PREVIEW_PX) ?: MAX_PREVIEW_PX
            val h = drawable.intrinsicHeight.takeIf { it > 0 }?.coerceAtMost(MAX_PREVIEW_PX) ?: MAX_PREVIEW_PX
            val bitmap = drawable.toBitmap(width = w, height = h, config = Bitmap.Config.ARGB_8888)
            val dir = File(context.cacheDir, NOTIF_MEDIA_SUBDIR).apply { mkdirs() }
            val safeId = msgId.replace(Regex("[^A-Za-z0-9_-]"), "").take(24).ifEmpty { "photo" }
            val file = File(dir, "${System.currentTimeMillis()}_$safeId.jpg")
            val ok = file.outputStream().use { out ->
                bitmap.compress(Bitmap.CompressFormat.JPEG, 82, out)
            }
            if (!ok) {
                file.delete()
                return null
            }
            pruneNotifMedia(dir)
            val uri = FileProvider.getUriForFile(context, "${context.packageName}.files", file)
            runCatching {
                context.grantUriPermission(SYSTEM_UI_PACKAGE, uri, Intent.FLAG_GRANT_READ_URI_PERMISSION)
            }
            uri
        } catch (e: CancellationException) {
            throw e
        } catch (_: Exception) {
            null
        }
    }

    private fun pruneNotifMedia(dir: File) {
        val files = dir.listFiles()?.filter { it.isFile } ?: return
        if (files.size <= MAX_MEDIA_FILES) return
        files.sortedByDescending { it.lastModified() }
            .drop(MAX_MEDIA_FILES)
            .forEach { it.delete() }
    }

    private val ringing = LinkedHashSet<String>()

    /**
     * "Bảo is calling": a real call notification for 1:1 calls (Answer opens the call screen and picks up, Decline
     * declines). Group calls can't be answered here, so they only say who is calling and open the official app.
     */
    private fun showCall(call: IncomingCall) {
        ensureCallChannel(context)
        val tag = callTag(call.callId.ifEmpty { call.callerId })
        ringing += tag
        val pendingFlags = PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT
        val ref = if (call.group && call.groupId != null) ThreadRef("group", call.groupId) else ThreadRef("user", call.callerId)
        val openChat = PendingIntent.getActivity(context, tag.hashCode() + 1, MainActivity.openChatIntent(context, ref), pendingFlags)
        val what = if (call.video) "Video call" else "Voice call"
        val callerName = call.callerName.ifBlank { "Someone" }
        val builder = NotificationCompat.Builder(context, CALLS_CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setPriority(NotificationCompat.PRIORITY_MAX)
            .setAutoCancel(true)
            // Zalo rings for about a minute; a call that never said it ended disappears on its own.
            .setTimeoutAfter(CALL_TIMEOUT_MS)
        if (call.callId.isNotEmpty() && !call.group) {
            val person = Person.Builder().setName(callerName).setImportant(true).build()
            val screen = { answerNow: Boolean ->
                CallActivity.intent(context, call.callId, call.callerId, callerName, video = call.video, answerNow = answerNow)
            }
            val answer = PendingIntent.getActivity(context, tag.hashCode() + 3, screen(true), pendingFlags)
            val decline = PendingIntent.getBroadcast(
                context,
                tag.hashCode() + 2,
                CallActionReceiver.declineIntent(context, call.callId, tag),
                pendingFlags,
            )
            val ringingScreen = PendingIntent.getActivity(context, tag.hashCode() + 4, screen(false), pendingFlags)
            builder
                .setContentTitle(callerName)
                .setContentText("Incoming ${what.lowercase()}")
                .setStyle(NotificationCompat.CallStyle.forIncomingCall(person, decline, answer))
                .setContentIntent(ringingScreen)
            // Android 14+ lets users revoke this; without it the heads-up notification still rings.
            if (Build.VERSION.SDK_INT < Build.VERSION_CODES.UPSIDE_DOWN_CAKE ||
                context.getSystemService(NotificationManager::class.java).canUseFullScreenIntent()
            ) {
                builder.setFullScreenIntent(ringingScreen, true)
            }
        } else {
            val zalo = context.packageManager.getLaunchIntentForPackage(ZALO_PACKAGE)
            val openZalo = zalo?.let { PendingIntent.getActivity(context, tag.hashCode(), it, pendingFlags) }
            val title = buildString {
                append(callerName)
                append(" is calling")
                call.groupName?.let { append(" in ").append(it) }
            }
            builder
                .setContentTitle("📞 $title")
                .setContentText(if (zalo != null) "$what · answer it in the Zalo app" else "$what · Zalỏ can't answer group calls")
                .setContentIntent(openZalo ?: openChat)
            if (openZalo != null) builder.addAction(R.drawable.ic_launcher_foreground, "Open Zalo", openZalo)
            else builder.addAction(R.drawable.ic_launcher_foreground, "Open chat", openChat)
        }
        val answerable = call.callId.isNotEmpty() && !call.group
        // A call Zalỏ can answer rings (ringtone, on repeat) until it is answered, declined or over; the others ring once.
        val notification = builder.build().apply { if (answerable) flags = flags or Notification.FLAG_INSISTENT }
        if (canNotify(context)) runCatching { NotificationManagerCompat.from(context).notify(tag, CALL_NOTIFICATION_ID, notification) }
        if (answerable) {
            popUp(call.callId, CallActivity.intent(context, call.callId, call.callerId, callerName, video = call.video))
        }
    }

    /**
     * Opens the ringing call screen over whatever is on screen (CallPopup), unless another call is in progress: that
     * call's screen stays, and this one only rings in the notification.
     */
    private fun popUp(callId: String, ringingScreen: Intent) {
        val scope = scope ?: return
        // Once per call: Zalo can repeat a call's request, and a second ringing screen would undo an answer in progress.
        if (!poppedUp.add(callId)) return
        while (poppedUp.size > 20) poppedUp.remove(poppedUp.first())
        scope.launch {
            val current = runCatching { ApiClient.onDevice(context).currentCall() }.getOrNull()
            // Another call in progress, or this one already answered from the notification.
            if (current != null && (current.callId != callId || current.state != "ringing")) return@launch
            if (callTag(callId) !in ringing) return@launch
            CallPopup.show(context, ringingScreen)
        }
    }

    private val poppedUp = LinkedHashSet<String>()

    private fun endCall(callId: String) {
        val tags = if (callId.isEmpty()) ringing.toList() else listOf(callTag(callId))
        for (tag in tags) {
            ringing -= tag
            NotificationManagerCompat.from(context).cancel(tag, CALL_NOTIFICATION_ID)
        }
    }

    private fun cancel(ref: ThreadRef) {
        synchronized(stateLock) {
            history.remove(ref.key)
            otpByThread.remove(ref.key)
            generationByThread[ref.key] = (generationByThread[ref.key] ?: 0L) + 1L
        }
        NotificationManagerCompat.from(context).cancel(ref.key, NOTIFICATION_ID)
    }

    companion object {
        const val CHANNEL_ID = "messages"
        /** Rings with the ringtone. A channel's sound can't change once created, so this replaced the first "calls". */
        const val CALLS_CHANNEL_ID = "calls_ringing"
        private const val OLD_CALLS_CHANNEL_ID = "calls"
        private const val CALL_TAG = "call:"

        /** Notification tag of the incoming call [callId]; the call screen cancels it when answering. */
        fun callTag(callId: String): String = CALL_TAG + callId
        const val CALL_NOTIFICATION_ID = 200
        private const val CALL_TIMEOUT_MS = 75_000L
        /** The official Zalo app, which answers the calls. */
        const val ZALO_PACKAGE = "com.zing.zalo"

        fun ensureCallChannel(context: Context) {
            val nm = context.getSystemService(NotificationManager::class.java)
            if (nm.getNotificationChannel(CALLS_CHANNEL_ID) != null) return
            nm.deleteNotificationChannel(OLD_CALLS_CHANNEL_ID)
            nm.createNotificationChannel(
                NotificationChannel(CALLS_CHANNEL_ID, "Incoming calls", NotificationManager.IMPORTANCE_HIGH).apply {
                    description = "Incoming Zalo calls: rings until you answer or decline"
                    setSound(
                        RingtoneManager.getDefaultUri(RingtoneManager.TYPE_RINGTONE),
                        AudioAttributes.Builder()
                            .setUsage(AudioAttributes.USAGE_NOTIFICATION_RINGTONE)
                            .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                            .build(),
                    )
                    enableVibration(true)
                    vibrationPattern = longArrayOf(0, 1_000, 1_000)
                    lockscreenVisibility = Notification.VISIBILITY_PUBLIC
                },
            )
        }
        /** Notifications are tagged with the chat key; this id is shared by all of them. */
        const val NOTIFICATION_ID = 100
        private const val MAX_LINES = 7
        private const val MAX_THREADS = 50
        private const val RECONNECT_MS = 3_000L
        private const val FETCH_TIMEOUT_MS = 1_500L
        private const val THREADS_REFRESH_MIN_MS = 30_000L
        private const val MAX_PREVIEW_PX = 720
        private const val MAX_MEDIA_FILES = 20
        private const val NOTIF_MEDIA_SUBDIR = "shared/notif_media"
        private const val SYSTEM_UI_PACKAGE = "com.android.systemui"

        fun ensureChannel(context: Context) {
            context.getSystemService(NotificationManager::class.java).createNotificationChannel(
                NotificationChannel(CHANNEL_ID, "Messages", NotificationManager.IMPORTANCE_HIGH).apply {
                    description = "New Zalo messages"
                },
            )
        }

        fun canNotify(context: Context): Boolean =
            Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
                context.checkSelfPermission(android.Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED

        /** What a notification says for a message: its text, or a short label for a photo, sticker, file… */
        fun notificationText(msg: ChatMessage): String {
            val a = msg.attachment
            return when (a?.kind) {
                "photo" -> "📷 Photo"
                "gif" -> "GIF"
                "sticker" -> "Sticker"
                "video" -> "🎬 Video"
                "voice" -> "🎤 Voice message"
                "file" -> "📎 ${a.title ?: "File"}"
                "link" -> a.title?.let { "🔗 $it" } ?: msg.text
                "contact" -> "Contact card: ${a.title.orEmpty()}"
                "location" -> "📍 ${a.title ?: "Location"}"
                "call" -> "📞 ${a.title ?: "Call"}"
                else -> msg.text
            }
        }
    }
}

/** "Decline" and "Answer (test)" on an incoming call notification: tells Zalo, then removes the notification. */
class CallActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val callId = intent.getStringExtra(EXTRA_CALL_ID) ?: return
        val tag = intent.getStringExtra(EXTRA_TAG) ?: return
        val answering = intent.action == ACTION_ANSWER_TEST
        val pending = goAsync()
        CoroutineScope(SupervisorJob() + Dispatchers.IO).launch {
            try {
                val api = ApiClient.onDevice(context)
                if (answering) api.answerCallTest(callId) else api.declineCall(callId)
                NotificationManagerCompat.from(context).cancel(tag, MessageNotifier.CALL_NOTIFICATION_ID)
            } catch (e: Exception) {
                // Say so on the notification instead of leaving the caller wondering whether it worked.
                StartupLog.write(context, "${if (answering) "Answer test" else "Decline"} failed: ${e.message}")
                showFailed(context, tag, if (answering) "Couldn't answer the call" else "Couldn't decline the call", e.message)
            } finally {
                pending.finish()
            }
        }
    }

    private fun showFailed(context: Context, tag: String, title: String, reason: String?) {
        if (!MessageNotifier.canNotify(context)) return
        MessageNotifier.ensureCallChannel(context)
        val n = NotificationCompat.Builder(context, MessageNotifier.CALLS_CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle(title)
            .setContentText(reason ?: "Zalo didn't answer")
            .setOnlyAlertOnce(true)
            .setSilent(true)
            .setAutoCancel(true)
            .setTimeoutAfter(30_000)
            .build()
        runCatching { NotificationManagerCompat.from(context).notify(tag, MessageNotifier.CALL_NOTIFICATION_ID, n) }
    }

    companion object {
        const val ACTION_DECLINE = "com.zalom3ui.app.action.DECLINE_CALL"
        const val ACTION_ANSWER_TEST = "com.zalom3ui.app.action.ANSWER_CALL_TEST"
        private const val EXTRA_CALL_ID = "callId"
        private const val EXTRA_TAG = "tag"

        fun declineIntent(context: Context, callId: String, tag: String): Intent = intent(context, ACTION_DECLINE, callId, tag)

        fun answerTestIntent(context: Context, callId: String, tag: String): Intent = intent(context, ACTION_ANSWER_TEST, callId, tag)

        private fun intent(context: Context, action: String, callId: String, tag: String): Intent =
            Intent(context, CallActionReceiver::class.java)
                .setAction(action)
                .putExtra(EXTRA_CALL_ID, callId)
                .putExtra(EXTRA_TAG, tag)
    }
}

/** "Reply" and "Mark as read" on a message notification, handled without opening the app. */
class NotificationActionReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val ref = ThreadRef(intent.getStringExtra(EXTRA_KIND) ?: return, intent.getStringExtra(EXTRA_ID) ?: return)
        val text = RemoteInput.getResultsFromIntent(intent)?.getCharSequence(KEY_TEXT)?.toString()?.trim()
        val pending = goAsync()
        CoroutineScope(SupervisorJob() + Dispatchers.IO).launch {
            try {
                val api = ApiClient.onDevice(context)
                if (intent.action == ACTION_REPLY && !text.isNullOrEmpty()) api.send(ref.kind, ref.id, text)
                // Replying reads the chat, as in the official app; the server's "read" event removes the notification.
                api.markRead(ref.kind, ref.id)
                NotificationManagerCompat.from(context).cancel(ref.key, MessageNotifier.NOTIFICATION_ID)
            } catch (e: Exception) {
                // Leave the notification so nothing is lost; the reply can be typed in the app.
                StartupLog.write(context, "notification action ${intent.action} failed: ${e.message}")
                if (intent.action == ACTION_REPLY) {
                    Handler(Looper.getMainLooper()).post {
                        runCatching { Toast.makeText(context.applicationContext, "Couldn't send reply", Toast.LENGTH_SHORT).show() }
                    }
                }
            } finally {
                pending.finish()
            }
        }
    }

    companion object {
        const val ACTION_REPLY = "com.zalom3ui.app.action.REPLY"
        const val ACTION_MARK_READ = "com.zalom3ui.app.action.MARK_READ"
        const val KEY_TEXT = "text"
        private const val EXTRA_KIND = "kind"
        private const val EXTRA_ID = "id"

        fun intent(context: Context, action: String, ref: ThreadRef): Intent =
            Intent(context, NotificationActionReceiver::class.java)
                .setAction(action)
                .putExtra(EXTRA_KIND, ref.kind)
                .putExtra(EXTRA_ID, ref.id)
    }
}
