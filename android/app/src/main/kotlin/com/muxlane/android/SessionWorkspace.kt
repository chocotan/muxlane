package com.muxlane.android

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectHorizontalDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.outlined.ViewSidebar
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material.icons.outlined.DeleteSweep
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material.icons.outlined.FormatSize
import androidx.compose.material.icons.outlined.Menu
import androidx.compose.material.icons.outlined.MoreVert
import androidx.compose.material.icons.outlined.NotificationsActive
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.Tab
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material.icons.outlined.Tune
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import muxlane.protocol.AgentPreset
import kotlin.math.roundToInt

@Composable
internal fun SessionWorkspace(
    model: MuxlaneViewModel,
    state: UiState,
    wideSplitMode: Boolean,
    sidebarVisible: Boolean,
    onToggleSidebar: (() -> Unit)?,
    onOpenNewTab: (String?) -> Unit,
    onOpenDiagnostics: () -> Unit,
) {
    val agentId = state.selectedAgent
    val tabSummaries = remember(state.openTabs, state.snapshot, state.chatSessions, state.globalApprovals) {
        buildSessionTabSummaries(
            openTabs = state.openTabs,
            snapshot = state.snapshot,
            chatSessions = state.chatSessions,
            globalApprovals = state.globalApprovals,
        )
    }

    var showTabsSheet by remember { mutableStateOf(false) }
    var showQuickDrawer by remember { mutableStateOf(false) }
    var showDetailsSheet by remember { mutableStateOf(false) }
    var showQuickCommandsSheet by remember { mutableStateOf(false) }
    var showTextSizeSheet by remember { mutableStateOf(false) }
    var showSlashMenu by remember(agentId) { mutableStateOf(false) }

    if (!wideSplitMode) {
        BackHandler(enabled = agentId != null) { model.closeAgent() }
    }

    if (agentId == null) {
        EmptySessionSplitPlaceholder(
            hasProjects = !state.snapshot?.projects.isNullOrEmpty(),
            connected = state.connected,
            openTabsCount = tabSummaries.size,
            onOpenFirstTab = { tabSummaries.lastOrNull()?.let { model.openAgent(it.agentId) } },
            onNewTab = { onOpenNewTab(state.snapshot?.projects?.firstOrNull()?.id) },
            onOpenDiagnostics = onOpenDiagnostics,
        )
        return
    }

    val agent = state.snapshot?.agents?.find { it.id == agentId }
    val project = state.snapshot?.projects?.find { it.id == agent?.project }
    val chatMeta = state.chatTimelines[agentId]?.meta ?: state.chatSessions[agentId]?.meta
    val canSwitchToChat = state.chatSessions.containsKey(agentId)
    val canSwitchToTerm = chatMeta == null || chatMeta.hasPty || chatMeta.supportsTerminalSwitch
    val activePaneMode = if (state.sessionMode == SessionPaneMode.CHAT && canSwitchToChat) {
        SessionPaneMode.CHAT
    } else {
        SessionPaneMode.TERMINAL
    }

    val otherApproval = state.globalApprovals.firstOrNull { it.agent != agentId }

    Column(Modifier.fillMaxSize().background(MuxBg0)) {
        SessionWorkspaceHeader(
            title = agent?.title ?: agentId,
            subtitle = buildString {
                append(project?.displayName() ?: agent?.project.orEmpty())
                if (!agent?.project.isNullOrBlank()) append(" · ")
                append(driverBadgeLabel(agent?.agentType ?: muxlane.protocol.AgentType.UNKNOWN, chatMeta?.driver))
            },
            isStreaming = chatMeta?.isStreaming == true,
            connected = state.connected,
            e2eeActive = state.e2eeActive,
            usingLan = state.usingLan,
            wideSplitMode = wideSplitMode,
            sidebarVisible = sidebarVisible,
            sessionMode = activePaneMode,
            canSwitchToChat = canSwitchToChat,
            canSwitchToTerm = canSwitchToTerm,
            onBack = model::closeAgent,
            onToggleSidebar = onToggleSidebar,
            onOpenQuickDrawer = { showQuickDrawer = true },
            onOpenDetails = { showDetailsSheet = true },
            onSelectMode = model::setSessionMode,
            onOpenTabsSheet = { showTabsSheet = true },
            onOpenNewTab = { onOpenNewTab(agent?.project ?: state.snapshot?.projects?.firstOrNull()?.id) },
            onOpenQuickCommands = { showQuickCommandsSheet = true },
            onOpenTextSize = { showTextSizeSheet = true },
            onClaimSize = model::claimTerminalSize,
            onClearTerminal = model::clearTerminal,
            onOpenDiagnostics = onOpenDiagnostics,
            onCloseTab = { model.closeTab(agentId) },
            onDeleteAgent = { model.requestDelete(agentId) },
        )

        if (tabSummaries.isNotEmpty()) {
            SessionTabBar(
                tabs = tabSummaries,
                activeAgentId = agentId,
                onStepTab = model::stepTab,
                onOpenTabsSheet = { showTabsSheet = true },
                onOpenNewTab = { onOpenNewTab(agent?.project ?: state.snapshot?.projects?.firstOrNull()?.id) },
            )
        }

        state.error?.let { err ->
            Box(Modifier.padding(horizontal = 10.dp, vertical = 6.dp)) { ErrorPanel(err) }
        }

        if (otherApproval != null) {
            val otherAgentTitle = state.snapshot?.agents?.find { it.id == otherApproval.agent }?.title ?: otherApproval.agent
            Surface(
                color = MuxWarningBg,
                border = BorderStroke(1.dp, MuxWarningLine),
                shape = Square,
                modifier = Modifier
                    .fillMaxWidth()
                    .clickable { model.openAgent(otherApproval.agent) },
            ) {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 7.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Icon(Icons.Outlined.NotificationsActive, null, tint = MuxWarningFg, modifier = Modifier.size(15.dp))
                    Text(
                        "$otherAgentTitle 等待审批 · 点击切换处理",
                        style = MaterialTheme.typography.labelSmall,
                        color = MuxWarningFg,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    Icon(Icons.AutoMirrored.Outlined.KeyboardArrowRight, null, tint = MuxWarningFg, modifier = Modifier.size(16.dp))
                }
            }
        }

        Box(Modifier.weight(1f).fillMaxWidth()) {
            if (activePaneMode == SessionPaneMode.CHAT) {
                ChatTimelinePane(
                    model = model,
                    agentId = agentId,
                    state = state,
                    canSwitchToTerm = canSwitchToTerm,
                    showSlashMenu = showSlashMenu,
                    onToggleSlash = { showSlashMenu = !showSlashMenu },
                    onDismissSlash = { showSlashMenu = false },
                    modifier = Modifier.fillMaxSize(),
                )
            } else {
                TerminalPane(
                    model = model,
                    agentId = agentId,
                    state = state,
                    modifier = Modifier.fillMaxSize(),
                )
            }
        }
    }

    if (showTabsSheet) {
        OpenTabsSheet(
            tabs = tabSummaries,
            activeAgentId = agentId,
            onSelect = { tab ->
                showTabsSheet = false
                model.openAgent(tab.agentId)
            },
            onCloseTab = { tab -> model.closeTab(tab.agentId) },
            onNewTab = {
                showTabsSheet = false
                onOpenNewTab(agent?.project ?: state.snapshot?.projects?.firstOrNull()?.id)
            },
            onDismiss = { showTabsSheet = false },
        )
    }

    if (showQuickDrawer) {
        WorkspaceQuickDrawerSheet(
            state = state,
            activeAgentId = agentId,
            onSelectAgent = { targetId ->
                showQuickDrawer = false
                model.openAgent(targetId)
            },
            onNewSession = { projId ->
                showQuickDrawer = false
                onOpenNewTab(projId)
            },
            onDismiss = { showQuickDrawer = false },
        )
    }

    if (showDetailsSheet) {
        SessionDetailsSheet(
            state = state,
            agentTitle = agent?.title ?: agentId,
            projectName = project?.displayName() ?: agent?.project.orEmpty(),
            projectPath = project?.path.orEmpty(),
            driverLabel = driverBadgeLabel(agent?.agentType ?: muxlane.protocol.AgentType.UNKNOWN, chatMeta?.driver),
            modelName = chatMeta?.currentModel?.let { "${it.name.ifBlank { it.id }} (${it.provider})" },
            thinkingLevel = chatMeta?.thinkingLevel,
            onOpenTextSize = {
                showDetailsSheet = false
                showTextSizeSheet = true
            },
            onDismiss = { showDetailsSheet = false },
        )
    }

    if (showQuickCommandsSheet) {
        QuickCommandsSheet(
            mode = activePaneMode,
            onRun = { cmd ->
                showQuickCommandsSheet = false
                if (activePaneMode == SessionPaneMode.CHAT) {
                    if (cmd.command == "/clear" || cmd.command == "/compact") {
                        model.sendChatPrompt(cmd.command, steer = false)
                    } else {
                        model.sendChatPrompt(cmd.command, steer = false)
                    }
                } else {
                    model.sendInput(cmd.command)
                }
            },
            onDismiss = { showQuickCommandsSheet = false },
        )
    }

    if (showTextSizeSheet) {
        SessionTextSizeSheet(
            mode = activePaneMode,
            chatScale = state.chatTextScale,
            terminalScale = state.terminalTextScale,
            onChatScaleChange = model::setChatTextScale,
            onTerminalScaleChange = model::setTerminalTextScale,
            onDismiss = { showTextSizeSheet = false },
        )
    }
}

@Composable
private fun EmptySessionSplitPlaceholder(
    hasProjects: Boolean,
    connected: Boolean,
    openTabsCount: Int,
    onOpenFirstTab: () -> Unit,
    onNewTab: () -> Unit,
    onOpenDiagnostics: () -> Unit,
) {
    Column(
        Modifier.fillMaxSize().background(MuxBg0).padding(32.dp),
        verticalArrangement = Arrangement.Center,
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Icon(Icons.Outlined.Terminal, null, tint = MuxFg2, modifier = Modifier.size(40.dp))
        Spacer(Modifier.height(10.dp))
        Text(stringResource(R.string.no_open_tab_title), style = MaterialTheme.typography.titleMedium)
        Spacer(Modifier.height(4.dp))
        Text(stringResource(R.string.no_open_tab_subtitle), style = MaterialTheme.typography.bodySmall, color = MuxFg2)
        Spacer(Modifier.height(16.dp))
        Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (openTabsCount > 0) {
                OutlinedButton(onClick = onOpenFirstTab, shape = Square) {
                    Text("恢复会话 ($openTabsCount)")
                }
            }
            if (hasProjects && connected) {
                Button(
                    onClick = onNewTab,
                    shape = Square,
                    colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                ) {
                    Icon(Icons.Outlined.Add, null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(4.dp))
                    Text(stringResource(R.string.new_tab_title))
                }
            } else {
                OutlinedButton(onClick = onOpenDiagnostics, shape = Square) {
                    Text(stringResource(R.string.host_settings))
                }
            }
        }
    }
}

@Composable
private fun SessionWorkspaceHeader(
    title: String,
    subtitle: String,
    isStreaming: Boolean,
    connected: Boolean,
    e2eeActive: Boolean,
    usingLan: Boolean,
    wideSplitMode: Boolean,
    sidebarVisible: Boolean,
    sessionMode: SessionPaneMode,
    canSwitchToChat: Boolean,
    canSwitchToTerm: Boolean,
    onBack: () -> Unit,
    onToggleSidebar: (() -> Unit)?,
    onOpenQuickDrawer: () -> Unit,
    onOpenDetails: () -> Unit,
    onSelectMode: (SessionPaneMode) -> Unit,
    onOpenTabsSheet: () -> Unit,
    onOpenNewTab: () -> Unit,
    onOpenQuickCommands: () -> Unit,
    onOpenTextSize: () -> Unit,
    onClaimSize: () -> Unit,
    onClearTerminal: () -> Unit,
    onOpenDiagnostics: () -> Unit,
    onCloseTab: () -> Unit,
    onDeleteAgent: () -> Unit,
) {
    var moreExpanded by remember { mutableStateOf(false) }
    Surface(color = MuxBg1, contentColor = MuxFg0) {
        Column(Modifier.fillMaxWidth().statusBarsPadding()) {
            Row(
                Modifier.fillMaxWidth().height(50.dp).padding(end = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
            ) {
                if (wideSplitMode && onToggleSidebar != null) {
                    IconButton(onClick = onToggleSidebar, modifier = Modifier.size(40.dp)) {
                        Icon(Icons.AutoMirrored.Outlined.ViewSidebar, if (sidebarVisible) "收起边栏" else "展开边栏", tint = MuxFg1, modifier = Modifier.size(20.dp))
                    }
                } else {
                    IconButton(onClick = onBack, modifier = Modifier.size(40.dp)) {
                        Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.back), tint = MuxFg0, modifier = Modifier.size(20.dp))
                    }
                    IconButton(onClick = onOpenQuickDrawer, modifier = Modifier.size(36.dp)) {
                        Icon(Icons.Outlined.Menu, stringResource(R.string.workspace_drawer_title), tint = MuxFg1, modifier = Modifier.size(19.dp))
                    }
                }

                Column(
                    Modifier
                        .weight(1f)
                        .clickable(onClick = onOpenDetails)
                        .padding(horizontal = 4.dp),
                ) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                        Text(
                            title,
                            style = MaterialTheme.typography.titleMedium,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f, fill = false),
                        )
                        if (isStreaming) {
                            SquareStepIndicator(blockSize = 4.dp, gap = 2.dp, blockCount = 3)
                        }
                    }
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(5.dp)) {
                        Box(Modifier.size(6.dp).background(if (connected) MuxGreen else MuxRed, Square))
                        Text(
                            subtitle + if (e2eeActive) " · E2EE" else "" + if (usingLan) " · LAN" else "",
                            style = MaterialTheme.typography.labelSmall,
                            color = MuxFg2,
                            fontFamily = MonoFamily,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }

                SessionModeSwitcher(
                    mode = sessionMode,
                    canSwitchToChat = canSwitchToChat,
                    canSwitchToTerm = canSwitchToTerm,
                    onSelect = onSelectMode,
                )

                Box {
                    IconButton(onClick = { moreExpanded = true }, modifier = Modifier.size(40.dp)) {
                        Icon(Icons.Outlined.MoreVert, "更多操作", tint = MuxFg1, modifier = Modifier.size(20.dp))
                    }
                    DropdownMenu(expanded = moreExpanded, onDismissRequest = { moreExpanded = false }) {
                        DropdownMenuItem(
                            text = { Text("全部已打开标签") },
                            leadingIcon = { Icon(Icons.Outlined.Tab, null) },
                            onClick = { moreExpanded = false; onOpenTabsSheet() },
                        )
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.new_tab_title)) },
                            leadingIcon = { Icon(Icons.Outlined.Add, null) },
                            onClick = { moreExpanded = false; onOpenNewTab() },
                        )
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.quick_commands)) },
                            leadingIcon = { Icon(Icons.Outlined.Terminal, null) },
                            onClick = { moreExpanded = false; onOpenQuickCommands() },
                        )
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.text_size_settings)) },
                            leadingIcon = { Icon(Icons.Outlined.FormatSize, null) },
                            onClick = { moreExpanded = false; onOpenTextSize() },
                        )
                        if (sessionMode == SessionPaneMode.TERMINAL) {
                            DropdownMenuItem(
                                text = { Text(stringResource(R.string.terminal_reclaim_size)) },
                                leadingIcon = { Icon(Icons.Outlined.Tune, null) },
                                onClick = { moreExpanded = false; onClaimSize() },
                            )
                            DropdownMenuItem(
                                text = { Text(stringResource(R.string.terminal_clear)) },
                                leadingIcon = { Icon(Icons.Outlined.DeleteSweep, null) },
                                onClick = { moreExpanded = false; onClearTerminal() },
                            )
                        }
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.host_settings)) },
                            leadingIcon = { Icon(Icons.Outlined.Settings, null) },
                            onClick = { moreExpanded = false; onOpenDiagnostics() },
                        )
                        HorizontalDivider(color = MuxLine)
                        DropdownMenuItem(
                            text = { Text(stringResource(R.string.close_tab)) },
                            leadingIcon = { Icon(Icons.Outlined.Close, null) },
                            onClick = { moreExpanded = false; onCloseTab() },
                        )
                        DropdownMenuItem(
                            text = { Text("销毁远端会话", color = MuxRed) },
                            leadingIcon = { Icon(Icons.Outlined.DeleteOutline, null, tint = MuxRed) },
                            onClick = { moreExpanded = false; onDeleteAgent() },
                        )
                    }
                }
            }
            HorizontalDivider(color = MuxLine)
        }
    }
}

@Composable
private fun SessionTabBar(
    tabs: List<SessionTabSummary>,
    activeAgentId: String,
    onStepTab: (Int) -> Unit,
    onOpenTabsSheet: () -> Unit,
    onOpenNewTab: () -> Unit,
) {
    val activeIndex = tabs.indexOfFirst { it.agentId == activeAgentId }.coerceAtLeast(0)
    val active = tabs.getOrNull(activeIndex)
    val hasPrev = activeIndex > 0
    val hasNext = activeIndex < tabs.lastIndex
    val otherNeedsYou = tabs.any { it.agentId != activeAgentId && it.group == TabPriorityGroup.NEEDS_YOU }
    var dragAccum by remember(activeAgentId) { mutableFloatStateOf(0f) }

    Surface(color = MuxBg0, contentColor = MuxFg0) {
        Column(Modifier.fillMaxWidth()) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .height(40.dp)
                    .padding(horizontal = 8.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                if (tabs.size > 1) {
                    Box(
                        Modifier
                            .size(28.dp)
                            .background(MuxBg1, Square)
                            .border(BorderStroke(1.dp, MuxLine), Square)
                            .clickable(enabled = hasPrev) { onStepTab(-1) },
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(
                            Icons.AutoMirrored.Outlined.KeyboardArrowLeft,
                            "上一个标签",
                            tint = if (hasPrev) MuxFg1 else MuxControlLine,
                            modifier = Modifier.size(16.dp),
                        )
                    }
                }

                Row(
                    Modifier
                        .weight(1f)
                        .height(32.dp)
                        .background(MuxBg1, Square)
                        .border(BorderStroke(1.dp, MuxControlLine), Square)
                        .pointerInput(activeAgentId, tabs.size) {
                            detectHorizontalDragGestures(
                                onDragStart = { dragAccum = 0f },
                                onDragEnd = {
                                    when {
                                        dragAccum < -48f && hasNext -> onStepTab(1)
                                        dragAccum > 48f && hasPrev -> onStepTab(-1)
                                    }
                                    dragAccum = 0f
                                },
                                onDragCancel = { dragAccum = 0f },
                            ) { _, delta -> dragAccum += delta }
                        }
                        .clickable(onClick = onOpenTabsSheet)
                        .padding(horizontal = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    if (active != null) {
                        Box(Modifier.size(7.dp).background(statusColor(active.status), Square))
                        Text(
                            active.title,
                            style = MaterialTheme.typography.labelMedium,
                            color = MuxFg0,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                            modifier = Modifier.weight(1f),
                        )
                        Text(
                            driverBadgeLabel(active.agentType, active.driverMode),
                            style = MaterialTheme.typography.labelSmall,
                            color = MuxFg2,
                            fontFamily = MonoFamily,
                            maxLines = 1,
                        )
                    } else {
                        Text("选择标签", style = MaterialTheme.typography.labelMedium, modifier = Modifier.weight(1f))
                    }
                    Row(
                        Modifier
                            .background(if (otherNeedsYou) MuxWarningBg else MuxBg2, Square)
                            .border(BorderStroke(1.dp, if (otherNeedsYou) MuxWarningLine else MuxLine), Square)
                            .padding(horizontal = 5.dp, vertical = 1.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(3.dp),
                    ) {
                        if (otherNeedsYou) {
                            Box(Modifier.size(5.dp).background(MuxYellow, Square))
                        }
                        Text(
                            "${tabs.size} 标签",
                            style = MaterialTheme.typography.labelSmall,
                            color = if (otherNeedsYou) MuxWarningFg else MuxFg1,
                            fontFamily = MonoFamily,
                        )
                    }
                }

                if (tabs.size > 1) {
                    Box(
                        Modifier
                            .size(28.dp)
                            .background(MuxBg1, Square)
                            .border(BorderStroke(1.dp, MuxLine), Square)
                            .clickable(enabled = hasNext) { onStepTab(1) },
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(
                            Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                            "下一个标签",
                            tint = if (hasNext) MuxFg1 else MuxControlLine,
                            modifier = Modifier.size(16.dp),
                        )
                    }
                }

                Box(
                    Modifier
                        .size(32.dp)
                        .background(MuxAccent, Square)
                        .border(BorderStroke(1.dp, MuxAccent), Square)
                        .clickable(onClick = onOpenNewTab),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(Icons.Outlined.Add, stringResource(R.string.new_tab_title), tint = MuxOnAccent, modifier = Modifier.size(18.dp))
                }
            }
            HorizontalDivider(color = MuxLine)
        }
    }
}

@Composable
private fun OpenTabsSheet(
    tabs: List<SessionTabSummary>,
    activeAgentId: String,
    onSelect: (SessionTabSummary) -> Unit,
    onCloseTab: (SessionTabSummary) -> Unit,
    onNewTab: () -> Unit,
    onDismiss: () -> Unit,
) {
    val grouped = remember(tabs) {
        listOf(
            TabPriorityGroup.NEEDS_YOU to "需要你确认",
            TabPriorityGroup.WORKING to "进行中",
            TabPriorityGroup.OTHER to "其他标签",
        ).mapNotNull { (grp, title) ->
            val items = tabs.filter { it.group == grp }
            if (items.isEmpty()) null else Triple(grp, title, items)
        }
    }
    MuxSheetFrame(
        onDismiss = onDismiss,
        title = stringResource(R.string.open_tabs_count, tabs.size),
        subtitle = "点按切换 · 右上角可新建标签",
        trailing = {
            Button(
                onClick = onNewTab,
                shape = Square,
                colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                modifier = Modifier.height(32.dp),
            ) {
                Icon(Icons.Outlined.Add, null, modifier = Modifier.size(15.dp))
                Spacer(Modifier.width(4.dp))
                Text(stringResource(R.string.new_tab_title), fontSize = 12.sp)
            }
        },
    ) {
        Column(
            Modifier.heightIn(max = 420.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            grouped.forEach { (grp, header, list) ->
                MuxSectionHeading(title = header, count = list.size)
                list.forEach { tab ->
                    val current = tab.agentId == activeAgentId
                    Surface(
                        shape = Square,
                        color = if (current) MuxAccentBg else MuxBg0,
                        border = BorderStroke(
                            1.dp,
                            when {
                                current -> MuxAccent
                                grp == TabPriorityGroup.NEEDS_YOU -> MuxYellow
                                else -> MuxLine
                            },
                        ),
                        modifier = Modifier.fillMaxWidth().clickable { onSelect(tab) },
                    ) {
                        Row(
                            Modifier.padding(start = 10.dp, end = 4.dp, top = 8.dp, bottom = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Box(Modifier.size(8.dp).background(statusColor(tab.status), Square))
                            Column(Modifier.weight(1f)) {
                                Text(tab.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Text(
                                    "${tab.projectName} · ${driverBadgeLabel(tab.agentType, tab.driverMode)}",
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MuxFg2,
                                    fontFamily = MonoFamily,
                                )
                            }
                            if (current) {
                                Text("当前", style = MaterialTheme.typography.labelSmall, color = MuxAccent)
                            } else if (tab.pendingApprovalsCount > 0) {
                                Text("${tab.pendingApprovalsCount} 待批", style = MaterialTheme.typography.labelSmall, color = MuxYellow)
                            }
                            IconButton(onClick = { onCloseTab(tab) }, modifier = Modifier.size(34.dp)) {
                                Icon(Icons.Outlined.Close, stringResource(R.string.close_tab), tint = MuxFg2, modifier = Modifier.size(17.dp))
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
internal fun NewTabSheet(
    model: MuxlaneViewModel,
    state: UiState,
) {
    val projectId = state.spawnProject ?: return
    val projects = state.snapshot?.projects.orEmpty()
    val (agentPresets, shellPreset) = remember(state.presets) { orderAgentPresetsPiFirst(state.presets) }
    var surface by remember(projectId) { mutableStateOf(NewTabSurface.CHAT) }
    var initialPrompt by remember(projectId) { mutableStateOf("") }

    MuxSheetFrame(
        onDismiss = model::closeSpawn,
        title = stringResource(R.string.new_tab_title),
        subtitle = projects.find { it.id == projectId }?.displayName() ?: projectId,
    ) {
        Column(
            Modifier.heightIn(max = 500.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            if (projects.size > 1) {
                Row(
                    Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()),
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    projects.forEach { proj ->
                        val selected = proj.id == projectId
                        Box(
                            Modifier
                                .background(if (selected) MuxAccent else MuxBg0, Square)
                                .border(BorderStroke(1.dp, if (selected) MuxAccent else MuxControlLine), Square)
                                .clickable { if (!selected) model.loadPresets(proj.id) }
                                .padding(horizontal = 10.dp, vertical = 6.dp),
                        ) {
                            Text(
                                proj.displayName(),
                                style = MaterialTheme.typography.labelMedium,
                                color = if (selected) MuxOnAccent else MuxFg1,
                            )
                        }
                    }
                }
            }

            MuxSegmentedSwitch(
                options = listOf(
                    NewTabSurface.CHAT to stringResource(R.string.surface_chat),
                    NewTabSurface.CLI to stringResource(R.string.surface_cli),
                ),
                selected = surface,
                onSelect = { surface = it },
                modifier = Modifier.fillMaxWidth(),
            )

            if (surface == NewTabSurface.CHAT) {
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Text("首条指令（可选）", style = MaterialTheme.typography.labelSmall, color = MuxFg2)
                    BasicTextField(
                        value = initialPrompt,
                        onValueChange = { initialPrompt = it },
                        maxLines = 3,
                        textStyle = MaterialTheme.typography.bodyMedium.copy(color = MuxFg0),
                        cursorBrush = SolidColor(MuxAccent),
                        modifier = Modifier
                            .fillMaxWidth()
                            .background(MuxBg0, Square)
                            .border(BorderStroke(1.dp, MuxControlLine), Square)
                            .padding(horizontal = 10.dp, vertical = 8.dp),
                        decorationBox = { inner ->
                            if (initialPrompt.isEmpty()) {
                                Text(stringResource(R.string.initial_prompt_hint), style = MaterialTheme.typography.bodySmall, color = MuxFg2)
                            }
                            inner()
                        },
                    )
                }
            }

            MuxSectionHeading(title = "Agent (Pi RPC / 通用 ACP)", count = agentPresets.size)
            if (agentPresets.isEmpty()) {
                Row(
                    Modifier.fillMaxWidth().padding(vertical = 8.dp),
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    SquareStepIndicator()
                    Text("正在加载预设…", style = MaterialTheme.typography.bodySmall, color = MuxFg2)
                }
            } else {
                Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    agentPresets.chunked(2).forEach { pair ->
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            pair.forEach { preset ->
                                AgentPresetTile(
                                    preset = preset,
                                    surface = surface,
                                    onClick = {
                                        if (surface == NewTabSurface.CHAT) {
                                            model.spawnChat(
                                                preset,
                                                initialPrompt = initialPrompt.trim().ifEmpty { null },
                                            )
                                        } else {
                                            model.spawn(preset)
                                        }
                                    },
                                    modifier = Modifier.weight(1f),
                                )
                            }
                            if (pair.size == 1) Spacer(Modifier.weight(1f))
                        }
                    }
                }
            }

            shellPreset?.let { shell ->
                HorizontalDivider(color = MuxLine)
                OutlinedButton(
                    onClick = { model.spawn(shell) },
                    shape = Square,
                    border = BorderStroke(1.dp, MuxControlLine),
                    contentPadding = PaddingValues(vertical = 10.dp),
                    modifier = Modifier.fillMaxWidth(),
                ) {
                    Icon(Icons.Outlined.Terminal, null, tint = MuxFg1, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(8.dp))
                    Text(stringResource(R.string.launch_shell_terminal), color = MuxFg0)
                }
            }
        }
    }
}

@Composable
private fun AgentPresetTile(
    preset: AgentPreset,
    surface: NewTabSurface,
    onClick: () -> Unit,
    modifier: Modifier = Modifier,
) {
    val driverTag = when {
        surface == NewTabSurface.CLI -> "CLI"
        preset.agentType == muxlane.protocol.AgentType.PI || preset.agentType == muxlane.protocol.AgentType.PIE -> "Pi RPC"
        else -> "ACP"
    }
    Surface(
        shape = Square,
        color = MuxBg0,
        border = BorderStroke(1.dp, MuxControlLine),
        modifier = modifier.clickable(onClick = onClick),
    ) {
        Column(Modifier.padding(10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    preset.label,
                    style = MaterialTheme.typography.titleMedium,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.weight(1f),
                )
                Box(
                    Modifier
                        .background(if (surface == NewTabSurface.CHAT) MuxAccentBg else MuxBg2, Square)
                        .border(BorderStroke(1.dp, if (surface == NewTabSurface.CHAT) MuxAccent.copy(alpha = 0.45f) else MuxLine), Square)
                        .padding(horizontal = 5.dp, vertical = 1.dp),
                ) {
                    Text(
                        driverTag,
                        style = MaterialTheme.typography.labelSmall,
                        color = if (surface == NewTabSurface.CHAT) MuxAccent else MuxFg2,
                        fontFamily = MonoFamily,
                    )
                }
            }
            Text(
                preset.program,
                style = MaterialTheme.typography.labelSmall,
                color = MuxFg2,
                fontFamily = MonoFamily,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
private fun WorkspaceQuickDrawerSheet(
    state: UiState,
    activeAgentId: String,
    onSelectAgent: (String) -> Unit,
    onNewSession: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    val snapshot = state.snapshot
    val grouped = snapshot?.agents.orEmpty().groupBy { it.project }
    MuxSheetFrame(
        onDismiss = onDismiss,
        title = stringResource(R.string.workspace_drawer_title),
        subtitle = state.pairing?.machineName ?: state.pairing?.hostId,
    ) {
        Column(
            Modifier.heightIn(max = 440.dp).verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            snapshot?.projects.orEmpty().forEach { project ->
                val list = grouped[project.id].orEmpty()
                Row(
                    Modifier.fillMaxWidth(),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        Icon(Icons.Outlined.FolderOpen, null, tint = MuxFg2, modifier = Modifier.size(16.dp))
                        Text(project.displayName(), style = MaterialTheme.typography.titleMedium)
                    }
                    Text(
                        "+ 新建",
                        style = MaterialTheme.typography.labelMedium,
                        color = MuxAccent,
                        modifier = Modifier.clickable { onNewSession(project.id) }.padding(4.dp),
                    )
                }
                if (list.isEmpty()) {
                    Text("暂无活动会话", style = MaterialTheme.typography.bodySmall, color = MuxFg2, modifier = Modifier.padding(start = 22.dp))
                } else {
                    list.forEach { agent ->
                        val active = agent.id == activeAgentId
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .background(if (active) MuxAccentBg else MuxBg0, Square)
                                .border(BorderStroke(1.dp, if (active) MuxAccent else MuxLine), Square)
                                .clickable { onSelectAgent(agent.id) }
                                .padding(horizontal = 10.dp, vertical = 8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Box(Modifier.size(8.dp).background(statusColor(agent.status), Square))
                            Text(
                                agent.title,
                                style = MaterialTheme.typography.bodyMedium,
                                fontWeight = if (active) FontWeight.Bold else FontWeight.Normal,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                                modifier = Modifier.weight(1f),
                            )
                            Text(
                                driverBadgeLabel(agent.agentType, state.chatSessions[agent.id]?.meta?.driver),
                                style = MaterialTheme.typography.labelSmall,
                                color = MuxFg2,
                                fontFamily = MonoFamily,
                            )
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun QuickCommandsSheet(
    mode: SessionPaneMode,
    onRun: (QuickCommandItem) -> Unit,
    onDismiss: () -> Unit,
) {
    val commands = remember(mode) {
        defaultQuickCommands().filter { if (mode == SessionPaneMode.CHAT) it.forChat else it.forTerminal }
    }
    MuxSheetFrame(
        onDismiss = onDismiss,
        title = stringResource(R.string.quick_commands),
        subtitle = if (mode == SessionPaneMode.CHAT) "发送到当前对话" else "输入到当前终端",
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            commands.forEach { cmd ->
                Surface(
                    shape = Square,
                    color = MuxBg0,
                    border = BorderStroke(1.dp, MuxControlLine),
                    modifier = Modifier.fillMaxWidth().clickable { onRun(cmd) },
                ) {
                    Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Text(cmd.label, style = MaterialTheme.typography.titleMedium)
                        Text(cmd.command.trim(), style = MaterialTheme.typography.bodySmall, color = MuxFg2, fontFamily = MonoFamily)
                    }
                }
            }
        }
    }
}

@Composable
private fun SessionTextSizeSheet(
    mode: SessionPaneMode,
    chatScale: Float,
    terminalScale: Float,
    onChatScaleChange: (Float) -> Unit,
    onTerminalScaleChange: (Float) -> Unit,
    onDismiss: () -> Unit,
) {
    MuxSheetFrame(
        onDismiss = onDismiss,
        title = stringResource(R.string.text_size_settings),
        subtitle = if (mode == SessionPaneMode.CHAT) "对话视图字号" else "终端网格字号（支持双指捏合）",
    ) {
        if (mode == SessionPaneMode.CHAT) {
            TextScaleRow(
                label = "当前比例 ${(chatScale * 100).roundToInt()}%",
                value = chatScale,
                range = 0.85f..1.4f,
                onChange = onChatScaleChange,
            )
        } else {
            TextScaleRow(
                label = "当前比例 ${(terminalScale * 100).roundToInt()}%",
                value = terminalScale,
                range = 0.7f..1.65f,
                onChange = onTerminalScaleChange,
            )
        }
    }
}

@Composable
private fun SessionDetailsSheet(
    state: UiState,
    agentTitle: String,
    projectName: String,
    projectPath: String,
    driverLabel: String,
    modelName: String?,
    thinkingLevel: String?,
    onOpenTextSize: () -> Unit,
    onDismiss: () -> Unit,
) {
    MuxSheetFrame(
        onDismiss = onDismiss,
        title = agentTitle,
        subtitle = projectName,
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            DetailMetaRow("驱动引擎", driverLabel)
            if (projectPath.isNotBlank()) DetailMetaRow("工作目录", projectPath)
            modelName?.let { DetailMetaRow("当前模型", it) }
            thinkingLevel?.let { DetailMetaRow("思考强度", it) }
            DetailMetaRow(
                "连接通道",
                buildString {
                    append(if (state.usingLan) "LAN 局域网" else "中继服务器")
                    if (state.e2eeActive) append(" · E2EE 端到端加密")
                },
            )
            Spacer(Modifier.height(4.dp))
            OutlinedButton(
                onClick = onOpenTextSize,
                shape = Square,
                border = BorderStroke(1.dp, MuxControlLine),
                modifier = Modifier.fillMaxWidth(),
            ) {
                Icon(Icons.Outlined.FormatSize, null, modifier = Modifier.size(16.dp))
                Spacer(Modifier.width(6.dp))
                Text(stringResource(R.string.text_size_settings))
            }
        }
    }
}

@Composable
private fun DetailMetaRow(label: String, value: String) {
    Row(
        Modifier
            .fillMaxWidth()
            .background(MuxBg0, Square)
            .border(BorderStroke(1.dp, MuxLine), Square)
            .padding(horizontal = 10.dp, vertical = 8.dp),
        horizontalArrangement = Arrangement.SpaceBetween,
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(label, style = MaterialTheme.typography.labelSmall, color = MuxFg2)
        Spacer(Modifier.width(12.dp))
        Text(
            value,
            style = MaterialTheme.typography.bodySmall,
            color = MuxFg0,
            fontFamily = MonoFamily,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
    }
}
