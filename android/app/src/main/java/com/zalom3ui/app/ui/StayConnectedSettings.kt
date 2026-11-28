package com.zalom3ui.app.ui

import android.content.Intent
import android.provider.Settings
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.selection.selectable
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Info
import androidx.compose.material.icons.outlined.NotificationsActive
import androidx.compose.material.icons.outlined.Password
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.data.StayConnected
import com.zalom3ui.app.server.StayConnectedCoordinator
import com.zalom3ui.app.server.ZaloNotificationListener

/**
 * Settings → "Stay connected" (only while Zalo runs on this phone, [showModes]): how long Zalỏ stays connected to Zalo.
 * Choosing a mode takes effect at once ([StayConnectedCoordinator.applyMode]). The notification bridge needs notification
 * access; without it that mode behaves like "While open", and this says so.
 *
 * Below it, always: "Codes from Zalo's notifications" (off by default), which reads one-time codes from the official
 * Zalo app's notifications. It uses the same notification access, and is offered right in the bridge's group when that
 * access is already granted.
 */
@Composable
fun StayConnectedSettings(showModes: Boolean = true, showCodes: Boolean = true) {
    val context = LocalContext.current
    val store = remember { SettingsStore(context) }
    var stay by remember { mutableStateOf(store.loadStay()) }
    var modesHelpOpen by remember { mutableStateOf(false) }
    var accessGranted by remember { mutableStateOf(ZaloNotificationListener.isAccessGranted(context)) }
    LifecycleResumeEffect(Unit) {
        accessGranted = ZaloNotificationListener.isAccessGranted(context)
        onPauseOrDispose { }
    }

    val transparent = ListItemDefaults.colors(containerColor = Color.Transparent)
    val setRescue = { on: Boolean ->
        stay = stay.copy(otpFromZaloNotifications = on)
        StayConnectedCoordinator.saveOtpFromZaloNotifications(context, on)
    }
    val openAccess = {
        runCatching {
            context.startActivity(Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
        }
        Unit
    }
    val items = buildList<@Composable () -> Unit> {
        for (mode in if (showModes) StayConnected.entries else emptyList()) {
            add {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .selectable(
                            selected = stay.mode == mode,
                            role = Role.RadioButton,
                            onClick = {
                                stay = stay.copy(mode = mode)
                                StayConnectedCoordinator.applyMode(context, stay)
                            },
                        )
                        .padding(horizontal = 16.dp, vertical = 12.dp),
                    verticalAlignment = Alignment.Top,
                ) {
                    RadioButton(selected = stay.mode == mode, onClick = null)
                    Spacer(Modifier.width(16.dp))
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(mode.label, style = MaterialTheme.typography.bodyLarge)
                        Text(
                            shortSummary(mode),
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
        if (showModes) {
            add {
                ListItem(
                    modifier = Modifier.clickable { modesHelpOpen = !modesHelpOpen },
                    colors = transparent,
                    leadingContent = { Icon(Icons.Outlined.Info, null) },
                    headlineContent = { Text("How these modes differ") },
                    supportingContent = {
                        if (modesHelpOpen) {
                            Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                                StayConnected.entries.forEach { m ->
                                    Text("${m.label}: ${m.summary}")
                                }
                            }
                        } else {
                            Text("Tap to read the details")
                        }
                    },
                )
            }
        }
        if (showModes && stay.mode.usesBridge) {
            add {
                ListItem(
                    colors = transparent,
                    leadingContent = { Icon(Icons.Outlined.NotificationsActive, null) },
                    headlineContent = { Text("Notification access") },
                    supportingContent = {
                        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            Text(
                                if (accessGranted) {
                                    "Allowed: Zalỏ sees the Zalo app's message notifications."
                                } else {
                                    "Not allowed yet. Until then this works like \"While open\"."
                                },
                            )
                            Button(onClick = {
                                runCatching {
                                    context.startActivity(
                                        Intent(Settings.ACTION_NOTIFICATION_LISTENER_SETTINGS).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK),
                                    )
                                }
                            }) { Text(if (accessGranted) "Open notification access" else "Allow notification access") }
                        }
                    },
                )
            }
            add {
                ListItem(
                    modifier = Modifier.clickable {
                        stay = stay.copy(hideZaloNotifications = !stay.hideZaloNotifications)
                        StayConnectedCoordinator.saveHideZaloNotifications(context, stay.hideZaloNotifications)
                    },
                    colors = transparent,
                    leadingContent = { Icon(Icons.Outlined.VisibilityOff, null) },
                    headlineContent = { Text("Hide Zalo's notifications") },
                    supportingContent = {
                        Text("Remove Zalo's message notification once Zalỏ shows its own. Calls are untouched.")
                    },
                    trailingContent = {
                        Switch(
                            checked = stay.hideZaloNotifications,
                            onCheckedChange = {
                                stay = stay.copy(hideZaloNotifications = it)
                                StayConnectedCoordinator.saveHideZaloNotifications(context, it)
                            },
                        )
                    },
                )
            }
            // Access is already allowed for the bridge: offer the code rescue here, where the user is looking.
            if (accessGranted && !stay.otpFromZaloNotifications) {
                add {
                    ListItem(
                        modifier = Modifier.clickable { setRescue(true) },
                        colors = transparent,
                        leadingContent = { Icon(Icons.Outlined.Password, null) },
                        headlineContent = { Text("Codes from Zalo's notifications") },
                        supportingContent = {
                            Text("Show one-time codes with a Copy button")
                        },
                        trailingContent = { Switch(checked = false, onCheckedChange = { setRescue(true) }) },
                    )
                }
            }
        }
    }

    val rescueItems = buildList<@Composable () -> Unit> {
        add {
            ListItem(
                modifier = Modifier.clickable { setRescue(!stay.otpFromZaloNotifications) },
                colors = transparent,
                leadingContent = { Icon(Icons.Outlined.Password, null) },
                headlineContent = { Text("Codes from Zalo's notifications") },
                supportingContent = {
                    Text("Show one-time codes from Zalo's notifications with a Copy button. Read in memory only, never stored.")
                },
                trailingContent = {
                    Switch(checked = stay.otpFromZaloNotifications, onCheckedChange = setRescue)
                },
            )
        }
        if (stay.otpFromZaloNotifications) {
            add {
                ListItem(
                    colors = transparent,
                    leadingContent = { Icon(Icons.Outlined.NotificationsActive, null) },
                    headlineContent = { Text("Notification access") },
                    supportingContent = {
                        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            Text(
                                if (accessGranted) {
                                    "Allowed: Zalỏ can read codes from Zalo's notifications."
                                } else {
                                    "Not allowed yet, so codes can't be read."
                                },
                            )
                            Button(onClick = openAccess) {
                                Text(if (accessGranted) "Open notification access" else "Allow notification access")
                            }
                        }
                    },
                )
            }
            // The same switch the bridge uses; shown here unless the bridge group above already shows it.
            if (!(showModes && stay.mode.usesBridge)) {
                add {
                    ListItem(
                        modifier = Modifier.clickable {
                            stay = stay.copy(hideZaloNotifications = !stay.hideZaloNotifications)
                            StayConnectedCoordinator.saveHideZaloNotifications(context, stay.hideZaloNotifications)
                        },
                        colors = transparent,
                        leadingContent = { Icon(Icons.Outlined.VisibilityOff, null) },
                        headlineContent = { Text("Hide Zalo's notification") },
                        supportingContent = {
                            Text("Remove Zalo's notification once Zalỏ shows the code, so you get one. Calls are untouched.")
                        },
                        trailingContent = {
                            Switch(
                                checked = stay.hideZaloNotifications,
                                onCheckedChange = {
                                    stay = stay.copy(hideZaloNotifications = it)
                                    StayConnectedCoordinator.saveHideZaloNotifications(context, it)
                                },
                            )
                        },
                    )
                }
            }
        }
    }

    Column {
        if (showModes) {
            SectionHeader("Stay connected")
            ExpressiveSettingsGroup(items = items)
        }
        if (showCodes) {
            SectionHeader("Codes from Zalo's notifications")
            ExpressiveSettingsGroup(items = rescueItems)
        }
    }
}

private fun shortSummary(mode: StayConnected): String = when (mode) {
    StayConnected.ALWAYS -> "Always connected, even when Zalỏ is closed"
    StayConnected.WHILE_OPEN -> "Connected only while Zalỏ is open"
    StayConnected.WHILE_OPEN_BRIDGE -> "While open, plus a brief wake-up on Zalo notifications"
}
