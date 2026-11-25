package com.zalom3ui.app.server

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.Person
import androidx.core.app.RemoteInput
import com.zalom3ui.app.MainActivity
import com.zalom3ui.app.R
import com.zalom3ui.app.data.ApiClient
import com.zalom3ui.app.data.ChatMessage
import com.zalom3ui.app.data.IncomingCall
import com.zalom3ui.app.data.ServerEvent
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.data.ThreadRef
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
    private val history = HashMap<String, ArrayDeque<Line>>()

    private data class Line(val sender: String, val text: String, val ts: Long)

    /** Listens until [scope] ends; reconnects while the server starts or restarts. */
    fun start(scope: CoroutineScope) {
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

    /** "Bảo is calling": Zalỏ can't answer calls, so it says who is calling and opens the official app. */
    private fun showCall(call: IncomingCall) {
        ensureCallChannel(context)
        val tag = CALL_TAG + call.callId.ifEmpty { call.callerId }
        ringing += tag
        val zalo = context.packageManager.getLaunchIntentForPackage(ZALO_PACKAGE)
        val openZalo = zalo?.let {
            PendingIntent.getActivity(context, tag.hashCode(), it, PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT)
        }
        val ref = if (call.group && call.groupId != null) ThreadRef("group", call.groupId) else ThreadRef("user", call.callerId)
        val openChat = PendingIntent.getActivity(
            context,
            tag.hashCode() + 1,
            MainActivity.openChatIntent(context, ref),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val what = if (call.video) "Video call" else "Voice call"
        val title = buildString {
            append(call.callerName.ifBlank { "Someone" })
            append(" is calling")
            call.groupName?.let { append(" in ").append(it) }
        }
        val builder = NotificationCompat.Builder(context, CALLS_CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle("📞 $title")
            .setContentText(if (zalo != null) "$what · answer it in the Zalo app" else "$what · Zalỏ can't answer calls yet")
            .setCategory(NotificationCompat.CATEGORY_CALL)
            .setPriority(NotificationCompat.PRIORITY_MAX)
            .setContentIntent(openZalo ?: openChat)
            .setAutoCancel(true)
            // Zalo rings for about a minute; a call that never said it ended disappears on its own.
            .setTimeoutAfter(CALL_TIMEOUT_MS)
        if (openZalo != null) builder.addAction(R.drawable.ic_launcher_foreground, "Open Zalo", openZalo)
        builder.addAction(R.drawable.ic_launcher_foreground, "Open chat", openChat)
        if (canNotify(context)) runCatching { NotificationManagerCompat.from(context).notify(tag, CALL_NOTIFICATION_ID, builder.build()) }
    }

    private fun endCall(callId: String) {
        val tags = if (callId.isEmpty()) ringing.toList() else listOf(CALL_TAG + callId).filter { it in ringing }.ifEmpty { ringing.toList() }
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
        const val CALLS_CHANNEL_ID = "calls"
        private const val CALL_TAG = "call:"
        private const val CALL_NOTIFICATION_ID = 200
        private const val CALL_TIMEOUT_MS = 75_000L
        /** The official Zalo app, which answers the calls. */
        const val ZALO_PACKAGE = "com.zing.zalo"

        fun ensureCallChannel(context: Context) {
            context.getSystemService(NotificationManager::class.java).createNotificationChannel(
                NotificationChannel(CALLS_CHANNEL_ID, "Incoming calls", NotificationManager.IMPORTANCE_HIGH).apply {
                    description = "Who is calling (calls are answered in the Zalo app)"
                },
            )
        }
        /** Notifications are tagged with the chat key; this id is shared by all of them. */
        const val NOTIFICATION_ID = 100
        private const val MAX_LINES = 7
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
            } catch (_: Exception) {
                // Leave the notification so nothing is lost; the reply can be typed in the app.
            } finally {
                NotificationManagerCompat.from(context).cancel(ref.key, MessageNotifier.NOTIFICATION_ID)
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
