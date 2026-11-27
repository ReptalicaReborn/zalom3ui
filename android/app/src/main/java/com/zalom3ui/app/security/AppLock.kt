package com.zalom3ui.app.security

import android.app.Activity
import android.app.KeyguardManager
import android.content.Context
import android.content.ContextWrapper
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.Build
import android.os.CancellationSignal
import android.view.WindowManager
import androidx.activity.ComponentActivity
import androidx.activity.result.ActivityResultLauncher
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.RequiresApi
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material.icons.outlined.LockOpen
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import java.util.concurrent.atomic.AtomicInteger

/** Pure, unit-testable lock timing policy for App Lock. */
object AppLockPolicy {
    /**
     * Returns whether Zalỏ should require unlocking when entering the foreground.
     *
     * - Returns `false` when App Lock is disabled.
     * - Returns `true` on cold start (`lastBackgroundMs <= 0L`).
     * - Returns `true` when `timeoutMs <= 0L` (immediate lock).
     * - Returns `true` if the clock went backward (`nowMs < lastBackgroundMs`, fail-closed).
     * - Otherwise returns `(nowMs - lastBackgroundMs) >= timeoutMs`.
     */
    fun shouldLock(
        enabled: Boolean,
        lastBackgroundMs: Long,
        nowMs: Long,
        timeoutMs: Long,
    ): Boolean {
        if (!enabled) return false
        if (lastBackgroundMs <= 0L) return true
        if (timeoutMs <= 0L) return true
        val elapsed = nowMs - lastBackgroundMs
        if (elapsed < 0L) return true
        return elapsed >= timeoutMs
    }
}

/**
 * Biometric / device-credential App Lock and `FLAG_SECURE` screen security helper.
 * Uses only Android platform APIs (API 26–36) with zero extra Gradle dependencies.
 */
object AppLock {
    private val launcherCounter = AtomicInteger(0)

    /** Returns true if the device supports biometric or PIN/pattern/password authentication. */
    fun canAuthenticate(context: Context): Boolean {
        val km = context.getSystemService(KeyguardManager::class.java)
        val deviceSecure = km?.isDeviceSecure == true
        return when {
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.R -> {
                val bm = context.getSystemService(BiometricManager::class.java)
                val authenticators = BiometricManager.Authenticators.BIOMETRIC_STRONG or
                    BiometricManager.Authenticators.BIOMETRIC_WEAK or
                    BiometricManager.Authenticators.DEVICE_CREDENTIAL
                val bioResult = bm?.canAuthenticate(authenticators)
                bioResult == BiometricManager.BIOMETRIC_SUCCESS || deviceSecure
            }
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q -> {
                val bm = context.getSystemService(BiometricManager::class.java)
                @Suppress("DEPRECATION")
                val bioOk = bm?.canAuthenticate() == BiometricManager.BIOMETRIC_SUCCESS
                bioOk || deviceSecure
            }
            else -> deviceSecure
        }
    }

    /**
     * Prompts the user to authenticate using biometrics or device PIN/pattern/password.
     * Invokes [onSuccess] on verified unlock or [onError] with a user-facing message on cancel/failure.
     */
    fun authenticate(
        activity: Activity,
        onSuccess: () -> Unit,
        onError: (String) -> Unit,
    ) {
        if (!canAuthenticate(activity)) {
            onError("Set up a screen lock in device settings to use App lock")
            return
        }
        when {
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q -> {
                authenticateWithBiometricPrompt(activity, onSuccess, onError)
            }
            Build.VERSION.SDK_INT >= Build.VERSION_CODES.P -> {
                authenticateApi28(activity, onSuccess, onError)
            }
            else -> {
                authenticateWithKeyguard(activity, onSuccess, onError)
            }
        }
    }

    /** Sets or clears [WindowManager.LayoutParams.FLAG_SECURE] on [activity]'s window. */
    fun applyScreenSecurity(activity: Activity, secure: Boolean) {
        if (secure) {
            activity.window.addFlags(WindowManager.LayoutParams.FLAG_SECURE)
        } else {
            activity.window.clearFlags(WindowManager.LayoutParams.FLAG_SECURE)
        }
    }

    /** Unwraps a [Context] (e.g. Compose `LocalContext.current`) to its hosting [Activity], if any. */
    tailrec fun findActivity(context: Context): Activity? = when (context) {
        is Activity -> context
        is ContextWrapper -> findActivity(context.baseContext)
        else -> null
    }

    @RequiresApi(Build.VERSION_CODES.Q)
    private fun authenticateWithBiometricPrompt(
        activity: Activity,
        onSuccess: () -> Unit,
        onError: (String) -> Unit,
    ) {
        try {
            val builder = BiometricPrompt.Builder(activity)
                .setTitle("Unlock Zalỏ")
                .setSubtitle("Verify your identity to continue")
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                builder.setAllowedAuthenticators(
                    BiometricManager.Authenticators.BIOMETRIC_STRONG or
                        BiometricManager.Authenticators.BIOMETRIC_WEAK or
                        BiometricManager.Authenticators.DEVICE_CREDENTIAL,
                )
            } else {
                @Suppress("DEPRECATION")
                builder.setDeviceCredentialAllowed(true)
            }
            val prompt = builder.build()
            val cancellationSignal = CancellationSignal()
            prompt.authenticate(
                cancellationSignal,
                activity.mainExecutor,
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        onSuccess()
                    }

                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        val message = errString.toString().ifBlank { "Authentication canceled" }
                        onError(message)
                    }
                },
            )
        } catch (e: Exception) {
            onError(e.message?.ifBlank { null } ?: "Authentication unavailable")
        }
    }

    @RequiresApi(Build.VERSION_CODES.P)
    private fun authenticateApi28(
        activity: Activity,
        onSuccess: () -> Unit,
        onError: (String) -> Unit,
    ) {
        try {
            val prompt = BiometricPrompt.Builder(activity)
                .setTitle("Unlock Zalỏ")
                .setSubtitle("Verify your identity to continue")
                .setNegativeButton("Use PIN", activity.mainExecutor) { _, _ ->
                    authenticateWithKeyguard(activity, onSuccess, onError)
                }
                .build()
            val cancellationSignal = CancellationSignal()
            prompt.authenticate(
                cancellationSignal,
                activity.mainExecutor,
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        onSuccess()
                    }

                    override fun onAuthenticationError(errorCode: Int, errString: CharSequence) {
                        val fallbackToPin = errorCode == BiometricPrompt.BIOMETRIC_ERROR_NO_BIOMETRICS ||
                            errorCode == BiometricPrompt.BIOMETRIC_ERROR_HW_NOT_PRESENT ||
                            errorCode == BiometricPrompt.BIOMETRIC_ERROR_HW_UNAVAILABLE
                        if (fallbackToPin) {
                            authenticateWithKeyguard(activity, onSuccess, onError)
                        } else {
                            onError(errString.toString().ifBlank { "Authentication canceled" })
                        }
                    }
                },
            )
        } catch (_: Exception) {
            authenticateWithKeyguard(activity, onSuccess, onError)
        }
    }

    private fun authenticateWithKeyguard(
        activity: Activity,
        onSuccess: () -> Unit,
        onError: (String) -> Unit,
    ) {
        val km = activity.getSystemService(KeyguardManager::class.java)
        @Suppress("DEPRECATION")
        val confirmIntent = km?.createConfirmDeviceCredentialIntent(
            "Unlock Zalỏ",
            "Verify your identity to continue",
        )
        if (confirmIntent == null) {
            onError("Device screen lock is not available")
            return
        }
        val componentActivity = activity as? ComponentActivity
        if (componentActivity == null) {
            onError("Device credential confirmation is not supported on this screen")
            return
        }
        val key = "zaloo_app_lock_${launcherCounter.incrementAndGet()}"
        var launcher: ActivityResultLauncher<android.content.Intent>? = null
        launcher = componentActivity.activityResultRegistry.register(
            key,
            ActivityResultContracts.StartActivityForResult(),
        ) { result ->
            launcher?.unregister()
            if (result.resultCode == Activity.RESULT_OK) {
                onSuccess()
            } else {
                onError("Authentication canceled")
            }
        }
        try {
            launcher.launch(confirmIntent)
        } catch (e: Exception) {
            launcher.unregister()
            onError(e.message?.ifBlank { null } ?: "Authentication unavailable")
        }
    }
}

/** Material 3 Expressive full-screen lock overlay shown whenever Zalỏ is locked. */
@Composable
fun AppLockScreen(
    onUnlock: () -> Unit,
    error: String?,
) {
    Surface(
        modifier = Modifier.fillMaxSize(),
        color = MaterialTheme.colorScheme.surface,
    ) {
        Box(
            modifier = Modifier
                .fillMaxSize()
                .padding(horizontal = 32.dp),
            contentAlignment = Alignment.Center,
        ) {
            Column(
                horizontalAlignment = Alignment.CenterHorizontally,
                verticalArrangement = Arrangement.Center,
            ) {
                Surface(
                    shape = RoundedCornerShape(28.dp),
                    color = MaterialTheme.colorScheme.primaryContainer,
                    modifier = Modifier.size(84.dp),
                ) {
                    Box(contentAlignment = Alignment.Center) {
                        Icon(
                            imageVector = Icons.Outlined.Lock,
                            contentDescription = null,
                            tint = MaterialTheme.colorScheme.onPrimaryContainer,
                            modifier = Modifier.size(40.dp),
                        )
                    }
                }
                Spacer(Modifier.height(24.dp))
                Text(
                    text = "Zalỏ is locked",
                    style = MaterialTheme.typography.headlineSmall,
                    fontWeight = FontWeight.SemiBold,
                    color = MaterialTheme.colorScheme.onSurface,
                    textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(8.dp))
                Text(
                    text = "Verify your identity to access your chats",
                    style = MaterialTheme.typography.bodyMedium,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    textAlign = TextAlign.Center,
                )
                Spacer(Modifier.height(28.dp))
                Button(
                    onClick = onUnlock,
                    shape = CircleShape,
                    contentPadding = PaddingValues(horizontal = 28.dp, vertical = 14.dp),
                ) {
                    Icon(
                        imageVector = Icons.Outlined.LockOpen,
                        contentDescription = null,
                        modifier = Modifier.size(20.dp),
                    )
                    Spacer(Modifier.width(8.dp))
                    Text("Unlock")
                }
                if (!error.isNullOrBlank()) {
                    Spacer(Modifier.height(16.dp))
                    Text(
                        text = error,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MaterialTheme.colorScheme.error,
                        textAlign = TextAlign.Center,
                    )
                }
            }
        }
    }
}
