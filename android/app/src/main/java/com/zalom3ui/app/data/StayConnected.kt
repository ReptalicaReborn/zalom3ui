package com.zalom3ui.app.data

/**
 * How long the on-phone Zalo core stays connected to Zalo (Settings → "Stay connected").
 *
 * Zalo allows one web listener per account, so while Zalỏ is connected, Zalo Web / PC elsewhere is kicked, and the other
 * way round. The modes trade always-on notifications against leaving that one slot free.
 */
enum class StayConnected(
    val key: String,
    val label: String,
    val summary: String,
) {
    /** Today's behavior: a foreground service keeps Zalỏ connected, so messages and calls arrive while it is closed. */
    ALWAYS(
        "always",
        "Always",
        "Stay connected in the background. Notifications and incoming calls work while Zalỏ is closed. " +
            "Zalo Web or PC elsewhere is signed out while you are connected.",
    ),

    /** No foreground service: connected only while the app is open (plus a short grace period after leaving it). */
    WHILE_OPEN(
        "while_open",
        "While open",
        "Connect only while Zalỏ is open. No notifications or incoming calls while it is closed, and your " +
            "history catches up when you open it again. Zalo Web or PC stays usable while Zalỏ is closed.",
    ),

    /** [WHILE_OPEN], plus: a message notification from the official Zalo app briefly brings Zalỏ online. */
    WHILE_OPEN_BRIDGE(
        "while_open_bridge",
        "While open + notification bridge",
        "Like \"While open\", but when the official Zalo app shows a message notification, Zalỏ connects for a minute, " +
            "fetches the message and shows its own notification (with Reply). Needs notification access. " +
            "Each bridge connection can briefly sign out Zalo Web or PC.",
    ),
    ;

    /** Runs [com.zalom3ui.app.server.ZaloService] and starts it at boot. */
    val keepsForegroundService: Boolean get() = this == ALWAYS

    /** Pauses the chat listener when the app is closed (after the grace period). */
    val pausesWhenClosed: Boolean get() = this != ALWAYS

    /** Listens to the official Zalo app's notifications to wake the core. */
    val usesBridge: Boolean get() = this == WHILE_OPEN_BRIDGE

    companion object {
        /**
         * The mode a fresh install starts in. This is the one line to change to make another mode the default once the
         * replay test (docs/replay-test.md) shows that history is safe without a foreground service.
         */
        val NEW_INSTALL_DEFAULT: StayConnected = ALWAYS

        /** The mode an install that existed before this setting migrates to: exactly the old behavior. */
        val UPGRADE_DEFAULT: StayConnected = ALWAYS

        fun fromKey(key: String?): StayConnected? = entries.firstOrNull { it.key == key }

        /** The mode to save the first time the app runs with this setting. */
        fun initialFor(isUpgrade: Boolean): StayConnected = if (isUpgrade) UPGRADE_DEFAULT else NEW_INSTALL_DEFAULT
    }
}

/** The "Stay connected" settings. */
data class StaySettings(
    val mode: StayConnected = StayConnected.ALWAYS,
    /** Bridge mode: cancel the official Zalo app's message notification once Zalỏ has shown its own for the chat. */
    val hideZaloNotifications: Boolean = true,
)
