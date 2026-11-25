package com.zalom3ui.app.data

import android.content.Context
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

data class Appearance(
    val themeMode: ThemeMode = ThemeMode.SYSTEM,
    /** Material You colors from the wallpaper (Android 12+) instead of the app's blue. */
    val dynamicColor: Boolean = false,
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

    fun loadAppearance(): Appearance = Appearance(
        themeMode = ThemeMode.entries.firstOrNull { it.name == prefs.getString(KEY_THEME, null) } ?: ThemeMode.SYSTEM,
        dynamicColor = prefs.getBoolean(KEY_DYNAMIC_COLOR, false),
    )

    fun saveAppearance(appearance: Appearance) {
        prefs.edit()
            .putString(KEY_THEME, appearance.themeMode.name)
            .putBoolean(KEY_DYNAMIC_COLOR, appearance.dynamicColor)
            .apply()
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

    private companion object {
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
    }
}
