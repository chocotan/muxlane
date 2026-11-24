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
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.CloudOff
import androidx.compose.material.icons.outlined.Computer
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.outlined.NotificationsActive
import androidx.compose.material.icons.outlined.PlayArrow
import androidx.compose.material.icons.outlined.QrCodeScanner
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Security
import androidx.compose.material.icons.outlined.Settings
import androidx.compose.material.icons.outlined.SmartToy
import androidx.compose.material.icons.outlined.Tab
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
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
import muxlane.protocol.AgentInstance
import muxlane.protocol.AgentStatus
import muxlane.protocol.BotDefinitionSummary
import muxlane.protocol.BotRunRecord
import muxlane.protocol.BotRunStatus
import muxlane.protocol.Project
import muxlane.session.Pairing

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
            .background(MuxBg0)
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
            Surface(shape = Square, color = MuxFg0, modifier = Modifier.size(44.dp)) {
                Box(contentAlignment = Alignment.Center) {
                    Icon(Icons.Outlined.Terminal, null, tint = MuxBg1)
                }
            }
            Spacer(Modifier.width(12.dp))
            Column {
                Text(stringResource(R.string.app_name), style = MaterialTheme.typography.titleLarge)
                Text("Pi RPC · 通用 ACP · 移动工作台", style = MaterialTheme.typography.labelSmall, color = MuxFg2)
            }
        }
        Spacer(Modifier.height(24.dp))
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
            Row(Modifier.fillMaxWidth().padding(vertical = 6.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
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
            Spacer(Modifier.height(12.dp))
            Button(
                onClick = model::pair,
                enabled = canPair,
                shape = Square,
                colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                modifier = Modifier.fillMaxWidth().height(50.dp),
            ) {
                if (state.connecting) SquareStepIndicator(color = Color.White)
                else Text(stringResource(R.string.pair_secure))
            }
        }
    }
}

@Composable
internal fun HomeHubScaffold(
    model: MuxlaneViewModel,
    state: UiState,
    onOpenNewTab: (projectId: String?) -> Unit,
    onOpenDiagnostics: () -> Unit,
) {
    val projection = remember(
        state.snapshot,
        state.chatSessions,
        state.globalApprovals,
        state.agentGroupBy,
        state.workspaceQuery,
        state.agentsNeedsYouOnly,
    ) {
        buildAgentListProjection(
            snapshot = state.snapshot,
            chatSessions = state.chatSessions,
            globalApprovals = state.globalApprovals,
            groupBy = state.agentGroupBy,
            query = state.workspaceQuery,
            needsYouOnly = state.agentsNeedsYouOnly,
        )
    }
    val botAttentionCount = remember(state.bots, state.botRuns) {
        val failedOrApprovalRuns = state.botRuns.count {
            it.status == BotRunStatus.FAILED || it.status == BotRunStatus.NEEDS_APPROVAL
        }
        failedOrApprovalRuns
    }
    var detailBot by remember { mutableStateOf<BotDefinitionSummary?>(null) }

    BackHandler(state.hubTab != HubTab.FEED) {
        model.setHubTab(HubTab.FEED)
    }

    Scaffold(
        containerColor = MuxBg0,
        topBar = {
            HomeHubHeader(
                state = state,
                onResumeTabs = {
                    val target = state.openTabs.lastOrNull() ?: return@HomeHubHeader
                    model.openAgent(target)
                },
                onReconnect = model::reconnect,
                onOpenDiagnostics = onOpenDiagnostics,
            )
        },
        bottomBar = {
            MuxBottomTabBar(
                activeTab = state.hubTab,
                needsYouCount = projection.needsYouCount + state.globalApprovals.size,
                botAttentionCount = botAttentionCount,
                onSelect = model::setHubTab,
            )
        },
    ) { padding ->
        Box(
            Modifier
                .fillMaxSize()
                .padding(padding),
            contentAlignment = Alignment.TopCenter,
        ) {
            Column(Modifier.fillMaxSize().widthIn(max = 860.dp)) {
                state.error?.let { err ->
                    Box(Modifier.padding(horizontal = 14.dp, vertical = 6.dp)) { ErrorPanel(err) }
                }
                when (state.hubTab) {
                    HubTab.FEED -> FeedHubContent(
                        model = model,
                        state = state,
                        projection = projection,
                        onOpenNewTab = { onOpenNewTab(null) },
                        onOpenBotDetail = { detailBot = it },
                    )
                    HubTab.WORKSPACES -> WorkspacesPane(
                        model = model,
                        state = state,
                        showSidebarChrome = false,
                        onOpenNewTab = onOpenNewTab,
                        onOpenDiagnostics = onOpenDiagnostics,
                    )
                    HubTab.AGENTS -> AgentsInboxContent(
                        model = model,
                        state = state,
                        projection = projection,
                        onOpenNewTab = { onOpenNewTab(null) },
                    )
                    HubTab.BOTS -> BotsHubContent(
                        model = model,
                        state = state,
                        onOpenBotDetail = { detailBot = it },
                    )
                }
            }
        }
    }

    detailBot?.let { bot ->
        BotDetailSheet(
            bot = bot,
            runs = state.botRuns.filter { it.botId == bot.id },
            liveAgentIds = state.snapshot?.agents?.map { it.id }?.toSet().orEmpty(),
            onTrigger = { model.triggerBot(bot.id) },
            onOpenLiveAgent = { agentId ->
                detailBot = null
                model.openAgent(agentId)
            },
            onDismiss = { detailBot = null },
        )
    }
}

@Composable
private fun HomeHubHeader(
    state: UiState,
    onResumeTabs: () -> Unit,
    onReconnect: () -> Unit,
    onOpenDiagnostics: () -> Unit,
) {
    val activeHost = state.pairing
    Surface(color = MuxBg1, contentColor = MuxFg0) {
        Column(Modifier.fillMaxWidth().statusBarsPadding()) {
            Row(
                Modifier
                    .fillMaxWidth()
                    .height(54.dp)
                    .padding(horizontal = 14.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                Column(Modifier.weight(1f)) {
                    Text(
                        when (state.hubTab) {
                            HubTab.FEED -> stringResource(R.string.app_name)
                            HubTab.WORKSPACES -> activeHost?.machineName ?: stringResource(R.string.tab_workspaces)
                            HubTab.AGENTS -> stringResource(R.string.tab_agents)
                            HubTab.BOTS -> stringResource(R.string.tab_bots)
                        },
                        style = MaterialTheme.typography.titleLarge,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    activeHost?.let { host ->
                        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                            Text(
                                host.machineName ?: host.hostId,
                                style = MaterialTheme.typography.labelSmall,
                                color = MuxFg2,
                                fontFamily = MonoFamily,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            ConnectionBadgesRow(
                                connected = state.connected,
                                connecting = state.connecting,
                                e2eeActive = state.e2eeActive,
                                usingLan = state.usingLan,
                            )
                        }
                    }
                }
                if (state.openTabs.isNotEmpty()) {
                    Row(
                        Modifier
                            .background(MuxAccentBg, Square)
                            .border(BorderStroke(1.dp, MuxAccent), Square)
                            .clickable(onClick = onResumeTabs)
                            .padding(horizontal = 8.dp, vertical = 6.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(4.dp),
                    ) {
                        Icon(Icons.Outlined.Tab, null, tint = MuxAccent, modifier = Modifier.size(15.dp))
                        Text(
                            "${state.openTabs.size} 标签",
                            style = MaterialTheme.typography.labelSmall,
                            color = MuxAccent,
                            fontFamily = MonoFamily,
                        )
                    }
                }
                if (!state.connected && activeHost != null) {
                    IconButton(onClick = onReconnect, modifier = Modifier.size(38.dp)) {
                        Icon(Icons.Outlined.Refresh, stringResource(R.string.retry), tint = MuxFg1, modifier = Modifier.size(20.dp))
                    }
                }
                IconButton(onClick = onOpenDiagnostics, modifier = Modifier.size(38.dp)) {
                    Icon(Icons.Outlined.Settings, stringResource(R.string.host_settings), tint = MuxFg1, modifier = Modifier.size(20.dp))
                }
            }
            HorizontalDivider(color = MuxLine)
        }
    }
}

@Composable
private fun FeedHubContent(
    model: MuxlaneViewModel,
    state: UiState,
    projection: AgentListProjection,
    onOpenNewTab: () -> Unit,
    onOpenBotDetail: (BotDefinitionSummary) -> Unit,
) {
    val activeRows = remember(projection) {
        projection.sections.flatMap { it.rows }.take(4)
    }
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(14.dp),
        verticalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        if (state.pairing != null && state.snapshot == null && state.connecting) {
            item("connecting") {
                Surface(shape = Square, color = MuxBg1, border = BorderStroke(1.dp, MuxLine)) {
                    Row(
                        Modifier.fillMaxWidth().padding(14.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        SquareStepIndicator()
                        Text("正在同步 ${state.pairing.machineName ?: state.pairing.hostId} 工作区…", style = MaterialTheme.typography.bodyMedium)
                    }
                }
            }
        }

        if (state.globalApprovals.isNotEmpty()) {
            item("feed_approvals") {
                NeedsYouApprovalSection(
                    approvals = state.globalApprovals,
                    snapshot = state.snapshot,
                    onOpenAgent = model::openAgent,
                    onApprove = { a, reply -> model.respondApproval(a.agent, a.id, reply) },
                )
            }
        }

        if (activeRows.isNotEmpty()) {
            item("feed_active_heading") {
                MuxSectionHeading(
                    title = "进行中与最近动态",
                    count = projection.totalCount,
                    aside = stringResource(R.string.see_all),
                    onAsideClick = { model.setHubTab(HubTab.AGENTS) },
                )
            }
            items(activeRows, key = { "feed_agent_${it.agent.id}" }) { row ->
                AgentHubRowCard(
                    row = row,
                    isActiveTab = state.selectedAgent == row.agent.id,
                    onOpen = { model.openAgent(row.agent.id) },
                    onDelete = { model.requestDelete(row.agent.id) },
                )
            }
        } else if (state.snapshot != null) {
            item("feed_start_prompt") {
                Surface(shape = Square, color = MuxBg1, border = BorderStroke(1.dp, MuxControlLine)) {
                    Row(
                        Modifier.fillMaxWidth().padding(14.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                            Text("准备就绪", style = MaterialTheme.typography.titleMedium)
                            Text("打开项目会话或直接启动 Pi / ACP 对话。", style = MaterialTheme.typography.bodySmall, color = MuxFg2)
                        }
                        Button(
                            onClick = onOpenNewTab,
                            shape = Square,
                            colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                        ) { Text(stringResource(R.string.new_session)) }
                    }
                }
            }
        }

        if (state.bots.isNotEmpty()) {
            item("feed_bots_heading") {
                MuxSectionHeading(
                    title = stringResource(R.string.bots_section),
                    count = state.bots.size,
                    aside = stringResource(R.string.see_all),
                    onAsideClick = { model.setHubTab(HubTab.BOTS) },
                )
            }
            items(state.bots.take(3), key = { "feed_bot_${it.id}" }) { bot ->
                val latestRun = state.botRuns.firstOrNull { it.botId == bot.id }
                BotHubCard(
                    bot = bot,
                    latestRun = latestRun,
                    connected = state.connected,
                    onTrigger = { model.triggerBot(bot.id) },
                    onOpenDetail = { onOpenBotDetail(bot) },
                )
            }
        }

        item("feed_desktops_heading") {
            MuxSectionHeading(
                title = "主机 / Desktops",
                count = state.pairings.size,
                aside = stringResource(R.string.add_machine),
                onAsideClick = { model.beginAddMachine() },
            )
        }
        items(state.pairings, key = { "host_${it.hostId}" }) { host ->
            val isCurrent = host.hostId == state.pairing?.hostId
            DesktopHostCard(
                host = host,
                isCurrent = isCurrent,
                connected = isCurrent && state.connected,
                connecting = isCurrent && state.connecting,
                e2eeActive = isCurrent && state.e2eeActive,
                usingLan = isCurrent && state.usingLan,
                projectCount = if (isCurrent) state.snapshot?.projects?.size ?: 0 else 0,
                activeAgentCount = if (isCurrent) projection.workingCount else 0,
                needsYouCount = if (isCurrent) projection.needsYouCount else 0,
                onOpenWorkspaces = {
                    if (!isCurrent) model.selectMachine(host.hostId)
                    model.setHubTab(HubTab.WORKSPACES)
                },
                onRemove = { model.requestRemoveMachine(host.hostId) },
            )
        }
    }
}

@Composable
internal fun WorkspacesPane(
    model: MuxlaneViewModel,
    state: UiState,
    showSidebarChrome: Boolean,
    onOpenNewTab: (projectId: String?) -> Unit,
    onOpenDiagnostics: () -> Unit,
    onHideSidebar: (() -> Unit)? = null,
) {
    val snapshot = state.snapshot
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

    Column(Modifier.fillMaxSize().background(MuxBg0)) {
        if (showSidebarChrome) {
            Surface(color = MuxBg1, contentColor = MuxFg0) {
                Column(Modifier.fillMaxWidth().statusBarsPadding()) {
                    Row(
                        Modifier.fillMaxWidth().height(52.dp).padding(horizontal = 12.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        Column(Modifier.weight(1f)) {
                            Text(
                                state.pairing?.machineName ?: stringResource(R.string.workspace_title),
                                style = MaterialTheme.typography.titleMedium,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                            ConnectionBadgesRow(
                                connected = state.connected,
                                connecting = state.connecting,
                                e2eeActive = state.e2eeActive,
                                usingLan = state.usingLan,
                            )
                        }
                        IconButton(onClick = onOpenDiagnostics, modifier = Modifier.size(36.dp)) {
                            Icon(Icons.Outlined.Settings, stringResource(R.string.host_settings), tint = MuxFg1, modifier = Modifier.size(18.dp))
                        }
                        if (onHideSidebar != null) {
                            IconButton(onClick = onHideSidebar, modifier = Modifier.size(36.dp)) {
                                Icon(Icons.AutoMirrored.Outlined.ArrowBack, "收起侧栏", tint = MuxFg1, modifier = Modifier.size(18.dp))
                            }
                        }
                    }
                    HorizontalDivider(color = MuxLine)
                }
            }
        }

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
                            SquareStepIndicator()
                            Spacer(Modifier.height(10.dp))
                            Text(stringResource(R.string.connecting_machine), color = MuxFg2)
                        }
                    } else {
                        Column(
                            Modifier.fillMaxSize().padding(24.dp),
                            verticalArrangement = Arrangement.Center,
                            horizontalAlignment = Alignment.CenterHorizontally,
                        ) {
                            Icon(Icons.Outlined.CloudOff, null, tint = MuxRed, modifier = Modifier.size(34.dp))
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
                projects.isEmpty() -> EmptyStateBox(
                    icon = Icons.Outlined.Folder,
                    title = if (state.workspaceQuery.isNotBlank() || state.agentsNeedsYouOnly) "没有匹配的项目或会话" else stringResource(R.string.no_projects),
                )
                else -> LazyColumn(
                    Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(horizontal = 12.dp, vertical = 10.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    for (project in projects) {
                        val projectAgents = agentsByProject[project.id].orEmpty()
                        val collapsed = project.id in state.collapsedProjects && state.workspaceQuery.isBlank()
                        item("proj_${project.id}") {
                            ProjectGroupHeader(
                                project = project,
                                agentCount = projectAgents.size,
                                collapsed = collapsed,
                                onToggle = { model.toggleProjectCollapsed(project.id) },
                                onAdd = { onOpenNewTab(project.id) },
                            )
                        }
                        if (!collapsed) {
                            if (projectAgents.isEmpty()) {
                                item("proj_empty_${project.id}") {
                                    Text(
                                        stringResource(R.string.no_project_sessions),
                                        style = MaterialTheme.typography.bodySmall,
                                        color = MuxFg2,
                                        modifier = Modifier.padding(start = 10.dp, bottom = 6.dp),
                                    )
                                }
                            } else {
                                items(projectAgents, key = { "ws_agent_${it.id}" }) { agent ->
                                    val chat = state.chatSessions[agent.id]
                                    val needsYou = agent.status == AgentStatus.BLOCKED ||
                                        agent.status == AgentStatus.FAILED ||
                                        (chat?.pendingApprovals ?: 0) > 0 ||
                                        state.globalApprovals.any { it.agent == agent.id }
                                    AgentHubRowCard(
                                        row = AgentListRowModel(
                                            agent = agent,
                                            projectName = project.displayName(),
                                            projectBranch = project.branch,
                                            isChatCapable = chat != null,
                                            driverLabel = driverBadgeLabel(agent.agentType, chat?.meta?.driver),
                                            pendingApprovalsCount = maxOf(
                                                chat?.pendingApprovals ?: 0,
                                                state.globalApprovals.count { it.agent == agent.id },
                                            ),
                                            isStreaming = chat?.meta?.isStreaming == true,
                                            needsYou = needsYou,
                                        ),
                                        isActiveTab = state.selectedAgent == agent.id,
                                        onOpen = { model.openAgent(agent.id) },
                                        onDelete = { model.requestDelete(agent.id) },
                                    )
                                }
                            }
                        }
                    }
                }
            }
        }

        MuxBottomSearchBar(
            value = state.workspaceQuery,
            onValueChange = model::setWorkspaceQuery,
            placeholder = stringResource(R.string.search_workspaces_hint),
            filterActive = state.agentsNeedsYouOnly,
            onToggleFilter = model::toggleAgentsNeedsYouOnly,
            primaryEnabled = state.connected && !snapshot?.projects.isNullOrEmpty(),
            onPrimaryClick = { onOpenNewTab(snapshot?.projects?.firstOrNull()?.id) },
        )
    }
}

@Composable
private fun AgentsInboxContent(
    model: MuxlaneViewModel,
    state: UiState,
    projection: AgentListProjection,
    onOpenNewTab: () -> Unit,
) {
    var idleExpanded by remember { mutableStateOf(false) }

    Column(Modifier.fillMaxSize()) {
        Surface(color = MuxBg1, contentColor = MuxFg0) {
            Column(Modifier.fillMaxWidth()) {
                Row(
                    Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    MuxSegmentedSwitch(
                        options = listOf(
                            AgentGroupBy.STATUS to stringResource(R.string.group_by_status),
                            AgentGroupBy.PROJECT to stringResource(R.string.group_by_project),
                            AgentGroupBy.DRIVER to stringResource(R.string.group_by_driver),
                        ),
                        selected = state.agentGroupBy,
                        onSelect = model::setAgentGroupBy,
                        modifier = Modifier.weight(1f),
                    )
                    Box(
                        Modifier
                            .height(34.dp)
                            .background(if (state.agentsNeedsYouOnly) MuxYellow.copy(alpha = 0.14f) else MuxBg0, Square)
                            .border(
                                BorderStroke(1.dp, if (state.agentsNeedsYouOnly) MuxYellow else MuxControlLine),
                                Square,
                            )
                            .clickable(onClick = model::toggleAgentsNeedsYouOnly)
                            .padding(horizontal = 10.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(
                            "待确认(${projection.needsYouCount})",
                            style = MaterialTheme.typography.labelMedium,
                            color = if (state.agentsNeedsYouOnly) MuxYellow else MuxFg1,
                        )
                    }
                }
                HorizontalDivider(color = MuxLine)
            }
        }

        Box(Modifier.weight(1f).fillMaxWidth()) {
            if (projection.totalCount == 0 && state.globalApprovals.isEmpty()) {
                EmptyStateBox(
                    icon = Icons.Outlined.Terminal,
                    title = if (state.agentsNeedsYouOnly || state.workspaceQuery.isNotBlank()) "没有匹配的会话" else stringResource(R.string.no_project_sessions),
                )
            } else {
                LazyColumn(
                    Modifier.fillMaxSize(),
                    contentPadding = PaddingValues(12.dp),
                    verticalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    if (state.globalApprovals.isNotEmpty()) {
                        item("agents_approvals") {
                            NeedsYouApprovalSection(
                                approvals = state.globalApprovals,
                                snapshot = state.snapshot,
                                onOpenAgent = model::openAgent,
                                onApprove = { a, reply -> model.respondApproval(a.agent, a.id, reply) },
                            )
                        }
                    }
                    for (section in projection.sections) {
                        item("sec_${section.key}") {
                            MuxSectionHeading(title = section.title, count = section.rows.size)
                        }
                        items(section.rows, key = { "inbox_${section.key}_${it.agent.id}" }) { row ->
                            AgentHubRowCard(
                                row = row,
                                isActiveTab = state.selectedAgent == row.agent.id,
                                onOpen = { model.openAgent(row.agent.id) },
                                onDelete = { model.requestDelete(row.agent.id) },
                            )
                        }
                    }
                    if (projection.idleRows.isNotEmpty()) {
                        item("idle_fold_header") {
                            Surface(
                                shape = Square,
                                color = MuxBg1,
                                border = BorderStroke(1.dp, MuxLine),
                                modifier = Modifier
                                    .fillMaxWidth()
                                    .clickable { idleExpanded = !idleExpanded },
                            ) {
                                Row(
                                    Modifier.fillMaxWidth().padding(horizontal = 12.dp, vertical = 10.dp),
                                    verticalAlignment = Alignment.CenterVertically,
                                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                                ) {
                                    Icon(
                                        if (idleExpanded) Icons.Outlined.KeyboardArrowDown else Icons.AutoMirrored.Outlined.KeyboardArrowRight,
                                        null,
                                        tint = MuxFg2,
                                        modifier = Modifier.size(18.dp),
                                    )
                                    Text(
                                        stringResource(R.string.idle_fold_count, projection.idleRows.size),
                                        style = MaterialTheme.typography.labelLarge,
                                        color = MuxFg1,
                                    )
                                }
                            }
                        }
                        if (idleExpanded) {
                            items(projection.idleRows, key = { "inbox_idle_${it.agent.id}" }) { row ->
                                AgentHubRowCard(
                                    row = row,
                                    isActiveTab = state.selectedAgent == row.agent.id,
                                    onOpen = { model.openAgent(row.agent.id) },
                                    onDelete = { model.requestDelete(row.agent.id) },
                                )
                            }
                        }
                    }
                }
            }
        }

        MuxBottomSearchBar(
            value = state.workspaceQuery,
            onValueChange = model::setWorkspaceQuery,
            placeholder = stringResource(R.string.search_agents_hint),
            filterActive = state.agentsNeedsYouOnly,
            onToggleFilter = model::toggleAgentsNeedsYouOnly,
            primaryEnabled = state.connected && !state.snapshot?.projects.isNullOrEmpty(),
            onPrimaryClick = onOpenNewTab,
        )
    }
}

@Composable
private fun BotsHubContent(
    model: MuxlaneViewModel,
    state: UiState,
    onOpenBotDetail: (BotDefinitionSummary) -> Unit,
) {
    if (state.bots.isEmpty()) {
        EmptyStateBox(
            icon = Icons.Outlined.SmartToy,
            title = "暂无自动化 Bots",
            subtitle = "请在桌面端 Bot Studio 创建定时 Cron / Interval / Webhook 自动化任务。",
        )
        return
    }
    LazyColumn(
        Modifier.fillMaxSize(),
        contentPadding = PaddingValues(14.dp),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        item("bots_header") {
            MuxSectionHeading(title = stringResource(R.string.bots_section), count = state.bots.size)
        }
        items(state.bots, key = { "bot_${it.id}" }) { bot ->
            val latestRun = state.botRuns.firstOrNull { it.botId == bot.id }
            BotHubCard(
                bot = bot,
                latestRun = latestRun,
                connected = state.connected,
                onTrigger = { model.triggerBot(bot.id) },
                onOpenDetail = { onOpenBotDetail(bot) },
            )
        }
        if (state.botRuns.isNotEmpty()) {
            item("bot_runs_heading") {
                MuxSectionHeading(title = "最近运行记录", count = state.botRuns.size)
            }
            items(state.botRuns.take(15), key = { "bot_run_${it.id}" }) { run ->
                val botName = state.bots.find { it.id == run.botId }?.name ?: run.botId
                val live = run.agentId != null && state.snapshot?.agents?.any { it.id == run.agentId } == true
                BotRunRowCard(
                    botName = botName,
                    run = run,
                    live = live,
                    onOpenLive = { run.agentId?.let(model::openAgent) },
                )
            }
        }
    }
}

@Composable
private fun ProjectGroupHeader(
    project: Project,
    agentCount: Int,
    collapsed: Boolean,
    onToggle: () -> Unit,
    onAdd: () -> Unit,
) {
    Surface(shape = Square, color = MuxBg1, border = BorderStroke(1.dp, MuxLine)) {
        Row(
            Modifier
                .fillMaxWidth()
                .clickable(onClick = onToggle)
                .padding(start = 10.dp, end = 6.dp, top = 8.dp, bottom = 8.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(6.dp),
        ) {
            Icon(
                if (collapsed) Icons.AutoMirrored.Outlined.KeyboardArrowRight else Icons.Outlined.KeyboardArrowDown,
                null,
                tint = MuxFg2,
                modifier = Modifier.size(18.dp),
            )
            Column(Modifier.weight(1f)) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(
                        project.displayName(),
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    project.branch?.takeIf { it.isNotBlank() }?.let { branch ->
                        Box(
                            Modifier
                                .background(MuxBg2, Square)
                                .border(BorderStroke(1.dp, MuxLine), Square)
                                .padding(horizontal = 5.dp, vertical = 1.dp),
                        ) {
                            Text(branch, style = MaterialTheme.typography.labelSmall, color = MuxFg2, fontFamily = MonoFamily)
                        }
                    }
                }
                Text(
                    "${project.path} · $agentCount 个会话",
                    style = MaterialTheme.typography.labelSmall,
                    color = MuxFg2,
                    fontFamily = MonoFamily,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            IconButton(onClick = onAdd, modifier = Modifier.size(36.dp)) {
                Icon(Icons.Outlined.Add, stringResource(R.string.new_session), tint = MuxAccent, modifier = Modifier.size(20.dp))
            }
        }
    }
}

@Composable
private fun AgentHubRowCard(
    row: AgentListRowModel,
    isActiveTab: Boolean,
    onOpen: () -> Unit,
    onDelete: () -> Unit,
) {
    val now = System.currentTimeMillis()
    val borderColor = when {
        isActiveTab -> MuxAccent
        row.needsYou -> MuxYellow
        else -> MuxLine
    }
    Surface(
        shape = Square,
        color = if (isActiveTab) MuxAccentBg.copy(alpha = 0.45f) else MuxBg1,
        border = BorderStroke(1.dp, borderColor),
        modifier = Modifier.fillMaxWidth().clickable(onClick = onOpen),
    ) {
        Row(
            Modifier.padding(start = 12.dp, top = 10.dp, bottom = 10.dp, end = 4.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(Modifier.size(9.dp).background(statusColor(row.agent.status), Square))
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(3.dp)) {
                Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                    Text(
                        row.agent.title,
                        style = MaterialTheme.typography.titleMedium,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f, fill = false),
                    )
                    if (!row.agent.seen) {
                        Box(Modifier.background(MuxAccent, Square).padding(horizontal = 5.dp, vertical = 1.dp)) {
                            Text("NEW", style = MaterialTheme.typography.labelSmall, color = MuxOnAccent, fontFamily = MonoFamily)
                        }
                    }
                }
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Box(
                        Modifier
                            .background(if (row.isChatCapable) MuxAccentBg else MuxBg2, Square)
                            .border(BorderStroke(1.dp, if (row.isChatCapable) MuxAccent.copy(alpha = 0.4f) else MuxLine), Square)
                            .padding(horizontal = 5.dp, vertical = 1.dp),
                    ) {
                        Text(
                            row.driverLabel,
                            style = MaterialTheme.typography.labelSmall,
                            color = if (row.isChatCapable) MuxAccent else MuxFg2,
                            fontFamily = MonoFamily,
                        )
                    }
                    Text(
                        row.projectName,
                        style = MaterialTheme.typography.labelSmall,
                        color = MuxFg2,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                    val age = formatRelativeAge(row.agent.statusSince, now)
                    if (age.isNotEmpty()) {
                        Text("· $age", style = MaterialTheme.typography.labelSmall, color = MuxFg2, fontFamily = MonoFamily)
                    }
                }
            }
            if (row.pendingApprovalsCount > 0) {
                Box(
                    Modifier
                        .background(MuxWarningBg, Square)
                        .border(BorderStroke(1.dp, MuxWarningLine), Square)
                        .padding(horizontal = 6.dp, vertical = 2.dp),
                ) {
                    Text(
                        "${row.pendingApprovalsCount} 待批",
                        style = MaterialTheme.typography.labelSmall,
                        color = MuxWarningFg,
                    )
                }
            } else {
                AgentStatusSquareBadge(row.agent.status)
            }
            IconButton(onClick = onDelete, modifier = Modifier.size(34.dp)) {
                Icon(Icons.Outlined.DeleteOutline, stringResource(R.string.delete), tint = MuxFg2, modifier = Modifier.size(18.dp))
            }
        }
    }
}

@Composable
private fun DesktopHostCard(
    host: Pairing,
    isCurrent: Boolean,
    connected: Boolean,
    connecting: Boolean,
    e2eeActive: Boolean,
    usingLan: Boolean,
    projectCount: Int,
    activeAgentCount: Int,
    needsYouCount: Int,
    onOpenWorkspaces: () -> Unit,
    onRemove: () -> Unit,
) {
    Surface(
        shape = Square,
        color = MuxBg1,
        border = BorderStroke(1.dp, if (isCurrent) MuxAccent else MuxLine),
        modifier = Modifier.fillMaxWidth().clickable(onClick = onOpenWorkspaces),
    ) {
        Row(
            Modifier.padding(12.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Box(
                Modifier
                    .size(36.dp)
                    .background(if (isCurrent) MuxAccentBg else MuxBg2, Square)
                    .border(BorderStroke(1.dp, if (isCurrent) MuxAccent else MuxControlLine), Square),
                contentAlignment = Alignment.Center,
            ) {
                Icon(Icons.Outlined.Computer, null, tint = if (isCurrent) MuxAccent else MuxFg1, modifier = Modifier.size(20.dp))
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(2.dp)) {
                Text(host.machineName ?: host.hostId, style = MaterialTheme.typography.titleMedium)
                if (isCurrent) {
                    ConnectionBadgesRow(
                        connected = connected,
                        connecting = connecting,
                        e2eeActive = e2eeActive,
                        usingLan = usingLan,
                    )
                    Text(
                        "$projectCount 个项目 · $activeAgentCount 运行中" + if (needsYouCount > 0) " · $needsYouCount 待确认" else "",
                        style = MaterialTheme.typography.labelSmall,
                        color = MuxFg2,
                    )
                } else {
                    Text(
                        host.relayUrl,
                        style = MaterialTheme.typography.labelSmall,
                        color = MuxFg2,
                        fontFamily = MonoFamily,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            if (isCurrent) {
                Text("查看工作区", style = MaterialTheme.typography.labelMedium, color = MuxAccent)
            }
            IconButton(onClick = onRemove, modifier = Modifier.size(34.dp)) {
                Icon(Icons.Outlined.DeleteOutline, stringResource(R.string.delete), tint = MuxFg2, modifier = Modifier.size(18.dp))
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
        shape = Square,
        color = MuxBg1,
        border = BorderStroke(1.dp, MuxLine),
        modifier = Modifier.fillMaxWidth().clickable(onClick = onOpenDetail),
    ) {
        Column(Modifier.padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Icon(Icons.Outlined.SmartToy, null, tint = MuxAccent, modifier = Modifier.size(18.dp))
                Text(bot.name, style = MaterialTheme.typography.titleMedium, modifier = Modifier.weight(1f))
                Box(
                    Modifier
                        .background(if (bot.enabled) MuxAccentBg else MuxBg2, Square)
                        .border(BorderStroke(1.dp, if (bot.enabled) MuxAccent else MuxControlLine), Square)
                        .padding(horizontal = 6.dp, vertical = 2.dp),
                ) {
                    Text(
                        stringResource(if (bot.enabled) R.string.bot_enabled else R.string.bot_paused),
                        style = MaterialTheme.typography.labelSmall,
                        color = if (bot.enabled) MuxAccent else MuxFg2,
                    )
                }
            }
            Text(
                "${bot.agentType.name} · ${bot.permission}" + if (bot.isolatedWorktree) " · git worktree" else "",
                style = MaterialTheme.typography.labelSmall,
                color = MuxFg2,
                fontFamily = MonoFamily,
            )
            latestRun?.let { run ->
                Text(
                    "最近: ${botRunStatusLabel(run.status)}" + (run.summary?.let { " · $it" } ?: run.error?.let { " · $it" } ?: ""),
                    style = MaterialTheme.typography.bodySmall,
                    color = if (run.status == BotRunStatus.FAILED) MuxRed else MuxFg1,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    onClick = onTrigger,
                    enabled = connected,
                    shape = Square,
                    colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                    contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp),
                    modifier = Modifier.height(34.dp),
                ) {
                    Icon(Icons.Outlined.PlayArrow, null, modifier = Modifier.size(15.dp))
                    Spacer(Modifier.width(4.dp))
                    Text(stringResource(R.string.bot_run_now), style = MaterialTheme.typography.labelMedium)
                }
                OutlinedButton(
                    onClick = onOpenDetail,
                    shape = Square,
                    border = BorderStroke(1.dp, MuxControlLine),
                    contentPadding = PaddingValues(horizontal = 12.dp, vertical = 6.dp),
                    modifier = Modifier.height(34.dp),
                ) {
                    Text("运行详情", style = MaterialTheme.typography.labelMedium, color = MuxFg1)
                }
            }
        }
    }
}

@Composable
private fun BotRunRowCard(
    botName: String,
    run: BotRunRecord,
    live: Boolean,
    onOpenLive: () -> Unit,
) {
    Surface(
        shape = Square,
        color = MuxBg1,
        border = BorderStroke(1.dp, MuxLine),
        modifier = Modifier
            .fillMaxWidth()
            .then(if (live) Modifier.clickable(onClick = onOpenLive) else Modifier),
    ) {
        Row(
            Modifier.padding(horizontal = 12.dp, vertical = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Box(Modifier.size(8.dp).background(botRunStatusColor(run.status), Square))
            Column(Modifier.weight(1f)) {
                Text("$botName · ${run.triggerSource}", style = MaterialTheme.typography.labelLarge)
                Text(
                    run.summary ?: run.error ?: botRunStatusLabel(run.status),
                    style = MaterialTheme.typography.bodySmall,
                    color = MuxFg2,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
            }
            if (live) {
                Text("实时会话", style = MaterialTheme.typography.labelSmall, color = MuxAccent)
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
        Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
            Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                Button(
                    onClick = onTrigger,
                    shape = Square,
                    colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                    modifier = Modifier.weight(1f),
                ) {
                    Icon(Icons.Outlined.PlayArrow, null, modifier = Modifier.size(16.dp))
                    Spacer(Modifier.width(4.dp))
                    Text(stringResource(R.string.bot_run_now))
                }
            }
            MuxSectionHeading(title = "历史运行 (${runs.size})")
            if (runs.isEmpty()) {
                Text("暂无运行记录", style = MaterialTheme.typography.bodySmall, color = MuxFg2)
            } else {
                Column(
                    Modifier
                        .heightIn(max = 320.dp)
                        .verticalScroll(rememberScrollState()),
                    verticalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    runs.take(12).forEach { run ->
                        val live = run.agentId != null && run.agentId in liveAgentIds
                        BotRunRowCard(
                            botName = botRunStatusLabel(run.status),
                            run = run,
                            live = live,
                            onOpenLive = { run.agentId?.let(onOpenLiveAgent) },
                        )
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
    Surface(shape = Square, color = MuxWarningBg, border = BorderStroke(1.dp, MuxWarningLine)) {
        Column(Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                Icon(Icons.Outlined.NotificationsActive, null, tint = MuxWarningFg, modifier = Modifier.size(18.dp))
                Text("${stringResource(R.string.needs_you)} (${approvals.size})", style = MaterialTheme.typography.titleMedium, color = MuxWarningFg)
            }
            approvals.take(4).forEach { approval ->
                val agentName = snapshot?.agents?.find { it.id == approval.agent }?.title ?: approval.agent
                Surface(shape = Square, color = MuxBg1, border = BorderStroke(1.dp, MuxWarningLine)) {
                    Column(Modifier.fillMaxWidth().padding(10.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                        Text("$agentName · ${approval.title}", style = MaterialTheme.typography.labelLarge)
                        val previewText = approval.commandOrInput?.takeIf { it.isNotBlank() } ?: approval.message
                        if (!previewText.isNullOrBlank()) {
                            Text(
                                previewText,
                                style = MaterialTheme.typography.bodySmall,
                                color = MuxFg1,
                                fontFamily = MonoFamily,
                                maxLines = 2,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
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
                                colors = ButtonDefaults.buttonColors(containerColor = MuxGreen),
                                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                                modifier = Modifier.height(34.dp),
                            ) { Text(allowOption?.label ?: stringResource(R.string.approve_allow), fontSize = 12.sp) }
                            OutlinedButton(
                                onClick = { onApprove(approval, denyReply) },
                                shape = Square,
                                border = BorderStroke(1.dp, MuxRed),
                                contentPadding = PaddingValues(horizontal = 10.dp, vertical = 4.dp),
                                modifier = Modifier.height(34.dp),
                            ) { Text(denyOption?.label ?: stringResource(R.string.approve_deny), color = MuxRed, fontSize = 12.sp) }
                            Spacer(Modifier.weight(1f))
                            TextButton(
                                onClick = { onOpenAgent(approval.agent) },
                                shape = Square,
                                modifier = Modifier.height(34.dp),
                            ) { Text("查看会话", fontSize = 12.sp) }
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
    BotRunStatus.RUNNING -> MuxAccent
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
