package com.zalom3ui.app.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Column
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Fullscreen
import androidx.compose.material.icons.outlined.Layers
import androidx.compose.material.icons.outlined.NotificationsActive
import androidx.compose.material.icons.outlined.PhoneInTalk
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
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

    val transparentColors = ListItemDefaults.colors(containerColor = Color.Transparent)
    val items = buildList<@Composable () -> Unit> {
        add {
            ListItem(
                modifier = Modifier.clickable { CallPopup.open(context, CallPopup.overlaySettings(context)) },
                colors = transparentColors,
                leadingContent = { Icon(Icons.Outlined.Layers, null) },
                headlineContent = { Text("Pop up over other apps") },
                supportingContent = {
                    Text(
                        if (overApps) {
                            "On: calls open over whatever you're doing"
                        } else {
                            "Off: calls only show a notification. Tap to allow"
                        },
                    )
                },
                trailingContent = { Switch(checked = overApps, onCheckedChange = { CallPopup.open(context, CallPopup.overlaySettings(context)) }) },
            )
        }
        if (CallPopup.needsFullScreenPermission) {
            add {
                ListItem(
                    modifier = Modifier.clickable { CallPopup.open(context, CallPopup.fullScreenSettings(context)) },
                    colors = transparentColors,
                    leadingContent = { Icon(Icons.Outlined.Fullscreen, null) },
                    headlineContent = { Text("Full screen on the lock screen") },
                    supportingContent = {
                        Text(if (fullScreen) "On: calls wake the screen with Answer / Decline" else "Off: tap to allow full-screen notifications")
                    },
                    trailingContent = { Switch(checked = fullScreen, onCheckedChange = { CallPopup.open(context, CallPopup.fullScreenSettings(context)) }) },
                )
            }
        }
        if (CallPopup.isXiaomi) {
            add {
                ListItem(
                    modifier = Modifier.clickable { CallPopup.open(context, CallPopup.xiaomiPermissions(context)) },
                    colors = transparentColors,
                    leadingContent = { Icon(Icons.Outlined.PhoneInTalk, null) },
                    headlineContent = { Text("Xiaomi pop-up permissions") },
                    supportingContent = {
                        Text("Allow background pop-ups and lock screen display")
                    },
                )
            }
        }
        add {
            ListItem(
                modifier = Modifier.clickable { CallPopup.open(context, CallPopup.callNotificationSettings(context)) },
                colors = transparentColors,
                leadingContent = { Icon(Icons.Outlined.NotificationsActive, null) },
                headlineContent = { Text("Call notification") },
                supportingContent = {
                    Text(
                        "Sound, banners and lock screen for calls",
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                },
            )
        }
    }

    Column {
        SectionHeader("Incoming calls")
        ExpressiveSettingsGroup(items = items)
    }
}
