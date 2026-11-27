package com.zalom3ui.app.data

import android.content.Context
import android.provider.Settings
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json

data class ServerSettings(
    /** Run the Zalo server inside this app instead of connecting to one elsewhere. */
    val onDevice: Boolean,
    /** Remote server address; unused when [onDevice]. */
    val serverUrl: String,
    /** Remote server access token; unused when [onDevice]. */
    val token: String,
) {
    val isConfigured: Boolean get() = onDevice || ApiClient.normalizeServerUrl(serverUrl) != null
}

enum class ThemeMode(val label: String) {
    SYSTEM("System default"),
    LIGHT("Light"),
    DARK("Dark"),
}

enum class ChatAccent(val key: String, val label: String) {
    ULTRAMARINE("ultramarine", "Ultramarine"),
    REPTDOC_GOLD("gold", "Gold"),
    CRIMSON("crimson", "Crimson"),
    FOREST("forest", "Forest"),
    PLUM("plum", "Plum"),
    TEAL("teal", "Teal"),
    SLATE("slate", "Slate"),
    ;

    companion object {
        fun fromKeyOrNull(key: String?): ChatAccent? = entries.firstOrNull { it.key == key }

        fun fromKey(key: String?): ChatAccent = fromKeyOrNull(key) ?: ULTRAMARINE
    }
}

data class Appearance(
    val themeMode: ThemeMode = ThemeMode.SYSTEM,
    /** Material You colors from the wallpaper (Android 12+) instead of the chosen chat accent. */
    val dynamicColor: Boolean = false,
    val accent: ChatAccent = ChatAccent.ULTRAMARINE,
    val tintedBackground: Boolean = false,
    /** Easter egg: hilariously oversized Material 3 Expressive buttons ("4x faster!"). */
    val ultraExpressive: Boolean = false,
)

enum class AppLockTimeout(val key: String, val label: String, val timeoutMs: Long) {
    IMMEDIATE("0", "Immediately", 0L),
    ONE_MIN("60", "After 1 minute", 60_000L),
    FIVE_MIN("300", "After 5 minutes", 300_000L),
    THIRTY_MIN("1800", "After 30 minutes", 1_800_000L),
    ;

    companion object {
        fun fromKey(key: String?): AppLockTimeout = entries.firstOrNull { it.key == key } ?: ONE_MIN
    }
}

/** Server settings, kept in app-private storage. */
class SettingsStore(context: Context) {
    private val prefs = context.getSharedPreferences("settings", Context.MODE_PRIVATE)

    fun load(): ServerSettings {
        val serverUrl = prefs.getString(KEY_URL, "").orEmpty()
        return ServerSettings(
            // New installs default to the on-phone server; existing remote setups keep working.
            onDevice = prefs.getBoolean(KEY_ON_DEVICE, serverUrl.isBlank()),
            serverUrl = serverUrl,
            token = prefs.getString(KEY_TOKEN, "").orEmpty(),
        )
    }

    fun save(settings: ServerSettings) {
        prefs.edit()
            .putBoolean(KEY_ON_DEVICE, settings.onDevice)
            .putString(KEY_URL, settings.serverUrl.trim())
            .putString(KEY_TOKEN, settings.token.trim())
            .commit()
    }

    /** The "Stay connected" settings; [StayConnected.UPGRADE_DEFAULT] until [ensureStayConnectedInitialised] has run. */
    fun loadStay(): StaySettings = StaySettings(
        mode = StayConnected.fromKey(prefs.getString(KEY_STAY_CONNECTED, null)) ?: StayConnected.UPGRADE_DEFAULT,
        hideZaloNotifications = prefs.getBoolean(KEY_HIDE_ZALO_NOTIFICATIONS, true),
        otpFromZaloNotifications = prefs.getBoolean(KEY_OTP_FROM_ZALO_NOTIFICATIONS, false),
    )

    fun saveStay(stay: StaySettings) {
        prefs.edit()
            .putString(KEY_STAY_CONNECTED, stay.mode.key)
            .putBoolean(KEY_HIDE_ZALO_NOTIFICATIONS, stay.hideZaloNotifications)
            .putBoolean(KEY_OTP_FROM_ZALO_NOTIFICATIONS, stay.otpFromZaloNotifications)
            .commit()
    }

    /**
     * Saves the starting "Stay connected" mode the first time this runs: [StayConnected.UPGRADE_DEFAULT] for an install
     * that existed before the setting ([isUpgrade]), [StayConnected.NEW_INSTALL_DEFAULT] otherwise. Later runs keep
     * whatever is saved, so changing the new-install default never changes an existing install.
     */
    fun ensureStayConnectedInitialised(isUpgrade: Boolean) {
        if (prefs.contains(KEY_STAY_CONNECTED)) return
        prefs.edit().putString(KEY_STAY_CONNECTED, StayConnected.initialFor(isUpgrade).key).apply()
    }

    fun loadAppearance(): Appearance {
        val rawTheme = prefs.getString(KEY_THEME, null)
        val legacyGold = rawTheme == "REPTDOC_GOLD" || prefs.getString(KEY_COLOR_THEME, null) == "GOLD"
        val themeMode = if (legacyGold) {
            ThemeMode.DARK
        } else {
            ThemeMode.entries.firstOrNull { it.name == rawTheme } ?: ThemeMode.SYSTEM
        }
        val accent = if (legacyGold) {
            ChatAccent.REPTDOC_GOLD
        } else {
            ChatAccent.fromKey(prefs.getString(KEY_CHAT_ACCENT, null))
        }
        return Appearance(
            themeMode = themeMode,
            dynamicColor = prefs.getBoolean(KEY_DYNAMIC_COLOR, false),
            accent = accent,
            tintedBackground = prefs.getBoolean(KEY_CHAT_TINTED_BG, false),
            ultraExpressive = prefs.getBoolean(KEY_ULTRA_EXPRESSIVE, false),
        )
    }

    fun saveAppearance(appearance: Appearance) {
        prefs.edit()
            .putString(KEY_THEME, appearance.themeMode.name)
            .remove(KEY_COLOR_THEME)
            .putBoolean(KEY_DYNAMIC_COLOR, appearance.dynamicColor)
            .putString(KEY_CHAT_ACCENT, appearance.accent.key)
            .putBoolean(KEY_CHAT_TINTED_BG, appearance.tintedBackground)
            .putBoolean(KEY_ULTRA_EXPRESSIVE, appearance.ultraExpressive)
            .apply()
    }

    fun loadChatAccents(): Map<String, ChatAccent> =
        decodeChatAccents(prefs.getString(KEY_CHAT_ACCENTS, null))

    fun saveChatAccent(threadKey: String, accent: ChatAccent?) = synchronized(this) {
        val updated = loadChatAccents().toMutableMap()
        if (accent == null) {
            updated.remove(threadKey)
        } else {
            updated[threadKey] = accent
        }
        prefs.edit().apply {
            if (updated.isEmpty()) remove(KEY_CHAT_ACCENTS) else putString(KEY_CHAT_ACCENTS, encodeChatAccents(updated))
        }.apply()
    }

    /** The drafts JSON last read or written, with its decoded map, so saving one draft doesn't decode them all again. */
    private var draftsCache: Pair<String?, Map<String, String>>? = null

    fun loadDrafts(): Map<String, String> = synchronized(this) {
        val raw = prefs.getString(KEY_CHAT_DRAFTS, null)
        draftsCache?.takeIf { it.first == raw }?.second ?: decodeDrafts(raw).also { draftsCache = raw to it }
    }

    fun saveDraft(threadKey: String, text: String) = synchronized(this) {
        val updated = loadDrafts().toMutableMap()
        if (text.isBlank()) {
            updated.remove(threadKey)
        } else {
            updated[threadKey] = text
        }
        val raw = if (updated.isEmpty()) null else encodeDrafts(updated)
        draftsCache = raw to updated
        prefs.edit().apply {
            if (raw == null) remove(KEY_CHAT_DRAFTS) else putString(KEY_CHAT_DRAFTS, raw)
        }.apply()
    }

    var appLockEnabled: Boolean
        get() = prefs.getBoolean(KEY_APP_LOCK_ENABLED, false)
        set(value) {
            prefs.edit().putBoolean(KEY_APP_LOCK_ENABLED, value).apply()
        }

    var appLockTimeout: AppLockTimeout
        get() = AppLockTimeout.fromKey(prefs.getString(KEY_APP_LOCK_TIMEOUT, null))
        set(value) {
            prefs.edit().putString(KEY_APP_LOCK_TIMEOUT, value.key).apply()
        }

    var screenSecurity: Boolean
        get() = prefs.getBoolean(KEY_SCREEN_SECURITY, false)
        set(value) {
            prefs.edit().putBoolean(KEY_SCREEN_SECURITY, value).apply()
        }

    var reducedMotion: Boolean
        get() = prefs.getBoolean(KEY_REDUCED_MOTION, false)
        set(value) {
            prefs.edit().putBoolean(KEY_REDUCED_MOTION, value).apply()
        }

    internal companion object {
        const val KEY_STAY_CONNECTED = "stay_connected"
        const val KEY_HIDE_ZALO_NOTIFICATIONS = "hide_zalo_notifications"
        const val KEY_OTP_FROM_ZALO_NOTIFICATIONS = "otp_from_zalo_notifications"
        const val KEY_ON_DEVICE = "on_device"
        const val KEY_URL = "server_url"
        const val KEY_TOKEN = "token"
        const val KEY_THEME = "theme"
        const val KEY_DYNAMIC_COLOR = "dynamic_color"
        const val KEY_COLOR_THEME = "color_theme"
        const val KEY_CHAT_ACCENT = "chat_accent"
        const val KEY_CHAT_TINTED_BG = "chat_tinted_bg"
        const val KEY_ULTRA_EXPRESSIVE = "ultra_expressive"
        const val KEY_CHAT_ACCENTS = "chat_accents"
        const val KEY_CHAT_DRAFTS = "chat_drafts"
        const val KEY_APP_LOCK_ENABLED = "app_lock_enabled"
        const val KEY_APP_LOCK_TIMEOUT = "app_lock_timeout"
        const val KEY_SCREEN_SECURITY = "screen_security"
        const val KEY_REDUCED_MOTION = "reduced_motion"

        fun encodeChatAccents(accents: Map<String, ChatAccent>): String =
            Json.encodeToString(accents.mapValues { it.value.key })

        fun decodeChatAccents(raw: String?): Map<String, ChatAccent> {
            if (raw.isNullOrBlank()) return emptyMap()
            val pairs = runCatching { Json.decodeFromString<Map<String, String>>(raw) }.getOrNull() ?: return emptyMap()
            return buildMap {
                for ((threadKey, accentKey) in pairs) {
                    val accent = ChatAccent.fromKeyOrNull(accentKey)
                    if (accent != null) put(threadKey, accent)
                }
            }
        }

        fun encodeDrafts(drafts: Map<String, String>): String =
            Json.encodeToString(drafts.filterValues { it.isNotBlank() })

        fun decodeDrafts(raw: String?): Map<String, String> {
            if (raw.isNullOrBlank()) return emptyMap()
            val map = runCatching { Json.decodeFromString<Map<String, String>>(raw) }.getOrNull() ?: return emptyMap()
            return map.filterValues { it.isNotBlank() }
        }
    }
}

/** True when Android's global animator duration scale disables animations (`0f`). */
internal fun isAnimatorScaleDisabled(scale: Float): Boolean = scale == 0f

/** True when Android's global "Remove animations" setting sets the animator duration scale to 0. */
fun systemReducedMotion(context: Context): Boolean = runCatching {
    isAnimatorScaleDisabled(
        Settings.Global.getFloat(context.contentResolver, Settings.Global.ANIMATOR_DURATION_SCALE, 1f),
    )
}.getOrDefault(false)

/** Reduced motion is active when either the in-app toggle or Android's system setting is enabled. */
fun effectiveReducedMotion(userPref: Boolean, systemPref: Boolean): Boolean = userPref || systemPref
