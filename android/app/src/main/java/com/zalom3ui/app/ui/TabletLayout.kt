package com.zalom3ui.app.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.VerticalDivider
import androidx.compose.runtime.Composable
import androidx.compose.runtime.key
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalConfiguration
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp

/** Width of the chat list beside the open chat: about a third of the screen, like Telegram on tablets. */
fun listPaneWidth(screenWidthDp: Int): Dp = (screenWidthDp * 0.36f).dp.coerceIn(320.dp, 440.dp)

/**
 * The home screen: on a phone just the chat list ([list]); on a tablet ([wide]) the list on the left and the open chat
 * ([chatPane]) on the right, with a placeholder until one is picked. [list] gets the key of the chat to highlight.
 */
@Composable
fun TwoPaneHome(
    wide: Boolean,
    chat: ChatState?,
    onCloseChat: () -> Unit,
    chatPane: @Composable (ChatState) -> Unit,
    list: @Composable (selectedKey: String?) -> Unit,
) {
    if (!wide) {
        list(null)
        return
    }
    // Back closes the open chat first, then leaves the app as usual.
    BackHandler(enabled = chat != null, onBack = onCloseChat)
    Row(Modifier.fillMaxSize()) {
        Box(Modifier.width(listPaneWidth(LocalConfiguration.current.screenWidthDp)).fillMaxHeight()) {
            list(chat?.ref?.key)
        }
        VerticalDivider()
        Box(Modifier.weight(1f).fillMaxHeight()) {
            if (chat == null) {
                NoChatSelected()
            } else {
                // A new chat gets fresh screen state (reply draft, open menus), as a new screen would on a phone.
                key(chat.ref.key) { chatPane(chat) }
            }
        }
    }
}

/** The empty right pane: the chat background with a hint, until a chat is picked. */
@Composable
fun NoChatSelected() {
    Box(Modifier.fillMaxSize().background(LocalChatColors.current.background), contentAlignment = Alignment.Center) {
        Surface(shape = RoundedCornerShape(16.dp), color = MaterialTheme.colorScheme.surfaceContainerHigh) {
            Text(
                "Select a chat to start messaging",
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(horizontal = 14.dp, vertical = 6.dp),
            )
        }
    }
}
