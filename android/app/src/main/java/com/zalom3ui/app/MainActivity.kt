package com.zalom3ui.app

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
import android.net.Uri
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.SystemBarStyle
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.viewModels
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import com.zalom3ui.app.data.PendingShare
import com.zalom3ui.app.data.ThreadRef
import com.zalom3ui.app.ui.App
import com.zalom3ui.app.ui.AppTheme
import com.zalom3ui.app.ui.AppViewModel
import com.zalom3ui.app.ui.VoicePlayback
import com.zalom3ui.app.ui.isAppInDarkTheme

class MainActivity : ComponentActivity() {
    private val viewModel: AppViewModel by viewModels()

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        setContent {
            val state by viewModel.state.collectAsStateWithLifecycle()
            val dark = isAppInDarkTheme(state.appearance)
            // Status/navigation bar icons follow the app's theme, which can differ from the system's.
            DisposableEffect(dark) {
                enableEdgeToEdge(
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
            AppTheme(state.appearance) { App(viewModel) }
        }
        handleOpenChat(intent)
        handleShareIntent(intent)
        // Lets the on-phone server show its "connected" and message notifications (Android 13+).
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) {
            requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 0)
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        handleOpenChat(intent)
        handleShareIntent(intent)
    }

    /** A tapped message notification names the chat to open. */
    private fun handleOpenChat(intent: Intent?) {
        val kind = intent?.getStringExtra(EXTRA_KIND) ?: return
        val id = intent.getStringExtra(EXTRA_ID) ?: return
        intent.removeExtra(EXTRA_KIND)
        intent.removeExtra(EXTRA_ID)
        viewModel.requestOpen(ThreadRef(kind, id))
    }

    /** Content shared into Zalỏ from another app via the Android share sheet. */
    private fun handleShareIntent(intent: Intent?) {
        val action = intent?.action ?: return
        if (action != Intent.ACTION_SEND && action != Intent.ACTION_SEND_MULTIPLE) return
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
        if (text != null || uris.isNotEmpty()) {
            viewModel.setPendingShare(PendingShare(text = text, uris = uris))
        }
    }

    override fun onStart() {
        super.onStart()
        viewModel.startLiveUpdates()
    }

    override fun onStop() {
        VoicePlayback.stop()
        viewModel.stopLiveUpdates()
        super.onStop()
    }

    companion object {
        private const val EXTRA_KIND = "open_kind"
        private const val EXTRA_ID = "open_id"

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
