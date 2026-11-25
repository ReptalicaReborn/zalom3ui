package com.zalom3ui.app.server

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.media.AudioAttributes
import android.media.RingtoneManager
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.widget.Toast
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.Person
import androidx.core.app.RemoteInput
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
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch

/**
 * Shows a notification for each new message the on-phone server flags (`notify`: not mine, not muted, not the chat on
 * screen, not replayed history), one per chat with its recent lines, and removes it once the chat is read anywhere.
 * Runs in the ":node" process next to the server, so it works while the app is closed.
 */
class MessageNotifier(private val context: Context) {
    private val history = object : LinkedHashMap<String, ArrayDeque<Line>>(MAX_THREADS, 0.75f, true) {
        override fun removeEldestEntry(eldest: MutableMap.MutableEntry<String, ArrayDeque<Line>>?): Boolean =
            size > MAX_THREADS
    }

    private data class Line(val sender: String, val text: String, val ts: Long)

    private var scope: CoroutineScope? = null

    /** Listens until [scope] ends; reconnects while the server starts or restarts. */
    fun start(scope: CoroutineScope) {
        this.scope = scope
        ensureChannel(context)
        scope.launch {
            while (isActive) {
                try {
                    val api = ApiClient(LocalServer.url(context), SettingsStore(context).localToken())
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
            is ServerEvent.Message -> if (event.notify) show(event.message, event.threadName)
            is ServerEvent.Read -> cancel(event.thread)
            is ServerEvent.CallIncoming -> showCall(event.call)
            is ServerEvent.CallEnded -> endCall(event.callId)
            else -> Unit
        }
    }

    private fun show(msg: ChatMessage, threadName: String) {
        val ref = ThreadRef(msg.threadKind, msg.threadId)
        val lines = history.getOrPut(ref.key) { ArrayDeque() }
        lines.addLast(Line(msg.fromName.ifBlank { threadName }, notificationText(msg), msg.ts))
        while (lines.size > MAX_LINES) lines.removeFirst()

        val isGroup = ref.kind == "group"
        val me = Person.Builder().setName("You").build()
        val style = NotificationCompat.MessagingStyle(me)
            .setConversationTitle(if (isGroup) threadName else null)
            .setGroupConversation(isGroup)
        for (line in lines) {
            style.addMessage(line.text, line.ts, Person.Builder().setName(line.sender.ifBlank { threadName }).build())
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

        val notification = NotificationCompat.Builder(context, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setStyle(style)
            .setContentTitle(threadName)
            .setContentText(lines.last().text)
            .setContentIntent(open)
            .setAutoCancel(true)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setWhen(msg.ts)
            .addAction(reply)
            .addAction(markRead)
            .build()
        if (canNotify(context)) {
            runCatching { NotificationManagerCompat.from(context).notify(ref.key, NOTIFICATION_ID, notification) }
        }
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
            val current = runCatching { ApiClient(LocalServer.url(context), SettingsStore(context).localToken()).currentCall() }.getOrNull()
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
        history.remove(ref.key)
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
                val api = ApiClient(LocalServer.url(context), SettingsStore(context).localToken())
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
                val api = ApiClient(LocalServer.url(context), SettingsStore(context).localToken())
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
