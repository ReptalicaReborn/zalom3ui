package com.zalom3ui.app

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.graphics.Color
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
import com.zalom3ui.app.data.ThreadRef
import com.zalom3ui.app.ui.App
import com.zalom3ui.app.ui.AppTheme
import com.zalom3ui.app.ui.AppViewModel
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
    }

    /** A tapped message notification names the chat to open. */
    private fun handleOpenChat(intent: Intent?) {
        val kind = intent?.getStringExtra(EXTRA_KIND) ?: return
        val id = intent.getStringExtra(EXTRA_ID) ?: return
        intent.removeExtra(EXTRA_KIND)
        intent.removeExtra(EXTRA_ID)
        viewModel.requestOpen(ThreadRef(kind, id))
    }

    override fun onStart() {
        super.onStart()
        viewModel.startLiveUpdates()
    }

    override fun onStop() {
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
