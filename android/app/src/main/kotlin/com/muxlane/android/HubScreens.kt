package com.muxlane.android

import android.content.ClipboardManager
import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
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
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.FormatListBulleted
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.CloudOff
import androidx.compose.material.icons.outlined.Computer
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.FolderOpen
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.outlined.NotificationsActive
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.QrCodeScanner
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.Security
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.SmartToy
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateMapOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import muxlane.chat.ApprovalReply
import muxlane.chat.PendingApproval
import muxlane.chat.SessionDriverMode
import muxlane.protocol.AgentInstance
import muxlane.protocol.AgentStatus
import muxlane.protocol.BotDefinitionSummary
import muxlane.protocol.BotRunRecord
import muxlane.protocol.BotRunStatus
import muxlane.protocol.Project

private enum class WorkspaceBrowseMode {
    TREE,
    FLAT,
}

private const val WORKSPACE_SESSION_WATERMARK = 5

@Composable
internal fun PairOnboardingScreen(model: MuxlaneViewModel, state: UiState) {
    val context = LocalContext.current
    val scan = rememberLauncherForActivityResult(ActivityResultContracts.StartActivityForResult()) { result ->
        val text = result.data?.getStringExtra(QrScanActivity.EXTRA_TEXT)
        if (!text.isNullOrBlank()) applyPairText(model, text)
    }
    val canPair = state.relayUrl.isNotBlank() && state.hostId.isNotBlank() && !state.connecting
    BackHandler(state.pairings.isNotEmpty()) { model.cancelAddMachine() }
    Column(
        Modifier
            .fillMaxSize()
            .background(MuxBg1)
            .statusBarsPadding()
            .navigationBarsPadding()
            .imePadding()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 24.dp, vertical = 20.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (state.pairings.isNotEmpty()) {
                IconButton(onClick = model::cancelAddMachine) {
                    Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.back))
                }
                Spacer(Modifier.width(4.dp))
            }
            Surface(shape = Square, color = MuxFg0, modifier = Modifier.size(40.dp)) {
                Box(contentAlignment = Alignment.Center) {
                    Text("M", color = MuxBg1, fontFamily = MonoFamily, fontWeight = FontWeight.Bold, fontSize = 18.sp)
                }
            }
            Spacer(Modifier.width(12.dp))
            Column {
                Text(stringResource(R.string.app_name), style = MaterialTheme.typography.titleLarge)
                Text("Pi RPC · ACP · 移动工作台", style = MaterialTheme.typography.labelSmall, color = MuxFg2)
            }
        }
        Spacer(Modifier.height(28.dp))
        Column(Modifier.fillMaxWidth().widthIn(max = 560.dp).align(Alignment.CenterHorizontally)) {
            Text(
                stringResource(R.string.connect_workspace),
                style = MaterialTheme.typography.headlineSmall,
                modifier = Modifier.padding(bottom = 18.dp),
            )
            OutlinedTextField(
                value = state.relayUrl,
                onValueChange = model::setRelay,
                label = { Text(stringResource(R.string.relay_label)) },
                leadingIcon = { Icon(Icons.Outlined.Dns, null) },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Next),
                singleLine = true,
                shape = Square,
                modifier = Modifier.fillMaxWidth(),
            )
            Row(Modifier.fillMaxWidth().padding(vertical = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                OutlinedButton(
                    onClick = { scan.launch(QrScanActivity.intent(context)) },
                    shape = Square,
                    border = BorderStroke(1.dp, MuxControlLine),
                    modifier = Modifier.weight(1f).heightIn(min = 44.dp),
                ) {
                    Icon(Icons.Outlined.QrCodeScanner, null, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(6.dp))
                    Text(stringResource(R.string.scan_qr))
                }
                OutlinedButton(
                    onClick = {
                        val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                        val text = clipboard.primaryClip?.getItemAt(0)?.coerceToText(context)?.toString().orEmpty()
                        applyPairText(model, text)
                    },
                    shape = Square,
                    border = BorderStroke(1.dp, MuxControlLine),
                    modifier = Modifier.weight(1f).heightIn(min = 44.dp),
                ) { Text(stringResource(R.string.paste_pairing)) }
            }
            OutlinedTextField(
                value = state.hostId,
                onValueChange = model::setHostId,
                label = { Text(stringResource(R.string.pair_code)) },
                leadingIcon = { Icon(Icons.Outlined.Security, null) },
                textStyle = TextStyle(fontFamily = MonoFamily, fontSize = 15.sp),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { if (canPair) model.pair() }),
                singleLine = true,
                shape = Square,
                modifier = Modifier.fillMaxWidth(),
            )
            ErrorPanel(state.error, Modifier.padding(top = 8.dp))
            Spacer(Modifier.height(14.dp))
            Button(
                onClick = model::pair,
                enabled = canPair,
                shape = Square,
                colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                modifier = Modifier.fillMaxWidth().height(48.dp),
            ) {
                if (state.connecting) SquareStepIndicator(color = Color.White)
                else Text(stringResource(R.string.pair_secure))
            }
        }
    }
}

/**
 * Unified dsh-style Mobile Workspace Home:
 * Eliminates the redundant 4-tab layout (Feed / Workspaces / Agents / Bots) and
 * presents a single cool-neutral workspace browser with inline search, tree/flat
 * toggle, on-demand Bots sheet, and one-tap return to the active session.
 */
@Composable
internal fun HomeHubScaffold(
    model: MuxlaneViewModel,
    state: UiState,
    onOpenNewTab: (projectId: String?) -> Unit,
    onOpenDiagnostics: () -> Unit,
) {
    Box(
        Modifier
            .fillMaxSize()
            .background(MuxBg0),
        contentAlignment = Alignment.TopCenter,
    ) {
        WorkspacesPane(
            model = model,
            state = state,
            showSidebarChrome = true,
            onOpenNewTab = onOpenNewTab,
            onOpenDiagnostics = onOpenDiagnostics,
            showResumeFooter = true,
            modifier = Modifier.fillMaxSize().widthIn(max = 860.dp),
        )
    }
}

/**
 * Unified dsh-inspired Workspace & Session Browser:
 * Reused across Phone Home, Tablet persistent left sidebar, and Session slide-out drawer.
 */
@Composable
internal fun WorkspacesPane(
    model: MuxlaneViewModel,
    state: UiState,
    showSidebarChrome: Boolean,
    onOpenNewTab: (projectId: String?) -> Unit,
    onOpenDiagnostics: () -> Unit,
    onHideSidebar: (() -> Unit)? = null,
    onAfterSelectAgent: (() -> Unit)? = null,
    showResumeFooter: Boolean = false,
    modifier: Modifier = Modifier,
) {
    val snapshot = state.snapshot
    var browseMode by remember { mutableStateOf(WorkspaceBrowseMode.TREE) }
    var showBotsSheet by remember { mutableStateOf(false) }
    var detailBot by remember { mutableStateOf<BotDefinitionSummary?>(null) }
    val expandedWatermarks = remember { mutableStateMapOf<String, Boolean>() }

    val (projects, agentsByProject) = remember(
        snapshot,
        state.workspaceQuery,
        state.agentsNeedsYouOnly,
        state.globalApprovals,
    ) {
        filterProjectsAndAgents(
            projects = snapshot?.projects.orEmpty(),
            agents = snapshot?.agents.orEmpty(),
            query = state.workspaceQuery,
            needsYouOnly = state.agentsNeedsYouOnly,
            approvals = state.globalApprovals,
        )
    }

    val orderedProjects = remember(projects, agentsByProject, state.chatSessions, state.globalApprovals) {
        projects.sortedWith(
            compareByDescending<Project> { proj ->
                val list = agentsByProject[proj.id].orEmpty()
                list.any { ag ->
                    ag.status == AgentStatus.BLOCKED ||
                        state.globalApprovals.any { it.agent == ag.id }
                }
            }.thenByDescending { proj ->
                val list = agentsByProject[proj.id].orEmpty()
                list.any { ag ->
                    ag.status == AgentStatus.WORKING ||
                        state.chatSessions[ag.id]?.meta?.isStreaming == true
                }
            }.thenByDescending { proj ->
                agentsByProject[proj.id]?.size ?: 0
            },
        )
    }

    val flatProjection = remember(
        snapshot,
        state.chatSessions,
        state.globalApprovals,
        state.workspaceQuery,
        state.agentsNeedsYouOnly,
    ) {
        buildAgentListProjection(
            snapshot = snapshot,
            chatSessions = state.chatSessions,
            globalApprovals = state.globalApprovals,
            groupBy = AgentGroupBy.STATUS,
            query = state.workspaceQuery,
            needsYouOnly = state.agentsNeedsYouOnly,
        )
    }

    val flatRows = remember(flatProjection) {
        flatProjection.sections.flatMap { it.rows } + flatProjection.idleRows
    }

    val attentionCount = flatProjection.needsYouCount + state.globalApprovals.size
    val botAttentionCount = remember(state.botRuns) {
        state.botRuns.count {
            it.status == BotRunStatus.FAILED || it.status == BotRunStatus.NEEDS_APPROVAL
        }
    }

    val resumeAgentId = if (showResumeFooter && state.selectedAgent == null) {
        state.openTabs.lastOrNull()?.takeIf { id ->
            snapshot?.agents?.any { it.id == id } == true
        }
    } else {
        null
    }
    val resumeAgent = resumeAgentId?.let { id -> snapshot?.agents?.find { it.id == id } }

    Column(modifier.fillMaxSize().background(MuxBg0)) {
        if (showSidebarChrome) {
            DshWorkspaceHeader(
                state = state,
                botsCount = state.bots.size,
                botAttentionCount = botAttentionCount,
                onOpenHostSettings = onOpenDiagnostics,
                onReconnect = model::reconnect,
                onOpenBots = { showBotsSheet = true },
                onHideSidebar = onHideSidebar,
            )
        }

        // Top action strip: New Session button + inline Search + Tree/Flat toggle (dsh SidebarRoot + WorkspaceBrowser)
        Column(
            Modifier
                .fillMaxWidth()
                .background(MuxBg0)
                .padding(horizontal = 12.dp, vertical = 10.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            // Primary New Session row (crisp dsh bordered button)
            Row(
                Modifier
                    .fillMaxWidth()
                    .height(40.dp)
                    .background(MuxBg1, Square)
                    .border(BorderStroke(1.dp, MuxControlLine), Square)
                    .clickable(enabled = state.connected && !snapshot?.projects.isNullOrEmpty()) {
                        val preferredProject = state.selectedAgent?.let { sel ->
                            snapshot?.agents?.find { it.id == sel }?.project
                        } ?: orderedProjects.firstOrNull()?.id
                        onAfterSelectAgent?.invoke()
                        onOpenNewTab(preferredProject)
                    }
                    .padding(horizontal = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Icon(
                    Icons.Outlined.Add,
                    contentDescription = null,
                    tint = if (state.connected) MuxFg0 else MuxFg2,
                    modifier = Modifier.size(17.dp),
                )
                Text(
                    stringResource(R.string.new_session),
                    style = MaterialTheme.typography.labelLarge,
                    color = if (state.connected) MuxFg0 else MuxFg2,
                    modifier = Modifier.weight(1f),
                )
                if (snapshot != null) {
                    Text(
                        "${snapshot.projects.size} 工作区 · ${snapshot.agents.size} 会话",
                        style = MaterialTheme.typography.labelSmall,
                        color = MuxFg2,
                        fontFamily = MonoFamily,
                    )
                }
            }

            // Inline search + compact Tree/Flat switcher
            Row(
                Modifier.fillMaxWidth(),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                Row(
                    Modifier
                        .weight(1f)
                        .height(36.dp)
                        .background(MuxBg1, Square)
                        .border(BorderStroke(1.dp, MuxControlLine), Square)
                        .padding(horizontal = 9.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Icon(Icons.Outlined.Search, null, tint = MuxFg2, modifier = Modifier.size(16.dp))
                    Box(Modifier.weight(1f), contentAlignment = Alignment.CenterStart) {
                        if (state.workspaceQuery.isEmpty()) {
                            Text(
                                "搜索工作区或会话…",
                                style = MaterialTheme.typography.bodySmall,
                                color = MuxFg2,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        BasicTextField(
                            value = state.workspaceQuery,
                            onValueChange = model::setWorkspaceQuery,
                            singleLine = true,
                            textStyle = MaterialTheme.typography.bodySmall.copy(color = MuxFg0),
                            cursorBrush = SolidColor(MuxCobalt),
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                    if (state.workspaceQuery.isNotEmpty()) {
                        Icon(
                            Icons.Outlined.Close,
                            contentDescription = "清空",
                            tint = MuxFg2,
                            modifier = Modifier
                                .size(15.dp)
                                .clickable { model.setWorkspaceQuery("") },
                        )
                    }
                }

                // Segmented Tree / Flat view mode toggle
                Row(
                    Modifier
                        .height(36.dp)
                        .background(MuxBg2, Square)
                        .border(BorderStroke(1.dp, MuxControlLine), Square),
                ) {
                    DshViewModeCell(
                        selected = browseMode == WorkspaceBrowseMode.TREE,
                        icon = Icons.Outlined.Folder,
                        contentDescription = "按工作区",
                        onClick = { browseMode = WorkspaceBrowseMode.TREE },
                    )
                    Box(Modifier.width(1.dp).fillMaxHeight().background(MuxControlLine))
                    DshViewModeCell(
                        selected = browseMode == WorkspaceBrowseMode.FLAT,
                        icon = Icons.AutoMirrored.Outlined.FormatListBulleted,
                        contentDescription = "全部会话",
                        onClick = { browseMode = WorkspaceBrowseMode.FLAT },
                    )
                }

                if (attentionCount > 0 || state.agentsNeedsYouOnly) {
                    Box(
                        Modifier
                            .height(36.dp)
                            .background(if (state.agentsNeedsYouOnly) MuxCobalt else MuxWarningBg, Square)
                            .border(
                                BorderStroke(1.dp, if (state.agentsNeedsYouOnly) MuxCobalt else MuxWarningLine),
                                Square,
                            )
                            .clickable { model.toggleAgentsNeedsYouOnly() }
                            .padding(horizontal = 8.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            "待处理 $attentionCount",
                            style = MaterialTheme.typography.labelSmall,
                            color = if (state.agentsNeedsYouOnly) MuxOnAccent else MuxWarningFg,
                            fontFamily = MonoFamily,
                        )
                    }
                }
            }
        }
        HorizontalDivider(color = MuxLine)

        state.error?.let { err ->
            Box(Modifier.padding(horizontal = 12.dp, vertical = 6.dp)) { ErrorPanel(err) }
        }

        // Main Workspace Browser region
        Box(Modifier.weight(1f).fillMaxWidth()) {
            when {
                state.pairing == null -> EmptyStateBox(Icons.Outlined.Computer, "未选择主机")
                !state.connected && snapshot == null -> {
                    if (state.connecting) {
                        Column(
                            Modifier.fillMaxSize(),
                            verticalArrangement = Arrangement.Center,
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            SquareStepIndicator(color = MuxCobalt)
                            Spacer(Modifier.height(10.dp))
                            Text(stringResource(R.string.connecting_machine), color = MuxFg2)
                        }
                    } else {
                        Column(
                            Modifier.fillMaxSize().padding(24.dp),
                            verticalArrangement = Arrangement.Center,
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            Icon(Icons.Outlined.CloudOff, null, tint = MuxRed, modifier = Modifier.size(32.dp))
                            Spacer(Modifier.height(8.dp))
                            Text(state.error ?: stringResource(R.string.connection_failed_title), color = MuxFg1)
                            Spacer(Modifier.height(12.dp))
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                OutlinedButton(onClick = onOpenDiagnostics, shape = Square) {
                                    Text(stringResource(R.string.host_settings))
                                }
                                Button(
                                    onClick = model::reconnect,
                                    shape = Square,
                                    colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                                ) { Text(stringResource(R.string.retry)) }
                            }
                        }
                    }
                }
                browseMode == WorkspaceBrowseMode.TREE && orderedProjects.isEmpty() -> {
                    EmptyStateBox(
                        icon = Icons.Outlined.Folder,
                        title = if (state.workspaceQuery.isNotBlank()) "未找到匹配的工作区或会话" else "暂无工作区",
                        subtitle = if (state.workspaceQuery.isNotBlank()) "可尝试清空搜索词" else "请先在桌面端添加项目目录",
                    )
                }
                browseMode == WorkspaceBrowseMode.FLAT && flatRows.isEmpty() -> {
                    EmptyStateBox(
                        icon = Icons.Outlined.Terminal,
                        title = if (state.agentsNeedsYouOnly) "暂无待处理会话" else "暂无会话",
                        subtitle = "点击上方「新建会话」直接启动 Pi / Pie / Agy 对话或终端。",
                    )
                }
                browseMode == WorkspaceBrowseMode.TREE -> {
                    LazyColumn(
                        Modifier.fillMaxSize(),
                        contentPadding = PaddingValues(top = 6.dp, bottom = 16.dp),
                    ) {
                        if (state.globalApprovals.isNotEmpty()) {
                            item("dsh_approvals") {
                                Box(Modifier.padding(horizontal = 12.dp, vertical = 6.dp)) {
                                    NeedsYouApprovalSection(
                                        approvals = state.globalApprovals,
                                        snapshot = snapshot,
                                        onOpenAgent = { id ->
                                            onAfterSelectAgent?.invoke()
                                            model.openAgent(id)
                                        },
                                        onApprove = { a, reply -> model.respondApproval(a.agent, a.id, reply) },
                                    )
                                }
                            }
                        }

                        orderedProjects.forEach { project ->
                            val projectAgents = agentsByProject[project.id].orEmpty().sortedWith(
                                compareByDescending<AgentInstance> { ag ->
                                    ag.status == AgentStatus.BLOCKED || state.globalApprovals.any { it.agent == ag.id }
                                }.thenByDescending { ag ->
                                    ag.status == AgentStatus.WORKING || state.chatSessions[ag.id]?.meta?.isStreaming == true
                                }.thenByDescending { it.statusSince },
                            )
                            val hasAttention = projectAgents.any { ag ->
                                ag.status == AgentStatus.BLOCKED || state.globalApprovals.any { it.agent == ag.id }
                            }
                            val hasWorking = projectAgents.any { ag ->
                                ag.status == AgentStatus.WORKING || state.chatSessions[ag.id]?.meta?.isStreaming == true
                            }
                            val isCollapsed = project.id in state.collapsedProjects
                            val showAll = expandedWatermarks[project.id] == true || state.workspaceQuery.isNotBlank()
                            val visibleAgents = if (isCollapsed) {
                                emptyList()
                            } else if (showAll || projectAgents.size <= WORKSPACE_SESSION_WATERMARK) {
                                projectAgents
                            } else {
                                // Keep attention/working/selected sessions always visible, plus up to watermark
                                val required = projectAgents.filterIndexed { index, ag ->
                                    index < WORKSPACE_SESSION_WATERMARK ||
                                        ag.id == state.selectedAgent ||
                                        ag.status == AgentStatus.WORKING ||
                                        ag.status == AgentStatus.BLOCKED ||
                                        state.globalApprovals.any { it.agent == ag.id }
                                }
                                required
                            }
                            val hiddenCount = if (isCollapsed) 0 else (projectAgents.size - visibleAgents.size).coerceAtLeast(0)

                            item(key = "ws_${project.id}") {
                                DshWorkspaceRow(
                                    project = project,
                                    sessionCount = projectAgents.size,
                                    collapsed = isCollapsed,
                                    active = hasWorking || hasAttention,
                                    hasAttention = hasAttention,
                                    canCreate = state.connected,
                                    onToggle = { model.toggleProjectCollapsed(project.id) },
                                    onNewSession = {
                                        onAfterSelectAgent?.invoke()
                                        onOpenNewTab(project.id)
                                    },
                                )
                            }

                            if (!isCollapsed && projectAgents.isEmpty()) {
                                item(key = "ws_empty_${project.id}") {
                                    Row(
                                        Modifier
                                            .fillMaxWidth()
                                            .clickable(enabled = state.connected) {
                                                onAfterSelectAgent?.invoke()
                                                onOpenNewTab(project.id)
                                            }
                                            .padding(start = 34.dp, end = 12.dp, top = 4.dp, bottom = 8.dp),
                                        verticalAlignment = Alignment.CenterVertically,
                                    ) {
                                        Text(
                                            "暂无会话 · 点击新建",
                                            style = MaterialTheme.typography.labelSmall,
                                            color = MuxFg2,
                                        )
                                    }
                                }
                            }

                            items(visibleAgents, key = { "sess_${it.id}" }) { agent ->
                                val chat = state.chatSessions[agent.id]
                                val approvalsCount = maxOf(
                                    chat?.pendingApprovals ?: 0,
                                    state.globalApprovals.count { it.agent == agent.id },
                                )
                                val isChatOnly = isChatOnlySession(agent)
                                val driverLabel = driverBadgeLabel(
                                    agent.agentType,
                                    if (isChatOnly) chat?.meta?.driver ?: SessionDriverMode.PI_RPC else SessionDriverMode.TERMINAL_PTY,
                                )
                                DshSessionRow(
                                    agent = agent,
                                    subtitle = null,
                                    driverLabel = driverLabel,
                                    isStreaming = chat?.meta?.isStreaming == true,
                                    pendingApprovalsCount = approvalsCount,
                                    selected = state.selectedAgent == agent.id,
                                    nested = true,
                                    onClick = {
                                        onAfterSelectAgent?.invoke()
                                        model.openAgent(agent.id)
                                    },
                                    onDelete = { model.requestDelete(agent.id) },
                                )
                            }

                            if (hiddenCount > 0) {
                                item(key = "ws_more_${project.id}") {
                                    Row(
                                        Modifier
                                            .fillMaxWidth()
                                            .clickable { expandedWatermarks[project.id] = true }
                                            .padding(start = 34.dp, end = 12.dp, top = 6.dp, bottom = 8.dp),
                                        verticalAlignment = Alignment.CenterVertically,
                                    ) {
                                        Text(
                                            "展开其余 $hiddenCount 个会话",
                                            style = MaterialTheme.typography.labelSmall,
                                            color = MuxCobalt,
                                        )
                                    }
                                }
                            }
                        }
                    }
                }
                else -> {
                    // Flat recency list (same crisp dsh rows, zero duplicate screen)
                    LazyColumn(
                        Modifier.fillMaxSize(),
                        contentPadding = PaddingValues(top = 6.dp, bottom = 16.dp),
                    ) {
                        if (state.globalApprovals.isNotEmpty()) {
                            item("flat_approvals") {
                                Box(Modifier.padding(horizontal = 12.dp, vertical = 6.dp)) {
                                    NeedsYouApprovalSection(
                                        approvals = state.globalApprovals,
                                        snapshot = snapshot,
                                        onOpenAgent = { id ->
                                            onAfterSelectAgent?.invoke()
                                            model.openAgent(id)
                                        },
                                        onApprove = { a, reply -> model.respondApproval(a.agent, a.id, reply) },
                                    )
                                }
                            }
                        }
                        items(flatRows, key = { "flat_${it.agent.id}" }) { row ->
                            DshSessionRow(
                                agent = row.agent,
                                subtitle = row.projectName,
                                driverLabel = row.driverLabel,
                                isStreaming = row.isStreaming,
                                pendingApprovalsCount = row.pendingApprovalsCount,
                                selected = state.selectedAgent == row.agent.id,
                                nested = false,
                                onClick = {
                                    onAfterSelectAgent?.invoke()
                                    model.openAgent(row.agent.id)
                                },
                                onDelete = { model.requestDelete(row.agent.id) },
                            )
                        }
                    }
                }
            }
        }

        // Bottom pinned quick-resume strip (only when browsing Home while a session is open)
        if (resumeAgent != null) {
            val resumeProject = snapshot?.projects?.find { it.id == resumeAgent.project }
            Surface(color = MuxBg1, contentColor = MuxFg0) {
                Column(Modifier.fillMaxWidth().navigationBarsPadding()) {
                    HorizontalDivider(color = MuxLine)
                    Row(
                        Modifier
                            .fillMaxWidth()
                            .clickable { model.openAgent(resumeAgent.id) }
                            .padding(horizontal = 14.dp, vertical = 11.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        Box(
                            Modifier
                                .size(7.dp)
                                .background(
                                    if (resumeAgent.status == AgentStatus.WORKING) MuxCobalt else MuxGreen,
                                    Square,
                                ),
                        )
                        Column(Modifier.weight(1f)) {
                            Text(
                                "返回会话 · ${resumeAgent.title}",
                                style = MaterialTheme.typography.labelLarge,
                                color = MuxFg0,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            resumeProject?.let {
                                Text(
                                    it.displayName(),
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MuxFg2,
                                    fontFamily = MonoFamily,
                                )
                            }
                        }
                        Icon(
                            Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                            contentDescription = null,
                            tint = MuxFg2,
                            modifier = Modifier.size(18.dp),
                        )
                    }
                }
            }
        } else if (showSidebarChrome && onHideSidebar == null) {
            Spacer(Modifier.navigationBarsPadding())
        }
    }

    if (showBotsSheet) {
        BotsSheet(
            state = state,
            onTrigger = model::triggerBot,
            onOpenDetail = { bot ->
                showBotsSheet = false
                detailBot = bot
            },
            onDismiss = { showBotsSheet = false },
        )
    }

    detailBot?.let { bot ->
        BotDetailSheet(
            bot = bot,
            runs = state.botRuns.filter { it.botId == bot.id },
            liveAgentIds = snapshot?.agents?.map { it.id }?.toSet().orEmpty(),
            onTrigger = { model.triggerBot(bot.id) },
            onOpenLiveAgent = { agentId ->
                detailBot = null
                onAfterSelectAgent?.invoke()
                model.openAgent(agentId)
            },
            onDismiss = { detailBot = null },
        )
    }
}

@Composable
private fun DshWorkspaceHeader(
    state: UiState,
    botsCount: Int,
    botAttentionCount: Int,
    onOpenHostSettings: () -> Unit,
    onReconnect: () -> Unit,
    onOpenBots: () -> Unit,
    onHideSidebar: (() -> Unit)?,
) {
    val activeHost = state.pairing
    val dotColor = when {
        state.connected -> MuxGreen
        state.connecting -> MuxYellow
        else -> MuxRed
    }
    Surface(color = MuxBg1, contentColor = MuxFg0) {
        Column(Modifier.fillMaxWidth().statusBarsPadding()) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .height(52.dp)
                    .padding(horizontal = 12.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Box(
                    Modifier
                        .size(26.dp)
                        .background(MuxFg0, Square),
                    contentAlignment = Alignment.Center,
                ) {
                    Text(
                        "M",
                        color = MuxBg1,
                        fontFamily = MonoFamily,
                        fontWeight = FontWeight.Bold,
                        fontSize = 13.sp,
                    )
                }

                // Host selector trigger (opens Host/Settings sheet to switch or add machine)
                Column(
                    Modifier
                        .weight(1f)
                        .clickable(onClick = onOpenHostSettings)
                        .padding(vertical = 4.dp),
                ) {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Text(
                            activeHost?.machineName ?: activeHost?.hostId ?: stringResource(R.string.app_name),
                            style = MaterialTheme.typography.titleMedium,
                            color = MuxFg0,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        Box(Modifier.size(6.dp).background(dotColor, Square))
                    }
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(6.dp),
                    ) {
                        Text(
                            buildString {
                                append(
                                    when {
                                        state.connected -> if (state.usingLan) "局域网" else "中继在线"
                                        state.connecting -> "连接中"
                                        else -> "未连接"
                                    },
                                )
                                if (state.connected && state.e2eeActive) append(" · E2EE")
                                if (state.pairings.size > 1) append(" · ${state.pairings.size} 主机")
                            },
                            style = MaterialTheme.typography.labelSmall,
                            color = MuxFg2,
                            fontFamily = MonoFamily,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                    }
                }

                if (!state.connected && activeHost != null) {
                    IconButton(onClick = onReconnect, modifier = Modifier.size(36.dp)) {
                        Icon(Icons.Outlined.Refresh, stringResource(R.string.retry), tint = MuxFg1, modifier = Modifier.size(19.dp))
                    }
                }

                if (botsCount > 0 || botAttentionCount > 0) {
                    Box {
                        IconButton(onClick = onOpenBots, modifier = Modifier.size(36.dp)) {
                            Icon(Icons.Outlined.SmartToy, stringResource(R.string.tab_bots), tint = MuxFg1, modifier = Modifier.size(19.dp))
                        }
                        if (botAttentionCount > 0) {
                            Box(
                                Modifier
                                    .align(Alignment.TopEnd)
                                    .padding(top = 6.dp, end = 6.dp)
                                    .size(7.dp)
                                    .background(MuxYellow, Square),
                            )
                        }
                    }
                }

                IconButton(onClick = onOpenHostSettings, modifier = Modifier.size(36.dp)) {
                    Icon(Icons.Outlined.Settings, stringResource(R.string.host_settings), tint = MuxFg1, modifier = Modifier.size(19.dp))
                }

                if (onHideSidebar != null) {
                    IconButton(onClick = onHideSidebar, modifier = Modifier.size(36.dp)) {
                        Icon(Icons.Outlined.Close, "关闭侧栏", tint = MuxFg1, modifier = Modifier.size(19.dp))
                    }
                }
            }
            HorizontalDivider(color = MuxLine)
        }
    }
}

@Composable
private fun DshViewModeCell(
    selected: Boolean,
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    contentDescription: String,
    onClick: () -> Unit,
) {
    Box(
        Modifier
            .width(34.dp)
            .fillMaxHeight()
            .background(if (selected) MuxBg1 else MuxBg2, Square)
            .clickable(onClick = onClick),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            icon,
            contentDescription = contentDescription,
            tint = if (selected) MuxFg0 else MuxFg2,
            modifier = Modifier.size(16.dp),
        )
    }
}

@Composable
private fun DshWorkspaceRow(
    project: Project,
    sessionCount: Int,
    collapsed: Boolean,
    active: Boolean,
    hasAttention: Boolean,
    canCreate: Boolean,
    onToggle: () -> Unit,
    onNewSession: () -> Unit,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .clickable(onClick = onToggle)
            .padding(start = 10.dp, end = 8.dp, top = 7.dp, bottom = 7.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        Icon(
            if (collapsed) Icons.AutoMirrored.Outlined.KeyboardArrowRight else Icons.Outlined.KeyboardArrowDown,
            contentDescription = null,
            tint = MuxFg2,
            modifier = Modifier.size(16.dp),
        )
        Icon(
            if (collapsed) Icons.Outlined.Folder else Icons.Outlined.FolderOpen,
            contentDescription = null,
            tint = when {
                hasAttention -> MuxYellow
                active -> MuxCobalt
                else -> MuxFg2
            },
            modifier = Modifier.size(17.dp),
        )
        Row(
            Modifier.weight(1f),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Text(
                project.displayName(),
                style = MaterialTheme.typography.labelLarge,
                color = MuxFg0,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f, fill = false),
            )
            project.branch?.takeIf { it.isNotBlank() }?.let { branch ->
                Text(
                    branch,
                    style = MaterialTheme.typography.labelSmall,
                    color = MuxFg2,
                    fontFamily = MonoFamily,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    modifier = Modifier.widthIn(max = 88.dp),
                )
            }
        }
        if (collapsed && sessionCount > 0) {
            Text(
                sessionCount.toString(),
                style = MaterialTheme.typography.labelSmall,
                color = MuxFg2,
                fontFamily = MonoFamily,
            )
        }
        Box(
            Modifier
                .size(28.dp)
                .clickable(enabled = canCreate, onClick = onNewSession),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                Icons.Outlined.Add,
                contentDescription = "在 ${project.displayName()} 中新建会话",
                tint = if (canCreate) MuxFg2 else MuxLine,
                modifier = Modifier.size(16.dp),
            )
        }
    }
}

@Composable
private fun DshSessionRow(
    agent: AgentInstance,
    subtitle: String?,
    driverLabel: String,
    isStreaming: Boolean,
    pendingApprovalsCount: Int,
    selected: Boolean,
    nested: Boolean,
    onClick: () -> Unit,
    onDelete: () -> Unit,
) {
    val isWaiting = pendingApprovalsCount > 0 || agent.status == AgentStatus.BLOCKED
    val isWorking = isStreaming || agent.status == AgentStatus.WORKING
    val dotColor = when {
        isWaiting -> MuxYellow
        isWorking -> MuxCobalt
        agent.status == AgentStatus.DONE -> MuxGreen
        else -> MuxControlLine
    }
    val rowBg = if (selected) MuxBg3 else Color.Transparent

    Row(
        Modifier
            .fillMaxWidth()
            .background(rowBg, Square)
            .clickable(onClick = onClick)
            .padding(
                start = if (nested) 22.dp else 12.dp,
                end = 8.dp,
                top = 8.dp,
                bottom = 8.dp,
            ),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        if (nested) {
            Box(
                Modifier
                    .width(if (selected) 2.dp else 1.dp)
                    .height(22.dp)
                    .background(if (selected) MuxCobalt else MuxLine),
            )
        }
        Box(Modifier.size(7.dp).background(dotColor, Square))
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(1.dp)) {
            Text(
                agent.title.ifBlank { agent.id },
                style = MaterialTheme.typography.bodyMedium,
                fontWeight = if (selected || isWorking || isWaiting) FontWeight.Medium else FontWeight.Normal,
                color = MuxFg0,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Row(
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(6.dp),
            ) {
                if (!subtitle.isNullOrBlank()) {
                    Text(
                        subtitle,
                        style = MaterialTheme.typography.labelSmall,
                        color = MuxFg2,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    Text("·", style = MaterialTheme.typography.labelSmall, color = MuxControlLine)
                }
                Text(
                    driverLabel,
                    style = MaterialTheme.typography.labelSmall,
                    color = if (isWorking) MuxCobalt else MuxFg2,
                    fontFamily = MonoFamily,
                )
                if (isWaiting) {
                    Text(
                        if (pendingApprovalsCount > 0) "待回复 $pendingApprovalsCount" else "待确认",
                        style = MaterialTheme.typography.labelSmall,
                        color = MuxWarningFg,
                    )
                } else {
                    val since = formatRelativeAge(agent.statusSince, System.currentTimeMillis())
                    if (since.isNotEmpty()) {
                        Text(since, style = MaterialTheme.typography.labelSmall, color = MuxFg2, fontFamily = MonoFamily)
                    }
                }
            }
        }
        Box(
            Modifier
                .size(28.dp)
                .clickable(onClick = onDelete),
            contentAlignment = Alignment.Center,
        ) {
            Icon(
                Icons.Outlined.DeleteOutline,
                contentDescription = stringResource(R.string.delete),
                tint = MuxFg2.copy(alpha = 0.7f),
                modifier = Modifier.size(16.dp),
            )
        }
    }
}

@Composable
private fun BotsSheet(
    state: UiState,
    onTrigger: (String) -> Unit,
    onOpenDetail: (BotDefinitionSummary) -> Unit,
    onDismiss: () -> Unit,
) {
    MuxSheetFrame(
        onDismiss = onDismiss,
        title = stringResource(R.string.bots_section),
        subtitle = "${state.bots.size} 个自动化任务",
    ) {
        if (state.bots.isEmpty()) {
            EmptyStateBox(
                icon = Icons.Outlined.SmartToy,
                title = "暂无定时任务 Bot",
                subtitle = "可在桌面端创建自动化 Bot，完成后这里会自动同步运行记录。",
                modifier = Modifier.height(220.dp),
            )
        } else {
            Column(
                Modifier
                    .fillMaxWidth()
                    .heightIn(max = 460.dp)
                    .verticalScroll(rememberScrollState()),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                state.bots.forEach { bot ->
                    val latestRun = state.botRuns.firstOrNull { it.botId == bot.id }
                    BotHubCard(
                        bot = bot,
                        latestRun = latestRun,
                        connected = state.connected,
                        onTrigger = { onTrigger(bot.id) },
                        onOpenDetail = { onOpenDetail(bot) },
                    )
                }
            }
        }
    }
}

@Composable
private fun BotHubCard(
    bot: BotDefinitionSummary,
    latestRun: BotRunRecord?,
    connected: Boolean,
    onTrigger: () -> Unit,
    onOpenDetail: () -> Unit,
) {
    Surface(
        modifier = Modifier.fillMaxWidth().clickable(onClick = onOpenDetail),
        shape = Square,
        color = MuxBg1,
        border = BorderStroke(1.dp, MuxLine),
    ) {
        Row(
            Modifier.fillMaxWidth().padding(12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Box(
                Modifier
                    .size(34.dp)
                    .background(MuxBg2, Square)
                    .border(BorderStroke(1.dp, MuxLine), Square),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Outlined.SmartToy, null, tint = if (bot.enabled) MuxCobalt else MuxFg2, modifier = Modifier.size(18.dp))
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(bot.name, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Text(
                    buildString {
                        append(bot.agentType.name.lowercase())
                        append(" · ${bot.permission}")
                        latestRun?.let { append(" · ${botRunStatusLabel(it.status)}") }
                    },
                    style = MaterialTheme.typography.labelSmall,
                    color = MuxFg2,
                    fontFamily = MonoFamily,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            OutlinedButton(
                onClick = onTrigger,
                enabled = connected,
                shape = Square,
                border = BorderStroke(1.dp, MuxControlLine),
                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                modifier = Modifier.height(34.dp),
            ) {
                Icon(Icons.Outlined.PlayArrow, null, tint = MuxFg0, modifier = Modifier.size(15.dp))
                Spacer(Modifier.width(3.dp))
                Text("运行", color = MuxFg0, fontSize = 12.sp)
            }
        }
    }
}

@Composable
private fun BotDetailSheet(
    bot: BotDefinitionSummary,
    runs: List<BotRunRecord>,
    liveAgentIds: Set<String>,
    onTrigger: () -> Unit,
    onOpenLiveAgent: (String) -> Unit,
    onDismiss: () -> Unit,
) {
    MuxSheetFrame(
        onDismiss = onDismiss,
        title = bot.name,
        subtitle = "${bot.agentType.name} · ${bot.permission}",
    ) {
        Column(
            Modifier
                .fillMaxWidth()
                .heightIn(max = 520.dp)
                .verticalScroll(rememberScrollState()),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text(
                buildString {
                    append("代理: ${bot.agentType.name.lowercase()}")
                    if (bot.isolatedWorktree) append(" · git worktree")
                    append(" · ${if (bot.enabled) "已启用" else "已暂停"}")
                },
                style = MaterialTheme.typography.labelMedium,
                color = MuxFg2,
                fontFamily = MonoFamily,
            )
            Button(
                onClick = onTrigger,
                shape = Square,
                colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                modifier = Modifier.fillMaxWidth().height(42.dp),
            ) {
                Icon(Icons.Outlined.PlayArrow, null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text("立即运行")
            }
            if (runs.isNotEmpty()) {
                Text("最近执行 (${runs.size})", style = MaterialTheme.typography.labelLarge, color = MuxFg0)
                runs.take(8).forEach { run ->
                    Surface(shape = Square, color = MuxBg0, border = BorderStroke(1.dp, MuxLine)) {
                        Column(Modifier.fillMaxWidth().padding(10.dp), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
                                Text(
                                    botRunStatusLabel(run.status),
                                    style = MaterialTheme.typography.labelMedium,
                                    color = botRunStatusColor(run.status),
                                    fontFamily = MonoFamily,
                                )
                                Text(run.triggerSource, style = MaterialTheme.typography.labelSmall, color = MuxFg2, fontFamily = MonoFamily)
                            }
                            (run.summary ?: run.error)?.takeIf { it.isNotBlank() }?.let {
                                Text(it, style = MaterialTheme.typography.bodySmall, color = MuxFg1, maxLines = 3, overflow = TextOverflow.Ellipsis)
                            }
                            run.agentId?.takeIf { it in liveAgentIds }?.let { liveId ->
                                TextButton(
                                    onClick = { onOpenLiveAgent(liveId) },
                                    shape = Square,
                                    contentPadding = PaddingValues(0.dp),
                                ) { Text("打开运行会话 →", color = MuxCobalt) }
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun NeedsYouApprovalSection(
    approvals: List<PendingApproval>,
    snapshot: muxlane.protocol.Snapshot?,
    onOpenAgent: (String) -> Unit,
    onApprove: (PendingApproval, ApprovalReply) -> Unit,
) {
    Surface(shape = Square, color = MuxBg1, border = BorderStroke(1.dp, MuxControlLine)) {
        Column(Modifier.fillMaxWidth()) {
            Box(Modifier.fillMaxWidth().height(2.dp).background(MuxCobalt))
            Column(Modifier.fillMaxWidth().padding(10.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Icon(Icons.Outlined.NotificationsActive, null, tint = MuxCobalt, modifier = Modifier.size(16.dp))
                    Text(
                        "${stringResource(R.string.needs_you)} (${approvals.size})",
                        style = MaterialTheme.typography.labelLarge,
                        color = MuxFg0,
                    )
                }
                approvals.take(3).forEach { approval ->
                    val agentName = snapshot?.agents?.find { it.id == approval.agent }?.title ?: approval.agent
                    val parsed = parseApprovalPrompt(approval)
                    Surface(shape = Square, color = MuxBg0, border = BorderStroke(1.dp, MuxLine)) {
                        Column(Modifier.fillMaxWidth().padding(9.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                            Row(
                                verticalAlignment = Alignment.CenterVertically,
                                horizontalArrangement = Arrangement.spacedBy(6.dp),
                            ) {
                                Box(
                                    Modifier
                                        .background(if (parsed.isQuestion) MuxAccentBg else MuxWarningBg, Square)
                                        .border(
                                            BorderStroke(
                                                1.dp,
                                                if (parsed.isQuestion) MuxCobalt.copy(alpha = 0.28f) else MuxWarningLine,
                                            ),
                                            Square,
                                        )
                                        .padding(horizontal = 6.dp, vertical = 1.dp),
                                ) {
                                    Text(
                                        parsed.badge,
                                        style = MaterialTheme.typography.labelSmall,
                                        color = if (parsed.isQuestion) MuxCobalt else MuxWarningFg,
                                    )
                                }
                                Text(
                                    "$agentName · ${parsed.questionTitle}",
                                    style = MaterialTheme.typography.labelMedium,
                                    color = MuxFg0,
                                    maxLines = 2,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            }
                            if (parsed.isQuestion || parsed.useListLayout) {
                                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                    Button(
                                        onClick = { onOpenAgent(approval.agent) },
                                        shape = Square,
                                        colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                                        contentPadding = PaddingValues(horizontal = 12.dp, vertical = 4.dp),
                                        modifier = Modifier.height(32.dp),
                                    ) { Text("前往回答", fontSize = 12.sp) }
                                    OutlinedButton(
                                        onClick = { onApprove(approval, ApprovalReply.Cancel) },
                                        shape = Square,
                                        border = BorderStroke(1.dp, MuxControlLine),
                                        contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                                        modifier = Modifier.height(32.dp),
                                    ) { Text("取消", color = MuxFg1, fontSize = 12.sp) }
                                }
                            } else {
                                val allowOption = approval.options.firstOrNull {
                                    val k = it.kind.orEmpty().lowercase()
                                    "reject" !in k && "deny" !in k
                                }
                                val denyOption = approval.options.firstOrNull {
                                    val k = it.kind.orEmpty().lowercase()
                                    "reject" in k || "deny" in k
                                }
                                val allowReply = allowOption?.let { ApprovalReply.SelectedOption(it.optionId) } ?: ApprovalReply.Confirm(true)
                                val denyReply = denyOption?.let { ApprovalReply.SelectedOption(it.optionId) } ?: ApprovalReply.Confirm(false)
                                Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                    Button(
                                        onClick = { onApprove(approval, allowReply) },
                                        shape = Square,
                                        colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                                        contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                                        modifier = Modifier.height(32.dp),
                                    ) { Text(allowOption?.label ?: stringResource(R.string.approve_allow), fontSize = 12.sp) }
                                    OutlinedButton(
                                        onClick = { onApprove(approval, denyReply) },
                                        shape = Square,
                                        border = BorderStroke(1.dp, MuxRed),
                                        contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                                        modifier = Modifier.height(32.dp),
                                    ) { Text(denyOption?.label ?: stringResource(R.string.approve_deny), color = MuxRed, fontSize = 12.sp) }
                                    Spacer(Modifier.weight(1f))
                                    TextButton(
                                        onClick = { onOpenAgent(approval.agent) },
                                        shape = Square,
                                        modifier = Modifier.height(32.dp),
                                    ) { Text("查看", fontSize = 12.sp, color = MuxCobalt) }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

private fun botRunStatusLabel(status: BotRunStatus): String = when (status) {
    BotRunStatus.QUEUED -> "排队中"
    BotRunStatus.RUNNING -> "运行中"
    BotRunStatus.NEEDS_APPROVAL -> "等待审批"
    BotRunStatus.SUCCEEDED -> "成功"
    BotRunStatus.FAILED -> "失败"
    BotRunStatus.TIMED_OUT -> "超时"
    BotRunStatus.CANCELLED -> "已取消"
}

private fun botRunStatusColor(status: BotRunStatus): Color = when (status) {
    BotRunStatus.RUNNING -> MuxCobalt
    BotRunStatus.NEEDS_APPROVAL -> MuxYellow
    BotRunStatus.SUCCEEDED -> MuxGreen
    BotRunStatus.FAILED, BotRunStatus.TIMED_OUT -> MuxRed
    else -> MuxFg2
}

internal fun applyPairText(model: MuxlaneViewModel, text: String) {
    val value = text.trim()
    parsePairLink(value)?.let { (relay, hostId) ->
        model.setRelay(relay)
        model.setHostId(hostId)
        return
    }
    Regex("""wss?://\S+""").find(value)?.value?.trimEnd(',', ';')?.let(model::setRelay)
    Regex("""machine_[A-Za-z0-9]+""").find(value)?.value?.let(model::setHostId)
}
