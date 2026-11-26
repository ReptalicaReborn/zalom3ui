package com.zalom3ui.app.ui

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.Logout
import androidx.compose.material.icons.filled.PlayArrow
import androidx.compose.material.icons.filled.PushPin
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.Delete
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.Groups
import androidx.compose.material.icons.outlined.Image
import androidx.compose.material.icons.outlined.MarkChatUnread
import androidx.compose.material.icons.outlined.Notifications
import androidx.compose.material.icons.outlined.NotificationsOff
import androidx.compose.material.icons.outlined.PersonAdd
import androidx.compose.material.icons.outlined.PushPin
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.rounded.Check
import androidx.compose.material3.ButtonGroupDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ExperimentalMaterial3ExpressiveApi
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LoadingIndicator
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.MediumFlexibleTopAppBar
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.ToggleButtonShapes
import androidx.compose.material3.TonalToggleButton
import androidx.compose.material3.TopAppBarDefaults
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.input.nestedscroll.nestedScroll
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import coil.compose.SubcomposeAsyncImage
import com.zalom3ui.app.data.ChatAccent
import com.zalom3ui.app.data.GroupMember
import com.zalom3ui.app.data.MuteOption
import com.zalom3ui.app.data.ThreadSummary

private data class SharedMediaItem(
    val id: String,
    val thumbUrl: String,
    val fullUrl: String,
    val isVideo: Boolean,
)

/** Conversation details screen: profile header, quick actions, shared media strip, and grouped settings cards. */
@OptIn(ExperimentalMaterial3Api::class, ExperimentalMaterial3ExpressiveApi::class)
@Composable
fun ChatInfoScreen(
    chat: ChatState,
    thread: ThreadSummary?,
    onBack: () -> Unit,
    onSearch: () -> Unit,
    onPin: (Boolean) -> Unit,
    onMute: (MuteOption?) -> Unit,
    onMarkUnread: () -> Unit,
    onLoadMembers: () -> Unit,
    onShowMembers: () -> Unit,
    onAddMembers: () -> Unit,
    onOpenMember: (GroupMember) -> Unit,
    onRenameGroup: (String) -> Unit,
    onLeaveGroup: (silent: Boolean) -> Unit,
    onDeleteConversation: () -> Unit,
    chatAccent: ChatAccent? = null,
    onSetChatAccent: (ChatAccent?) -> Unit = {},
) {
    val context = LocalContext.current
    val isGroup = chat.kind == "group"
    val pinned = thread?.pinned == true
    val muted = thread?.isMuted == true
    var dialog by rememberSaveable { mutableStateOf<String?>(null) }
    var viewingPhotoIndex by rememberSaveable { mutableStateOf<Int?>(null) }

    LaunchedEffect(chat.ref, isGroup) {
        if (isGroup && chat.members == null) onLoadMembers()
    }

    val sharedMedia = remember(chat.messages) {
        chat.messages.asReversed().mapNotNull { msg ->
            if (msg.recalled) return@mapNotNull null
            val a = msg.attachment ?: return@mapNotNull null
            if (a.kind !in setOf("photo", "gif", "video")) return@mapNotNull null
            val thumb = a.image ?: a.remoteThumb ?: return@mapNotNull null
            val full = a.remoteHref ?: thumb
            SharedMediaItem(id = msg.id, thumbUrl = thumb, fullUrl = full, isVideo = a.kind == "video")
        }
    }
    val sharedPhotos = remember(sharedMedia) {
        sharedMedia.filterNot { it.isVideo }
    }
    val sharedPhotoUrls = remember(sharedPhotos) {
        sharedPhotos.map { it.fullUrl }
    }

    val scrollBehavior = TopAppBarDefaults.exitUntilCollapsedScrollBehavior()

    Scaffold(
        modifier = Modifier.nestedScroll(scrollBehavior.nestedScrollConnection),
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            MediumFlexibleTopAppBar(
                colors = TopAppBarDefaults.topAppBarColors(containerColor = MaterialTheme.colorScheme.background),
                title = { Text(if (isGroup) "Group info" else "Chat info", maxLines = 1, overflow = TextOverflow.Ellipsis) },
                navigationIcon = {
                    IconButton(onClick = onBack) {
                        Icon(Icons.AutoMirrored.Filled.ArrowBack, contentDescription = "Back")
                    }
                },
                scrollBehavior = scrollBehavior,
            )
        },
    ) { padding ->
        LazyColumn(
            modifier = Modifier
                .fillMaxSize()
                .padding(padding)
                .navigationBarsPadding(),
            contentPadding = PaddingValues(bottom = 28.dp),
            verticalArrangement = Arrangement.spacedBy(16.dp),
        ) {
            item(key = "header") {
                Column(
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 24.dp, vertical = 4.dp),
                    horizontalAlignment = Alignment.CenterHorizontally,
                ) {
                    HeroAvatar(
                        name = chat.name,
                        url = thread?.avatar.orEmpty(),
                        key = chat.ref.key,
                        isGroup = isGroup,
                    )
                    Spacer(Modifier.height(14.dp))
                    Text(
                        chat.name,
                        style = MaterialTheme.typography.headlineSmall,
                        fontWeight = FontWeight.Bold,
                        textAlign = TextAlign.Center,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                    val subtitle = when {
                        isGroup && chat.members != null -> "${chat.members.size + 1} members"
                        isGroup -> "Group conversation"
                        chat.lastOnline != null -> formatPresence(chat.lastOnline)
                        else -> null
                    }
                    subtitle?.let {
                        Spacer(Modifier.height(4.dp))
                        Text(
                            it,
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = TextAlign.Center,
                        )
                    }
                    if (muted && thread?.mutedUntil != null) {
                        Spacer(Modifier.height(4.dp))
                        Text(
                            muteLabel(thread.mutedUntil),
                            style = MaterialTheme.typography.labelMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            textAlign = TextAlign.Center,
                        )
                    }
                }
            }

            item(key = "quick_actions") {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp),
                    horizontalArrangement = Arrangement.spacedBy(ButtonGroupDefaults.ConnectedSpaceBetween),
                ) {
                    QuickActionButton(
                        label = "Search",
                        icon = Icons.Outlined.Search,
                        checked = false,
                        shapes = ButtonGroupDefaults.connectedLeadingButtonShapes(),
                        onClick = onSearch,
                        modifier = Modifier.weight(1f),
                    )
                    QuickActionButton(
                        label = if (muted) "Unmute" else "Mute",
                        icon = if (muted) Icons.Outlined.NotificationsOff else Icons.Outlined.Notifications,
                        checked = muted,
                        shapes = ButtonGroupDefaults.connectedMiddleButtonShapes(),
                        onClick = { if (muted) onMute(null) else dialog = "mute" },
                        modifier = Modifier.weight(1f),
                    )
                    QuickActionButton(
                        label = if (pinned) "Unpin" else "Pin",
                        icon = if (pinned) Icons.Filled.PushPin else Icons.Outlined.PushPin,
                        checked = pinned,
                        shapes = ButtonGroupDefaults.connectedMiddleButtonShapes(),
                        onClick = { onPin(!pinned) },
                        modifier = Modifier.weight(1f),
                    )
                    if (isGroup) {
                        QuickActionButton(
                            label = "Add",
                            icon = Icons.Outlined.PersonAdd,
                            checked = false,
                            shapes = ButtonGroupDefaults.connectedTrailingButtonShapes(),
                            onClick = onAddMembers,
                            modifier = Modifier.weight(1f),
                        )
                    } else {
                        QuickActionButton(
                            label = "Unread",
                            icon = Icons.Outlined.MarkChatUnread,
                            checked = false,
                            shapes = ButtonGroupDefaults.connectedTrailingButtonShapes(),
                            onClick = onMarkUnread,
                            modifier = Modifier.weight(1f),
                        )
                    }
                }
            }

            item(key = "shared_media") {
                InfoSectionCard {
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .padding(start = 18.dp, end = 18.dp, top = 16.dp, bottom = 10.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            "Shared media",
                            style = MaterialTheme.typography.titleSmall,
                            fontWeight = FontWeight.SemiBold,
                            modifier = Modifier.weight(1f),
                        )
                        if (sharedMedia.isNotEmpty()) {
                            Text(
                                "${sharedMedia.size}",
                                style = MaterialTheme.typography.labelLarge,
                                color = MaterialTheme.colorScheme.onSurfaceVariant,
                            )
                        }
                    }
                    if (sharedMedia.isEmpty()) {
                        Text(
                            "No shared photos or videos yet",
                            style = MaterialTheme.typography.bodyMedium,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(start = 18.dp, end = 18.dp, bottom = 16.dp),
                        )
                    } else {
                        LazyRow(
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            contentPadding = PaddingValues(start = 16.dp, end = 16.dp, bottom = 16.dp),
                        ) {
                            items(sharedMedia, key = { it.id }) { item ->
                                Box(
                                    Modifier
                                        .size(88.dp)
                                        .clip(RoundedCornerShape(14.dp))
                                        .background(MaterialTheme.colorScheme.surfaceContainerHighest)
                                        .clickable {
                                            if (item.isVideo) {
                                                openExternally(context, item.fullUrl, "video/*")
                                            } else {
                                                val idx = sharedPhotos.indexOfFirst { it.id == item.id }
                                                if (idx >= 0) viewingPhotoIndex = idx
                                            }
                                        },
                                ) {
                                    SubcomposeAsyncImage(
                                        model = item.thumbUrl,
                                        contentDescription = if (item.isVideo) "Video" else "Photo",
                                        contentScale = ContentScale.Crop,
                                        modifier = Modifier.fillMaxSize(),
                                        loading = {
                                            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                                                Icon(Icons.Outlined.Image, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                                            }
                                        },
                                        error = {
                                            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                                                Icon(Icons.Outlined.Image, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                                            }
                                        },
                                    )
                                    if (item.isVideo) {
                                        Surface(
                                            shape = CircleShape,
                                            color = Color.Black.copy(alpha = 0.55f),
                                            contentColor = Color.White,
                                            modifier = Modifier.align(Alignment.Center),
                                        ) {
                                            Icon(Icons.Filled.PlayArrow, contentDescription = null, modifier = Modifier.padding(4.dp).size(20.dp))
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            if (isGroup) {
                item(key = "members_card") {
                    val members = chat.members
                    InfoSectionCard {
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .padding(start = 18.dp, end = 18.dp, top = 16.dp, bottom = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                if (members != null) "${members.size + 1} members" else "Members",
                                style = MaterialTheme.typography.titleSmall,
                                fontWeight = FontWeight.SemiBold,
                                modifier = Modifier.weight(1f),
                            )
                        }
                        if (members == null) {
                            Box(Modifier.fillMaxWidth().padding(vertical = 20.dp), contentAlignment = Alignment.Center) {
                                LoadingIndicator()
                            }
                        } else {
                            val preview = members.take(5)
                            preview.forEach { member ->
                                Row(
                                    Modifier
                                        .fillMaxWidth()
                                        .clickable { onOpenMember(member) }
                                        .padding(horizontal = 16.dp, vertical = 10.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                ) {
                                    Avatar(member.name, "", key = member.id, isGroup = false, size = 36.dp)
                                    Spacer(Modifier.width(12.dp))
                                    Text(
                                        member.name,
                                        style = MaterialTheme.typography.bodyLarge,
                                        maxLines = 1,
                                        overflow = TextOverflow.Ellipsis,
                                        modifier = Modifier.weight(1f),
                                    )
                                }
                            }
                            HorizontalDivider(
                                modifier = Modifier.padding(horizontal = 16.dp),
                                color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f),
                            )
                            Row(
                                Modifier
                                    .fillMaxWidth()
                                    .clickable(onClick = onShowMembers)
                                    .padding(horizontal = 18.dp, vertical = 14.dp),
                                verticalAlignment = Alignment.CenterVertically,
                            ) {
                                Text(
                                    "See all ${members.size + 1} members",
                                    style = MaterialTheme.typography.bodyLarge,
                                    color = MaterialTheme.colorScheme.primary,
                                    fontWeight = FontWeight.Medium,
                                    modifier = Modifier.weight(1f),
                                )
                                Icon(
                                    Icons.Outlined.ChevronRight,
                                    contentDescription = null,
                                    tint = MaterialTheme.colorScheme.primary,
                                )
                            }
                        }
                    }
                }
            }

            item(key = "chat_color") {
                ChatAccentCard(
                    selected = chatAccent,
                    onSelect = onSetChatAccent,
                )
            }

            if (isGroup) {
                item(key = "group_settings") {
                    InfoSectionCard {
                        InfoActionRow(
                            label = "Rename group",
                            icon = Icons.Outlined.Edit,
                            onClick = { dialog = "rename" },
                        )
                        HorizontalDivider(
                            modifier = Modifier.padding(horizontal = 16.dp),
                            color = MaterialTheme.colorScheme.outlineVariant.copy(alpha = 0.5f),
                        )
                        InfoActionRow(
                            label = "Leave group",
                            icon = Icons.AutoMirrored.Outlined.Logout,
                            destructive = true,
                            onClick = { dialog = "leave" },
                        )
                    }
                }
            }

            item(key = "danger_zone") {
                InfoSectionCard {
                    InfoActionRow(
                        label = "Delete conversation",
                        icon = Icons.Outlined.Delete,
                        destructive = true,
                        onClick = { dialog = "delete" },
                    )
                }
            }
        }
    }

    when (dialog) {
        "mute" -> MuteDialog(
            onDismiss = { dialog = null },
            onPick = {
                dialog = null
                onMute(it)
            },
        )
        "delete" -> DeleteConversationDialog(
            onDismiss = { dialog = null },
            onConfirm = {
                dialog = null
                onDeleteConversation()
            },
        )
        "rename" -> RenameGroupDialog(
            current = chat.name,
            onDismiss = { dialog = null },
            onRename = {
                dialog = null
                onRenameGroup(it)
            },
        )
        "leave" -> LeaveGroupDialog(
            name = chat.name,
            onDismiss = { dialog = null },
            onLeave = {
                dialog = null
                onLeaveGroup(it)
            },
        )
    }

    viewingPhotoIndex?.let { index ->
        if (sharedPhotoUrls.isNotEmpty()) {
            PhotoViewer(
                photos = sharedPhotoUrls,
                initialIndex = index.coerceIn(0, sharedPhotoUrls.lastIndex),
                onDismiss = { viewingPhotoIndex = null },
            )
        } else {
            viewingPhotoIndex = null
        }
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun ChatAccentCard(
    selected: ChatAccent?,
    onSelect: (ChatAccent?) -> Unit,
) {
    InfoSectionCard {
        Column(
            modifier = Modifier
                .fillMaxWidth()
                .padding(start = 18.dp, end = 18.dp, top = 16.dp, bottom = 12.dp),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                "Chat color",
                style = MaterialTheme.typography.titleSmall,
                fontWeight = FontWeight.SemiBold,
            )
            Text(
                selected?.label ?: "Using app default",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
        }
        Row(
            modifier = Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(start = 16.dp, end = 16.dp, bottom = 16.dp),
            horizontalArrangement = Arrangement.spacedBy(12.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            val isDefault = selected == null
            val defaultCorner by animateDpAsState(
                targetValue = if (isDefault) 14.dp else 22.dp,
                animationSpec = MaterialTheme.motionScheme.fastSpatialSpec(),
                label = "chatDefaultAccentCorner",
            )
            val defaultShape = RoundedCornerShape(defaultCorner)
            Surface(
                shape = defaultShape,
                color = if (isDefault) {
                    MaterialTheme.colorScheme.primaryContainer
                } else {
                    MaterialTheme.colorScheme.surfaceContainerHigh
                },
                contentColor = if (isDefault) {
                    MaterialTheme.colorScheme.onPrimaryContainer
                } else {
                    MaterialTheme.colorScheme.onSurfaceVariant
                },
                border = if (isDefault) {
                    BorderStroke(1.5.dp, MaterialTheme.colorScheme.primary)
                } else {
                    BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant)
                },
                modifier = Modifier.height(44.dp),
            ) {
                Row(
                    modifier = Modifier
                        .fillMaxHeight()
                        .semantics {
                            contentDescription = "Default"
                            this.selected = isDefault
                        }
                        .clickable(
                            role = Role.RadioButton,
                            onClick = { onSelect(null) },
                        )
                        .padding(horizontal = 14.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    if (isDefault) {
                        Icon(
                            Icons.Rounded.Check,
                            contentDescription = null,
                            modifier = Modifier.size(18.dp),
                        )
                    }
                    Text(
                        "Default",
                        style = MaterialTheme.typography.labelLarge,
                        fontWeight = FontWeight.Medium,
                    )
                }
            }

            ChatAccent.entries.forEach { accent ->
                val isSelected = selected == accent
                val cornerRadius by animateDpAsState(
                    targetValue = if (isSelected) 14.dp else 22.dp,
                    animationSpec = MaterialTheme.motionScheme.fastSpatialSpec(),
                    label = "chatAccentCorner",
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
                            role = Role.RadioButton,
                            onClick = { onSelect(if (isSelected) null else accent) },
                        ),
                    contentAlignment = Alignment.Center,
                ) {
                    val checkScale by animateFloatAsState(
                        targetValue = if (isSelected) 1f else 0f,
                        animationSpec = MaterialTheme.motionScheme.fastSpatialSpec(),
                        label = "chatAccentCheck",
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

/** 92.dp hero avatar: circular for 1:1 conversations, expressive squircle for groups. */
@Composable
private fun HeroAvatar(name: String, url: String, key: String, isGroup: Boolean) {
    val shape = if (isGroup) RoundedCornerShape(28.dp) else CircleShape
    val fallback = @Composable {
        val colors = avatarColors(key)
        Box(
            Modifier.size(92.dp).clip(shape).background(colors.container),
            contentAlignment = Alignment.Center,
        ) {
            if (isGroup && name.isBlank()) {
                Icon(Icons.Outlined.Groups, contentDescription = null, tint = colors.content, modifier = Modifier.size(44.dp))
            } else {
                Text(
                    initial(name),
                    color = colors.content,
                    fontWeight = FontWeight.SemiBold,
                    fontSize = 36.sp,
                )
            }
        }
    }
    if (url.isBlank()) {
        fallback()
    } else {
        SubcomposeAsyncImage(
            model = url,
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier = Modifier.size(92.dp).clip(shape),
            loading = { fallback() },
            error = { fallback() },
        )
    }
}

@OptIn(ExperimentalMaterial3ExpressiveApi::class)
@Composable
private fun QuickActionButton(
    label: String,
    icon: ImageVector,
    checked: Boolean,
    shapes: ToggleButtonShapes,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    TonalToggleButton(
        checked = checked,
        onCheckedChange = { onClick() },
        shapes = shapes,
        contentPadding = PaddingValues(horizontal = 4.dp, vertical = 10.dp),
        modifier = modifier,
    ) {
        Column(horizontalAlignment = Alignment.CenterHorizontally) {
            Icon(icon, contentDescription = label, modifier = Modifier.size(20.dp))
            Spacer(Modifier.height(4.dp))
            Text(label, style = MaterialTheme.typography.labelSmall, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
private fun InfoSectionCard(content: @Composable () -> Unit) {
    Surface(
        shape = RoundedCornerShape(24.dp),
        color = MaterialTheme.colorScheme.surfaceContainerLow,
        modifier = Modifier
            .fillMaxWidth()
            .padding(horizontal = 16.dp),
    ) {
        Column { content() }
    }
}

@Composable
private fun InfoActionRow(
    label: String,
    icon: ImageVector,
    onClick: () -> Unit,
    destructive: Boolean = false,
) {
    val tint = if (destructive) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurface
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onClick)
            .padding(horizontal = 18.dp, vertical = 16.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Icon(icon, contentDescription = null, tint = tint, modifier = Modifier.size(22.dp))
        Spacer(Modifier.width(14.dp))
        Text(label, style = MaterialTheme.typography.bodyLarge, color = tint, fontWeight = FontWeight.Medium)
    }
}
