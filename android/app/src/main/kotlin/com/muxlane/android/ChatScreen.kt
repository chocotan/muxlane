package com.muxlane.android

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.BorderStroke
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
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.ContentCopy
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.outlined.Lock
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
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
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.launch
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import muxlane.chat.ApprovalReply
import muxlane.chat.DiffOp
import muxlane.chat.NoticeLevel
import muxlane.chat.PendingApproval
import muxlane.chat.PlanEntryStatus
import muxlane.chat.TimelineEntry
import muxlane.chat.ToolCallKind
import muxlane.chat.ToolCallState
import muxlane.chat.ToolDiff
import muxlane.chat.computeUnifiedDiffLines
import muxlane.protocol.Snapshot

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
    tool.kind == ToolCallKind.READ ||
        tool.kind == ToolCallKind.SEARCH ||
        tool.kind == ToolCallKind.FETCH ||
        tool.kind == ToolCallKind.THINK

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
internal fun SessionModeSwitcher(
    mode: SessionPaneMode,
    canSwitchToChat: Boolean,
    canSwitchToTerm: Boolean,
    onSelect: (SessionPaneMode) -> Unit,
) {
    if (!canSwitchToChat) return
    Row(
        Modifier
            .border(BorderStroke(1.dp, MuxControlLine), Square)
            .background(MuxBg2, Square),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        ModeSegmentButton(
            label = stringResource(R.string.chat_tab),
            selected = mode == SessionPaneMode.CHAT,
            enabled = true,
            onClick = { onSelect(SessionPaneMode.CHAT) },
        )
        if (canSwitchToTerm) {
            Box(Modifier.width(1.dp).height(18.dp).background(MuxControlLine))
            ModeSegmentButton(
                label = stringResource(R.string.terminal_tab),
                selected = mode == SessionPaneMode.TERMINAL,
                enabled = true,
                onClick = { onSelect(SessionPaneMode.TERMINAL) },
            )
        }
    }
}

@Composable
private fun ModeSegmentButton(
    label: String,
    selected: Boolean,
    enabled: Boolean,
    onClick: () -> Unit,
) {
    Box(
        Modifier
            .background(if (selected) MuxAccent else Color.Transparent, Square)
            .clickable(enabled = enabled, onClick = onClick)
            .padding(horizontal = 8.dp, vertical = 4.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(
            label,
            style = MaterialTheme.typography.labelMedium,
            color = if (selected) MuxOnAccent else MuxFg1,
        )
    }
}

@Composable
internal fun ChatTimelinePane(
    model: MuxlaneViewModel,
    agentId: String,
    state: UiState,
    canSwitchToTerm: Boolean,
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
    var draft by remember(agentId) { mutableStateOf("") }

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
                    is TimelineEntry.ToolCall -> (e.output?.length ?: 0) + e.state.hashCode()
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
        if (meta != null && meta.driver == muxlane.chat.SessionDriverMode.TERMINAL_PTY) {
            val isLive = meta.supportsChatInput
            Surface(
                color = if (isLive) MuxBg1 else MuxWarningBg,
                shape = Square,
                border = BorderStroke(1.dp, if (isLive) MuxLine else MuxWarningLine),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 12.dp, vertical = 6.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        if (isLive) "PTY 双向同步 · 与终端实时共享" else "终端已退回 Shell · 输入已锁定（只读）",
                        style = MaterialTheme.typography.labelSmall,
                        color = if (isLive) MuxAccent else MuxWarningFg,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    if (canSwitchToTerm) {
                        Box(
                            Modifier
                                .background(MuxBg2, Square)
                                .border(BorderStroke(1.dp, MuxControlLine), Square)
                                .clickable { model.setSessionMode(SessionPaneMode.TERMINAL) }
                                .padding(horizontal = 8.dp, vertical = 3.dp),
                        ) {
                            Text(
                                "查看终端",
                                style = MaterialTheme.typography.labelSmall,
                                color = MuxFg0,
                            )
                        }
                    }
                }
            }
        }

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
                                verticalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                if (!state.connected) SquareStepIndicator()
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
                                contentPadding = PaddingValues(horizontal = 12.dp, vertical = 12.dp),
                                verticalArrangement = Arrangement.spacedBy(8.dp),
                            ) {
                                items(items, key = { it.key }) { item ->
                                    when (item) {
                                        is MobileTimelineItem.ExplorationCluster -> {
                                            val isExpanded = expandedClusters[item.id] == true
                                            ExplorationClusterCard(
                                                tools = item.tools,
                                                expanded = isExpanded,
                                                onToggle = { expandedClusters[item.id] = !isExpanded },
                                            )
                                        }
                                        is MobileTimelineItem.Single -> when (val entry = item.entry) {
                                            is TimelineEntry.UserMessage -> UserMessageCard(entry)
                                            is TimelineEntry.Thinking -> {
                                                val isExpanded = expandedThinking[entry.id] ?: entry.streaming
                                                ThinkingCard(
                                                    entry = entry,
                                                    expanded = isExpanded,
                                                    onToggle = { expandedThinking[entry.id] = !isExpanded },
                                                )
                                            }
                                            is TimelineEntry.AssistantText -> AssistantMessageCard(entry)
                                            is TimelineEntry.ToolCall -> ToolCallCard(entry)
                                            is TimelineEntry.Plan -> PlanCard(entry)
                                            is TimelineEntry.Notice -> NoticeCard(entry)
                                        }
                                    }
                                }
                            }
                            if (!atBottom) {
                                Surface(
                                    modifier = Modifier
                                        .align(Alignment.BottomEnd)
                                        .padding(14.dp)
                                        .clickable {
                                            scope.launch { listState.animateScrollToItem(items.lastIndex) }
                                        },
                                    shape = Square,
                                    color = MuxBg1,
                                    border = BorderStroke(1.dp, MuxAccent),
                                ) {
                                    Text(
                                        "↓ 最新",
                                        style = MaterialTheme.typography.labelMedium,
                                        color = MuxAccent,
                                        modifier = Modifier.padding(horizontal = 10.dp, vertical = 6.dp),
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
                .background(MuxBg0)
                .imePadding()
                .navigationBarsPadding(),
        ) {
            pendingApprovals.firstOrNull()?.let { approval ->
                InlineApprovalBanner(
                    approval = approval,
                    onReply = { reply -> model.respondApproval(agentId, approval.id, reply) },
                )
            }
            if (showSlashMenu) {
                SlashQuickMenu(
                    onPick = { cmd ->
                        onDismissSlash()
                        when (cmd) {
                            "/abort" -> model.abortChat()
                            "/terminal" -> if (canSwitchToTerm) model.setSessionMode(SessionPaneMode.TERMINAL)
                            else -> model.sendChatPrompt(cmd, steer = false)
                        }
                    },
                )
            }
            ChatComposerBar(
                draft = draft,
                onDraftChange = { draft = it },
                inputEnabled = meta?.supportsChatInput != false,
                isStreaming = meta?.isStreaming == true,
                contextUsed = meta?.contextUsedTokens,
                contextWindow = meta?.contextWindowTokens,
                costUsd = meta?.totalCostUsd,
                slashOpen = showSlashMenu,
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
}

@Composable
private fun ChatModelConfigStrip(
    meta: muxlane.chat.ChatSessionMeta,
    onConfigureModel: (String?, String) -> Unit,
    onConfigureThinking: (String) -> Unit,
) {
    var modelMenuOpen by remember { mutableStateOf(false) }
    var thinkingMenuOpen by remember { mutableStateOf(false) }
    Surface(color = MuxBg1, contentColor = MuxFg0) {
        Column(Modifier.fillMaxWidth()) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .horizontalScroll(rememberScrollState())
                    .padding(horizontal = 12.dp, vertical = 6.dp),
                horizontalArrangement = Arrangement.spacedBy(6.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                val modelLabel = meta.currentModel?.name?.takeIf { it.isNotBlank() }
                    ?: meta.currentModel?.id
                    ?: stringResource(R.string.model_picker)
                Box {
                    Surface(
                        modifier = Modifier.clickable(enabled = meta.availableModels.isNotEmpty()) { modelMenuOpen = true },
                        shape = Square,
                        color = MuxBg2,
                        border = BorderStroke(1.dp, MuxLine),
                    ) {
                        Text(
                            modelLabel,
                            style = MaterialTheme.typography.labelSmall,
                            color = MuxFg1,
                            fontFamily = MonoFamily,
                            modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                        )
                    }
                    DropdownMenu(expanded = modelMenuOpen, onDismissRequest = { modelMenuOpen = false }) {
                        meta.availableModels.forEach { m ->
                            DropdownMenuItem(
                                text = { Text("${m.name.ifBlank { m.id }} (${m.provider})", fontFamily = MonoFamily) },
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
                        Surface(
                            modifier = Modifier.clickable(enabled = meta.availableThinkingLevels.isNotEmpty()) { thinkingMenuOpen = true },
                            shape = Square,
                            color = MuxBg2,
                            border = BorderStroke(1.dp, MuxLine),
                        ) {
                            Text(
                                "${stringResource(R.string.thinking_picker)}: ${meta.thinkingLevel ?: "default"}",
                                style = MaterialTheme.typography.labelSmall,
                                color = MuxFg1,
                                fontFamily = MonoFamily,
                                modifier = Modifier.padding(horizontal = 8.dp, vertical = 4.dp),
                            )
                        }
                        DropdownMenu(expanded = thinkingMenuOpen, onDismissRequest = { thinkingMenuOpen = false }) {
                            meta.availableThinkingLevels.forEach { level ->
                                DropdownMenuItem(
                                    text = { Text(level, fontFamily = MonoFamily) },
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
private fun UserMessageCard(entry: TimelineEntry.UserMessage) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = Square,
        color = MuxBg1,
        border = BorderStroke(1.dp, MuxControlLine),
    ) {
        Row(Modifier.fillMaxWidth()) {
            Box(Modifier.width(3.dp).height(44.dp).background(MuxAccent))
            Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text("YOU", style = MaterialTheme.typography.labelSmall, color = MuxAccent, fontFamily = MonoFamily)
                Text(entry.text, style = MaterialTheme.typography.bodyMedium, color = MuxFg0)
                if (entry.attachments.isNotEmpty()) {
                    Text(
                        entry.attachments.joinToString(", ") { it.name },
                        style = MaterialTheme.typography.labelSmall,
                        color = MuxFg2,
                        fontFamily = MonoFamily,
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
            .padding(horizontal = 4.dp, vertical = 4.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
            Text(
                entry.model?.ifBlank { null } ?: "ASSISTANT",
                style = MaterialTheme.typography.labelSmall,
                color = MuxFg2,
                fontFamily = MonoFamily,
            )
            if (entry.streaming) SquareStepIndicator(blockSize = 4.dp, gap = 2.dp, blockCount = 3)
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
                        .background(MuxBg2, Square)
                        .padding(8.dp),
                ) {
                    Box(Modifier.width(2.dp).height(20.dp).background(MuxAccent))
                    Spacer(Modifier.width(8.dp))
                    Text(inlineMarkdown(block.text), style = MaterialTheme.typography.bodyMedium, color = MuxFg1)
                }
                is MarkdownBlock.BulletItem -> Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Text(block.marker, style = MaterialTheme.typography.bodyMedium, color = MuxAccent, fontFamily = MonoFamily)
                    Text(inlineMarkdown(block.text), style = MaterialTheme.typography.bodyMedium, color = MuxFg0, modifier = Modifier.weight(1f))
                }
                is MarkdownBlock.CodeBlock -> Surface(
                    modifier = Modifier.fillMaxWidth(),
                    shape = Square,
                    color = MuxBg2,
                    border = BorderStroke(1.dp, MuxLine),
                ) {
                    Column {
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .background(MuxBg1)
                                .padding(horizontal = 10.dp, vertical = 4.dp),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Text(
                                block.language.ifBlank { "code" },
                                style = MaterialTheme.typography.labelSmall,
                                color = MuxFg2,
                                fontFamily = MonoFamily,
                            )
                            Icon(
                                Icons.Outlined.ContentCopy,
                                contentDescription = "复制代码",
                                tint = MuxFg2,
                                modifier = Modifier
                                    .size(16.dp)
                                    .clickable {
                                        val cm = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                                        cm.setPrimaryClip(ClipData.newPlainText("code", block.code))
                                    },
                            )
                        }
                        HorizontalDivider(color = MuxLine)
                        Box(Modifier.horizontalScroll(rememberScrollState()).padding(10.dp)) {
                            Text(
                                block.code,
                                style = MaterialTheme.typography.bodySmall.copy(fontFamily = MonoFamily),
                                color = MuxFg0,
                            )
                        }
                    }
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
                withStyle(SpanStyle(fontFamily = MonoFamily, background = MuxBg2, color = MuxFg0)) {
                    append(tok.substring(1, tok.length - 1))
                }
            }
            tok.startsWith("**") && tok.endsWith("**") -> {
                withStyle(SpanStyle(fontWeight = FontWeight.Bold)) {
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
    Surface(
        modifier = Modifier.fillMaxWidth().clickable(onClick = onToggle),
        shape = Square,
        color = MuxBg1,
        border = BorderStroke(1.dp, MuxLine),
    ) {
        Column(Modifier.padding(10.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Icon(
                    if (expanded) Icons.Outlined.KeyboardArrowDown else Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                    null,
                    tint = MuxFg2,
                    modifier = Modifier.size(16.dp),
                )
                Text(
                    buildString {
                        append(stringResource(R.string.thinking_title))
                        entry.durationMs?.let { append(" · ${it}ms") }
                    },
                    style = MaterialTheme.typography.labelMedium,
                    color = MuxFg2,
                )
                Spacer(Modifier.weight(1f))
                if (entry.streaming) SquareStepIndicator(blockSize = 4.dp, gap = 2.dp, blockCount = 3)
            }
            if (expanded && entry.text.isNotBlank()) {
                Text(
                    entry.text,
                    style = MaterialTheme.typography.bodySmall,
                    color = MuxFg1,
                    fontFamily = MonoFamily,
                )
            }
        }
    }
}

@Composable
private fun ExplorationClusterCard(
    tools: List<TimelineEntry.ToolCall>,
    expanded: Boolean,
    onToggle: () -> Unit,
) {
    val active = tools.any { it.state == ToolCallState.PENDING || it.state == ToolCallState.RUNNING }
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = Square,
        color = MuxBg1,
        border = BorderStroke(1.dp, MuxLine),
    ) {
        Column(Modifier.padding(10.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(
                Modifier.fillMaxWidth().clickable(onClick = onToggle),
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
                    "已查看 ${tools.size} 个项目",
                    style = MaterialTheme.typography.labelMedium,
                    color = MuxFg1,
                )
                Spacer(Modifier.weight(1f))
                if (active) SquareStepIndicator(blockSize = 4.dp, gap = 2.dp, blockCount = 3)
            }
            if (expanded) {
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    tools.forEach { t ->
                        Text(
                            "• ${t.name} ${toolSummary(t)}",
                            style = MaterialTheme.typography.bodySmall,
                            color = MuxFg2,
                            fontFamily = MonoFamily,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }
            }
        }
    }
}

@Composable
private fun ToolCallCard(entry: TimelineEntry.ToolCall) {
    var outputExpanded by remember(entry.id) {
        mutableStateOf(entry.diff != null || entry.isError || entry.kind == ToolCallKind.BASH)
    }
    val statusColor = when {
        entry.isError || entry.state == ToolCallState.FAILED -> MuxRed
        entry.state == ToolCallState.COMPLETED -> MuxGreen
        else -> MuxYellow
    }
    val summary = toolSummary(entry)
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = Square,
        color = MuxBg1,
        border = BorderStroke(1.dp, if (entry.isError) MuxErrorLine else MuxLine),
    ) {
        Column(Modifier.padding(10.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .clickable { outputExpanded = !outputExpanded },
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Box(Modifier.size(8.dp).background(statusColor, Square))
                Text(
                    entry.name.uppercase(),
                    style = MaterialTheme.typography.labelMedium,
                    color = statusColor,
                    fontFamily = MonoFamily,
                )
                if (summary.isNotBlank()) {
                    Text(
                        summary,
                        style = MaterialTheme.typography.bodySmall,
                        color = MuxFg1,
                        fontFamily = MonoFamily,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                } else {
                    Spacer(Modifier.weight(1f))
                }
                entry.exitCode?.let { code ->
                    Text(
                        "exit $code",
                        style = MaterialTheme.typography.labelSmall,
                        color = if (code == 0) MuxGreen else MuxRed,
                        fontFamily = MonoFamily,
                    )
                }
                entry.durationMs?.let { ms ->
                    Text(
                        "${ms}ms",
                        style = MaterialTheme.typography.labelSmall,
                        color = MuxFg2,
                        fontFamily = MonoFamily,
                    )
                }
            }
            if (outputExpanded) {
                entry.diff?.let { diff ->
                    UnifiedDiffCard(diff)
                }
                entry.output?.takeIf { it.isNotBlank() }?.let { out ->
                    Box(
                        Modifier
                            .fillMaxWidth()
                            .background(MuxBg2, Square)
                            .border(BorderStroke(1.dp, MuxLine), Square)
                            .horizontalScroll(rememberScrollState())
                            .padding(8.dp),
                    ) {
                        Text(
                            out.take(4000),
                            style = MaterialTheme.typography.bodySmall.copy(fontFamily = MonoFamily),
                            color = if (entry.isError) MuxErrorFg else MuxFg1,
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
            .background(MuxBg2, Square),
    ) {
        diff.path?.let { path ->
            Text(
                path,
                style = MaterialTheme.typography.labelSmall,
                color = MuxFg1,
                fontFamily = MonoFamily,
                modifier = Modifier
                    .fillMaxWidth()
                    .background(MuxBg1)
                    .padding(horizontal = 8.dp, vertical = 4.dp),
            )
            HorizontalDivider(color = MuxLine)
        }
        Column(Modifier.horizontalScroll(rememberScrollState()).padding(vertical = 4.dp)) {
            lines.forEach { line ->
                val (bg, fg, prefix) = when (line.op) {
                    DiffOp.ADD -> Triple(Color(0xFFE6F4EA), Color(0xFF005C4B), "+")
                    DiffOp.DEL -> Triple(Color(0xFFFFEDED), Color(0xFF991B1B), "-")
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
                        style = MaterialTheme.typography.bodySmall.copy(fontFamily = MonoFamily, fontSize = 11.sp),
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
        Column(Modifier.padding(10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text("PLAN", style = MaterialTheme.typography.labelSmall, color = MuxAccent, fontFamily = MonoFamily)
            entry.entries.forEach { item ->
                val (marker, color) = when (item.status) {
                    PlanEntryStatus.COMPLETED -> "[x]" to MuxGreen
                    PlanEntryStatus.IN_PROGRESS -> "[▸]" to MuxAccent
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
            modifier = Modifier.padding(10.dp),
        )
    }
}

@Composable
private fun InlineApprovalBanner(
    approval: PendingApproval,
    onReply: (ApprovalReply) -> Unit,
) {
    var replyText by remember(approval.id) { mutableStateOf(approval.prefill.orEmpty()) }
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = Square,
        color = MuxWarningBg,
        border = BorderStroke(1.dp, MuxWarningLine),
    ) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween, verticalAlignment = Alignment.CenterVertically) {
                Text(approval.title, style = MaterialTheme.typography.titleMedium, color = MuxWarningFg)
                approval.toolName?.let {
                    Text(it, style = MaterialTheme.typography.labelSmall, color = MuxWarningFg, fontFamily = MonoFamily)
                }
            }
            approval.commandOrInput?.takeIf { it.isNotBlank() }?.let { cmd ->
                Box(
                    Modifier
                        .fillMaxWidth()
                        .background(MuxBg1, Square)
                        .border(BorderStroke(1.dp, MuxWarningLine), Square)
                        .padding(8.dp),
                ) {
                    Text(cmd, style = MaterialTheme.typography.bodySmall, color = MuxFg0, fontFamily = MonoFamily, maxLines = 4)
                }
            }
            approval.message?.takeIf { it.isNotBlank() }?.let { msg ->
                Text(msg, style = MaterialTheme.typography.bodySmall, color = MuxFg1)
            }
            if (approval.options.isNotEmpty()) {
                Row(
                    Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    approval.options.forEach { option ->
                        val isReject = "reject" in option.kind.orEmpty().lowercase() || "deny" in option.kind.orEmpty().lowercase()
                        if (isReject) {
                            OutlinedButton(
                                onClick = { onReply(ApprovalReply.SelectedOption(option.optionId)) },
                                shape = Square,
                                border = BorderStroke(1.dp, MuxRed),
                            ) { Text(option.label, color = MuxRed) }
                        } else {
                            Button(
                                onClick = { onReply(ApprovalReply.SelectedOption(option.optionId)) },
                                shape = Square,
                                colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                            ) { Text(option.label) }
                        }
                    }
                }
            } else {
                Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    Button(
                        onClick = { onReply(ApprovalReply.Confirm(true)) },
                        shape = Square,
                        colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                    ) { Text(stringResource(R.string.approve_allow)) }
                    OutlinedButton(
                        onClick = { onReply(ApprovalReply.Confirm(false)) },
                        shape = Square,
                        border = BorderStroke(1.dp, MuxRed),
                    ) { Text(stringResource(R.string.approve_deny), color = MuxRed) }
                }
            }
            if (approval.prefill != null || approval.options.isEmpty()) {
                Row(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalAlignment = Alignment.CenterVertically) {
                    BasicTextField(
                        value = replyText,
                        onValueChange = { replyText = it },
                        singleLine = true,
                        textStyle = MaterialTheme.typography.bodySmall.copy(color = MuxFg0, fontFamily = MonoFamily),
                        cursorBrush = SolidColor(MuxAccent),
                        modifier = Modifier
                            .weight(1f)
                            .background(MuxBg1, Square)
                            .border(BorderStroke(1.dp, MuxControlLine), Square)
                            .padding(horizontal = 8.dp, vertical = 8.dp),
                        decorationBox = { inner ->
                            if (replyText.isEmpty()) Text(stringResource(R.string.approve_reply_hint), style = MaterialTheme.typography.bodySmall, color = MuxFg2)
                            inner()
                        },
                    )
                    OutlinedButton(
                        onClick = { if (replyText.isNotBlank()) onReply(ApprovalReply.Text(replyText.trim())) },
                        shape = Square,
                        border = BorderStroke(1.dp, MuxControlLine),
                    ) { Text(stringResource(R.string.approve_reply), color = MuxFg0) }
                }
            }
        }
    }
}

@Composable
private fun SlashQuickMenu(onPick: (String) -> Unit) {
    Surface(
        modifier = Modifier.fillMaxWidth(),
        shape = Square,
        color = MuxBg1,
        border = BorderStroke(1.dp, MuxLine),
    ) {
        Row(
            Modifier
                .fillMaxWidth()
                .horizontalScroll(rememberScrollState())
                .padding(horizontal = 12.dp, vertical = 6.dp),
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            listOf("/clear", "/compact", "/terminal", "/abort").forEach { cmd ->
                Box(
                    Modifier
                        .background(MuxBg2, Square)
                        .border(BorderStroke(1.dp, MuxControlLine), Square)
                        .clickable { onPick(cmd) }
                        .padding(horizontal = 10.dp, vertical = 5.dp),
                ) {
                    Text(cmd, style = MaterialTheme.typography.labelMedium, color = MuxFg0, fontFamily = MonoFamily)
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
    contextUsed: Long?,
    contextWindow: Long?,
    costUsd: Double?,
    slashOpen: Boolean,
    onToggleSlash: () -> Unit,
    onSend: (Boolean) -> Unit,
    onAbort: () -> Unit,
) {
    Surface(color = MuxBg1, contentColor = MuxFg0) {
        Column(Modifier.fillMaxWidth()) {
            HorizontalDivider(color = MuxLine)
            if (contextUsed != null || costUsd != null) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 12.dp, vertical = 4.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    val tokText = when {
                        contextUsed != null && contextWindow != null && contextWindow > 0 ->
                            "CTX ${contextUsed / 1000}k / ${contextWindow / 1000}k"
                        contextUsed != null -> "CTX $contextUsed tok"
                        else -> ""
                    }
                    Text(tokText, style = MaterialTheme.typography.labelSmall, color = MuxFg2, fontFamily = MonoFamily)
                    costUsd?.let {
                        Text(
                            "$${"%.4f".format(it)}",
                            style = MaterialTheme.typography.labelSmall,
                            color = MuxFg2,
                            fontFamily = MonoFamily,
                        )
                    }
                }
            }
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 10.dp, vertical = 8.dp),
                verticalAlignment = Alignment.Bottom,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Box(
                    Modifier
                        .size(38.dp)
                        .background(if (slashOpen) MuxAccent else MuxBg2, Square)
                        .border(BorderStroke(1.dp, MuxControlLine), Square)
                        .clickable(enabled = inputEnabled, onClick = onToggleSlash),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        "/",
                        style = MaterialTheme.typography.titleMedium,
                        color = if (slashOpen) MuxOnAccent else if (inputEnabled) MuxFg1 else MuxFg2,
                        fontFamily = MonoFamily,
                    )
                }
                BasicTextField(
                    value = draft,
                    onValueChange = onDraftChange,
                    enabled = inputEnabled,
                    maxLines = 5,
                    textStyle = MaterialTheme.typography.bodyMedium.copy(color = if (inputEnabled) MuxFg0 else MuxFg2),
                    cursorBrush = SolidColor(MuxAccent),
                    modifier = Modifier
                        .weight(1f)
                        .background(MuxBg0, Square)
                        .border(BorderStroke(1.dp, MuxControlLine), Square)
                        .padding(horizontal = 10.dp, vertical = 9.dp),
                    decorationBox = { inner ->
                        if (draft.isEmpty()) {
                            Text(
                                if (inputEnabled) stringResource(R.string.chat_input_hint) else "终端已退回 Shell，输入已暂停",
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
                        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 8.dp),
                    ) { Text(stringResource(R.string.chat_abort), color = MuxRed) }
                } else {
                    Button(
                        onClick = { onSend(isStreaming) },
                        enabled = inputEnabled && draft.isNotBlank(),
                        shape = Square,
                        colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                        contentPadding = PaddingValues(horizontal = 14.dp, vertical = 8.dp),
                    ) {
                        Text(
                            stringResource(if (isStreaming) R.string.chat_steer else R.string.chat_send),
                        )
                    }
                }
            }
        }
    }
}

private fun toolSummary(entry: TimelineEntry.ToolCall): String {
    entry.title?.takeIf { it.isNotBlank() }?.let { return it }
    val obj = entry.input as? JsonObject
    if (obj != null) {
        listOf("path", "file_path", "file", "command", "cmd", "pattern", "query", "url").forEach { key ->
            val v = (obj[key] as? JsonPrimitive)?.content
            if (!v.isNullOrBlank()) return v
        }
    }
    return entry.locations.firstOrNull().orEmpty()
}
