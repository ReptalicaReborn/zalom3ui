package com.zalom3ui.app.server

import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.os.PersistableBundle
import android.widget.Toast
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import com.zalom3ui.app.R

/**
 * One-time codes in notifications: the "Copy 482910" action, the lock-screen privacy, and the standalone notification
 * for a code read from the official Zalo app's notification ([ZaloNotificationListener]).
 *
 * Clipboard from a notification action, per Android version (no trampoline is needed on any of them):
 * - The action is a broadcast ([OtpCopyReceiver]), not an activity. It never starts an activity, so Android 12+'s
 *   notification-trampoline restrictions (no `startActivity` from a receiver or service that a notification started)
 *   do not apply.
 * - Android 10+ stops apps in the background from *reading* the clipboard. *Writing* it (`setPrimaryClip`) is allowed
 *   from the background, which is all this does. The write happens in `onReceive`, while the system is delivering the
 *   tap, so the process is running. Nothing is read.
 * - Android 13+ marks the clip sensitive (`ClipDescription.EXTRA_IS_SENSITIVE`): the keyboard's clipboard preview and
 *   the system's copy overlay hide the text. It shows its own "copied" confirmation, so Zalỏ shows its toast only on
 *   Android 12 and older (Android 11+ allows text toasts from the background; only custom toasts are blocked).
 *   Before 33 the same extra is set under its literal key; it is harmless where it is not recognised.
 * - Some manufacturers' power managers can block a background clipboard write. Only a real phone can tell; the toast
 *   (pre-13) or the paste is the check.
 */
object OtpNotifications {
    const val CODES_CHANNEL_ID = "codes"
    const val RESCUE_NOTIFICATION_ID = 300
    private const val RESCUE_TAG_PREFIX = "otp:"
    private const val RESCUE_TIMEOUT_MS = 10 * 60_000L

    /** Shared by the chat notifications and the rescue, so a code is announced once. */
    val dedupe = OtpDedupe()

    fun ensureCodesChannel(context: Context) {
        context.getSystemService(NotificationManager::class.java).createNotificationChannel(
            NotificationChannel(CODES_CHANNEL_ID, "Codes", NotificationManager.IMPORTANCE_HIGH).apply {
                description = "One-time codes, with a Copy action"
            },
        )
    }

    /** "Copy 482910": writes the code to the clipboard (marked sensitive) without opening the app. */
    fun copyAction(context: Context, code: String): NotificationCompat.Action {
        // A unique request code and data URI per code: PendingIntents match on action and data, not extras, and
        // FLAG_UPDATE_CURRENT would otherwise rewrite the code of an older notification.
        val intent = Intent(context, OtpCopyReceiver::class.java)
            .setAction(OtpCopyReceiver.ACTION_COPY)
            .setData(Uri.parse("zaloo-otp://copy/$code"))
            .putExtra(OtpCopyReceiver.EXTRA_CODE, code)
        val pending = PendingIntent.getBroadcast(
            context,
            code.hashCode(),
            intent,
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Action.Builder(R.drawable.ic_launcher_foreground, OtpText.copyLabel(code), pending)
            .setShowsUserInterface(false)
            .build()
    }

    /**
     * Hides the code on the lock screen: the notification is private (needs unlock for its content) and the public
     * version, shown instead while locked, names neither the code nor the sender.
     */
    fun applyLockScreenPrivacy(context: Context, builder: NotificationCompat.Builder, channelId: String) {
        val public = NotificationCompat.Builder(context, channelId)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle(OtpText.PUBLIC_TITLE)
            .setContentText(OtpText.PUBLIC_TEXT)
            .build()
        builder.setVisibility(NotificationCompat.VISIBILITY_PRIVATE).setPublicVersion(public)
    }

    /** Posts "Code 482910 · title" for a code read from the Zalo app's notification, unless it was announced already. */
    fun postRescued(context: Context, code: String, zaloTitle: String?, now: Long = System.currentTimeMillis()): RescueResult {
        if (!OtpText.isValidCode(code) || !MessageNotifier.canNotify(context)) return RescueResult.NOT_SHOWN
        if (!dedupe.tryRescue(code, now)) return RescueResult.ALREADY_SHOWN
        ensureCodesChannel(context)
        val open = PendingIntent.getActivity(
            context,
            RESCUE_NOTIFICATION_ID,
            context.packageManager.getLaunchIntentForPackage(context.packageName) ?: Intent(),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val builder = NotificationCompat.Builder(context, CODES_CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_launcher_foreground)
            .setContentTitle(OtpText.title(code, zaloTitle))
            .setContentText("From the Zalo app's notification")
            .setContentIntent(open)
            .setCategory(NotificationCompat.CATEGORY_MESSAGE)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setAutoCancel(true)
            // A code is only good for a few minutes.
            .setTimeoutAfter(RESCUE_TIMEOUT_MS)
            .addAction(copyAction(context, code))
        applyLockScreenPrivacy(context, builder, CODES_CHANNEL_ID)
        val posted = runCatching {
            NotificationManagerCompat.from(context).notify(RESCUE_TAG_PREFIX + code, RESCUE_NOTIFICATION_ID, builder.build())
        }.isSuccess
        return if (posted) RescueResult.POSTED else RescueResult.NOT_SHOWN
    }
}

/** Writes a code to the clipboard as a sensitive clip. Write only: the clipboard is never read. */
object OtpClipboard {
    /** The literal value of `ClipDescription.EXTRA_IS_SENSITIVE` (API 33), set on older versions too. */
    private const val IS_SENSITIVE_KEY = "android.content.extra.IS_SENSITIVE"

    fun copy(context: Context, code: String): Boolean = runCatching {
        val cm = context.getSystemService(ClipboardManager::class.java) ?: return false
        val clip = ClipData.newPlainText("Zalo code", code)
        val extras = PersistableBundle().apply {
            putBoolean(if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) ClipDescription.EXTRA_IS_SENSITIVE else IS_SENSITIVE_KEY, true)
        }
        clip.description.extras = extras
        cm.setPrimaryClip(clip)
        true
    }.getOrDefault(false)

    /** Copies and tells the user, where Android does not already (Android 13+ shows its own confirmation). */
    fun copyWithFeedback(context: Context, code: String) {
        val ok = copy(context, code)
        if (!ok) {
            toast(context, "Couldn't copy the code")
        } else if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) {
            toast(context, OtpText.copiedToast(code))
        }
    }

    private fun toast(context: Context, text: String) {
        Handler(Looper.getMainLooper()).post {
            runCatching { Toast.makeText(context.applicationContext, text, Toast.LENGTH_SHORT).show() }
        }
    }
}

/** The "Copy 482910" action on a notification. Leaves the notification where it is, so the code can be copied again. */
class OtpCopyReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        if (intent.action != ACTION_COPY) return
        val code = intent.getStringExtra(EXTRA_CODE)
        if (!OtpText.isValidCode(code)) return
        OtpClipboard.copyWithFeedback(context, code!!)
    }

    companion object {
        const val ACTION_COPY = "com.zalom3ui.app.action.COPY_OTP"
        const val EXTRA_CODE = "code"
    }
}
