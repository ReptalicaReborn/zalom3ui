package com.zalom3ui.app.ui

import android.os.Build
import android.widget.Toast
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.Logout
import androidx.compose.material.icons.outlined.Cloud
import androidx.compose.material.icons.outlined.DarkMode
import androidx.compose.material.icons.outlined.DeleteSweep
import androidx.compose.material.icons.outlined.FormatColorFill
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.outlined.MotionPhotosOff
import androidx.compose.material.icons.outlined.Palette
import androidx.compose.material.icons.outlined.PhoneAndroid
import androidx.compose.material.icons.outlined.RestartAlt
import androidx.compose.material.icons.outlined.Schedule
import androidx.compose.material.icons.outlined.Security
import androidx.compose.material.icons.outlined.Share
import androidx.compose.material.icons.outlined.Sync
import androidx.compose.material.icons.outlined.Visibility
import androidx.compose.material.icons.outlined.VisibilityOff
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.ListItem
import androidx.compose.material3.ListItemDefaults
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MediumFlexibleTopAppBar
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.ToggleButton
import androidx.compose.material3.ToggleButtonDefaults
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.lifecycle.compose.LifecycleResumeEffect
import com.zalom3ui.app.data.ApiClient
import com.zalom3ui.app.data.AppLockTimeout
import com.zalom3ui.app.data.Appearance
import com.zalom3ui.app.data.ChatAccent
import com.zalom3ui.app.data.ServerSettings
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.data.ThemeMode
import com.zalom3ui.app.security.AppLock

@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun SettingsScreen(
    current: ServerSettings,
    appearance: Appearance,
    profileName: String?,
    profileId: String?,
    profileAvatar: String? = null,
    loggedIn: Boolean,
    canGoBack: Boolean,
    onBack: () -> Unit,
    onSave: (ServerSettings) -> Unit,
    onAppearance: (Appearance) -> Unit,
    onRestartServer: () -> Unit,
    onLogout: () -> Unit,
    reducedMotion: Boolean = false,
    onReducedMotion: (Boolean) -> Unit = {},
    onShareCallLog: () -> Unit = {},
    onClearCallLog: () -> Unit = {},
    onSyncHistory: () -> Unit = {},
) {
    var onDevice by rememberSaveable { mutableStateOf(current.onDevice) }
    var serverUrl by rememberSaveable { mutableStateOf(current.serverUrl) }
    var token by remember { mutableStateOf(current.token) }
    var showToken by remember { mutableStateOf(false) }
    var confirmLogout by rememberSaveable { mutableStateOf(false) }
    var pickTheme by rememberSaveable { mutableStateOf(false) }
    var previewTaps by rememberSaveable { mutableStateOf(0) }
    var easterEggUnlocked by rememberSaveable { mutableStateOf(appearance.ultraExpressive) }
    val edited = ServerSettings(onDevice, serverUrl.trim(), token.trim())
    val changed = edited != current
    val urlValid = onDevice || ApiClient.normalizeServerUrl(serverUrl) != null
    val dynamicActive = appearance.dynamicColor && Build.VERSION.SDK_INT >= Build.VERSION_CODES.S
    val scrollBehavior = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()
    val transparentListItemColors = ListItemDefaults.colors(containerColor = Color.Transparent)

    Scaffold(
        modifier = Modifier.nestedScroll(scrollBehavior.nestedScrollConnection),
        topBar = {
            MediumFlexibleTopAppBar(
                title = { Text("Settings") },
                navigationIcon = {
                    if (canGoBack) {
                        IconButton(onClick = onBack) {
                            Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                        }
                    }
                },
                scrollBehavior = scrollBehavior,
            )
        },
    ) { padding ->
        Column(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .verticalScroll(rememberScrollState()),
        ) {
            if (loggedIn) {
                Surface(
                    shape = RoundedCornerShape(24.dp),
                    color = MaterialTheme.colorScheme.surfaceContainerLow,
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 8.dp),
                ) {
                    Row(
                        Modifier.fillMaxWidth().padding(16.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Avatar(profileName.orEmpty(), profileAvatar.orEmpty(), key = profileId ?: "me", isGroup = false, size = 64.dp)
                        Spacer(Modifier.width(16.dp))
                        Column {
                            Text(profileName ?: "Zalo account", style = MaterialTheme.typography.titleLarge)
                            Text(
                                "Logged in to Zalo",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                }
            }

            SectionHeader("Appearance")
            val showEasterEgg = easterEggUnlocked || appearance.ultraExpressive
            val appearanceItems = buildList<@Composable () -> Unit> {
                add {
                    Column(
                        Modifier
                            .fillMaxWidth()
                            .padding(16.dp),
                        verticalArrangement = Arrangement.spacedBy(16.dp),
                    ) {
                        MiniChatPreview(
                            ultraExpressive = appearance.ultraExpressive,
                            onTap = {
                                if (!showEasterEgg) {
                                    previewTaps++
                                    if (previewTaps >= 5) {
                                        easterEggUnlocked = true
                                        onAppearance(appearance.copy(ultraExpressive = true))
                                    }
                                }
                            },
                        )
                        if (!showEasterEgg && previewTaps in 2..4) {
                            val left = 5 - previewTaps
                            Text(
                                "Tap $left more ${if (left == 1) "time" else "times"} to make the app 4x faster…",
                                style = MaterialTheme.typography.labelMedium,
                                color = MaterialTheme.colorScheme.primary,
                            )
                        }
                        AccentSwatchPicker(
                            selected = appearance.accent,
                            enabled = !dynamicActive,
                            onSelect = { onAppearance(appearance.copy(accent = it)) },
                        )
                    }
                }
                add {
                    ListItem(
                        modifier = Modifier.clickable { pickTheme = true },
                        colors = transparentListItemColors,
                        leadingContent = { Icon(Icons.Outlined.DarkMode, null) },
                        headlineContent = { Text("Dark mode") },
                        supportingContent = { Text(appearance.themeMode.label) },
                    )
                }
                add {
                    ListItem(
                        modifier = Modifier.clickable {
                            onAppearance(appearance.copy(tintedBackground = !appearance.tintedBackground))
                        },
                        colors = transparentListItemColors,
                        leadingContent = { Icon(Icons.Outlined.FormatColorFill, null) },
                        headlineContent = { Text("Tinted chat background") },
                        supportingContent = { Text("Subtle accent wash behind messages") },
                        trailingContent = {
                            Switch(
                                checked = appearance.tintedBackground,
                                onCheckedChange = { onAppearance(appearance.copy(tintedBackground = it)) },
                            )
                        },
                    )
                }
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.S) {
                    add {
                        ListItem(
                            modifier = Modifier.clickable {
                                onAppearance(appearance.copy(dynamicColor = !appearance.dynamicColor))
                            },
                            colors = transparentListItemColors,
                            leadingContent = { Icon(Icons.Outlined.Palette, null) },
                            headlineContent = { Text("Wallpaper colors") },
                            supportingContent = { Text("Use colors from your wallpaper instead of Color") },
                            trailingContent = {
                                Switch(
                                    checked = appearance.dynamicColor,
                                    onCheckedChange = { onAppearance(appearance.copy(dynamicColor = it)) },
                                )
                            },
                        )
                    }
                }
                add {
                    ListItem(
                        modifier = Modifier.clickable { onReducedMotion(!reducedMotion) },
                        colors = transparentListItemColors,
                        leadingContent = { Icon(Icons.Outlined.MotionPhotosOff, null) },
                        headlineContent = { Text("Reduced motion") },
                        supportingContent = { Text("Simplify transitions, disable blur and wave animations") },
                        trailingContent = {
                            Switch(
                                checked = reducedMotion,
                                onCheckedChange = onReducedMotion,
                            )
                        },
                    )
                }
                if (showEasterEgg) {
                    add {
                        ListItem(
                            modifier = Modifier.clickable {
                                onAppearance(appearance.copy(ultraExpressive = !appearance.ultraExpressive))
                            },
                            colors = transparentListItemColors,
                            leadingContent = { FourXBadge(size = 36.dp, compact = true) },
                            headlineContent = { Text("4x faster mode") },
                            supportingContent = {
                                Text("Easter egg · Users spot key buttons up to 4x faster (M3 Expressive™)")
                            },
                            trailingContent = {
                                Switch(
                                    checked = appearance.ultraExpressive,
                                    onCheckedChange = { onAppearance(appearance.copy(ultraExpressive = it)) },
                                )
                            },
                        )
                    }
                }
            }
            ExpressiveSettingsGroup(items = appearanceItems)

            PrivacySecuritySettings()

            SectionHeader("Zalo runs on")
            Surface(
                shape = RoundedCornerShape(24.dp),
                color = MaterialTheme.colorScheme.surfaceContainerLow,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp),
            ) {
                Column(
                    Modifier
                        .fillMaxWidth()
                        .padding(16.dp),
                ) {
                    val serverToggleColors = ToggleButtonDefaults.tonalToggleButtonColors(
                        containerColor = MaterialTheme.colorScheme.surfaceContainerHigh,
                        contentColor = MaterialTheme.colorScheme.onSurfaceVariant,
                        checkedContainerColor = MaterialTheme.colorScheme.primaryContainer,
                        checkedContentColor = MaterialTheme.colorScheme.onPrimaryContainer,
                    )
                    Row(
                        modifier = Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween),
                    ) {
                        ToggleButton(
                            checked = onDevice,
                            onCheckedChange = { if (it) onDevice = true },
                            shapes = ButtonGroupDefaults.connectedLeadingButtonShapes(),
                            colors = serverToggleColors,
                            modifier = Modifier.weight(1f),
                        ) {
                            Icon(
                                Icons.Outlined.PhoneAndroid,
                                contentDescription = null,
                                modifier = Modifier.size(ToggleButtonDefaults.IconSize),
                            )
                            Spacer(Modifier.size(ToggleButtonDefaults.IconSpacing))
                            Text("This phone")
                        }
                        ToggleButton(
                            checked = !onDevice,
                            onCheckedChange = { if (it) onDevice = false },
                            shapes = ButtonGroupDefaults.connectedTrailingButtonShapes(),
                            colors = serverToggleColors,
                            modifier = Modifier.weight(1f),
                        ) {
                            Icon(
                                Icons.Outlined.Cloud,
                                contentDescription = null,
                                modifier = Modifier.size(ToggleButtonDefaults.IconSize),
                            )
                            Spacer(Modifier.size(ToggleButtonDefaults.IconSpacing))
                            Text("Remote server")
                        }
                    }
                    Text(
                        if (onDevice) {
                            "The Zalo connection runs inside this app and stays connected in the background."
                        } else {
                            "Connect to a Zalỏ server running on another computer or VPS."
                        },
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                        modifier = Modifier.padding(top = 12.dp),
                    )
                    if (!onDevice) {
                        Spacer(Modifier.height(14.dp))
                        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                            OutlinedTextField(
                                value = serverUrl,
                                onValueChange = { serverUrl = it },
                                label = { Text("Server address") },
                                placeholder = { Text("https://zalo.example.com") },
                                singleLine = true,
                                isError = serverUrl.isNotBlank() && !urlValid,
                                supportingText = if (serverUrl.isNotBlank() && !urlValid) ({ Text("Not a valid address") }) else null,
                                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri),
                                shape = RoundedCornerShape(16.dp),
                                modifier = Modifier.fillMaxWidth(),
                            )
                            OutlinedTextField(
                                value = token,
                                onValueChange = { token = it },
                                label = { Text("Access token") },
                                singleLine = true,
                                visualTransformation = if (showToken) VisualTransformation.None else PasswordVisualTransformation(),
                                trailingIcon = {
                                    IconButton(onClick = { showToken = !showToken }) {
                                        Icon(
                                            if (showToken) Icons.Outlined.VisibilityOff else Icons.Outlined.Visibility,
                                            contentDescription = if (showToken) "Hide token" else "Show token",
                                        )
                                    }
                                },
                                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Password),
                                shape = RoundedCornerShape(16.dp),
                                modifier = Modifier.fillMaxWidth(),
                            )
                        }
                    }
                    if (changed) {
                        Row(
                            Modifier.fillMaxWidth().padding(top = 16.dp),
                            horizontalArrangement = Arrangement.End,
                        ) {
                            Button(
                                onClick = { onSave(edited) },
                                enabled = urlValid,
                                shape = CircleShape,
                            ) {
                                Text("Save changes")
                            }
                        }
                    }
                }
            }

            if (current.onDevice) IncomingCallSettings()

            // How long Zalo stays connected (only the on-phone core has this choice), and codes from Zalo's notifications.
            StayConnectedSettings(showModes = current.onDevice)

            if (current.onDevice || loggedIn) {
                SectionHeader("Account")
                val accountItems = buildList<@Composable () -> Unit> {
                    if (current.onDevice) {
                        add {
                            ListItem(
                                modifier = Modifier.clickable(onClick = onRestartServer),
                                colors = transparentListItemColors,
                                leadingContent = { Icon(Icons.Outlined.RestartAlt, null) },
                                headlineContent = { Text("Restart Zalo service") },
                                supportingContent = { Text("If messages stop arriving") },
                            )
                        }
                    }
                    if (loggedIn) {
                        add {
                            ListItem(
                                modifier = Modifier.clickable(onClick = onSyncHistory),
                                colors = transparentListItemColors,
                                leadingContent = { Icon(Icons.Outlined.Sync, null) },
                                headlineContent = { Text("Sync messages from phone") },
                                supportingContent = {
                                    Text("Copy your chat history from Zalo on your phone (the phone asks you to confirm)")
                                },
                            )
                        }
                        add {
                            ListItem(
                                modifier = Modifier.clickable { confirmLogout = true },
                                colors = transparentListItemColors,
                                leadingContent = {
                                    Icon(Icons.AutoMirrored.Outlined.Logout, null, tint = MaterialTheme.colorScheme.error)
                                },
                                headlineContent = {
                                    Text("Log out of Zalo", color = MaterialTheme.colorScheme.error)
                                },
                                supportingContent = {
                                    Text("Removes the session and message history from this device")
                                },
                            )
                        }
                    }
                }
                ExpressiveSettingsGroup(items = accountItems)
            }

            if (current.onDevice && loggedIn) {
                SectionHeader("Developer")
                ExpressiveSettingsGroup(
                    items = listOf<@Composable () -> Unit>(
                        {
                            ListItem(
                                modifier = Modifier.clickable(onClick = onShareCallLog),
                                colors = transparentListItemColors,
                                leadingContent = { Icon(Icons.Outlined.Share, null) },
                                headlineContent = { Text("Share call log") },
                                supportingContent = {
                                    Text("Call events Zalo sent (call ids, server addresses; no messages), to help debug calls")
                                },
                            )
                        },
                        {
                            ListItem(
                                modifier = Modifier.clickable(onClick = onClearCallLog),
                                colors = transparentListItemColors,
                                leadingContent = { Icon(Icons.Outlined.DeleteSweep, null) },
                                headlineContent = { Text("Clear call log") },
                            )
                        },
                    ) + replayDebugItems() + socketLogItems(),
                )
            }
            Spacer(Modifier.height(28.dp))
        }
    }

    if (pickTheme) {
        AlertDialog(
            onDismissRequest = { pickTheme = false },
            title = { Text("Dark mode") },
            text = {
                Column {
                    ThemeMode.entries.forEach { mode ->
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .selectable(selected = appearance.themeMode == mode, onClick = {
                                    onAppearance(appearance.copy(themeMode = mode))
                                    pickTheme = false
                                })
                                .padding(vertical = 4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(selected = appearance.themeMode == mode, onClick = null)
                            Spacer(Modifier.width(12.dp))
                            Text(mode.label, style = MaterialTheme.typography.bodyLarge)
                        }
                    }
                }
            },
            confirmButton = {},
            dismissButton = { TextButton(onClick = { pickTheme = false }) { Text("Cancel") } },
        )
    }

    if (confirmLogout) {
        AlertDialog(
            onDismissRequest = { confirmLogout = false },
            title = { Text("Log out?") },
            text = { Text("You'll need to scan a QR code with the Zalo app to log in again.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmLogout = false
                    onLogout()
                }) { Text("Log out", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { confirmLogout = false }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun PrivacySecuritySettings() {
    val context = LocalContext.current
    val activity = remember(context) { AppLock.findActivity(context) }
    val store = remember(context) { SettingsStore(context) }
    var canAuth by remember { mutableStateOf(AppLock.canAuthenticate(context)) }
    var appLockEnabled by remember { mutableStateOf(store.appLockEnabled) }
    var appLockTimeout by remember { mutableStateOf(store.appLockTimeout) }
    var screenSecurity by remember { mutableStateOf(store.screenSecurity) }
    var pickTimeout by rememberSaveable { mutableStateOf(false) }

    LifecycleResumeEffect(Unit) {
        canAuth = AppLock.canAuthenticate(context)
        onPauseOrDispose { }
    }

    val transparentListItemColors = ListItemDefaults.colors(containerColor = Color.Transparent)

    fun setAppLock(enable: Boolean) {
        if (!enable) {
            store.appLockEnabled = false
            appLockEnabled = false
            return
        }
        if (!canAuth) {
            Toast.makeText(context, "Set up a screen lock in device settings first", Toast.LENGTH_SHORT).show()
            return
        }
        if (activity != null) {
            AppLock.authenticate(
                activity = activity,
                onSuccess = {
                    store.appLockEnabled = true
                    appLockEnabled = true
                },
                onError = { message ->
                    if (message.isNotBlank()) {
                        Toast.makeText(context, message, Toast.LENGTH_SHORT).show()
                    }
                },
            )
        } else {
            store.appLockEnabled = true
            appLockEnabled = true
        }
    }

    fun setScreenSecurity(secure: Boolean) {
        store.screenSecurity = secure
        screenSecurity = secure
        activity?.let { AppLock.applyScreenSecurity(it, secure) }
    }

    SectionHeader("Privacy & security")
    val privacyItems = buildList<@Composable () -> Unit> {
        add {
            ListItem(
                modifier = Modifier.clickable(enabled = canAuth || appLockEnabled) {
                    setAppLock(!appLockEnabled)
                },
                colors = transparentListItemColors,
                leadingContent = { Icon(Icons.Outlined.Lock, null) },
                headlineContent = { Text("App lock") },
                supportingContent = {
                    Text(
                        if (canAuth || appLockEnabled) {
                            "Require biometrics or device PIN to unlock Zalỏ"
                        } else {
                            "Set up a screen lock in device settings to enable"
                        },
                    )
                },
                trailingContent = {
                    Switch(
                        checked = appLockEnabled,
                        enabled = canAuth || appLockEnabled,
                        onCheckedChange = ::setAppLock,
                    )
                },
            )
        }
        if (appLockEnabled) {
            add {
                ListItem(
                    modifier = Modifier.clickable { pickTimeout = true },
                    colors = transparentListItemColors,
                    leadingContent = { Icon(Icons.Outlined.Schedule, null) },
                    headlineContent = { Text("Auto-lock") },
                    supportingContent = { Text(appLockTimeout.label) },
                )
            }
        }
        add {
            ListItem(
                modifier = Modifier.clickable { setScreenSecurity(!screenSecurity) },
                colors = transparentListItemColors,
                leadingContent = { Icon(Icons.Outlined.Security, null) },
                headlineContent = { Text("Screen security") },
                supportingContent = { Text("Block screenshots & hide app preview in Recents") },
                trailingContent = {
                    Switch(
                        checked = screenSecurity,
                        onCheckedChange = ::setScreenSecurity,
                    )
                },
            )
        }
    }
    ExpressiveSettingsGroup(items = privacyItems)

    if (pickTimeout) {
        AlertDialog(
            onDismissRequest = { pickTimeout = false },
            title = { Text("Auto-lock") },
            text = {
                Column {
                    AppLockTimeout.entries.forEach { option ->
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .selectable(
                                    selected = appLockTimeout == option,
                                    role = Role.RadioButton,
                                    onClick = {
                                        store.appLockTimeout = option
                                        appLockTimeout = option
                                        pickTimeout = false
                                    },
                                )
                                .padding(vertical = 4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            RadioButton(selected = appLockTimeout == option, onClick = null)
                            Spacer(Modifier.width(12.dp))
                            Text(option.label, style = MaterialTheme.typography.bodyLarge)
                        }
                    }
                }
            },
            confirmButton = {},
            dismissButton = { TextButton(onClick = { pickTimeout = false }) { Text("Cancel") } },
        )
    }
}

@Composable
private fun MiniChatPreview(
    ultraExpressive: Boolean = false,
    onTap: () -> Unit = {},
) {
    val chat = LocalChatColors.current
    Box(Modifier.fillMaxWidth()) {
        Surface(
            shape = RoundedCornerShape(18.dp),
            color = chat.background,
            border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.55f)),
            modifier = Modifier
                .fillMaxWidth()
                .clip(RoundedCornerShape(18.dp))
                .clickable(onClick = onTap),
        ) {
            Column(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 14.dp, vertical = 14.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Box(
                    Modifier
                        .align(Alignment.Start)
                        .widthIn(max = 260.dp)
                        .background(
                            chat.incoming,
                            RoundedCornerShape(topStart = 18.dp, topEnd = 18.dp, bottomStart = 6.dp, bottomEnd = 18.dp),
                        )
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                ) {
                    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Text(
                            if (ultraExpressive) "Whoa, why are the buttons so huge?" else "How does the new theme look?",
                            style = MaterialTheme.typography.bodyMedium,
                            color = chat.onIncoming,
                        )
                        Text(
                            "10:41",
                            style = MaterialTheme.typography.labelSmall,
                            color = chat.onIncoming.copy(alpha = 0.65f),
                            modifier = Modifier.align(Alignment.End),
                        )
                    }
                }
                Box(
                    Modifier
                        .align(Alignment.End)
                        .widthIn(max = 260.dp)
                        .background(
                            chat.outgoing,
                            RoundedCornerShape(topStart = 18.dp, topEnd = 18.dp, bottomStart = 18.dp, bottomEnd = 6.dp),
                        )
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                ) {
                    Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Text(
                            if (ultraExpressive) "4x faster spotting. Pure science." else "Crisp, calm, and expressive.",
                            style = MaterialTheme.typography.bodyMedium,
                            color = chat.onOutgoing,
                        )
                        Row(
                            modifier = Modifier.align(Alignment.End),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            Text(
                                "10:42",
                                style = MaterialTheme.typography.labelSmall,
                                color = chat.onOutgoing.copy(alpha = 0.80f),
                            )
                            DeliveryIcon(
                                state = Delivery.READ,
                                tint = chat.onOutgoing.copy(alpha = 0.90f),
                                background = chat.outgoing,
                            )
                        }
                    }
                }
            }
        }
        if (ultraExpressive) {
            FourXBadge(
                size = 68.dp,
                modifier = Modifier
                    .align(Alignment.TopEnd)
                    .padding(top = 4.dp, end = 4.dp),
            )
        }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun AccentSwatchPicker(
    selected: ChatAccent,
    enabled: Boolean,
    onSelect: (ChatAccent) -> Unit,
) {
    Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(
            Modifier.fillMaxWidth(),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                "Color",
                style = MaterialTheme.typography.titleSmall,
                color = if (enabled) {
                    MaterialTheme.colorScheme.onSurface
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
            )
            Text(
                if (enabled) selected.label else "Wallpaper colors active",
                style = MaterialTheme.typography.labelMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .alpha(if (enabled) 1f else 0.38f),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            ChatAccent.entries.forEach { accent ->
                val isSelected = enabled && accent == selected
                val cornerRadius by animateDpAsState(
                    targetValue = if (isSelected) 14.dp else 22.dp,
                    animationSpec = MaterialTheme.motionScheme.fastSpatialSpec(),
                    label = "accentCorner",
                )
                val shape = RoundedCornerShape(cornerRadius)
                Box(
                    modifier = Modifier
                        .size(44.dp)
                        .clip(shape)
                        .background(accentSwatchColor(accent), shape)
                        .semantics {
                            contentDescription = accent.label
                            this.selected = isSelected
                        }
                        .clickable(
                            enabled = enabled,
                            role = Role.RadioButton,
                            onClick = { onSelect(accent) },
                        ),
                    contentAlignment = Alignment.Center,
                ) {
                    val checkScale by animateFloatAsState(
                        targetValue = if (isSelected) 1f else 0f,
                        animationSpec = MaterialTheme.motionScheme.fastSpatialSpec(),
                        label = "accentCheck",
                    )
                    if (checkScale > 0.01f) {
                        Icon(
                            Icons.Rounded.Check,
                            contentDescription = null,
                            tint = if (accent == ChatAccent.REPTDOC_GOLD) Color(0xFF1A1206) else Color.White,
                            modifier = Modifier
                                .size(22.dp)
                                .graphicsLayer {
                                    scaleX = checkScale
                                    scaleY = checkScale
                                    alpha = checkScale.coerceIn(0f, 1f)
                                },
                        )
                    }
                }
            }
        }
    }
}
