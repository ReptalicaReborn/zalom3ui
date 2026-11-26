package com.zalom3ui.app.data

import android.content.Context
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import java.io.File
import java.io.FileOutputStream
import java.io.RandomAccessFile
import java.security.SecureRandom
import java.util.Properties

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
    REPTDOC_GOLD("gold", "ReptDoc gold"),
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

/** Server settings, kept in app-private storage shared by the UI and the on-phone server process. */
class SettingsStore(context: Context) {
    private val prefs = context.getSharedPreferences("settings", Context.MODE_PRIVATE)
    private val appFilesDir: File = (context.applicationContext ?: context).filesDir

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

    /**
     * Secret shared between the UI and the on-phone server, so other apps on the phone can't use
     * the server's localhost port. Created on first use.
     */
    fun localToken(): String = readOrInitLocalConfig().token

    /**
     * Loopback port of this install's on-phone server, picked at random once. A fixed port would clash with another app
     * that runs the same server (Zalo Lite uses 38123): the UI would then talk to that app's server, which refuses our
     * token, and ours could not start.
     */
    fun localPort(): Int = readOrInitLocalConfig().port

    private data class LocalConfig(val token: String, val port: Int)

    /**
     * Stores [KEY_LOCAL_TOKEN] and [KEY_LOCAL_PORT] in a private properties file coordinated by both an
     * intra-process lock and an inter-process [java.nio.channels.FileChannel.lock], avoiding cross-process
     * SharedPreferences races between the UI and `:node` processes.
     */
    private fun readOrInitLocalConfig(): LocalConfig = synchronized(localConfigLock) {
        appFilesDir.mkdirs()
        val lockFile = File(appFilesDir, LOCAL_LOCK_FILE)
        val propsFile = File(appFilesDir, LOCAL_PROPS_FILE)
        RandomAccessFile(lockFile, "rw").use { raf ->
            raf.channel.lock().use {
                val props = Properties()
                if (propsFile.isFile) {
                    runCatching { propsFile.inputStream().use { props.load(it) } }
                }
                var token = props.getProperty(KEY_LOCAL_TOKEN)?.takeIf { it.isNotBlank() }
                var port = props.getProperty(KEY_LOCAL_PORT)?.toIntOrNull()?.takeIf { it in LOCAL_PORT_RANGE }
                var dirty = false
                if (token == null) {
                    token = prefs.getString(KEY_LOCAL_TOKEN, null)?.takeIf { it.isNotBlank() }
                        ?: ByteArray(24).also { SecureRandom().nextBytes(it) }.joinToString("") { "%02x".format(it) }
                    props.setProperty(KEY_LOCAL_TOKEN, token)
                    dirty = true
                }
                if (port == null) {
                    port = prefs.getInt(KEY_LOCAL_PORT, 0).takeIf { it in LOCAL_PORT_RANGE }
                        ?: (LOCAL_PORT_RANGE.first + SecureRandom().nextInt(LOCAL_PORT_RANGE.last - LOCAL_PORT_RANGE.first + 1))
                    props.setProperty(KEY_LOCAL_PORT, port.toString())
                    dirty = true
                }
                if (dirty || !propsFile.isFile) {
                    val tmp = File(appFilesDir, "$LOCAL_PROPS_FILE.tmp")
                    FileOutputStream(tmp).use { out ->
                        props.store(out, null)
                        out.flush()
                        runCatching { out.fd.sync() }
                    }
                    if (!tmp.renameTo(propsFile)) {
                        tmp.copyTo(propsFile, overwrite = true)
                        tmp.delete()
                    }
                }
                LocalConfig(token, port)
            }
        }
    }

    internal companion object {
        val localConfigLock = Any()
        val LOCAL_PORT_RANGE = 40000..59999
        const val LOCAL_PROPS_FILE = "local-server.properties"
        const val LOCAL_LOCK_FILE = "local-server.lock"
        const val KEY_LOCAL_PORT = "local_port"
        const val KEY_ON_DEVICE = "on_device"
        const val KEY_URL = "server_url"
        const val KEY_TOKEN = "token"
        const val KEY_LOCAL_TOKEN = "local_token"
        const val KEY_THEME = "theme"
        const val KEY_DYNAMIC_COLOR = "dynamic_color"
        const val KEY_COLOR_THEME = "color_theme"
        const val KEY_CHAT_ACCENT = "chat_accent"
        const val KEY_CHAT_TINTED_BG = "chat_tinted_bg"
        const val KEY_ULTRA_EXPRESSIVE = "ultra_expressive"
        const val KEY_CHAT_ACCENTS = "chat_accents"
        const val KEY_CHAT_DRAFTS = "chat_drafts"

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
