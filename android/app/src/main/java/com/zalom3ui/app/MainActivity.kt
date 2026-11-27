package com.zalom3ui.app

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.res.Configuration
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.view.ViewTreeObserver
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.core.content.pm.ShortcutManagerCompat
import androidx.lifecycle.compose.LifecycleResumeEffect
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.zalom3ui.app.data.PendingShare
import com.zalom3ui.app.data.SettingsStore
import com.zalom3ui.app.data.ThreadRef
import com.zalom3ui.app.data.effectiveReducedMotion
import com.zalom3ui.app.data.systemReducedMotion
import com.zalom3ui.app.security.AppLock
import com.zalom3ui.app.security.AppLockPolicy
import com.zalom3ui.app.security.AppLockScreen
import com.zalom3ui.app.server.ConversationShortcuts
import com.zalom3ui.app.server.StartupLog
import com.zalom3ui.app.server.StartupTiming
import com.zalom3ui.app.server.ZaloCore
import com.zalom3ui.app.ui.App
import com.zalom3ui.app.ui.AppTheme
import com.zalom3ui.app.ui.AppViewModel
import com.zalom3ui.app.ui.VoicePlayback
import com.zalom3ui.app.ui.isAppInDarkTheme
import kotlinx.coroutines.delay

class MainActivity : ComponentActivity() {
    private val viewModel: AppViewModel by viewModels()
    private var lastBackgroundMs: Long = 0L
    private var isLocked by mutableStateOf(false)
    private var lockError by mutableStateOf<String?>(null)
    private var lockPromptGeneration by mutableStateOf(0)
    private var processStartUptimeMs = 0L
    private var trackStartup = false
    private var firstFrameUptimeMs = 0L
    private var fullyDrawnReported = false
    private var notificationPermissionAsked = false
    private var firstEdgeToEdgePass = true

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        processStartUptimeMs = android.os.Process.getStartUptimeMillis()
        trackStartup = StartupTiming.isColdStart(
            processStartUptimeMs, SystemClock.uptimeMillis(), hasSavedState = savedInstanceState != null,
        ) && StartupTiming.claim()
        val settings = SettingsStore(this)
        // Start the on-phone core now, in parallel with the first frame (it used to start with the first API call).
        // Only for an activity launch: a start from a notification or broadcast keeps its own path.
        if (settings.load().onDevice) {
            val app = applicationContext
            Thread({ runCatching { ZaloCore.ensureStarted(app) } }, "core-start").apply { isDaemon = true }.start()
        }
        AppLock.applyScreenSecurity(this, settings.screenSecurity)
        if (savedInstanceState != null) {
            isLocked = savedInstanceState.getBoolean(KEY_LOCKED, false)
            lastBackgroundMs = savedInstanceState.getLong(KEY_LAST_BG_MS, 0L)
        } else {
            isLocked = AppLockPolicy.shouldLock(
                enabled = settings.appLockEnabled,
                lastBackgroundMs = 0L,
                nowMs = SystemClock.elapsedRealtime(),
                timeoutMs = settings.appLockTimeout.timeoutMs,
            )
            if (isLocked) lockPromptGeneration++
        }
        setContent {
            val state by viewModel.state.collectAsStateWithLifecycle()
            // Once the chat list has data: report fully drawn and time the start; the notification permission is asked
            // afterwards (or after a short wait when there is no list yet), so neither competes with the first frames.
            val hasThreads = state.threads.isNotEmpty()
            LaunchedEffect(hasThreads) {
                if (hasThreads) onListData(state.threads.size) else delay(NOTIFICATION_ASK_FALLBACK_MS)
                delay(NOTIFICATION_ASK_DELAY_MS)
                requestNotificationPermissionOnce()
            }
            val userReducedMotion by viewModel.reducedMotion.collectAsStateWithLifecycle()
            var systemReduced by remember { mutableStateOf(systemReducedMotion(this@MainActivity)) }
            LifecycleResumeEffect(Unit) {
                systemReduced = systemReducedMotion(this@MainActivity)
                onPauseOrDispose {}
            }
            val reducedMotion = effectiveReducedMotion(userReducedMotion, systemReduced)
            val dark = isAppInDarkTheme(state.appearance)
            // Status/navigation bar icons follow the app's theme, which can differ from the system's.
            DisposableEffect(dark) {
                // onCreate already applied the system-default style, which is identical when the app's theme matches the system's.
                val systemDark = (resources.configuration.uiMode and Configuration.UI_MODE_NIGHT_MASK) == Configuration.UI_MODE_NIGHT_YES
                val redundant = firstEdgeToEdgePass && dark == systemDark
                firstEdgeToEdgePass = false
                if (!redundant) enableEdgeToEdge(
                    statusBarStyle = if (dark) {
                        SystemBarStyle.dark(Color.TRANSPARENT)
                    } else {
                        SystemBarStyle.light(Color.TRANSPARENT, Color.TRANSPARENT)
                    },
                    navigationBarStyle = if (dark) {
                        SystemBarStyle.dark(DARK_SCRIM)
                    } else {
                        SystemBarStyle.light(LIGHT_SCRIM, DARK_SCRIM)
                    },
                )
                onDispose {}
            }
            AppTheme(state.appearance, reducedMotion = reducedMotion) {
                if (isLocked) {
                    LaunchedEffect(lockPromptGeneration) {
                        triggerUnlock()
                    }
                    AppLockScreen(
                        onUnlock = { triggerUnlock() },
                        error = lockError,
                    )
                } else {
                    App(viewModel)
                }
            }
        }
        handleOpenChat(intent)
        handleShareIntent(intent)
        if (trackStartup) {
            val decor = window.decorView
            decor.viewTreeObserver.addOnPreDrawListener(object : ViewTreeObserver.OnPreDrawListener {
                override fun onPreDraw(): Boolean {
                    firstFrameUptimeMs = SystemClock.uptimeMillis()
                    decor.viewTreeObserver.removeOnPreDrawListener(this)
                    return true
                }
            })
        }
    }

    /** The chat list has its first data on screen. */
    private fun onListData(threads: Int) {
        val now = SystemClock.uptimeMillis()
        if (!fullyDrawnReported) {
            fullyDrawnReported = true
            reportFullyDrawn()
        }
        if (trackStartup) {
            trackStartup = false
            val line = StartupTiming.format(
                firstFrameMs = StartupTiming.sinceStart(processStartUptimeMs, firstFrameUptimeMs),
                coreReadyMs = StartupTiming.sinceStart(processStartUptimeMs, StartupTiming.coreReadyUptimeMs),
                listDataMs = now - processStartUptimeMs,
                threads = threads,
                steps = StartupTiming.formatMarks(processStartUptimeMs, StartupTiming.snapshotMarks()),
            )
            StartupLog.write(this, line)
        }
    }

    /** Lets the on-phone server show its "connected" and message notifications (Android 13+); asked once per start until granted. */
    private fun requestNotificationPermissionOnce() {
        if (notificationPermissionAsked) return
        notificationPermissionAsked = true
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 0)
        }
    }

    override fun onSaveInstanceState(outState: Bundle) {
        super.onSaveInstanceState(outState)
        outState.putBoolean(KEY_LOCKED, isLocked)
        outState.putLong(KEY_LAST_BG_MS, lastBackgroundMs)
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleOpenChat(intent)
        handleShareIntent(intent)
    }

    private fun triggerUnlock() {
        lockError = null
        AppLock.authenticate(
            activity = this,
            onSuccess = {
                isLocked = false
                lockError = null
                lastBackgroundMs = 0L
            },
            onError = { message ->
                lockError = message
            },
        )
    }

    /** A tapped message notification names the chat to open. */
    private fun handleOpenChat(intent: Intent?) {
        val kind = intent?.getStringExtra(EXTRA_KIND) ?: return
        val id = intent.getStringExtra(EXTRA_ID) ?: return
        intent.removeExtra(EXTRA_KIND)
        intent.removeExtra(EXTRA_ID)
        viewModel.requestOpen(ThreadRef(kind, id))
    }

    /** Content shared into Zalỏ from another app via the Android share sheet (including Direct Share targets). */
    private fun handleShareIntent(intent: Intent?) {
        val action = intent?.action ?: return
        if (action != Intent.ACTION_SEND && action != Intent.ACTION_SEND_MULTIPLE) return
        val shortcutId = intent.getStringExtra(ShortcutManagerCompat.EXTRA_SHORTCUT_ID)
        val directTarget = ConversationShortcuts.parseShortcutId(shortcutId)
        val text = intent.getStringExtra(Intent.EXTRA_TEXT)?.takeIf { it.isNotBlank() }
        val uris: List<Uri> = when (action) {
            Intent.ACTION_SEND -> {
                val uri = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableExtra(Intent.EXTRA_STREAM, Uri::class.java)
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableExtra(Intent.EXTRA_STREAM)
                }
                listOfNotNull(uri)
            }
            Intent.ACTION_SEND_MULTIPLE -> {
                if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
                    intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM, Uri::class.java).orEmpty()
                } else {
                    @Suppress("DEPRECATION")
                    intent.getParcelableArrayListExtra<Uri>(Intent.EXTRA_STREAM).orEmpty()
                }
            }
            else -> emptyList()
        }
        intent.action = null
        intent.removeExtra(Intent.EXTRA_TEXT)
        intent.removeExtra(Intent.EXTRA_STREAM)
        intent.removeExtra(ShortcutManagerCompat.EXTRA_SHORTCUT_ID)
        if (text != null || uris.isNotEmpty()) {
            viewModel.setPendingShare(PendingShare(text = text, uris = uris, target = directTarget))
        }
    }

    override fun onStart() {
        super.onStart()
        val settings = SettingsStore(this)
        if (!isLocked && lastBackgroundMs > 0L && AppLockPolicy.shouldLock(
                enabled = settings.appLockEnabled,
                lastBackgroundMs = lastBackgroundMs,
                nowMs = SystemClock.elapsedRealtime(),
                timeoutMs = settings.appLockTimeout.timeoutMs,
            )
        ) {
            isLocked = true
            lockError = null
            lockPromptGeneration++
        }
        viewModel.startLiveUpdates()
    }

    override fun onResume() {
        super.onResume()
        AppLock.applyScreenSecurity(this, SettingsStore(this).screenSecurity)
    }

    override fun onStop() {
        if (!isLocked && !isChangingConfigurations) {
            lastBackgroundMs = SystemClock.elapsedRealtime()
        }
        VoicePlayback.stop()
        viewModel.stopLiveUpdates()
        super.onStop()
    }

    companion object {
        private const val EXTRA_KIND = "open_kind"
        private const val EXTRA_ID = "open_id"
        private const val NOTIFICATION_ASK_DELAY_MS = 400L
        private const val NOTIFICATION_ASK_FALLBACK_MS = 2_500L
        private const val KEY_LOCKED = "app_lock_locked"
        private const val KEY_LAST_BG_MS = "app_lock_last_bg_ms"

        fun openChatIntent(context: Context, ref: ThreadRef): Intent =
            Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TOP or Intent.FLAG_ACTIVITY_SINGLE_TOP)
                .putExtra(EXTRA_KIND, ref.kind)
                .putExtra(EXTRA_ID, ref.id)

        // Same scrims androidx uses by default for 3-button navigation.
        private val LIGHT_SCRIM = Color.argb(0xe6, 0xFF, 0xFF, 0xFF)
        private val DARK_SCRIM = Color.argb(0x80, 0x1b, 0x1b, 0x1b)
    }
}
