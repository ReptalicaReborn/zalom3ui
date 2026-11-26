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
import androidx.compose.material.icons.outlined.NotificationsActive
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
 * Settings → "Stay connected" (only while Zalo runs on this phone): how long Zalỏ stays connected to Zalo. Choosing a
 * mode takes effect at once ([StayConnectedCoordinator.applyMode]). The notification bridge needs notification access;
 * without it that mode behaves like "While open", and this says so.
 */
@Composable
fun StayConnectedSettings() {
    val context = LocalContext.current
    val store = remember { SettingsStore(context) }
    var stay by remember { mutableStateOf(store.loadStay()) }
    var accessGranted by remember { mutableStateOf(ZaloNotificationListener.isAccessGranted(context)) }
    LifecycleResumeEffect(Unit) {
        accessGranted = ZaloNotificationListener.isAccessGranted(context)
        onPauseOrDispose { }
    }

    val transparent = ListItemDefaults.colors(containerColor = Color.Transparent)
    val items = buildList<@Composable () -> Unit> {
        for (mode in StayConnected.entries) {
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
                            mode.summary,
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
        if (stay.mode.usesBridge) {
            add {
                ListItem(
                    colors = transparent,
                    leadingContent = { Icon(Icons.Outlined.NotificationsActive, null) },
                    headlineContent = { Text("Notification access") },
                    supportingContent = {
                        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                            Text(
                                if (accessGranted) {
                                    "Granted: Zalỏ notices when the Zalo app shows a message notification."
                                } else {
                                    "Not granted. Until you allow it, this works like \"While open\": " +
                                        "nothing connects when the Zalo app shows a notification."
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
                        Text("Remove the Zalo app's message notification once Zalỏ has shown its own for that chat. Calls are never touched.")
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

    Column {
        SectionHeader("Stay connected")
        ExpressiveSettingsGroup(items = items)
    }
}
