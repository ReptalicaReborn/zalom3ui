package com.zalom3ui.app.call

import android.app.NotificationManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.provider.Settings
import com.zalom3ui.app.server.MessageNotifier

/**
 * Making an incoming call take over the screen, like the official app does.
 *
 * The notification's full-screen intent only opens the call screen when the device is locked or its screen is off;
 * while it is in use Android shows a heads-up notification instead (and many tablets have those off by default). To
 * open the call screen over whatever is on screen, Zalỏ starts it itself, which Android allows only while Zalỏ is
 * visible or when it may "Display over other apps". Xiaomi phones also want their own "pop-up windows while running
 * in the background" switch.
 */
object CallPopup {
    /** "Display over other apps": lets Zalỏ open the call screen from the background. */
    fun canDrawOverApps(context: Context): Boolean = Settings.canDrawOverlays(context)

    /** Android 14+ lets users turn off full-screen notifications per app (the call screen on the lock screen). */
    fun canUseFullScreen(context: Context): Boolean =
        Build.VERSION.SDK_INT < Build.VERSION_CODES.UPSIDE_DOWN_CAKE ||
            context.getSystemService(NotificationManager::class.java).canUseFullScreenIntent()

    val needsFullScreenPermission: Boolean get() = Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE

    /** MIUI / HyperOS (Xiaomi, Redmi, POCO) block background pop-ups with a switch of their own. */
    val isXiaomi: Boolean
        get() = listOf(Build.MANUFACTURER, Build.BRAND).any { it.equals("Xiaomi", true) || it.equals("Redmi", true) || it.equals("POCO", true) }

    fun overlaySettings(context: Context): Intent =
        Intent(Settings.ACTION_MANAGE_OVERLAY_PERMISSION, Uri.parse("package:${context.packageName}"))

    fun fullScreenSettings(context: Context): Intent =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE) {
            Intent(Settings.ACTION_MANAGE_APP_USE_FULL_SCREEN_INTENT, Uri.parse("package:${context.packageName}"))
        } else {
            appDetails(context)
        }

    /** The "Incoming calls" notification channel: sound, banners / floating notifications, lock screen. */
    fun callNotificationSettings(context: Context): Intent {
        MessageNotifier.ensureCallChannel(context)
        return Intent(Settings.ACTION_CHANNEL_NOTIFICATION_SETTINGS)
            .putExtra(Settings.EXTRA_APP_PACKAGE, context.packageName)
            .putExtra(Settings.EXTRA_CHANNEL_ID, MessageNotifier.CALLS_CHANNEL_ID)
    }

    /** Xiaomi's per-app permission page ("Display pop-up windows while running in the background", "Show on lock screen"). */
    fun xiaomiPermissions(context: Context): Intent =
        Intent("miui.intent.action.APP_PERM_EDITOR")
            .setClassName("com.miui.securitycenter", "com.miui.permcenter.permissions.PermissionsEditorActivity")
            .putExtra("extra_pkgname", context.packageName)

    fun appDetails(context: Context): Intent =
        Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:${context.packageName}"))

    /** Opens [intent], falling back to the app's details page when a phone has no such settings screen. */
    fun open(context: Context, intent: Intent) {
        val flags = Intent.FLAG_ACTIVITY_NEW_TASK
        runCatching { context.startActivity(intent.addFlags(flags)) }
            .onFailure { runCatching { context.startActivity(appDetails(context).addFlags(flags)) } }
    }

    /**
     * Brings the ringing call screen up. Android ignores the request when Zalỏ isn't allowed to start screens from the
     * background, and the notification still rings then.
     */
    fun show(context: Context, ringingScreen: Intent) {
        runCatching { context.startActivity(ringingScreen.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)) }
    }
}
