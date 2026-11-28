package com.zalom3ui.app.ui

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Poll
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Checkbox
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.Button
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.RadioButton
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.Stable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateListOf
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.zalom3ui.app.data.CreatePollRequest
import com.zalom3ui.app.data.PollDetail
import com.zalom3ui.app.data.PollOption
import com.zalom3ui.app.data.PollRef
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.launch
import java.text.DateFormat
import java.util.Date

/**
 * Loaded polls, shared by every poll card and the sheet. Poll messages only carry an id and a question, so the
 * options and votes are fetched once per poll and reloaded when the server says the poll changed.
 */
@Stable
class Polls(
    private val scope: CoroutineScope,
    private val doLoad: suspend (Long) -> PollDetail,
    private val doVote: suspend (Long, List<Long>) -> PollDetail,
    private val doAddOptions: suspend (Long, List<String>) -> PollDetail,
    private val doLock: suspend (Long) -> PollDetail,
    private val doCreate: suspend (CreatePollRequest) -> PollDetail,
) {
    val cache = mutableStateMapOf<Long, PollDetail>()
    private val loading = HashSet<Long>()

    /** Loads the poll for a card once; failures leave the card showing only its question. */
    fun ensure(pollId: Long) {
        if (pollId !in cache) reload(pollId)
    }

    fun reload(pollId: Long) {
        if (!loading.add(pollId)) return
        scope.launch {
            try {
                cache[pollId] = load(pollId)
            } catch (e: CancellationException) {
                throw e
            } catch (_: Exception) {
                // Keep what the card already shows.
            } finally {
                loading.remove(pollId)
            }
        }
    }

    /** The server said this poll changed: refresh it only if something on screen has loaded it. */
    fun onChanged(pollId: Long) {
        if (pollId in cache) {
            loading.remove(pollId)
            reload(pollId)
        }
    }

    suspend fun load(pollId: Long): PollDetail = doLoad(pollId).also { cache[pollId] = it }

    suspend fun vote(pollId: Long, optionIds: List<Long>): PollDetail = doVote(pollId, optionIds).also { cache[pollId] = it }

    suspend fun addOptions(pollId: Long, options: List<String>): PollDetail = doAddOptions(pollId, options).also { cache[pollId] = it }

    suspend fun lock(pollId: Long): PollDetail = doLock(pollId).also { cache[pollId] = it }

    suspend fun create(request: CreatePollRequest): PollDetail = doCreate(request).also { cache[it.pollId] = it }
}

/** Set by the app shell; null where polls aren't wired (previews, tests), and the poll UI then stays quiet. */
val LocalPolls = staticCompositionLocalOf<Polls?> { null }

private const val CARD_OPTIONS = 4

/** "Ends 12/05/26, 18:30" for a poll's deadline. */
private fun deadlineLabel(ms: Long): String = DateFormat.getDateTimeInstance(DateFormat.SHORT, DateFormat.SHORT).format(Date(ms))

private fun PollDetail.statusLabel(): String? = when {
    closed && expired -> "Ended"
    closed -> "Closed"
    expiresAt > 0 -> "Ends ${deadlineLabel(expiresAt)}"
    else -> null
}

/** The poll in a chat bubble: question, a few options with bars, vote total. Tap opens the vote sheet. */
@Composable
fun PollCard(ref: PollRef, onOutgoing: Boolean) {
    val polls = LocalPolls.current
    var open by remember { mutableStateOf(false) }
    LaunchedEffect(ref.pollId) { polls?.ensure(ref.pollId) }
    val detail = polls?.cache?.get(ref.pollId)

    if (ref.event != null) {
        // A vote or close notice: a one-line pointer to the poll rather than a second card.
        Row(
            Modifier.clip(RoundedCornerShape(8.dp)).clickable(enabled = polls != null) { open = true }.padding(vertical = 2.dp),
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Icon(Icons.Outlined.Poll, null, modifier = Modifier.size(18.dp))
            Spacer(Modifier.width(6.dp))
            Text("Poll updated: ${ref.question}", style = MaterialTheme.typography.bodyMedium, maxLines = 2, overflow = TextOverflow.Ellipsis)
        }
    } else {
        val tint = if (onOutgoing) Color.White.copy(alpha = 0.18f) else MaterialTheme.colorScheme.surface.copy(alpha = 0.6f)
        val bar = if (onOutgoing) Color.White.copy(alpha = 0.3f) else MaterialTheme.colorScheme.primary.copy(alpha = 0.25f)
        Column(
            Modifier
                .widthIn(min = 220.dp, max = 280.dp)
                .clip(RoundedCornerShape(12.dp))
                .background(tint)
                .clickable(enabled = polls != null) { open = true }
                .padding(12.dp),
        ) {
            Row(verticalAlignment = Alignment.CenterVertically) {
                Icon(Icons.Outlined.Poll, null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("Poll", style = MaterialTheme.typography.labelMedium)
                detail?.statusLabel()?.let {
                    Spacer(Modifier.width(8.dp))
                    Text("· $it", style = MaterialTheme.typography.labelMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                }
            }
            Spacer(Modifier.height(6.dp))
            Text(detail?.question?.ifBlank { null } ?: ref.question, style = MaterialTheme.typography.bodyLarge, fontWeight = FontWeight.SemiBold)
            if (detail != null) {
                Spacer(Modifier.height(8.dp))
                detail.options.take(CARD_OPTIONS).forEach { option ->
                    OptionBar(option, detail, bar, Modifier.padding(vertical = 2.dp))
                }
                if (detail.options.size > CARD_OPTIONS) {
                    Text("+${detail.options.size - CARD_OPTIONS} more", style = MaterialTheme.typography.bodySmall)
                }
                Spacer(Modifier.height(6.dp))
                Text(detail.votesLabel, style = MaterialTheme.typography.bodySmall)
            } else {
                Spacer(Modifier.height(6.dp))
                Text("Tap to view and vote", style = MaterialTheme.typography.bodySmall)
            }
        }
    }
    if (open) PollSheet(ref.pollId, onDismiss = { open = false })
}

/** One option as a row with a bar behind it showing its share, its count on the right, and a check when I chose it. */
@Composable
private fun OptionBar(option: PollOption, poll: PollDetail, barColor: Color, modifier: Modifier = Modifier) {
    val share = poll.share(option)
    Row(
        modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .drawBehind { drawRect(barColor, size = Size(size.width * share, size.height)) }
            .padding(horizontal = 8.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(
            option.text,
            style = MaterialTheme.typography.bodyMedium,
            fontWeight = if (option.voted) FontWeight.Bold else FontWeight.Normal,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.weight(1f),
        )
        if (option.voted) Text(" ✓", style = MaterialTheme.typography.bodyMedium, fontWeight = FontWeight.Bold)
        if (!poll.resultsHidden) {
            Spacer(Modifier.width(8.dp))
            Text(option.votes.toString(), style = MaterialTheme.typography.labelMedium)
        }
    }
}

/** Vote, change or remove my vote, add an option, see who voted, close the poll. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PollSheet(pollId: Long, onDismiss: () -> Unit) {
    val polls = LocalPolls.current ?: return
    val scope = rememberCoroutineScope()
    val detail = polls.cache[pollId]
    var error by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var confirmClose by remember { mutableStateOf(false) }
    var newOption by remember { mutableStateOf("") }
    // Picked options start as my current votes and reset when the server reports different ones.
    var selected by remember(detail?.myVotes) { mutableStateOf(detail?.myVotes?.toSet() ?: emptySet()) }

    LaunchedEffect(pollId) {
        try {
            polls.load(pollId)
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            error = e.message ?: "Couldn't load the poll"
        }
    }

    fun run(block: suspend () -> Unit) {
        if (busy) return
        busy = true
        error = null
        scope.launch {
            try {
                block()
            } catch (e: CancellationException) {
                throw e
            } catch (e: Exception) {
                error = e.message ?: "Something went wrong"
            } finally {
                busy = false
            }
        }
    }

    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)) {
        Column(Modifier.verticalScroll(rememberScrollState()).padding(horizontal = 20.dp).padding(bottom = 24.dp)) {
            if (detail == null) {
                Box(Modifier.fillMaxWidth().heightIn(min = 120.dp), contentAlignment = Alignment.Center) {
                    if (error == null) CircularProgressIndicator()
                }
            } else {
                Text(detail.question, style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold)
                Spacer(Modifier.height(4.dp))
                Text(
                    listOfNotNull(
                        detail.creatorName.takeIf { it.isNotBlank() }?.let { "By $it" },
                        if (detail.allowMultiple) "Multiple choice" else "Single choice",
                        if (detail.anonymous) "Anonymous" else null,
                        detail.statusLabel(),
                        detail.votesLabel,
                    ).joinToString(" · "),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                Spacer(Modifier.height(12.dp))
                detail.options.forEach { option ->
                    val checked = option.id in selected
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .clip(RoundedCornerShape(12.dp))
                            .clickable(enabled = detail.canVote && !busy) {
                                selected = when {
                                    detail.allowMultiple && checked -> selected - option.id
                                    detail.allowMultiple -> selected + option.id
                                    else -> setOf(option.id)
                                }
                            },
                        verticalAlignment = Alignment.Top,
                    ) {
                        if (detail.allowMultiple) {
                            Checkbox(checked = checked, onCheckedChange = null, enabled = detail.canVote, modifier = Modifier.padding(12.dp))
                        } else {
                            RadioButton(selected = checked, onClick = null, enabled = detail.canVote, modifier = Modifier.padding(12.dp))
                        }
                        Column(Modifier.weight(1f).padding(vertical = 10.dp)) {
                            OptionBar(option, detail, MaterialTheme.colorScheme.primary.copy(alpha = 0.18f))
                            if (option.voters.isNotEmpty()) {
                                Text(
                                    option.voters.joinToString(", ") { it.name.ifBlank { it.id } },
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    modifier = Modifier.padding(start = 8.dp, top = 2.dp),
                                )
                            }
                        }
                    }
                }
                if (detail.canAddOptions) {
                    Spacer(Modifier.height(4.dp))
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        OutlinedTextField(
                            value = newOption,
                            onValueChange = { newOption = it.take(MAX_OPTION_LENGTH) },
                            singleLine = true,
                            label = { Text("Add an option") },
                            modifier = Modifier.weight(1f),
                        )
                        Spacer(Modifier.width(8.dp))
                        TextButton(
                            enabled = newOption.isNotBlank() && !busy,
                            onClick = {
                                val text = newOption.trim()
                                run {
                                    polls.addOptions(pollId, listOf(text))
                                    newOption = ""
                                }
                            },
                        ) { Text("Add") }
                    }
                }
                error?.let {
                    Spacer(Modifier.height(8.dp))
                    Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium)
                }
                Spacer(Modifier.height(12.dp))
                if (detail.canVote) {
                    Button(
                        enabled = !busy && selected.isNotEmpty() && selected != detail.myVotes.toSet(),
                        onClick = { run { polls.vote(pollId, selected.toList()) } },
                        modifier = Modifier.fillMaxWidth(),
                    ) { Text(if (detail.myVotes.isEmpty()) "Vote" else "Change vote") }
                    if (detail.myVotes.isNotEmpty()) {
                        TextButton(enabled = !busy, onClick = { run { polls.vote(pollId, emptyList()) } }, modifier = Modifier.fillMaxWidth()) {
                            Text("Remove my vote")
                        }
                    }
                }
                if (detail.canClose) {
                    TextButton(enabled = !busy, onClick = { confirmClose = true }, modifier = Modifier.fillMaxWidth()) {
                        Text("Close poll", color = MaterialTheme.colorScheme.error)
                    }
                }
            }
            if (detail == null) {
                error?.let { Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium) }
            }
        }
    }

    if (confirmClose) {
        AlertDialog(
            onDismissRequest = { confirmClose = false },
            title = { Text("Close this poll?") },
            text = { Text("Nobody can vote after it is closed. Results stay visible.") },
            confirmButton = {
                TextButton(onClick = {
                    confirmClose = false
                    run { polls.lock(pollId) }
                }) { Text("Close poll", color = MaterialTheme.colorScheme.error) }
            },
            dismissButton = { TextButton(onClick = { confirmClose = false }) { Text("Cancel") } },
        )
    }
}

private const val MAX_OPTION_LENGTH = 200
private const val MIN_OPTIONS = 2
private const val MAX_OPTIONS = 20

/** Deadline choices for a new poll: label and how long from now (0 = none). */
internal val POLL_DEADLINES = listOf(
    "None" to 0L,
    "1 hour" to 3_600_000L,
    "1 day" to 86_400_000L,
    "1 week" to 7 * 86_400_000L,
)

/** The poll a create dialog would send, with its deadline counted from [now]. */
internal fun buildPollRequest(
    question: String,
    options: List<String>,
    allowMultiple: Boolean,
    allowAddOptions: Boolean,
    deadlineMs: Long,
    now: Long,
) = CreatePollRequest(
    question = question.trim(),
    options = options.map { it.trim() }.filter { it.isNotEmpty() }.distinct(),
    allowMultiple = allowMultiple,
    allowAddOptions = allowAddOptions,
    expiresAt = if (deadlineMs > 0) now + deadlineMs else 0L,
)

/** Question, two or more options, multiple-choice and "members can add options" switches, optional deadline. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun CreatePollDialog(onDismiss: () -> Unit) {
    val polls = LocalPolls.current ?: return
    val scope = rememberCoroutineScope()
    var question by remember { mutableStateOf("") }
    val options = remember { mutableStateListOf("", "") }
    var multiple by remember { mutableStateOf(false) }
    var allowAdd by remember { mutableStateOf(false) }
    var deadline by remember { mutableStateOf(0L) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    val request = buildPollRequest(question, options, multiple, allowAdd, deadline, System.currentTimeMillis())

    AlertDialog(
        onDismissRequest = { if (!busy) onDismiss() },
        title = { Text("Create poll") },
        text = {
            Column(Modifier.verticalScroll(rememberScrollState())) {
                OutlinedTextField(
                    value = question,
                    onValueChange = { question = it.take(MAX_OPTION_LENGTH) },
                    label = { Text("Question") },
                    modifier = Modifier.fillMaxWidth(),
                )
                Spacer(Modifier.height(8.dp))
                options.forEachIndexed { index, text ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        OutlinedTextField(
                            value = text,
                            onValueChange = { options[index] = it.take(MAX_OPTION_LENGTH) },
                            singleLine = true,
                            label = { Text("Option ${index + 1}") },
                            modifier = Modifier.weight(1f),
                        )
                        if (options.size > MIN_OPTIONS) {
                            IconButton(onClick = { options.removeAt(index) }) { Icon(Icons.Outlined.Close, contentDescription = "Remove option") }
                        }
                    }
                    Spacer(Modifier.height(6.dp))
                }
                if (options.size < MAX_OPTIONS) {
                    TextButton(onClick = { options.add("") }) {
                        Icon(Icons.Outlined.Add, null, modifier = Modifier.size(18.dp))
                        Spacer(Modifier.width(4.dp))
                        Text("Add option")
                    }
                }
                SwitchRow("Multiple choice", multiple) { multiple = it }
                SwitchRow("Members can add options", allowAdd) { allowAdd = it }
                Text("Deadline", style = MaterialTheme.typography.labelLarge, modifier = Modifier.padding(top = 8.dp, bottom = 4.dp))
                // Chips wrap onto a new line as whole units; a label never gets squeezed into one letter per line.
                FlowRow(
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    POLL_DEADLINES.forEach { (label, ms) ->
                        FilterChip(
                            selected = deadline == ms,
                            onClick = { deadline = ms },
                            label = { Text(label, maxLines = 1, softWrap = false) },
                        )
                    }
                }
                error?.let {
                    Text(it, color = MaterialTheme.colorScheme.error, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(top = 8.dp))
                }
            }
        },
        confirmButton = {
            TextButton(
                enabled = request.isValid && !busy,
                onClick = {
                    busy = true
                    error = null
                    scope.launch {
                        try {
                            polls.create(buildPollRequest(question, options, multiple, allowAdd, deadline, System.currentTimeMillis()))
                            onDismiss()
                        } catch (e: CancellationException) {
                            throw e
                        } catch (e: Exception) {
                            error = e.message ?: "Couldn't create the poll"
                            busy = false
                        }
                    }
                },
            ) { Text(if (busy) "Creating…" else "Create") }
        },
        dismissButton = { TextButton(enabled = !busy, onClick = onDismiss) { Text("Cancel") } },
    )
}

@Composable
private fun SwitchRow(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().clickable { onChange(!checked) }.padding(vertical = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, style = MaterialTheme.typography.bodyLarge, color = LocalContentColor.current, modifier = Modifier.weight(1f))
        Switch(checked = checked, onCheckedChange = null)
    }
}
