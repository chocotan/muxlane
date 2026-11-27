package com.muxlane.android

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.BitmapFactory
import android.util.Base64
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Image
import androidx.compose.foundation.background
import androidx.compose.foundation.border
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
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Check
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.Edit
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.derivedStateOf
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.ImageBitmap
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.asImageBitmap
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonArray
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import muxlane.chat.ApprovalOption
import muxlane.chat.ApprovalReply
import muxlane.chat.BackgroundJob
import muxlane.chat.ChatAttachment
import muxlane.chat.ChatSessionMeta
import muxlane.chat.ChatSlashCommand
import muxlane.chat.DiffOp
import muxlane.chat.MessageUsage
import muxlane.chat.NoticeLevel
import muxlane.chat.PendingApproval
import muxlane.chat.PlanEntryStatus
import muxlane.chat.QueuedMessage
import muxlane.chat.SessionGoal
import muxlane.chat.SessionStats
import muxlane.chat.SessionSubagent
import muxlane.chat.SessionTask
import muxlane.chat.StreamingBehavior
import muxlane.chat.TimelineEntry
import muxlane.chat.ToolCallKind
import muxlane.chat.ToolCallState
import muxlane.chat.ToolDiff
import muxlane.chat.computeUnifiedDiffLines

internal sealed class MobileTimelineItem {
    abstract val key: String

    data class Single(val entry: TimelineEntry) : MobileTimelineItem() {
        override val key: String = entry.id
    }

    data class ExplorationCluster(
        val id: String,
        val tools: List<TimelineEntry.ToolCall>,
    ) : MobileTimelineItem() {
        override val key: String = id
    }
}

internal fun buildMobileTimelineItems(entries: List<TimelineEntry>): List<MobileTimelineItem> {
    val out = ArrayList<MobileTimelineItem>(entries.size)
    var index = 0
    while (index < entries.size) {
        val current = entries[index]
        if (current is TimelineEntry.ToolCall && isExplorationTool(current)) {
            val group = ArrayList<TimelineEntry.ToolCall>()
            while (index < entries.size) {
                val candidate = entries[index]
                if (candidate is TimelineEntry.ToolCall && isExplorationTool(candidate)) {
                    group.add(candidate)
                    index += 1
                } else {
                    break
                }
            }
            if (group.size >= 2) {
                out.add(MobileTimelineItem.ExplorationCluster("cluster:${group.first().id}", group))
            } else {
                group.forEach { out.add(MobileTimelineItem.Single(it)) }
            }
        } else {
            out.add(MobileTimelineItem.Single(current))
            index += 1
        }
    }
    return out
}

private fun isExplorationTool(tool: TimelineEntry.ToolCall): Boolean =
    (
        tool.kind == ToolCallKind.READ ||
            tool.kind == ToolCallKind.SEARCH ||
            tool.kind == ToolCallKind.FETCH ||
            tool.kind == ToolCallKind.THINK
        ) &&
        !tool.isError &&
        tool.diff == null &&
        tool.attachments.isEmpty()

internal sealed class MarkdownBlock {
    data class Heading(val level: Int, val text: String) : MarkdownBlock()
    data class Paragraph(val text: String) : MarkdownBlock()
    data class CodeBlock(val language: String, val code: String) : MarkdownBlock()
    data class BulletItem(val marker: String, val text: String) : MarkdownBlock()
    data class Quote(val text: String) : MarkdownBlock()
}

internal fun parseMarkdownBlocks(raw: String): List<MarkdownBlock> {
    if (raw.isBlank()) return emptyList()
    val lines = raw.replace("\r\n", "\n").lines()
    val blocks = ArrayList<MarkdownBlock>()
    val para = StringBuilder()

    fun flushPara() {
        val trimmed = para.toString().trimEnd()
        if (trimmed.isNotEmpty()) blocks.add(MarkdownBlock.Paragraph(trimmed))
        para.clear()
    }

    var idx = 0
    while (idx < lines.size) {
        val line = lines[idx]
        val trimmed = line.trimStart()
        if (trimmed.startsWith("```")) {
            flushPara()
            val lang = trimmed.removePrefix("```").trim()
            val code = StringBuilder()
            idx += 1
            while (idx < lines.size && !lines[idx].trimStart().startsWith("```")) {
                if (code.isNotEmpty()) code.append('\n')
                code.append(lines[idx])
                idx += 1
            }
            blocks.add(MarkdownBlock.CodeBlock(lang, code.toString()))
            idx += 1
            continue
        }
        if (trimmed.isEmpty()) {
            flushPara()
            idx += 1
            continue
        }
        val headingMatch = Regex("""^(#{1,4})\s+(.+)$""").matchEntire(trimmed)
        if (headingMatch != null) {
            flushPara()
            blocks.add(MarkdownBlock.Heading(headingMatch.groupValues[1].length, headingMatch.groupValues[2].trim()))
            idx += 1
            continue
        }
        if (trimmed.startsWith("> ")) {
            flushPara()
            val quote = StringBuilder(trimmed.removePrefix("> ").trim())
            while (idx + 1 < lines.size && lines[idx + 1].trimStart().startsWith("> ")) {
                idx += 1
                quote.append('\n').append(lines[idx].trimStart().removePrefix("> ").trim())
            }
            blocks.add(MarkdownBlock.Quote(quote.toString()))
            idx += 1
            continue
        }
        val bulletMatch = Regex("""^([-*]|\d+\.)\s+(.+)$""").matchEntire(trimmed)
        if (bulletMatch != null) {
            flushPara()
            val marker = if (bulletMatch.groupValues[1] in listOf("-", "*")) "•" else bulletMatch.groupValues[1]
            blocks.add(MarkdownBlock.BulletItem(marker, bulletMatch.groupValues[2]))
            idx += 1
            continue
        }
        if (para.isNotEmpty()) para.append('\n')
        para.append(line)
        idx += 1
    }
    flushPara()
    return blocks
}

@Composable
internal fun ChatTimelinePane(
    model: MuxlaneViewModel,
    agentId: String,
    state: UiState,
    showSlashMenu: Boolean,
    onToggleSlash: () -> Unit,
    onDismissSlash: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val timeline = state.chatTimelines[agentId]
    val meta = timeline?.meta ?: state.chatSessions[agentId]?.meta
    val entries = timeline?.entries.orEmpty()
    val pendingApprovals = timeline?.pendingApprovals ?: state.globalApprovals.filter { it.agent == agentId }
    val items = remember(entries) { buildMobileTimelineItems(entries) }
    val expandedClusters = remember(agentId) { mutableStateMapOf<String, Boolean>() }
    val expandedThinking = remember(agentId) { mutableStateMapOf<String, Boolean>() }
    var lightboxAttachment by remember(agentId) { mutableStateOf<ChatAttachment?>(null) }
    var draft by remember(agentId) { mutableStateOf("") }
    var steerWhenStreaming by remember(agentId) { mutableStateOf(true) }
    val draftSlashQuery = remember(draft) {
        if (draft.startsWith("/") && !draft.contains('\n') && !draft.contains(' ')) {
            draft.removePrefix("/")
        } else {
            null
        }
    }
    val effectiveSlashOpen = showSlashMenu || draftSlashQuery != null
    val slashItems = remember(meta?.availableCommands, draftSlashQuery) {
        filterMobileSlashCommands(
            available = meta?.availableCommands.orEmpty(),
            query = draftSlashQuery.orEmpty(),
        )
    }

    val listState = rememberLazyListState()
    val scope = rememberCoroutineScope()
    val atBottom by remember {
        derivedStateOf {
            val info = listState.layoutInfo
            val lastVisible = info.visibleItemsInfo.lastOrNull()?.index ?: 0
            info.totalItemsCount == 0 || lastVisible >= info.totalItemsCount - 2
        }
    }
    LaunchedEffect(
        items.size,
        items.lastOrNull()?.let {
            when (it) {
                is MobileTimelineItem.Single -> when (val e = it.entry) {
                    is TimelineEntry.AssistantText -> e.text.length
                    is TimelineEntry.Thinking -> e.text.length
                    is TimelineEntry.ToolCall -> (e.output?.length ?: 0) + e.attachments.size + e.state.hashCode()
                    else -> 0
                }
                is MobileTimelineItem.ExplorationCluster -> it.tools.size
            }
        },
    ) {
        if (items.isNotEmpty() && atBottom) {
            listState.scrollToItem(items.lastIndex)
        }
    }

    val baseDensity = androidx.compose.ui.platform.LocalDensity.current
    val scaledDensity = androidx.compose.ui.unit.Density(
        density = baseDensity.density,
        fontScale = baseDensity.fontScale * state.chatTextScale,
    )

    Column(modifier.fillMaxSize().background(MuxBg0)) {
        if (meta != null && (meta.currentModel != null || meta.availableModels.isNotEmpty() || meta.thinkingLevel != null)) {
            ChatModelConfigStrip(
                meta = meta,
                onConfigureModel = { provider, modelId ->
                    model.configureChat(provider = provider, modelId = modelId)
                },
                onConfigureThinking = { level ->
                    model.configureChat(thinkingLevel = level)
                },
            )
        }

        Box(
            Modifier
                .weight(1f)
                .fillMaxWidth(),
            contentAlignment = Alignment.TopCenter,
        ) {
            androidx.compose.runtime.CompositionLocalProvider(
                androidx.compose.ui.platform.LocalDensity provides scaledDensity,
            ) {
                Column(
                    Modifier
                        .fillMaxHeight()
                        .fillMaxWidth()
                        .widthIn(max = 820.dp),
                ) {
                    if (items.isEmpty()) {
                        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                            Column(
                                horizontalAlignment = Alignment.CenterHorizontally,
                                verticalArrangement = Arrangement.spacedBy(10.dp),
                                modifier = Modifier.padding(horizontal = 28.dp),
                            ) {
                                if (!state.connected) SquareStepIndicator(color = MuxCobalt)
                                Text(
                                    stringResource(R.string.empty_chat),
                                    style = MaterialTheme.typography.bodyMedium,
                                    color = MuxFg2,
                                )
                            }
                        }
                    } else {
                        Box(Modifier.fillMaxSize()) {
                            LazyColumn(
                                state = listState,
                                modifier = Modifier.fillMaxSize(),
                                contentPadding = PaddingValues(horizontal = 16.dp, vertical = 14.dp),
                                verticalArrangement = Arrangement.spacedBy(10.dp),
                            ) {
                                items(items, key = { it.key }) { item ->
                                    when (item) {
                                        is MobileTimelineItem.ExplorationCluster -> {
                                            val isExpanded = expandedClusters[item.id] == true
                                            ExplorationClusterCard(
                                                tools = item.tools,
                                                expanded = isExpanded,
                                                onToggle = { expandedClusters[item.id] = !isExpanded },
                                                onPreviewImage = { lightboxAttachment = it },
                                            )
                                        }
                                        is MobileTimelineItem.Single -> when (val entry = item.entry) {
                                            is TimelineEntry.UserMessage -> UserMessageCard(
                                                entry = entry,
                                                onPreviewImage = { lightboxAttachment = it },
                                            )
                                            is TimelineEntry.Thinking -> {
                                                val isExpanded = expandedThinking[entry.id] ?: entry.streaming
                                                ThinkingCard(
                                                    entry = entry,
                                                    expanded = isExpanded,
                                                    onToggle = { expandedThinking[entry.id] = !isExpanded },
                                                )
                                            }
                                            is TimelineEntry.AssistantText -> AssistantMessageCard(entry)
                                            is TimelineEntry.ToolCall -> ToolCallCard(
                                                entry = entry,
                                                onPreviewImage = { lightboxAttachment = it },
                                            )
                                            is TimelineEntry.Plan -> PlanCard(entry)
                                            is TimelineEntry.Notice -> NoticeCard(entry)
                                        }
                                    }
                                }
                            }
                            if (!atBottom) {
                                Surface(
                                    modifier = Modifier
                                        .align(Alignment.BottomCenter)
                                        .padding(bottom = 12.dp)
                                        .clickable {
                                            scope.launch { listState.animateScrollToItem(items.lastIndex) }
                                        },
                                    shape = Square,
                                    color = MuxBg1,
                                    border = BorderStroke(1.dp, MuxControlLine),
                                ) {
                                    Text(
                                        "↓ 回到最新",
                                        style = MaterialTheme.typography.labelMedium,
                                        color = MuxFg0,
                                        modifier = Modifier.padding(horizontal = 12.dp, vertical = 6.dp),
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }

        Column(
            Modifier
                .fillMaxWidth()
                .background(MuxBg1)
                .imePadding()
                .navigationBarsPadding(),
        ) {
            if (meta != null) {
                SessionDockStrip(
                    goal = meta.goal,
                    tasks = meta.tasks,
                    subagents = meta.subagents,
                    backgroundJobs = meta.backgroundJobs,
                )
            }
            if (!meta?.queuedMessages.isNullOrEmpty()) {
                QueuedMessagesStrip(
                    messages = meta!!.queuedMessages,
                    onDelete = { msgId -> model.deleteQueuedMessage(agentId, msgId) },
                )
            }
            pendingApprovals.firstOrNull()?.let { approval ->
                InlineApprovalBanner(
                    approval = approval,
                    onReply = { reply -> model.respondApproval(agentId, approval.id, reply) },
                )
            }
            if (effectiveSlashOpen && slashItems.isNotEmpty()) {
                SlashQuickMenu(
                    commands = slashItems,
                    onPick = { cmd ->
                        onDismissSlash()
                        when (cmd.name) {
                            "abort" -> {
                                draft = ""
                                model.abortChat()
                            }
                            else -> {
                                if (!cmd.argumentHint.isNullOrBlank()) {
                                    draft = "/${cmd.name} "
                                } else {
                                    draft = ""
                                    model.sendChatPrompt("/${cmd.name}", steer = steerWhenStreaming)
                                }
                            }
                        }
                    },
                )
            }
            ChatComposerBar(
                draft = draft,
                onDraftChange = { draft = it },
                inputEnabled = meta?.supportsChatInput != false,
                isStreaming = meta?.isStreaming == true,
                steerWhenStreaming = steerWhenStreaming,
                onToggleSteerMode = { steerWhenStreaming = !steerWhenStreaming },
                contextUsed = meta?.contextUsedTokens,
                contextWindow = meta?.contextWindowTokens,
                costUsd = meta?.totalCostUsd ?: meta?.stats?.totalCostUsd,
                stats = meta?.stats,
                slashOpen = effectiveSlashOpen,
                onToggleSlash = onToggleSlash,
                onSend = { steer ->
                    val text = draft.trim()
                    if (text.isNotEmpty()) {
                        model.sendChatPrompt(text, steer = steer)
                        draft = ""
                    }
                },
                onAbort = model::abortChat,
            )
        }
    }

    lightboxAttachment?.let { att ->
        ImageLightboxDialog(
            attachment = att,
            onDismiss = { lightboxAttachment = null },
        )
    }
}

@Composable
private fun rememberDecodedAttachmentBitmap(dataB64: String?): ImageBitmap? =
    remember(dataB64) {
        if (dataB64.isNullOrBlank()) {
            null
        } else {
            runCatching {
                val bytes = Base64.decode(dataB64, Base64.DEFAULT)
                BitmapFactory.decodeByteArray(bytes, 0, bytes.size)?.asImageBitmap()
            }.getOrNull()
        }
    }

@Composable
private fun AttachmentImageStrip(
    attachments: List<ChatAttachment>,
    onPreviewImage: (ChatAttachment) -> Unit,
    modifier: Modifier = Modifier,
) {
    if (attachments.isEmpty()) return
    Column(modifier = modifier, verticalArrangement = Arrangement.spacedBy(8.dp)) {
        attachments.forEach { att ->
            val bitmap = rememberDecodedAttachmentBitmap(att.dataB64)
            if (bitmap != null) {
                Surface(
                    shape = Square,
                    color = MuxBg1,
                    border = BorderStroke(1.dp, MuxLine),
                    modifier = Modifier.clickable { onPreviewImage(att) },
                ) {
                    Column(Modifier.padding(6.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Image(
                            bitmap = bitmap,
                            contentDescription = att.name,
                            contentScale = ContentScale.Fit,
                            modifier = Modifier
                                .fillMaxWidth()
                                .heightIn(min = 72.dp, max = 240.dp)
                                .background(MuxBg0, Square),
                        )
                        Row(
                            Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                att.name,
                                style = MaterialTheme.typography.labelSmall,
                                color = MuxFg2,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.weight(1f),
                            )
                            Text(
                                "点击查看大图",
                                style = MaterialTheme.typography.labelSmall,
                                color = MuxCobalt,
                            )
                        }
                    }
                }
            } else {
                Surface(
                    shape = Square,
                    color = MuxBg2,
                    border = BorderStroke(1.dp, MuxLine),
                ) {
                    Text(
                        att.path ?: att.name,
                        style = MaterialTheme.typography.labelSmall,
                        color = MuxFg1,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun ImageLightboxDialog(
    attachment: ChatAttachment,
    onDismiss: () -> Unit,
) {
    val bitmap = rememberDecodedAttachmentBitmap(attachment.dataB64)
    if (bitmap == null) {
        onDismiss()
        return
    }
    Dialog(
        onDismissRequest = onDismiss,
        properties = DialogProperties(usePlatformDefaultWidth = false),
    ) {
        Box(
            Modifier
                .fillMaxSize()
                .background(Color(0xE60D0F14))
                .clickable(onClick = onDismiss)
                .padding(16.dp),
            contentAlignment = Alignment.Center,
        ) {
            Surface(
                shape = Square,
                color = MuxBg1,
                border = BorderStroke(1.dp, MuxControlLine),
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable(enabled = false) {},
            ) {
                Column(Modifier.fillMaxWidth()) {
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .background(MuxBg0)
                            .padding(horizontal = 12.dp, vertical = 10.dp),
                        horizontalArrangement = Arrangement.SpaceBetween,
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        Text(
                            "${attachment.name} · ${attachment.mimeType}",
                            style = MaterialTheme.typography.labelMedium,
                            color = MuxFg0,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f),
                        )
                        Spacer(Modifier.width(8.dp))
                        Box(
                            Modifier
                                .size(28.dp)
                                .background(MuxBg1, Square)
                                .border(BorderStroke(1.dp, MuxControlLine), Square)
                                .clickable(onClick = onDismiss),
                            contentAlignment = Alignment.Center,
                        ) {
                            Icon(Icons.Outlined.Close, "关闭", tint = MuxFg0, modifier = Modifier.size(16.dp))
                        }
                    }
                    HorizontalDivider(color = MuxLine)
                    Box(
                        Modifier
                            .fillMaxWidth()
                            .background(MuxBg0)
                            .padding(10.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Image(
                            bitmap = bitmap,
                            contentDescription = attachment.name,
                            contentScale = ContentScale.Fit,
                            modifier = Modifier
                                .fillMaxWidth()
                                .heightIn(min = 160.dp, max = 560.dp),
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun ChatModelConfigStrip(
    meta: ChatSessionMeta,
    onConfigureModel: (String?, String) -> Unit,
    onConfigureThinking: (String) -> Unit,
) {
    var modelMenuOpen by remember { mutableStateOf(false) }
    var thinkingMenuOpen by remember { mutableStateOf(false) }
    Surface(color = MuxBg0, contentColor = MuxFg0) {
        Column(Modifier.fillMaxWidth()) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState())
                    .padding(horizontal = 14.dp, vertical = 7.dp),
                horizontalArrangement = Arrangement.spacedBy(8.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                val modelLabel = meta.currentModel?.name?.takeIf { it.isNotBlank() }
                    ?: meta.currentModel?.id
                    ?: stringResource(R.string.model_picker)
                Box {
                    Row(
                        modifier = Modifier
                            .background(MuxBg1, Square)
                            .border(BorderStroke(1.dp, MuxLine), Square)
                            .clickable(enabled = meta.availableModels.isNotEmpty()) { modelMenuOpen = true }
                            .padding(horizontal = 9.dp, vertical = 4.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        Text(
                            modelLabel,
                            style = MaterialTheme.typography.labelMedium,
                            color = MuxFg0,
                        )
                        if (meta.availableModels.isNotEmpty()) {
                            Icon(Icons.Outlined.KeyboardArrowDown, null, tint = MuxFg2, modifier = Modifier.size(14.dp))
                        }
                    }
                    DropdownMenu(expanded = modelMenuOpen, onDismissRequest = { modelMenuOpen = false }) {
                        meta.availableModels.forEach { m ->
                            DropdownMenuItem(
                                text = { Text("${m.name.ifBlank { m.id }} (${m.provider})", style = MaterialTheme.typography.bodySmall) },
                                onClick = {
                                    modelMenuOpen = false
                                    onConfigureModel(m.provider.ifBlank { null }, m.id)
                                },
                            )
                        }
                    }
                }
                if (meta.thinkingLevel != null || meta.availableThinkingLevels.isNotEmpty()) {
                    Box {
                        Row(
                            modifier = Modifier
                                .background(MuxBg1, Square)
                                .border(BorderStroke(1.dp, MuxLine), Square)
                                .clickable(enabled = meta.availableThinkingLevels.isNotEmpty()) { thinkingMenuOpen = true }
                                .padding(horizontal = 9.dp, vertical = 4.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(4.dp),
                        ) {
                            Text(
                                "${stringResource(R.string.thinking_picker)} · ${meta.thinkingLevel ?: "default"}",
                                style = MaterialTheme.typography.labelMedium,
                                color = MuxFg1,
                            )
                            if (meta.availableThinkingLevels.isNotEmpty()) {
                                Icon(Icons.Outlined.KeyboardArrowDown, null, tint = MuxFg2, modifier = Modifier.size(14.dp))
                            }
                        }
                        DropdownMenu(expanded = thinkingMenuOpen, onDismissRequest = { thinkingMenuOpen = false }) {
                            meta.availableThinkingLevels.forEach { level ->
                                DropdownMenuItem(
                                    text = { Text(level, style = MaterialTheme.typography.bodySmall) },
                                    onClick = {
                                        thinkingMenuOpen = false
                                        onConfigureThinking(level)
                                    },
                                )
                            }
                        }
                    }
                }
            }
            HorizontalDivider(color = MuxLine)
        }
    }
}

@Composable
private fun UserMessageCard(
    entry: TimelineEntry.UserMessage,
    onPreviewImage: (ChatAttachment) -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .padding(top = 6.dp),
        horizontalArrangement = Arrangement.End,
    ) {
        Surface(
            modifier = Modifier.widthIn(max = 330.dp),
            shape = Square,
            color = MuxUserSurface,
            border = BorderStroke(1.dp, MuxLine),
        ) {
            Column(
                Modifier.padding(horizontal = 13.dp, vertical = 10.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                if (entry.text.isNotBlank()) {
                    Text(
                        entry.text,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MuxFg0,
                    )
                }
                if (entry.attachments.isNotEmpty()) {
                    AttachmentImageStrip(
                        attachments = entry.attachments,
                        onPreviewImage = onPreviewImage,
                    )
                }
            }
        }
    }
}

@Composable
private fun AssistantMessageCard(entry: TimelineEntry.AssistantText) {
    val blocks = remember(entry.text) { parseMarkdownBlocks(entry.text) }
    val context = LocalContext.current
    Column(
        Modifier
            .fillMaxWidth()
            .padding(vertical = 4.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (entry.streaming) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                SquareStepIndicator(color = MuxCobalt, blockSize = 4.dp, gap = 2.dp, blockCount = 3)
                Text(
                    entry.model?.ifBlank { null } ?: "正在输出",
                    style = MaterialTheme.typography.labelSmall,
                    color = MuxFg2,
                )
            }
        }
        blocks.forEach { block ->
            when (block) {
                is MarkdownBlock.Heading -> Text(
                    inlineMarkdown(block.text),
                    style = when (block.level) {
                        1 -> MaterialTheme.typography.titleLarge
                        2 -> MaterialTheme.typography.titleMedium
                        else -> MaterialTheme.typography.bodyLarge.copy(fontWeight = FontWeight.SemiBold)
                    },
                    color = MuxFg0,
                )
                is MarkdownBlock.Paragraph -> Text(
                    inlineMarkdown(block.text),
                    style = MaterialTheme.typography.bodyMedium,
                    color = MuxFg0,
                )
                is MarkdownBlock.Quote -> Row(
                    Modifier
                        .fillMaxWidth()
                        .background(MuxCodeBg, Square)
                        .border(BorderStroke(1.dp, MuxLine), Square)
                        .padding(horizontal = 10.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.Top,
                ) {
                    Box(Modifier.width(2.dp).height(18.dp).background(MuxCobalt))
                    Spacer(Modifier.width(8.dp))
                    Text(inlineMarkdown(block.text), style = MaterialTheme.typography.bodyMedium, color = MuxFg1)
                }
                is MarkdownBlock.BulletItem -> Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.Top,
                ) {
                    Text(
                        block.marker,
                        style = MaterialTheme.typography.bodyMedium,
                        color = MuxCobalt,
                        fontWeight = FontWeight.SemiBold,
                    )
                    Text(
                        inlineMarkdown(block.text),
                        style = MaterialTheme.typography.bodyMedium,
                        color = MuxFg0,
                        modifier = Modifier.weight(1f),
                    )
                }
                is MarkdownBlock.CodeBlock -> Surface(
                    modifier = Modifier.fillMaxWidth(),
                    shape = Square,
                    color = MuxCodeBg,
                    border = BorderStroke(1.dp, MuxLine),
                ) {
                    Column {
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .background(MuxCodeHeader)
                                .padding(horizontal = 10.dp, vertical = 5.dp),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                block.language.ifBlank { "code" },
                                style = MaterialTheme.typography.labelSmall,
                                color = MuxFg2,
                                fontFamily = MonoFamily,
                            )
                            Row(
                                modifier = Modifier
                                    .clickable {
                                        val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                                        cm.setPrimaryClip(ClipData.newPlainText("code", block.code))
                                    }
                                    .padding(horizontal = 4.dp, vertical = 2.dp),
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(4.dp),
                            ) {
                                Icon(
                                    Icons.Outlined.ContentCopy,
                                    contentDescription = "复制代码",
                                    tint = MuxFg2,
                                    modifier = Modifier.size(13.dp),
                                )
                                Text("复制", style = MaterialTheme.typography.labelSmall, color = MuxFg2)
                            }
                        }
                        HorizontalDivider(color = MuxLine)
                        Box(Modifier.horizontalScroll(rememberScrollState()).padding(10.dp)) {
                            Text(
                                block.code,
                                style = MaterialTheme.typography.bodySmall.copy(
                                    fontFamily = MonoFamily,
                                    lineHeight = 20.sp,
                                ),
                                color = MuxFg0,
                            )
                        }
                    }
                }
            }
        }
        if (!entry.streaming && entry.text.isNotBlank()) {
            val usageSummary = entry.usage?.let(::formatMessageUsage).orEmpty()
            val footerParts = buildList {
                entry.model?.takeIf { it.isNotBlank() }?.let(::add)
                if (usageSummary.isNotEmpty()) add(usageSummary)
            }
            if (footerParts.isNotEmpty()) {
                Row(
                    Modifier.fillMaxWidth(),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        footerParts.joinToString(" · "),
                        style = MaterialTheme.typography.labelSmall,
                        color = MuxFg2,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    Icon(
                        Icons.Outlined.ContentCopy,
                        contentDescription = "复制回复",
                        tint = MuxFg2,
                        modifier = Modifier
                            .size(14.dp)
                            .clickable {
                                val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                                cm.setPrimaryClip(ClipData.newPlainText("assistant", entry.text))
                            },
                    )
                }
            }
        }
    }
}

private fun inlineMarkdown(text: String): AnnotatedString = buildAnnotatedString {
    val tokenRegex = Regex("""(`[^`]+`|\*\*[^*]+\*\*|\*[^*]+\*)""")
    var lastIndex = 0
    for (match in tokenRegex.findAll(text)) {
        if (match.range.first > lastIndex) {
            append(text.substring(lastIndex, match.range.first))
        }
        val tok = match.value
        when {
            tok.startsWith("`") && tok.endsWith("`") -> {
                withStyle(SpanStyle(fontFamily = MonoFamily, background = MuxCodeBg, color = MuxFg0)) {
                    append(tok.substring(1, tok.length - 1))
                }
            }
            tok.startsWith("**") && tok.endsWith("**") -> {
                withStyle(SpanStyle(fontWeight = FontWeight.SemiBold, color = MuxFg0)) {
                    append(tok.substring(2, tok.length - 2))
                }
            }
            tok.startsWith("*") && tok.endsWith("*") -> {
                withStyle(SpanStyle(fontStyle = FontStyle.Italic)) {
                    append(tok.substring(1, tok.length - 1))
                }
            }
            else -> append(tok)
        }
        lastIndex = match.range.last + 1
    }
    if (lastIndex < text.length) append(text.substring(lastIndex))
}

@Composable
private fun ThinkingCard(
    entry: TimelineEntry.Thinking,
    expanded: Boolean,
    onToggle: () -> Unit,
) {
    Column(Modifier.fillMaxWidth()) {
        Row(
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onToggle)
                .padding(vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Icon(
                if (expanded) Icons.Outlined.KeyboardArrowDown else Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                null,
                tint = MuxFg2,
                modifier = Modifier.size(16.dp),
            )
            Text(
                buildString {
                    append(if (entry.streaming) "正在思考" else stringResource(R.string.thinking_title))
                    entry.durationMs?.takeIf { it > 0 }?.let { ms ->
                        append(if (ms >= 1000) " · ${"%.1f".format(ms / 1000.0)}s" else " · ${ms}ms")
                    }
                },
                style = MaterialTheme.typography.labelMedium,
                color = MuxFg2,
            )
            Spacer(Modifier.weight(1f))
            if (entry.streaming) SquareStepIndicator(color = MuxCobalt, blockSize = 4.dp, gap = 2.dp, blockCount = 3)
        }
        if (expanded && entry.text.isNotBlank()) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(start = 7.dp, top = 2.dp, bottom = 4.dp),
            ) {
                Box(Modifier.width(1.dp).background(MuxLine))
                Column(
                    Modifier
                        .fillMaxWidth()
                        .border(BorderStroke(1.dp, MuxLine), Square)
                        .background(MuxBg1, Square)
                        .padding(horizontal = 10.dp, vertical = 8.dp),
                ) {
                    Text(
                        entry.text,
                        style = MaterialTheme.typography.bodySmall,
                        color = MuxFg1,
                    )
                }
            }
        }
    }
}

@Composable
private fun ExplorationClusterCard(
    tools: List<TimelineEntry.ToolCall>,
    expanded: Boolean,
    onToggle: () -> Unit,
    onPreviewImage: (ChatAttachment) -> Unit,
) {
    val active = tools.any { it.state == ToolCallState.PENDING || it.state == ToolCallState.RUNNING }
    Column(Modifier.fillMaxWidth()) {
        Row(
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onToggle)
                .padding(vertical = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Icon(
                if (expanded) Icons.Outlined.KeyboardArrowDown else Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                null,
                tint = MuxFg2,
                modifier = Modifier.size(16.dp),
            )
            Text(
                if (active) "正在探索 (${tools.size} 步)" else "已完成探索 · ${tools.size} 步",
                style = MaterialTheme.typography.labelMedium,
                color = MuxFg1,
                modifier = Modifier.weight(1f),
            )
            if (active) SquareStepIndicator(color = MuxCobalt, blockSize = 4.dp, gap = 2.dp, blockCount = 3)
        }
        if (expanded) {
            Column(
                Modifier
                    .fillMaxWidth()
                    .padding(start = 10.dp, top = 2.dp, bottom = 4.dp)
                    .border(BorderStroke(1.dp, MuxLine), Square)
                    .background(MuxBg1, Square)
                    .padding(horizontal = 10.dp, vertical = 8.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                tools.forEach { t ->
                    ToolCallCard(entry = t, onPreviewImage = onPreviewImage, compact = true)
                }
            }
        }
    }
}

private fun toolVerbLabel(entry: TimelineEntry.ToolCall): String {
    val running = entry.state == ToolCallState.PENDING || entry.state == ToolCallState.RUNNING
    return when (entry.kind) {
        ToolCallKind.READ -> if (running) "正在读取" else "读取"
        ToolCallKind.SEARCH -> if (running) "正在搜索" else "搜索"
        ToolCallKind.EDIT -> if (running) "正在编辑" else "编辑"
        ToolCallKind.WRITE -> if (running) "正在写入" else "写入"
        ToolCallKind.BASH -> if (running) "正在运行" else "运行"
        ToolCallKind.FETCH -> if (running) "正在抓取" else "抓取"
        ToolCallKind.THINK -> if (running) "正在思考" else "思考"
        ToolCallKind.OTHER -> entry.name.ifBlank { "工具" }
    }
}

@Composable
private fun ToolCallCard(
    entry: TimelineEntry.ToolCall,
    onPreviewImage: (ChatAttachment) -> Unit,
    compact: Boolean = false,
) {
    val hasExpandableDetail = entry.diff != null || !entry.output.isNullOrBlank()
    var outputExpanded by remember(entry.id) {
        mutableStateOf(entry.isError)
    }
    val running = entry.state == ToolCallState.PENDING || entry.state == ToolCallState.RUNNING
    val failed = entry.isError || entry.state == ToolCallState.FAILED || (entry.exitCode != null && entry.exitCode != 0)
    val statusColor = when {
        failed -> MuxRed
        running -> MuxCobalt
        else -> MuxGreen
    }
    val verb = toolVerbLabel(entry)
    val summary = remember(entry.id, entry.title, entry.input, entry.locations) { toolSummary(entry) }
    val durationLabel = entry.durationMs?.takeIf { it > 0 }?.let { ms ->
        if (ms >= 1000) "${"%.1f".format(ms / 1000.0)}s" else "${ms}ms"
    }

    Column(Modifier.fillMaxWidth()) {
        Row(
            Modifier
                .fillMaxWidth()
                .then(
                    if (hasExpandableDetail) {
                        Modifier.clickable { outputExpanded = !outputExpanded }
                    } else {
                        Modifier
                    },
                )
                .padding(vertical = if (compact) 2.dp else 4.dp),
            verticalAlignment = Alignment.Top,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(
                Modifier
                    .padding(top = 6.dp)
                    .size(7.dp)
                    .background(statusColor, Square),
            )
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Row(
                    Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Text(
                        verb,
                        style = MaterialTheme.typography.labelMedium.copy(fontWeight = FontWeight.SemiBold),
                        color = if (failed) MuxRed else MuxFg0,
                    )
                    if (entry.kind != ToolCallKind.OTHER && entry.name.isNotBlank() && entry.name.lowercase() != verb.lowercase()) {
                        Text(
                            entry.name,
                            style = MaterialTheme.typography.labelSmall,
                            color = MuxFg2,
                            fontFamily = MonoFamily,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                    Spacer(Modifier.weight(1f))
                    entry.exitCode?.takeIf { it != 0 }?.let { code ->
                        Box(
                            Modifier
                                .background(MuxErrorBg, Square)
                                .border(BorderStroke(1.dp, MuxErrorLine), Square)
                                .padding(horizontal = 5.dp, vertical = 1.dp),
                        ) {
                            Text(
                                "exit $code",
                                style = MaterialTheme.typography.labelSmall,
                                color = MuxErrorFg,
                                fontFamily = MonoFamily,
                            )
                        }
                    }
                    if (durationLabel != null) {
                        Text(
                            durationLabel,
                            style = MaterialTheme.typography.labelSmall,
                            color = MuxFg2,
                            fontFamily = MonoFamily,
                        )
                    }
                    if (hasExpandableDetail) {
                        Icon(
                            if (outputExpanded) Icons.Outlined.KeyboardArrowDown else Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                            contentDescription = null,
                            tint = MuxFg2,
                            modifier = Modifier.size(15.dp),
                        )
                    }
                }
                if (summary.isNotBlank()) {
                    // Use default sans-serif + generous CJK line height (21sp) and maxLines = 2
                    // so Chinese tool paths/descriptions are never vertically clipped or cut in half.
                    Text(
                        summary,
                        style = MaterialTheme.typography.bodySmall,
                        color = if (failed) MuxErrorFg else MuxFg1,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }

        if (entry.attachments.isNotEmpty()) {
            AttachmentImageStrip(
                attachments = entry.attachments,
                onPreviewImage = onPreviewImage,
                modifier = Modifier
                    .fillMaxWidth()
                    .padding(start = 15.dp, top = 4.dp, bottom = 4.dp),
            )
        }

        if (outputExpanded && hasExpandableDetail) {
            Column(
                Modifier
                    .fillMaxWidth()
                    .padding(start = 15.dp, top = 4.dp, bottom = 4.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                entry.diff?.let { diff ->
                    UnifiedDiffCard(diff)
                }
                entry.output?.takeIf { it.isNotBlank() }?.let { out ->
                    Box(
                        Modifier
                            .fillMaxWidth()
                            .background(if (failed) MuxErrorBg else MuxCodeBg, Square)
                            .border(BorderStroke(1.dp, if (failed) MuxErrorLine else MuxLine), Square)
                            .horizontalScroll(rememberScrollState())
                            .padding(horizontal = 10.dp, vertical = 8.dp),
                    ) {
                        Text(
                            out.take(4000),
                            style = MaterialTheme.typography.bodySmall.copy(
                                fontFamily = MonoFamily,
                                lineHeight = 20.sp,
                            ),
                            color = if (failed) MuxErrorFg else MuxFg1,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun UnifiedDiffCard(diff: ToolDiff) {
    val lines = remember(diff.oldText, diff.newText) {
        computeUnifiedDiffLines(diff.oldText, diff.newText, maxLines = 120)
    }
    Column(
        Modifier
            .fillMaxWidth()
            .border(BorderStroke(1.dp, MuxLine), Square)
            .background(MuxBg1, Square),
    ) {
        diff.path?.let { path ->
            Text(
                path,
                style = MaterialTheme.typography.labelSmall,
                color = MuxFg1,
                fontFamily = MonoFamily,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier
                    .fillMaxWidth()
                    .background(MuxCodeHeader)
                    .padding(horizontal = 9.dp, vertical = 5.dp),
            )
            HorizontalDivider(color = MuxLine)
        }
        Column(Modifier.horizontalScroll(rememberScrollState()).padding(vertical = 4.dp)) {
            lines.forEach { line ->
                val (bg, fg, prefix) = when (line.op) {
                    DiffOp.ADD -> Triple(Color(0xFFECFDF3), Color(0xFF166534), "+")
                    DiffOp.DEL -> Triple(Color(0xFFFEF2F2), Color(0xFFB91C1C), "-")
                    DiffOp.CONTEXT -> Triple(Color.Transparent, MuxFg2, " ")
                }
                Row(
                    Modifier
                        .fillMaxWidth()
                        .background(bg)
                        .padding(horizontal = 8.dp, vertical = 1.dp),
                ) {
                    Text(
                        "${line.lineNum?.toString()?.padStart(3) ?: "   "} $prefix ${line.text}",
                        style = MaterialTheme.typography.bodySmall.copy(
                            fontFamily = MonoFamily,
                            fontSize = 12.sp,
                            lineHeight = 19.sp,
                        ),
                        color = fg,
                    )
                }
            }
        }
    }
}

@Composable
private fun PlanCard(entry: TimelineEntry.Plan) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = Square,
        color = MuxBg1,
        border = BorderStroke(1.dp, MuxLine),
    ) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Text("执行计划", style = MaterialTheme.typography.labelMedium, color = MuxCobalt)
            entry.entries.forEach { item ->
                val (marker, color) = when (item.status) {
                    PlanEntryStatus.COMPLETED -> "[x]" to MuxGreen
                    PlanEntryStatus.IN_PROGRESS -> "[▸]" to MuxCobalt
                    PlanEntryStatus.PENDING -> "[ ]" to MuxFg2
                }
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(marker, style = MaterialTheme.typography.bodySmall, color = color, fontFamily = MonoFamily)
                    Text(item.content, style = MaterialTheme.typography.bodySmall, color = MuxFg0)
                }
            }
        }
    }
}

@Composable
private fun NoticeCard(entry: TimelineEntry.Notice) {
    val (bg, fg, border) = when (entry.level) {
        NoticeLevel.ERROR -> Triple(MuxErrorBg, MuxErrorFg, MuxErrorLine)
        NoticeLevel.WARNING -> Triple(MuxWarningBg, MuxWarningFg, MuxWarningLine)
        NoticeLevel.COMPACTION -> Triple(MuxBg2, MuxFg1, MuxControlLine)
        NoticeLevel.INFO -> Triple(MuxBg1, MuxFg1, MuxLine)
    }
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = Square,
        color = bg,
        border = BorderStroke(1.dp, border),
    ) {
        Text(
            entry.text,
            style = MaterialTheme.typography.bodySmall,
            color = fg,
            modifier = Modifier.padding(horizontal = 12.dp, vertical = 9.dp),
        )
    }
}

@Composable
private fun InlineApprovalBanner(
    approval: PendingApproval,
    onReply: (ApprovalReply) -> Unit,
) {
    var replyText by remember(approval.id) { mutableStateOf(approval.prefill.orEmpty()) }
    val parsed = remember(approval) { parseApprovalPrompt(approval) }
    val selectedIndices = remember(parsed.isMultiSelectInput, replyText) {
        if (!parsed.isMultiSelectInput) {
            emptySet()
        } else {
            val tokens = replyText.trim().split(Regex("[,\\s]+")).filter { it.isNotEmpty() }
            if (tokens.all { tok -> tok.all { it.isDigit() } }) tokens.toSet() else emptySet()
        }
    }
    val accentColor = if (parsed.isQuestion) MuxCobalt else MuxYellow
    val badgeBg = if (parsed.isQuestion) MuxAccentBg else MuxWarningBg
    val badgeBorder = if (parsed.isQuestion) MuxCobalt.copy(alpha = 0.28f) else MuxWarningLine
    val badgeFg = if (parsed.isQuestion) MuxCobalt else MuxWarningFg
    val showTextInput = parsed.isInputOnly || approval.prefill != null
    val showConfirmFallback = !parsed.isInputOnly && parsed.options.isEmpty()

    Surface(
        modifier = Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 6.dp),
        shape = Square,
        color = MuxBg1,
        border = BorderStroke(1.dp, MuxControlLine),
    ) {
        Column(Modifier.fillMaxWidth()) {
            // Top accent hairline
            Box(Modifier.fillMaxWidth().height(2.dp).background(accentColor))

            // Header bar with tag + dismiss
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .background(MuxBg2)
                    .padding(horizontal = 12.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Box(
                    modifier = Modifier
                        .background(badgeBg, Square)
                        .border(BorderStroke(1.dp, badgeBorder), Square)
                        .padding(horizontal = 7.dp, vertical = 2.dp),
                ) {
                    Text(
                        parsed.badge,
                        style = MaterialTheme.typography.labelSmall,
                        color = badgeFg,
                        fontWeight = FontWeight.SemiBold,
                    )
                }
                approval.toolName?.takeIf { it.isNotBlank() }?.let { tool ->
                    Box(
                        modifier = Modifier
                            .background(MuxBg1, Square)
                            .border(BorderStroke(1.dp, MuxLine), Square)
                            .padding(horizontal = 6.dp, vertical = 2.dp),
                    ) {
                        Text(
                            tool,
                            style = MaterialTheme.typography.labelSmall,
                            color = MuxFg2,
                            fontFamily = MonoFamily,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                Spacer(Modifier.weight(1f))
                Box(
                    modifier = Modifier
                        .background(MuxBg1, Square)
                        .border(BorderStroke(1.dp, MuxControlLine), Square)
                        .clickable { onReply(ApprovalReply.Cancel) }
                        .padding(horizontal = 8.dp, vertical = 4.dp),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(3.dp),
                    ) {
                        Icon(
                            Icons.Outlined.Close,
                            contentDescription = "取消",
                            tint = MuxFg2,
                            modifier = Modifier.size(13.dp),
                        )
                        Text(
                            "取消",
                            style = MaterialTheme.typography.labelSmall,
                            color = MuxFg1,
                        )
                    }
                }
            }
            HorizontalDivider(color = MuxLine)

            // Scrollable body capped so composer stays reachable
            Column(
                modifier = Modifier
                    .fillMaxWidth()
                    .heightIn(max = 340.dp)
                    .verticalScroll(rememberScrollState())
                    .padding(12.dp),
                verticalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                Text(
                    text = parsed.questionTitle,
                    style = MaterialTheme.typography.titleMedium,
                    color = MuxFg0,
                )

                approval.message?.takeIf { it.isNotBlank() }?.let { msg ->
                    Text(msg, style = MaterialTheme.typography.bodySmall, color = MuxFg1)
                }

                approval.commandOrInput?.takeIf { it.isNotBlank() }?.let { cmd ->
                    Box(
                        Modifier
                            .fillMaxWidth()
                            .background(MuxCodeBg, Square)
                            .border(BorderStroke(1.dp, MuxLine), Square)
                            .padding(9.dp),
                    ) {
                        Text(
                            cmd,
                            style = MaterialTheme.typography.bodySmall,
                            color = MuxFg0,
                            fontFamily = MonoFamily,
                            maxLines = 5,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }

                parsed.previews.forEach { preview ->
                    Column(
                        Modifier
                            .fillMaxWidth()
                            .background(MuxCodeBg, Square)
                            .border(BorderStroke(1.dp, MuxLine), Square),
                    ) {
                        Text(
                            preview.title,
                            style = MaterialTheme.typography.labelSmall,
                            color = MuxFg2,
                            fontFamily = MonoFamily,
                            modifier = Modifier
                                .fillMaxWidth()
                                .background(MuxCodeHeader)
                                .padding(horizontal = 9.dp, vertical = 5.dp),
                        )
                        HorizontalDivider(color = MuxLine)
                        Text(
                            preview.body,
                            style = MaterialTheme.typography.bodySmall,
                            color = MuxFg0,
                            fontFamily = MonoFamily,
                            modifier = Modifier.padding(9.dp),
                        )
                    }
                }

                // Vertical aligned option list (0px architectural rows, fixed index rail)
                if (parsed.options.isNotEmpty() && parsed.useListLayout) {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .background(MuxBg0, Square)
                            .border(BorderStroke(1.dp, MuxControlLine), Square),
                    ) {
                        parsed.options.forEachIndexed { idx, opt ->
                            if (idx > 0) HorizontalDivider(color = MuxLine)
                            PromptOptionRow(
                                option = opt,
                                selected = false,
                                interactive = true,
                                onClick = { onReply(ApprovalReply.SelectedOption(opt.optionId)) },
                            )
                        }
                    }
                }

                // Embedded options parsed from Pi multi-select / custom input title
                if (parsed.embeddedOptions.isNotEmpty()) {
                    Column(
                        modifier = Modifier
                            .fillMaxWidth()
                            .background(MuxBg0, Square)
                            .border(BorderStroke(1.dp, MuxControlLine), Square),
                    ) {
                        parsed.embeddedOptions.forEachIndexed { idx, opt ->
                            if (idx > 0) HorizontalDivider(color = MuxLine)
                            val indexToken = opt.indexLabel.orEmpty()
                            val checked = parsed.isMultiSelectInput && indexToken.isNotEmpty() && indexToken in selectedIndices
                            PromptOptionRow(
                                option = opt,
                                selected = checked,
                                interactive = parsed.isMultiSelectInput,
                                onClick = {
                                    if (parsed.isMultiSelectInput && indexToken.isNotEmpty()) {
                                        replyText = toggleMultiSelectNumber(replyText, indexToken)
                                    }
                                },
                            )
                        }
                    }
                }

                // Compact binary action buttons (only for short confirm / permission approvals)
                if (parsed.options.isNotEmpty() && !parsed.useListLayout) {
                    Row(
                        Modifier.fillMaxWidth(),
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        parsed.options.forEach { opt ->
                            if (opt.isReject) {
                                OutlinedButton(
                                    onClick = { onReply(ApprovalReply.SelectedOption(opt.optionId)) },
                                    shape = Square,
                                    border = BorderStroke(1.dp, MuxRed),
                                    contentPadding = PaddingValues(horizontal = 14.dp, vertical = 8.dp),
                                ) { Text(opt.headline, color = MuxRed, style = MaterialTheme.typography.labelLarge) }
                            } else {
                                Button(
                                    onClick = { onReply(ApprovalReply.SelectedOption(opt.optionId)) },
                                    shape = Square,
                                    colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                                    contentPadding = PaddingValues(horizontal = 14.dp, vertical = 8.dp),
                                ) { Text(opt.headline, style = MaterialTheme.typography.labelLarge) }
                            }
                        }
                    }
                }

                if (showConfirmFallback) {
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        Button(
                            onClick = { onReply(ApprovalReply.Confirm(true)) },
                            shape = Square,
                            colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                            contentPadding = PaddingValues(horizontal = 14.dp, vertical = 8.dp),
                        ) { Text(stringResource(R.string.approve_allow), style = MaterialTheme.typography.labelLarge) }
                        OutlinedButton(
                            onClick = { onReply(ApprovalReply.Confirm(false)) },
                            shape = Square,
                            border = BorderStroke(1.dp, MuxRed),
                            contentPadding = PaddingValues(horizontal = 14.dp, vertical = 8.dp),
                        ) { Text(stringResource(R.string.approve_deny), color = MuxRed, style = MaterialTheme.typography.labelLarge) }
                    }
                }

                parsed.hint?.let { hintText ->
                    Text(hintText, style = MaterialTheme.typography.labelSmall, color = MuxFg2)
                }

                if (showTextInput) {
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        BasicTextField(
                            value = replyText,
                            onValueChange = { replyText = it },
                            singleLine = true,
                            textStyle = MaterialTheme.typography.bodySmall.copy(color = MuxFg0),
                            cursorBrush = SolidColor(MuxCobalt),
                            modifier = Modifier
                                .weight(1f)
                                .background(MuxBg0, Square)
                                .border(BorderStroke(1.dp, MuxControlLine), Square)
                                .padding(horizontal = 10.dp, vertical = 9.dp),
                            decorationBox = { inner ->
                                if (replyText.isEmpty()) {
                                    Text(
                                        if (parsed.isMultiSelectInput) {
                                            "输入选项序号（如 1,3）或自定义内容…"
                                        } else {
                                            stringResource(R.string.approve_reply_hint)
                                        },
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MuxFg2,
                                    )
                                }
                                inner()
                            },
                        )
                        Button(
                            onClick = { if (replyText.isNotBlank()) onReply(ApprovalReply.Text(replyText.trim())) },
                            enabled = replyText.isNotBlank(),
                            shape = Square,
                            colors = ButtonDefaults.buttonColors(
                                containerColor = MuxAccent,
                                contentColor = MuxOnAccent,
                            ),
                            contentPadding = PaddingValues(horizontal = 14.dp, vertical = 8.dp),
                            modifier = Modifier.height(38.dp),
                        ) {
                            Text(stringResource(R.string.approve_reply), style = MaterialTheme.typography.labelMedium)
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun PromptOptionRow(
    option: ParsedPromptOption,
    selected: Boolean,
    interactive: Boolean,
    onClick: () -> Unit,
) {
    val rowBg = when {
        selected -> MuxAccentBg
        option.isCustom -> MuxBg2.copy(alpha = 0.65f)
        else -> MuxBg1
    }
    val badgeBg = when {
        selected -> MuxCobalt
        option.isReject -> MuxErrorBg
        option.isCustom -> MuxBg2
        else -> MuxBg0
    }
    val badgeBorder = when {
        selected -> MuxCobalt
        option.isReject -> MuxErrorLine
        else -> MuxControlLine
    }
    val badgeFg = when {
        selected -> MuxOnAccent
        option.isReject -> MuxRed
        option.isCustom -> MuxFg2
        else -> MuxFg1
    }
    val rowModifier = if (interactive) {
        Modifier
            .fillMaxWidth()
            .background(rowBg)
            .clickable(onClick = onClick)
            .padding(horizontal = 11.dp, vertical = 10.dp)
    } else {
        Modifier
            .fillMaxWidth()
            .background(rowBg)
            .padding(horizontal = 11.dp, vertical = 9.dp)
    }

    Row(
        modifier = rowModifier,
        verticalAlignment = Alignment.Top,
        horizontalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (option.indexLabel != null || option.isCustom || selected) {
            Box(
                modifier = Modifier
                    .padding(top = 1.dp)
                    .size(22.dp)
                    .background(badgeBg, Square)
                    .border(BorderStroke(1.dp, badgeBorder), Square),
                contentAlignment = Alignment.Center,
            ) {
                when {
                    selected -> Icon(
                        Icons.Outlined.Check,
                        contentDescription = null,
                        tint = badgeFg,
                        modifier = Modifier.size(13.dp),
                    )
                    option.isCustom -> Icon(
                        Icons.Outlined.Edit,
                        contentDescription = null,
                        tint = badgeFg,
                        modifier = Modifier.size(12.dp),
                    )
                    else -> Text(
                        option.indexLabel.orEmpty(),
                        style = MaterialTheme.typography.labelSmall,
                        color = badgeFg,
                        fontFamily = MonoFamily,
                        fontWeight = FontWeight.SemiBold,
                    )
                }
            }
        }
        Column(
            modifier = Modifier.weight(1f),
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Text(
                text = option.headline,
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = FontWeight.Medium,
                color = when {
                    option.isReject -> MuxRed
                    option.isCustom -> MuxFg1
                    else -> MuxFg0
                },
            )
            option.description?.takeIf { it.isNotBlank() }?.let { desc ->
                Text(
                    text = desc,
                    style = MaterialTheme.typography.bodySmall,
                    color = MuxFg2,
                )
            }
        }
        if (interactive && !selected) {
            Icon(
                Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                contentDescription = null,
                tint = MuxFg2,
                modifier = Modifier.padding(top = 2.dp).size(16.dp),
            )
        }
    }
}

@Composable
private fun SessionDockStrip(
    goal: SessionGoal?,
    tasks: List<SessionTask>,
    subagents: List<SessionSubagent>,
    backgroundJobs: List<BackgroundJob>,
) {
    val hasContent = goal != null || tasks.isNotEmpty() || subagents.isNotEmpty() || backgroundJobs.isNotEmpty()
    if (!hasContent) return

    var expanded by remember { mutableStateOf(false) }
    val doneTasks = tasks.count { it.status == "completed" }
    val runningSubagents = subagents.count { it.status == "running" || it.status == "queued" }
    val runningJobs = backgroundJobs.count { it.status == "running" }

    val summary = buildList {
        if (goal != null) {
            val rounds = if (goal.maxRounds > 0) " ${goal.rounds}/${goal.maxRounds}" else ""
            add("目标: ${goal.objective.take(24)}$rounds")
        }
        if (tasks.isNotEmpty()) add("任务 $doneTasks/${tasks.size}")
        if (subagents.isNotEmpty()) {
            add(if (runningSubagents > 0) "子代理 $runningSubagents 运行中" else "子代理 ${subagents.size}")
        }
        if (backgroundJobs.isNotEmpty()) {
            add(if (runningJobs > 0) "后台 $runningJobs 运行中" else "后台 ${backgroundJobs.size}")
        }
    }.joinToString(" · ")

    Column(Modifier.fillMaxWidth().background(MuxBg0)) {
        HorizontalDivider(color = MuxLine)
        Row(
            Modifier
                .fillMaxWidth()
                .clickable { expanded = !expanded }
                .padding(horizontal = 14.dp, vertical = 7.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Icon(
                if (expanded) Icons.Outlined.KeyboardArrowDown else Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                contentDescription = null,
                tint = MuxCobalt,
                modifier = Modifier.size(16.dp),
            )
            Text(
                summary,
                style = MaterialTheme.typography.labelMedium,
                color = MuxFg1,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f),
            )
            if (runningSubagents > 0 || runningJobs > 0) {
                SquareStepIndicator(color = MuxCobalt, blockSize = 4.dp, gap = 2.dp, blockCount = 3)
            }
        }
        if (expanded) {
            HorizontalDivider(color = MuxLine)
            Column(
                Modifier
                    .fillMaxWidth()
                    .background(MuxBg1)
                    .padding(horizontal = 14.dp, vertical = 9.dp),
                verticalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                goal?.let { g ->
                    val rounds = if (g.maxRounds > 0) " (${g.rounds}/${g.maxRounds})" else ""
                    Text(
                        "目标 [${g.status}]$rounds · ${g.objective}",
                        style = MaterialTheme.typography.bodySmall,
                        color = MuxFg0,
                    )
                }
                tasks.take(12).forEach { t ->
                    val (mark, color) = when (t.status) {
                        "completed" -> "[x]" to MuxGreen
                        "in_progress" -> "[▸]" to MuxCobalt
                        else -> "[ ]" to MuxFg2
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        Text(mark, style = MaterialTheme.typography.labelSmall, color = color, fontFamily = MonoFamily)
                        Text(
                            t.activeForm?.takeIf { t.status == "in_progress" && it.isNotBlank() } ?: t.subject,
                            style = MaterialTheme.typography.bodySmall,
                            color = MuxFg1,
                            maxLines = 2,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
                subagents.take(8).forEach { s ->
                    val color = when (s.status) {
                        "completed" -> MuxGreen
                        "error" -> MuxRed
                        "running", "queued" -> MuxCobalt
                        else -> MuxFg2
                    }
                    Text(
                        "子代理 ${s.agent} [${s.status}] · ${s.description}",
                        style = MaterialTheme.typography.bodySmall,
                        color = color,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
                backgroundJobs.take(8).forEach { j ->
                    val color = when (j.status) {
                        "completed" -> MuxGreen
                        "failed" -> MuxRed
                        "running" -> MuxCobalt
                        else -> MuxFg2
                    }
                    Text(
                        "后台 ${j.name} [${j.status}] · ${j.description}",
                        style = MaterialTheme.typography.bodySmall,
                        color = color,
                        maxLines = 2,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
        }
    }
}

@Composable
private fun QueuedMessagesStrip(
    messages: List<QueuedMessage>,
    onDelete: (String) -> Unit,
) {
    Column(Modifier.fillMaxWidth().background(MuxBg0)) {
        HorizontalDivider(color = MuxLine)
        Column(
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 14.dp, vertical = 7.dp),
            verticalArrangement = Arrangement.spacedBy(5.dp),
        ) {
            Text(
                "待发送队列 (${messages.size})",
                style = MaterialTheme.typography.labelSmall,
                color = MuxFg2,
            )
            messages.forEach { msg ->
                val isSteer = msg.behavior == StreamingBehavior.STEER
                Row(
                    Modifier
                        .fillMaxWidth()
                        .background(MuxBg1, Square)
                        .border(BorderStroke(1.dp, MuxLine), Square)
                        .padding(horizontal = 10.dp, vertical = 6.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        if (isSteer) "插队" else "排队",
                        style = MaterialTheme.typography.labelSmall,
                        color = if (isSteer) MuxCobalt else MuxFg2,
                    )
                    Text(
                        msg.text,
                        style = MaterialTheme.typography.bodySmall,
                        color = MuxFg0,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    Box(
                        Modifier
                            .size(24.dp)
                            .clickable { onDelete(msg.id) },
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(
                            Icons.Outlined.Close,
                            contentDescription = "删除排队消息",
                            tint = MuxFg2,
                            modifier = Modifier.size(15.dp),
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun SlashQuickMenu(
    commands: List<ChatSlashCommand>,
    onPick: (ChatSlashCommand) -> Unit,
) {
    Column(Modifier.fillMaxWidth().background(MuxBg0)) {
        HorizontalDivider(color = MuxLine)
        Column(
            Modifier
                .fillMaxWidth()
                .padding(horizontal = 12.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(5.dp),
        ) {
            commands.take(8).forEach { cmd ->
                Row(
                    Modifier
                        .fillMaxWidth()
                        .background(MuxBg1, Square)
                        .border(BorderStroke(1.dp, MuxLine), Square)
                        .clickable { onPick(cmd) }
                        .padding(horizontal = 10.dp, vertical = 7.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Text(
                        "/${cmd.name}",
                        style = MaterialTheme.typography.labelMedium,
                        color = MuxFg0,
                        fontFamily = MonoFamily,
                    )
                    val desc = cmd.description
                    if (!desc.isNullOrBlank()) {
                        Text(
                            desc,
                            style = MaterialTheme.typography.bodySmall,
                            color = MuxFg2,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f),
                        )
                    } else {
                        Spacer(Modifier.weight(1f))
                    }
                    Text(
                        cmd.kind.uppercase(),
                        style = MaterialTheme.typography.labelSmall,
                        color = if (cmd.kind == "skill") MuxCobalt else MuxFg2,
                        fontFamily = MonoFamily,
                    )
                }
            }
        }
    }
}

@Composable
private fun ChatComposerBar(
    draft: String,
    onDraftChange: (String) -> Unit,
    inputEnabled: Boolean,
    isStreaming: Boolean,
    steerWhenStreaming: Boolean,
    onToggleSteerMode: () -> Unit,
    contextUsed: Long?,
    contextWindow: Long?,
    costUsd: Double?,
    stats: SessionStats?,
    slashOpen: Boolean,
    onToggleSlash: () -> Unit,
    onSend: (Boolean) -> Unit,
    onAbort: () -> Unit,
) {
    Surface(color = MuxBg1, contentColor = MuxFg0) {
        Column(Modifier.fillMaxWidth()) {
            HorizontalDivider(color = MuxLine)
            val hasFooterInfo = contextUsed != null || costUsd != null || (stats != null && stats.turns > 0) || isStreaming
            if (hasFooterInfo) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 14.dp, vertical = 5.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    val leftText = buildList {
                        when {
                            contextUsed != null && contextWindow != null && contextWindow > 0 ->
                                add("CTX ${formatCompactTokens(contextUsed)}/${formatCompactTokens(contextWindow)}")
                            contextUsed != null -> add("CTX ${formatCompactTokens(contextUsed)}")
                        }
                        if (stats != null && stats.turns > 0) {
                            add("${stats.turns}T")
                        }
                        costUsd?.takeIf { it > 0.0 }?.let {
                            add("$${"%.4f".format(it)}")
                        }
                    }.joinToString(" · ")
                    Text(
                        leftText,
                        style = MaterialTheme.typography.labelSmall,
                        color = MuxFg2,
                        fontFamily = MonoFamily,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    if (isStreaming && inputEnabled) {
                        Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                            Box(
                                Modifier
                                    .background(if (steerWhenStreaming) MuxAccentBg else MuxBg2, Square)
                                    .border(BorderStroke(1.dp, if (steerWhenStreaming) MuxCobalt else MuxControlLine), Square)
                                    .clickable(onClick = onToggleSteerMode)
                                    .padding(horizontal = 7.dp, vertical = 2.dp),
                            ) {
                                Text(
                                    if (steerWhenStreaming) "插队" else "排队",
                                    style = MaterialTheme.typography.labelSmall,
                                    color = if (steerWhenStreaming) MuxCobalt else MuxFg1,
                                )
                            }
                            if (draft.isNotBlank()) {
                                Box(
                                    Modifier
                                        .background(MuxErrorBg, Square)
                                        .border(BorderStroke(1.dp, MuxErrorLine), Square)
                                        .clickable(onClick = onAbort)
                                        .padding(horizontal = 7.dp, vertical = 2.dp),
                                ) {
                                    Text(
                                        stringResource(R.string.chat_abort),
                                        style = MaterialTheme.typography.labelSmall,
                                        color = MuxRed,
                                    )
                                }
                            }
                        }
                    }
                }
            }
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 12.dp, vertical = 10.dp),
                verticalAlignment = Alignment.Bottom,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Box(
                    Modifier
                        .size(40.dp)
                        .background(if (slashOpen) MuxAccent else MuxBg0, Square)
                        .border(BorderStroke(1.dp, if (slashOpen) MuxAccent else MuxControlLine), Square)
                        .clickable(enabled = inputEnabled, onClick = onToggleSlash),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        "/",
                        style = MaterialTheme.typography.titleMedium,
                        color = if (slashOpen) MuxOnAccent else if (inputEnabled) MuxFg0 else MuxFg2,
                        fontFamily = MonoFamily,
                    )
                }
                BasicTextField(
                    value = draft,
                    onValueChange = onDraftChange,
                    enabled = inputEnabled,
                    maxLines = 5,
                    textStyle = MaterialTheme.typography.bodyMedium.copy(color = if (inputEnabled) MuxFg0 else MuxFg2),
                    cursorBrush = SolidColor(MuxCobalt),
                    modifier = Modifier
                        .weight(1f)
                        .background(MuxBg0, Square)
                        .border(BorderStroke(1.dp, MuxControlLine), Square)
                        .padding(horizontal = 11.dp, vertical = 9.dp),
                    decorationBox = { inner ->
                        if (draft.isEmpty()) {
                            Text(
                                if (inputEnabled) stringResource(R.string.chat_input_hint) else "当前会话只读",
                                style = MaterialTheme.typography.bodyMedium,
                                color = MuxFg2,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        inner()
                    },
                )
                if (isStreaming && draft.isBlank() && inputEnabled) {
                    OutlinedButton(
                        onClick = onAbort,
                        shape = Square,
                        border = BorderStroke(1.dp, MuxRed),
                        contentPadding = PaddingValues(horizontal = 13.dp, vertical = 9.dp),
                        modifier = Modifier.height(40.dp),
                    ) { Text(stringResource(R.string.chat_abort), color = MuxRed, style = MaterialTheme.typography.labelMedium) }
                } else {
                    Button(
                        onClick = { onSend(if (isStreaming) steerWhenStreaming else false) },
                        enabled = inputEnabled && draft.isNotBlank(),
                        shape = Square,
                        colors = ButtonDefaults.buttonColors(
                            containerColor = MuxAccent,
                            disabledContainerColor = MuxBg2,
                            disabledContentColor = MuxFg2,
                        ),
                        contentPadding = PaddingValues(horizontal = 15.dp, vertical = 9.dp),
                        modifier = Modifier.height(40.dp),
                    ) {
                        Text(
                            when {
                                !isStreaming -> stringResource(R.string.chat_send)
                                steerWhenStreaming -> stringResource(R.string.chat_steer)
                                else -> "排队"
                            },
                            style = MaterialTheme.typography.labelMedium,
                        )
                    }
                }
            }
        }
    }
}

internal fun filterMobileSlashCommands(
    available: List<ChatSlashCommand>,
    query: String,
): List<ChatSlashCommand> {
    val builtins = buildList {
        if (available.none { it.name == "clear" }) {
            add(ChatSlashCommand(name = "clear", description = "清空当前上下文", kind = "command"))
        }
        if (available.none { it.name == "compact" }) {
            add(ChatSlashCommand(name = "compact", description = "压缩对话上下文", kind = "command"))
        }
        if (available.none { it.name == "abort" }) {
            add(ChatSlashCommand(name = "abort", description = "中止当前生成", kind = "builtin"))
        }
    }
    val combined = available + builtins
    val q = query.trim().lowercase()
    if (q.isEmpty()) return combined
    return combined
        .filter {
            it.name.lowercase().contains(q) ||
                it.description.orEmpty().lowercase().contains(q)
        }
        .sortedBy {
            when {
                it.name.lowercase() == q -> 0
                it.name.lowercase().startsWith(q) -> 1
                else -> 2
            }
        }
}

internal fun formatCompactTokens(tokens: Long): String = when {
    tokens >= 1_000_000 -> "${"%.1f".format(tokens / 1_000_000.0)}M"
    tokens >= 1_000 -> "${"%.1f".format(tokens / 1_000.0)}k"
    else -> tokens.toString()
}

internal fun formatMessageUsage(usage: MessageUsage): String {
    val parts = ArrayList<String>(4)
    val total = if (usage.totalTokens > 0) usage.totalTokens else usage.inputTokens + usage.outputTokens
    if (total > 0) {
        parts.add("${formatCompactTokens(usage.inputTokens)}↑ ${formatCompactTokens(usage.outputTokens)}↓")
    }
    usage.durationMs?.takeIf { it > 0 }?.let { ms ->
        parts.add(if (ms >= 1000) "${"%.1f".format(ms / 1000.0)}s" else "${ms}ms")
    }
    usage.costUsd?.takeIf { it > 0.0 }?.let { cost ->
        parts.add("$${"%.4f".format(cost)}")
    }
    return parts.joinToString(" · ")
}

private fun toolSummary(entry: TimelineEntry.ToolCall): String {
    entry.title?.takeIf { it.isNotBlank() }?.let { return it }
    val obj = entry.input as? JsonObject
    if (obj != null) {
        listOf(
            "CommandLine",
            "AbsolutePath",
            "TargetFile",
            "Query",
            "SearchPath",
            "path",
            "file_path",
            "filePath",
            "file",
            "command",
            "cmd",
            "pattern",
            "query",
            "url",
            "description",
        ).forEach { key ->
            val v = (obj[key] as? JsonPrimitive)?.content
            if (!v.isNullOrBlank()) return v
        }
        val questions = obj["questions"] as? JsonArray
        if (questions != null) {
            for (item in questions) {
                val q = ((item as? JsonObject)?.get("question") as? JsonPrimitive)?.content?.trim()
                if (!q.isNullOrBlank()) return q
            }
        }
    }
    return entry.locations.firstOrNull().orEmpty()
}

internal data class ParsedPromptOption(
    val optionId: String,
    val indexLabel: String?,
    val headline: String,
    val description: String?,
    val isCustom: Boolean,
    val isReject: Boolean,
)

internal data class ParsedPromptPreview(
    val title: String,
    val body: String,
)

internal data class ParsedApprovalPrompt(
    val isQuestion: Boolean,
    val isInputOnly: Boolean,
    val isMultiSelectInput: Boolean,
    val useListLayout: Boolean,
    val badge: String,
    val questionTitle: String,
    val hint: String?,
    val previews: List<ParsedPromptPreview>,
    val options: List<ParsedPromptOption>,
    val embeddedOptions: List<ParsedPromptOption>,
)

private val customSentinelRegex = Regex("^(type something\\.?|other|自定义输入|输入其他.*)$", RegexOption.IGNORE_CASE)
private val leadingIndexRegex = Regex("^(\\d+)[.)、]\\s+(.+)$", setOf(RegexOption.DOT_MATCHES_ALL))
private val headerTagRegex = Regex("^\\[([^\\[\\]\\n]{1,32})]\\s*(.*)$")
private val previewHeaderRegex = Regex("^---\\s*(.+?)\\s*---$")
private val multiSelectHintRegex = Regex("(comma-separated|all that apply|numbers|多选)", RegexOption.IGNORE_CASE)
private val typeYourAnswerRegex = Regex("^type your answer:?$", RegexOption.IGNORE_CASE)

private fun splitHeadlineAndDescription(body: String): Pair<String, String?> {
    for (sep in listOf(" — ", " – ")) {
        val idx = body.indexOf(sep)
        if (idx > 0) {
            val left = body.substring(0, idx).trim()
            val right = body.substring(idx + sep.length).trim()
            if (left.isNotEmpty() && right.isNotEmpty()) {
                return left to right
            }
        }
    }
    val dashIdx = body.indexOf(" - ")
    if (dashIdx in 1..48) {
        val left = body.substring(0, dashIdx).trim()
        val right = body.substring(dashIdx + 3).trim()
        if (left.isNotEmpty() && right.isNotEmpty()) {
            return left to right
        }
    }
    return body to null
}

internal fun parsePromptOption(
    option: ApprovalOption,
    fallbackIndex: Int,
    showFallbackIndex: Boolean,
): ParsedPromptOption {
    val rawLabel = option.label.replace("\r\n", "\n").trim()
    val idxMatch = leadingIndexRegex.matchEntire(rawLabel)
    val indexLabel = idxMatch?.groupValues?.get(1)
        ?: if (showFallbackIndex) (fallbackIndex + 1).toString() else null
    val body = (idxMatch?.groupValues?.get(2) ?: rawLabel).trim()
    val isCustom = customSentinelRegex.matches(body)
    val kindLower = option.kind.orEmpty().lowercase()
    val idLower = option.optionId.lowercase()
    val isReject = kindLower.startsWith("reject") ||
        "deny" in kindLower ||
        idLower == "deny" ||
        idLower == "reject" ||
        "reject_" in idLower ||
        "deny_" in idLower

    if (isCustom) {
        return ParsedPromptOption(
            optionId = option.optionId,
            indexLabel = indexLabel,
            headline = "自定义输入",
            description = "填写其他回答",
            isCustom = true,
            isReject = false,
        )
    }

    val (headline, description) = splitHeadlineAndDescription(body)
    return ParsedPromptOption(
        optionId = option.optionId,
        indexLabel = indexLabel,
        headline = headline.ifBlank { rawLabel },
        description = description,
        isCustom = false,
        isReject = isReject,
    )
}

internal fun parseApprovalPrompt(approval: PendingApproval): ParsedApprovalPrompt {
    val method = approval.method.lowercase()
    val isInputOnly = method == "input" || method == "editor"
    val isQuestion = method == "select" || isInputOnly

    val normalized = approval.title.replace("\r\n", "\n").trim()
    val blocks = normalized
        .split(Regex("\\n{2,}"))
        .map { it.trim() }
        .filter { it.isNotEmpty() }

    var badge = if (isQuestion) {
        if (isInputOnly) "等待输入" else "单选提问"
    } else {
        "需要确认"
    }
    val questionLines = ArrayList<String>()
    val previews = ArrayList<ParsedPromptPreview>()
    val embeddedOptions = ArrayList<ParsedPromptOption>()
    val hintLines = ArrayList<String>()

    blocks.forEachIndexed { blockIndex, block ->
        val lines = block.split("\n")
        val firstTrimmed = lines.firstOrNull()?.trim().orEmpty()
        val previewMatch = previewHeaderRegex.matchEntire(firstTrimmed)
        if (previewMatch != null) {
            previews.add(
                ParsedPromptPreview(
                    title = previewMatch.groupValues[1].trim(),
                    body = lines.drop(1).joinToString("\n").trim(),
                ),
            )
            return@forEachIndexed
        }

        if (blockIndex == 0) {
            val tagMatch = headerTagRegex.matchEntire(firstTrimmed)
            if (tagMatch != null && tagMatch.groupValues[1].trim().isNotEmpty()) {
                badge = tagMatch.groupValues[1].trim()
                val restFirst = tagMatch.groupValues[2].trim()
                if (restFirst.isNotEmpty()) questionLines.add(restFirst)
                lines.drop(1).map { it.trim() }.filter { it.isNotEmpty() }.forEach { questionLines.add(it) }
            } else {
                lines.map { it.trim() }.filter { it.isNotEmpty() }.forEach { questionLines.add(it) }
            }
            return@forEachIndexed
        }

        val nonEmptyLines = lines.map { it.trim() }.filter { it.isNotEmpty() }
        val allNumbered = nonEmptyLines.isNotEmpty() && nonEmptyLines.all { leadingIndexRegex.matches(it) }
        if (allNumbered && isInputOnly) {
            nonEmptyLines.forEachIndexed { i, line ->
                embeddedOptions.add(
                    parsePromptOption(
                        ApprovalOption(optionId = line, label = line, kind = "select"),
                        i,
                        true,
                    ),
                )
            }
            return@forEachIndexed
        }

        if (isInputOnly && blockIndex == blocks.lastIndex) {
            hintLines.add(nonEmptyLines.joinToString("\n"))
        } else {
            questionLines.add(nonEmptyLines.joinToString("\n"))
        }
    }

    val rawHint = hintLines.joinToString("\n").trim().takeIf { it.isNotEmpty() }
    val isMultiSelectInput = isInputOnly &&
        embeddedOptions.isNotEmpty() &&
        rawHint != null &&
        multiSelectHintRegex.containsMatchIn(rawHint)

    val hint = when {
        isMultiSelectInput -> {
            if (badge == "等待输入") badge = "多选提问"
            "可直接点选上方选项，或输入序号（如 1,3）与自定义内容"
        }
        rawHint != null && typeYourAnswerRegex.matches(rawHint) -> "请输入你的自定义回答"
        else -> rawHint
    }

    val parsedOptions = approval.options.mapIndexed { idx, opt ->
        parsePromptOption(opt, idx, method == "select")
    }
    val useListLayout = method == "select" ||
        parsedOptions.size > 2 ||
        parsedOptions.any { !it.description.isNullOrBlank() || it.headline.length > 18 }

    return ParsedApprovalPrompt(
        isQuestion = isQuestion,
        isInputOnly = isInputOnly,
        isMultiSelectInput = isMultiSelectInput,
        useListLayout = useListLayout,
        badge = badge,
        questionTitle = questionLines.joinToString("\n").trim().ifBlank { normalized.ifBlank { "等待回复" } },
        hint = hint,
        previews = previews,
        options = parsedOptions,
        embeddedOptions = embeddedOptions,
    )
}

internal fun toggleMultiSelectNumber(current: String, indexLabel: String): String {
    val trimmed = current.trim()
    if (trimmed.isEmpty()) return indexLabel
    val tokens = trimmed.split(Regex("[,\\s]+")).filter { it.isNotEmpty() }
    if (!tokens.all { tok -> tok.all { it.isDigit() } }) {
        return indexLabel
    }
    val next = if (indexLabel in tokens) {
        tokens.filterNot { it == indexLabel }
    } else {
        tokens + indexLabel
    }
    return next.sortedBy { it.toIntOrNull() ?: Int.MAX_VALUE }.joinToString(",")
}

