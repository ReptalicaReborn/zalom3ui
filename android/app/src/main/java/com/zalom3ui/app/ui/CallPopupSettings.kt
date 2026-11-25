package com.zalom3ui.app.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.padding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Fullscreen
import androidx.compose.material.icons.outlined.Layers
import androidx.compose.material.icons.outlined.NotificationsActive
import androidx.compose.material.icons.outlined.PhoneInTalk
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import com.zalom3ui.app.call.CallPopup

/**
 * Settings → "Incoming calls": the switches Android (and Xiaomi) keep for a call screen that pops up over other apps
 * and on the lock screen. Each row opens the system page for it; the state is read again on returning.
 */
@Composable
fun IncomingCallSettings() {
    val context = LocalContext.current
    var overApps by remember { mutableStateOf(CallPopup.canDrawOverApps(context)) }
    var fullScreen by remember { mutableStateOf(CallPopup.canUseFullScreen(context)) }
    LifecycleResumeEffect(Unit) {
        overApps = CallPopup.canDrawOverApps(context)
        fullScreen = CallPopup.canUseFullScreen(context)
        onPauseOrDispose { }
    }

    Column {
        HorizontalDivider(Modifier.padding(top = 8.dp))
        SectionHeader("Incoming calls")
        ListItem(
            modifier = Modifier.clickable { CallPopup.open(context, CallPopup.overlaySettings(context)) },
            leadingContent = { Icon(Icons.Outlined.Layers, null) },
            headlineContent = { Text("Pop up over other apps") },
            supportingContent = {
                Text(
                    if (overApps) {
                        "On: a call opens its screen whatever you're doing"
                    } else {
                        "Off: calls only show a notification while you use the device. Tap, then allow \"Display over other apps\""
                    },
                )
            },
            trailingContent = { Switch(checked = overApps, onCheckedChange = { CallPopup.open(context, CallPopup.overlaySettings(context)) }) },
        )
        if (CallPopup.needsFullScreenPermission) {
            ListItem(
                modifier = Modifier.clickable { CallPopup.open(context, CallPopup.fullScreenSettings(context)) },
                leadingContent = { Icon(Icons.Outlined.Fullscreen, null) },
                headlineContent = { Text("Full screen on the lock screen") },
                supportingContent = {
                    Text(if (fullScreen) "On: a call wakes the screen and shows Answer / Decline" else "Off: tap, then allow full-screen notifications")
                },
                trailingContent = { Switch(checked = fullScreen, onCheckedChange = { CallPopup.open(context, CallPopup.fullScreenSettings(context)) }) },
            )
        }
        if (CallPopup.isXiaomi) {
            ListItem(
                modifier = Modifier.clickable { CallPopup.open(context, CallPopup.xiaomiPermissions(context)) },
                leadingContent = { Icon(Icons.Outlined.PhoneInTalk, null) },
                headlineContent = { Text("Xiaomi pop-up permissions") },
                supportingContent = {
                    Text("Allow \"Display pop-up windows while running in the background\" and \"Show on Lock screen\"")
                },
            )
        }
        ListItem(
            modifier = Modifier.clickable { CallPopup.open(context, CallPopup.callNotificationSettings(context)) },
            leadingContent = { Icon(Icons.Outlined.NotificationsActive, null) },
            headlineContent = { Text("Call notification") },
            supportingContent = {
                Text(
                    "Sound, banners (floating notifications) and lock screen for \"Incoming calls\"",
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
            },
        )
    }
}
