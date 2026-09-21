package com.muxlane.android

import android.Manifest
import android.content.ClipboardManager
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import androidx.activity.compose.BackHandler
import androidx.activity.compose.rememberLauncherForActivityResult
import androidx.activity.result.contract.ActivityResultContracts
import androidx.annotation.StringRes
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.focusable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.requiredHeight
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.ArrowBack
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.outlined.KeyboardReturn
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.ChevronRight
import androidx.compose.material.icons.outlined.DeleteOutline
import androidx.compose.material.icons.outlined.DeleteSweep
import androidx.compose.material.icons.outlined.Dns
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.Keyboard
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.outlined.KeyboardArrowUp
import androidx.compose.material.icons.outlined.KeyboardHide
import androidx.compose.material.icons.outlined.LinkOff
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Security
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.IconButtonDefaults
import androidx.compose.material3.LinearProgressIndicator
import androidx.compose.material3.ListItem
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.role
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import muxlane.protocol.AgentInstance
import muxlane.protocol.AgentStatus
import muxlane.protocol.Project
import muxlane.term.VirtualTerminal
import kotlin.math.floor

@Composable
fun MuxlaneRoot(model: MuxlaneViewModel) {
    val state by model.state.collectAsState()
    val context = LocalContext.current
    val notifications = rememberLauncherForActivityResult(ActivityResultContracts.RequestPermission()) {}
    LaunchedEffect(state.pairing) {
        if (state.pairing != null &&
            Build.VERSION.SDK_INT >= 33 &&
            ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED
        ) notifications.launch(Manifest.permission.POST_NOTIFICATIONS)
    }
    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        when {
            state.selectedAgent != null -> TerminalScreen(model)
            state.pairing != null -> WorkspaceScreen(model)
            state.addingMachine || state.pairings.isEmpty() -> PairScreen(model)
            else -> MachineScreen(model)
        }
    }
    state.confirmDelete?.let { DeleteConfirm(model, it) }
    state.confirmRemoveHost?.let { RemoveMachineConfirm(model, it) }
    state.spawnProject?.let { SpawnSheet(model) }
}

@Composable
private fun PairScreen(model: MuxlaneViewModel) {
    val state by model.state.collectAsState()
    val context = LocalContext.current
    val canPair = state.relayUrl.isNotBlank() && state.hostId.isNotBlank() && !state.connecting
    BackHandler(state.pairings.isNotEmpty()) { model.cancelAddMachine() }
    Column(
        Modifier
            .fillMaxSize()
            .statusBarsPadding()
            .navigationBarsPadding()
            .imePadding()
            .padding(horizontal = 24.dp, vertical = 20.dp),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically) {
            if (state.pairings.isNotEmpty()) {
                IconButton(onClick = model::cancelAddMachine) {
                    Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.back))
                }
                Spacer(Modifier.width(4.dp))
            }
            Surface(shape = Square, color = MaterialTheme.colorScheme.onSurface, modifier = Modifier.size(44.dp)) {
                Box(contentAlignment = Alignment.Center) {
                    Icon(Icons.Outlined.Terminal, null, tint = MaterialTheme.colorScheme.surface)
                }
            }
            Spacer(Modifier.width(12.dp))
            Column {
                Text(stringResource(R.string.app_name), style = MaterialTheme.typography.titleLarge)
                Text(
                    stringResource(R.string.app_subtitle),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    fontFamily = MonoFamily,
                )
            }
        }
        Spacer(Modifier.weight(0.7f))
        Column(Modifier.fillMaxWidth().widthIn(max = 560.dp).align(Alignment.CenterHorizontally)) {
            Text(stringResource(R.string.connect_workspace), style = MaterialTheme.typography.headlineSmall)
            Text(
                stringResource(R.string.pair_body),
                style = MaterialTheme.typography.bodyMedium,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.padding(top = 6.dp, bottom = 22.dp),
            )
            OutlinedTextField(
                value = state.relayUrl,
                onValueChange = model::setRelay,
                label = { Text(stringResource(R.string.relay_label)) },
                placeholder = { Text(stringResource(R.string.relay_placeholder)) },
                leadingIcon = { Icon(Icons.Outlined.Dns, null) },
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Uri, imeAction = ImeAction.Next),
                singleLine = true,
                shape = Square,
                modifier = Modifier.fillMaxWidth(),
            )
            Row(
                Modifier.fillMaxWidth(),
                horizontalArrangement = Arrangement.SpaceBetween,
                verticalAlignment = Alignment.CenterVertically,
            ) {
                Text(
                    stringResource(R.string.relay_tls_hint),
                    style = MaterialTheme.typography.bodySmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                )
                TextButton(onClick = {
                    val clipboard = context.getSystemService(Context.CLIPBOARD_SERVICE) as ClipboardManager
                    val text = clipboard.primaryClip?.getItemAt(0)?.coerceToText(context)?.toString().orEmpty()
                    applyPairText(model, text)
                }, shape = Square) { Text(stringResource(R.string.paste_pairing)) }
            }
            OutlinedTextField(
                value = state.hostId,
                onValueChange = model::setHostId,
                label = { Text(stringResource(R.string.pair_code)) },
                leadingIcon = { Icon(Icons.Outlined.Security, null) },
                textStyle = TextStyle(fontFamily = MonoFamily, fontSize = 16.sp),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Done),
                keyboardActions = KeyboardActions(onDone = { if (canPair) model.pair() }),
                singleLine = true,
                shape = Square,
                modifier = Modifier.fillMaxWidth(),
            )
            ErrorPanel(state.error)
            Button(
                onClick = model::pair,
                enabled = canPair,
                shape = Square,
                modifier = Modifier.fillMaxWidth().height(52.dp).padding(top = 4.dp),
            ) {
                if (state.connecting) CircularProgressIndicator(Modifier.size(20.dp), strokeWidth = 2.dp, color = Color.White)
                else Text(stringResource(R.string.pair_secure))
            }
            Surface(
                color = MaterialTheme.colorScheme.surfaceVariant,
                shape = Square,
                modifier = Modifier.fillMaxWidth().padding(top = 16.dp),
            ) {
                Row(Modifier.padding(12.dp), verticalAlignment = Alignment.CenterVertically) {
                    Icon(Icons.Outlined.Security, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(18.dp))
                    Spacer(Modifier.width(10.dp))
                    Text(
                        stringResource(R.string.pair_security_note),
                        style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
            }
        }
        Spacer(Modifier.weight(1f))
    }
}

@Composable
private fun MachineScreen(model: MuxlaneViewModel) {
    val state by model.state.collectAsState()
    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            Surface(color = MaterialTheme.colorScheme.surface, contentColor = MaterialTheme.colorScheme.onSurface) {
                Column(Modifier.statusBarsPadding()) {
                    Row(Modifier.fillMaxWidth().height(52.dp), verticalAlignment = Alignment.CenterVertically) {
                        Box(
                            Modifier.padding(start = 16.dp).size(32.dp).background(MaterialTheme.colorScheme.onSurface, Square),
                            contentAlignment = Alignment.Center,
                        ) { Icon(Icons.Outlined.Terminal, null, tint = MaterialTheme.colorScheme.surface, modifier = Modifier.size(18.dp)) }
                        Spacer(Modifier.width(12.dp))
                        Text(stringResource(R.string.machines_title), style = MaterialTheme.typography.titleLarge, modifier = Modifier.weight(1f))
                        IconButton(onClick = { model.beginAddMachine() }) { Icon(Icons.Outlined.Add, stringResource(R.string.add_machine)) }
                    }
                    HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
                }
            }
        },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.TopCenter) {
            LazyColumn(
                Modifier.fillMaxHeight().fillMaxWidth().widthIn(max = 760.dp),
                contentPadding = PaddingValues(16.dp),
                verticalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                items(state.pairings, key = { it.hostId }) { pairing ->
                    Surface(
                        modifier = Modifier.fillMaxWidth().heightIn(min = 76.dp).clickable { model.selectMachine(pairing.hostId) },
                        shape = Square,
                        color = MaterialTheme.colorScheme.surface,
                        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
                    ) {
                        Row(Modifier.padding(start = 14.dp, end = 4.dp, top = 12.dp, bottom = 12.dp), verticalAlignment = Alignment.CenterVertically) {
                            Box(
                                Modifier.size(40.dp).background(MaterialTheme.colorScheme.primaryContainer, Square),
                                contentAlignment = Alignment.Center,
                            ) { Icon(Icons.Outlined.Terminal, null, tint = MaterialTheme.colorScheme.primary, modifier = Modifier.size(20.dp)) }
                            Spacer(Modifier.width(12.dp))
                            Column(Modifier.weight(1f)) {
                                Text(pairing.machineName, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                                Spacer(Modifier.height(3.dp))
                                Text(
                                    stringResource(R.string.machine_relay, relayHost(pairing.relayUrl)),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    fontFamily = MonoFamily,
                                    maxLines = 1,
                                    overflow = TextOverflow.Ellipsis,
                                )
                            }
                            IconButton(onClick = { model.requestRemoveMachine(pairing.hostId) }) {
                                Icon(Icons.Outlined.DeleteOutline, stringResource(R.string.delete), tint = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                            Icon(Icons.Outlined.ChevronRight, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(20.dp))
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun WorkspaceScreen(model: MuxlaneViewModel) {
    val state by model.state.collectAsState()
    val snapshot = state.snapshot
    BackHandler { model.backToMachines() }
    Scaffold(
        containerColor = MaterialTheme.colorScheme.background,
        topBar = {
            WorkspaceTopBar(
                title = stringResource(R.string.workspace_title),
                detail = snapshot?.machine?.name ?: state.pairing?.machineName.orEmpty(),
                connected = state.connected,
                loading = state.connecting,
                onBack = model::backToMachines,
                onRefresh = model::reconnect,
                onUnpair = model::unpair,
            )
        },
    ) { padding ->
        Box(Modifier.fillMaxSize().padding(padding), contentAlignment = Alignment.TopCenter) {
            when {
                snapshot == null && state.connecting -> LoadingState(stringResource(R.string.connecting_machine))
                snapshot == null -> ConnectionErrorState(state.error, model::reconnect, model::unpair)
                snapshot.projects.isEmpty() -> EmptyState(
                    Icons.Outlined.Folder,
                    stringResource(R.string.no_projects),
                    stringResource(R.string.connection_failed_body),
                )
                else -> {
                    val blocked = snapshot.agents.count { it.status == AgentStatus.BLOCKED }
                    LazyColumn(
                        Modifier.fillMaxHeight().fillMaxWidth().widthIn(max = 760.dp),
                        contentPadding = PaddingValues(16.dp),
                        verticalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        if (blocked > 0) item {
                            StatusBanner(pluralStringResource(R.plurals.blocked_sessions, blocked, blocked), MuxYellow)
                        }
                        state.error?.let { error -> item { ErrorPanel(error) } }
                        snapshot.projects.forEach { project ->
                            val agents = snapshot.agents
                                .filter { it.project == project.id }
                                .sortedBy { statusPriority(it.status) }
                            item(key = "project:${project.id}") {
                                ProjectHeader(project, onAdd = { model.loadPresets(project.id) })
                            }
                            if (agents.isEmpty()) item(key = "empty:${project.id}") {
                                Text(
                                    stringResource(R.string.no_project_sessions),
                                    style = MaterialTheme.typography.bodySmall,
                                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                                    modifier = Modifier.padding(horizontal = 4.dp, vertical = 8.dp),
                                )
                            }
                            items(agents, key = { it.id }) { agent ->
                                AgentRow(agent, onOpen = { model.openAgent(agent.id) }, onDelete = { model.requestDelete(agent.id) })
                            }
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun WorkspaceTopBar(
    title: String,
    detail: String,
    connected: Boolean,
    loading: Boolean,
    onBack: () -> Unit,
    onRefresh: () -> Unit,
    onUnpair: () -> Unit,
) {
    Surface(color = MaterialTheme.colorScheme.surface, contentColor = MaterialTheme.colorScheme.onSurface) {
        Column(Modifier.statusBarsPadding()) {
            Row(Modifier.fillMaxWidth().height(52.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.back)) }
                CompactHeader(title, detail, Modifier.weight(1f))
                IconButton(onClick = onRefresh, enabled = !loading) { Icon(Icons.Outlined.Refresh, stringResource(R.string.refresh)) }
                IconButton(onClick = onUnpair) { Icon(Icons.Outlined.LinkOff, stringResource(R.string.unpair)) }
                Box(
                    Modifier.padding(end = 16.dp).size(9.dp).background(
                        if (connected) MaterialTheme.colorScheme.primary else MaterialTheme.colorScheme.onSurfaceVariant,
                        Square,
                    ),
                )
            }
            HorizontalDivider(color = MaterialTheme.colorScheme.outlineVariant)
            if (loading) LinearProgressIndicator(Modifier.fillMaxWidth().height(2.dp))
        }
    }
}

@Composable
private fun ProjectHeader(project: Project, onAdd: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().padding(start = 4.dp, top = 12.dp, bottom = 2.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Column(Modifier.weight(1f)) {
            Text(project.displayName(), style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold)
            project.branch?.let {
                Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, fontFamily = MonoFamily)
            }
        }
        IconButton(onClick = onAdd) { Icon(Icons.Outlined.Add, stringResource(R.string.new_session)) }
    }
}

@Composable
private fun AgentRow(agent: AgentInstance, onOpen: () -> Unit, onDelete: () -> Unit) {
    Surface(
        modifier = Modifier.fillMaxWidth().heightIn(min = 76.dp).clickable(onClick = onOpen).semantics { role = Role.Button },
        shape = Square,
        color = MaterialTheme.colorScheme.surface,
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outlineVariant),
    ) {
        Row(Modifier.padding(start = 14.dp, end = 4.dp, top = 12.dp, bottom = 12.dp), verticalAlignment = Alignment.CenterVertically) {
            StatusIcon(agent.status)
            Spacer(Modifier.width(12.dp))
            Column(Modifier.weight(1f)) {
                Text(agent.title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
                Spacer(Modifier.height(3.dp))
                Text(
                    "${agent.agentType.name.lowercase()} · ${stringResource(statusLabelRes(agent.status))}",
                    style = MaterialTheme.typography.bodySmall,
                    color = statusColor(agent.status),
                    fontFamily = MonoFamily,
                    maxLines = 1,
                )
            }
            IconButton(onClick = onDelete) { Icon(Icons.Outlined.DeleteOutline, stringResource(R.string.delete), tint = MaterialTheme.colorScheme.onSurfaceVariant) }
            Icon(Icons.Outlined.ChevronRight, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(20.dp))
        }
    }
}

@Composable
private fun StatusIcon(status: AgentStatus) {
    Box(
        Modifier.size(40.dp).background(
            if (status == AgentStatus.BLOCKED) Color(0xFFFFE5C0) else MaterialTheme.colorScheme.primaryContainer,
            Square,
        ),
        contentAlignment = Alignment.Center,
    ) {
        Icon(Icons.Outlined.Terminal, null, tint = statusColor(status), modifier = Modifier.size(20.dp))
    }
}

@Composable
private fun TerminalScreen(model: MuxlaneViewModel) {
    val state by model.state.collectAsState()
    val revision by model.terminalRevision.collectAsState()
    val agent = state.snapshot?.agents?.find { it.id == state.selectedAgent }
    val terminalFocusRequester = remember { FocusRequester() }
    val localView = LocalView.current
    DisposableEffect(localView) {
        val listener = android.view.ViewTreeObserver.OnWindowFocusChangeListener { focused ->
            if (focused) model.claimTerminalSize()
        }
        localView.viewTreeObserver.addOnWindowFocusChangeListener(listener)
        onDispose { localView.viewTreeObserver.removeOnWindowFocusChangeListener(listener) }
    }
    LaunchedEffect(state.selectedAgent) {
        if (state.selectedAgent != null) {
            terminalFocusRequester.requestFocus()
            model.claimTerminalSize()
        }
    }
    var showVirtualKeyboard by remember(state.selectedAgent) { mutableStateOf(true) }
    var ctrl by remember(state.selectedAgent) { mutableStateOf(false) }
    var alt by remember(state.selectedAgent) { mutableStateOf(false) }
    var imeBuffer by remember(state.selectedAgent) { mutableStateOf("") }
    val focusRequester = remember { FocusRequester() }
    val softwareKeyboard = LocalSoftwareKeyboardController.current
    BackHandler { model.closeAgent() }

    fun openSystemKeyboard() {
        focusRequester.requestFocus()
        softwareKeyboard?.show()
    }

    Scaffold(
        modifier = Modifier.imePadding(),
        containerColor = MuxCanvas,
        topBar = {
            TerminalTopBar(
                title = agent?.title ?: stringResource(R.string.terminal),
                detail = listOfNotNull(agent?.agentType?.name?.lowercase(), state.snapshot?.projects?.find { it.id == agent?.project }?.displayName())
                    .joinToString(" · "),
                connected = state.connected,
                showVirtualKeyboard = showVirtualKeyboard,
                onBack = model::closeAgent,
                onClear = model::clearTerminal,
                onToggleKeyboard = { showVirtualKeyboard = !showVirtualKeyboard },
            )
        },
        bottomBar = {
            if (showVirtualKeyboard || !state.error.isNullOrBlank()) {
                Surface(color = TerminalSurface, contentColor = TerminalForeground) {
                    Column(Modifier.navigationBarsPadding()) {
                        ErrorPanel(state.error, Modifier.padding(horizontal = 12.dp))
                        if (showVirtualKeyboard) TerminalVirtualKeyboard(
                            enabled = state.connected,
                            ctrl = ctrl,
                            alt = alt,
                            onCtrl = { ctrl = !ctrl },
                            onAlt = { alt = !alt },
                            send = { model.sendInput(it) },
                        )
                    }
                }
            }
        },
    ) { padding ->
        Box(
            Modifier
                .fillMaxSize()
                .padding(padding)
                .background(MuxCanvas)
                .focusRequester(terminalFocusRequester)
                .focusable()
                .onFocusChanged { if (it.isFocused) model.claimTerminalSize() }
                .clickable(
                    indication = null,
                    interactionSource = remember { MutableInteractionSource() },
                    onClick = ::openSystemKeyboard,
                ),
        ) {
            StableTerminalViewport(model, revision, Modifier.fillMaxSize())
            BasicTextField(
                value = imeBuffer,
                onValueChange = { next ->
                    terminalInputDelta(imeBuffer, next)?.let { delta ->
                        model.sendInput(applyTerminalModifiers(delta, ctrl, alt))
                    }
                    imeBuffer = if (next.length > 256) next.takeLast(64) else next
                },
                enabled = state.connected,
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Ascii, imeAction = ImeAction.None),
                modifier = Modifier
                    .align(Alignment.BottomStart)
                    .size(1.dp)
                    .alpha(0.01f)
                    .focusRequester(focusRequester)
                    .semantics { contentDescription = "终端输入" },
            )
            if (!state.connected) StatusBanner(
                stringResource(R.string.terminal_disconnected),
                MuxRed,
                Modifier.align(Alignment.TopCenter).padding(top = 8.dp),
            )
        }
    }
}

@Composable
private fun TerminalTopBar(
    title: String,
    detail: String,
    connected: Boolean,
    showVirtualKeyboard: Boolean,
    onBack: () -> Unit,
    onClear: () -> Unit,
    onToggleKeyboard: () -> Unit,
) {
    Surface(color = TerminalSurface, contentColor = TerminalForeground) {
        Column(Modifier.statusBarsPadding()) {
            Row(Modifier.fillMaxWidth().height(52.dp), verticalAlignment = Alignment.CenterVertically) {
                IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Outlined.ArrowBack, stringResource(R.string.back)) }
                CompactHeader(title, detail, Modifier.weight(1f), detailColor = TerminalAccent)
                IconButton(onClick = onClear) { Icon(Icons.Outlined.DeleteSweep, stringResource(R.string.terminal_clear)) }
                IconButton(onClick = onToggleKeyboard) {
                    Icon(
                        if (showVirtualKeyboard) Icons.Outlined.KeyboardHide else Icons.Outlined.Keyboard,
                        stringResource(if (showVirtualKeyboard) R.string.terminal_hide_keyboard else R.string.terminal_show_keyboard),
                    )
                }
                Box(Modifier.padding(end = 16.dp).size(9.dp).background(if (connected) TerminalAccent else TerminalMuted, Square))
            }
            HorizontalDivider(color = TerminalDivider)
        }
    }
}

@Composable
private fun StableTerminalViewport(model: MuxlaneViewModel, revision: Long, modifier: Modifier = Modifier) {
    val density = LocalDensity.current
    val imeVisible = WindowInsets.ime.getBottom(density) > 0
    var visibleHeight by remember { mutableIntStateOf(0) }
    var normalHeight by remember { mutableIntStateOf(0) }
    var scrollRows by remember { mutableIntStateOf(0) }
    val fontPx = 14f * density.density
    val cellHeight = fontPx * 1.35f
    LaunchedEffect(revision) { scrollRows = scrollRows.coerceAtMost(model.terminal.maxScrollRows) }
    Box(
        modifier.clipToBounds().onSizeChanged { size ->
            visibleHeight = size.height
            if (!imeVisible) normalHeight = size.height
        },
    ) {
        val visibleRows = floor(visibleHeight / cellHeight).toInt().coerceAtLeast(1)
        val terminalHeight = maxOf(visibleHeight, normalHeight)
        val offsetRows = if (imeVisible && scrollRows == 0) {
            (model.terminal.cursor.row - visibleRows + 1).coerceAtLeast(0)
        } else {
            0
        }
        TerminalCanvas(
            terminal = model.terminal,
            revision = revision,
            scrollRows = scrollRows,
            startRow = 0,
            suppressResize = imeVisible,
            // Match the desktop terminal: a swipe is a mouse-wheel report to tmux,
            // not a second local scrollback viewport.
            onScroll = model::scrollTerminal,
            onResize = model::resize,
            modifier = Modifier
                .fillMaxWidth()
                .requiredHeight(with(density) { terminalHeight.toDp() })
                .graphicsLayer { translationY = -offsetRows * cellHeight },
        )
        if (scrollRows > 0) TextButton(
            onClick = { scrollRows = 0 },
            shape = Square,
            modifier = Modifier.align(Alignment.TopStart).heightIn(min = 48.dp),
        ) { Text(stringResource(R.string.terminal_history, scrollRows), color = TerminalForeground, fontSize = 12.sp) }
    }
}

@Composable
private fun TerminalCanvas(
    terminal: VirtualTerminal,
    revision: Long,
    scrollRows: Int,
    startRow: Int,
    suppressResize: Boolean,
    onScroll: (Int) -> Unit,
    onResize: (Int, Int) -> Unit,
    modifier: Modifier,
) {
    val density = LocalDensity.current
    val fontPx = 14f * density.density
    val cellW = fontPx * 0.62f
    val cellH = fontPx * 1.35f
    val paint = remember(fontPx) {
        android.graphics.Paint().apply {
            textSize = fontPx
            typeface = android.graphics.Typeface.MONOSPACE
            isAntiAlias = true
            isSubpixelText = true
        }
    }
    val monoTypeface = remember { android.graphics.Typeface.create("monospace", android.graphics.Typeface.NORMAL) }
    val symbolTypeface = remember { android.graphics.Typeface.create("sans-serif", android.graphics.Typeface.NORMAL) }
    val currentOnScroll by rememberUpdatedState(onScroll)
    Canvas(
        modifier
            .background(MuxCanvas)
            .pointerInput(Unit) {
                var remainder = 0f
                detectVerticalDragGestures(
                    onDragStart = { remainder = 0f },
                    onDragEnd = { remainder = 0f },
                    onDragCancel = { remainder = 0f },
                ) { _, amount ->
                    remainder += amount
                    val rows = (remainder / cellH).toInt()
                    if (rows != 0) {
                        currentOnScroll(rows)
                        remainder -= rows * cellH
                    }
                }
            }
            .onSizeChanged { size ->
                if (!suppressResize) {
                    onResize(
                        floor(size.width / cellW).toInt().coerceIn(20, 200),
                        floor(size.height / cellH).toInt().coerceIn(8, 120),
                    )
                }
            },
    ) {
        revision
        val screen = terminal.viewport(scrollRows)
        val drawnRows = floor(size.height / cellH).toInt().coerceAtMost(screen.size - startRow)
        for (drawRow in 0 until drawnRows) {
            val line = screen[drawRow + startRow]
            for (column in 0 until terminal.cols.coerceAtMost(line.size)) {
                val cell = line[column]
                val fg = Color(if (cell.inverse) cell.bg else cell.fg)
                val bg = Color(if (cell.inverse) cell.fg else cell.bg)
                if (bg != MuxCanvas) drawRect(bg, Offset(column * cellW, drawRow * cellH), Size(cellW + 0.5f, cellH + 0.5f))
                if (cell.ch != " ") drawIntoCanvas { canvas ->
                    paint.color = fg.toArgb()
                    val codePoint = cell.ch.codePointAt(0)
                    paint.typeface = if (codePoint in 0x2600..0x27BF || codePoint >= 0x1F000) {
                        symbolTypeface
                    } else {
                        monoTypeface
                    }
                    paint.isFakeBoldText = cell.bold
                    paint.isUnderlineText = cell.underline
                    canvas.nativeCanvas.drawText(cell.ch, column * cellW, drawRow * cellH + cellH * 0.78f, paint)
                }
            }
        }
        if (scrollRows == 0) {
            val cursor = terminal.cursor
            val cursorRow = cursor.row - startRow
            if (cursorRow in 0 until drawnRows) {
                drawRect(TerminalAccent, Offset(cursor.col * cellW, cursorRow * cellH), Size(2f * density.density, cellH))
            }
        }
    }
}

@Composable
private fun TerminalVirtualKeyboard(
    enabled: Boolean,
    ctrl: Boolean,
    alt: Boolean,
    onCtrl: () -> Unit,
    onAlt: () -> Unit,
    send: (String) -> Unit,
) {
    Column(Modifier.fillMaxWidth().padding(horizontal = 6.dp, vertical = 5.dp), verticalArrangement = Arrangement.spacedBy(5.dp)) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(5.dp)) {
            TerminalKeycap("Esc", 1f, enabled = enabled) { send("\u001b") }
            TerminalKeycap("Tab", 1f, enabled = enabled) { send("\t") }
            TerminalKeycap("Ctrl", 1.15f, active = ctrl, enabled = enabled, onClick = onCtrl)
            TerminalKeycap("Alt", 1f, active = alt, enabled = enabled, onClick = onAlt)
            TerminalKeycap("Home", 1.15f, enabled = enabled) { send("\u001b[H") }
            TerminalKeycap("End", 1f, enabled = enabled) { send("\u001b[F") }
            TerminalKeycap("Del", 1f, enabled = enabled) { send("\u001b[3~") }
        }
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(5.dp)) {
            TerminalKeycap("PgUp", 1.2f, enabled = enabled) { send("\u001b[5~") }
            TerminalKeycap("PgDn", 1.2f, enabled = enabled) { send("\u001b[6~") }
            TerminalIconKey(Icons.AutoMirrored.Outlined.KeyboardArrowLeft, "左", 1f, enabled) { send("\u001b[D") }
            TerminalIconKey(Icons.Outlined.KeyboardArrowDown, "下", 1f, enabled) { send("\u001b[B") }
            TerminalIconKey(Icons.Outlined.KeyboardArrowUp, "上", 1f, enabled) { send("\u001b[A") }
            TerminalIconKey(Icons.AutoMirrored.Outlined.KeyboardArrowRight, "右", 1f, enabled) { send("\u001b[C") }
            TerminalIconKey(Icons.AutoMirrored.Outlined.KeyboardReturn, "回车", 1.35f, enabled) { send("\r") }
        }
    }
}

@Composable
private fun RowScope.TerminalKeycap(
    label: String,
    weight: Float,
    active: Boolean = false,
    enabled: Boolean,
    onClick: () -> Unit,
) {
    Surface(
        modifier = Modifier.weight(weight).height(48.dp).clickable(enabled = enabled, onClick = onClick),
        shape = Square,
        color = if (active) TerminalAccent else TerminalKeycap,
        border = BorderStroke(1.dp, if (active) TerminalAccent else TerminalBorder),
    ) {
        Box(contentAlignment = Alignment.Center) {
            Text(label, color = if (active) MuxOnAccent else if (enabled) TerminalForeground else TerminalMuted, fontFamily = MonoFamily, fontSize = 11.sp)
        }
    }
}

@Composable
private fun RowScope.TerminalIconKey(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    description: String,
    weight: Float,
    enabled: Boolean,
    onClick: () -> Unit,
) {
    Surface(
        modifier = Modifier.weight(weight).height(48.dp).clickable(enabled = enabled, onClick = onClick),
        shape = Square,
        color = TerminalKeycap,
        border = BorderStroke(1.dp, TerminalBorder),
    ) {
        Box(contentAlignment = Alignment.Center) {
            Icon(icon, description, tint = if (enabled) TerminalForeground else TerminalMuted, modifier = Modifier.size(20.dp))
        }
    }
}

@Composable
private fun CompactHeader(title: String, detail: String, modifier: Modifier = Modifier, detailColor: Color = MaterialTheme.colorScheme.onSurfaceVariant) {
    Row(modifier, verticalAlignment = Alignment.CenterVertically) {
        Text(title, style = MaterialTheme.typography.titleMedium, maxLines = 1, overflow = TextOverflow.Ellipsis)
        if (detail.isNotBlank()) {
            Spacer(Modifier.width(8.dp))
            Text(detail, style = MaterialTheme.typography.labelMedium, color = detailColor, fontFamily = MonoFamily, maxLines = 1, overflow = TextOverflow.Ellipsis)
        }
    }
}

@Composable
private fun ConnectionErrorState(error: String?, retry: () -> Unit, unpair: () -> Unit) {
    Column(Modifier.fillMaxSize().padding(32.dp), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
        Icon(Icons.Outlined.LinkOff, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(36.dp))
        Spacer(Modifier.height(14.dp))
        Text(stringResource(R.string.connection_failed_title), style = MaterialTheme.typography.titleLarge)
        Text(
            error ?: stringResource(R.string.connection_failed_body),
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            style = MaterialTheme.typography.bodyMedium,
            modifier = Modifier.padding(top = 6.dp, bottom = 18.dp),
        )
        Button(onClick = retry, shape = Square) { Text(stringResource(R.string.retry)) }
        TextButton(onClick = unpair, shape = Square) { Text(stringResource(R.string.unpair)) }
    }
}

@Composable
private fun LoadingState(label: String) {
    Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
        CircularProgressIndicator(Modifier.size(28.dp), strokeWidth = 2.5.dp)
        Spacer(Modifier.height(12.dp))
        Text(label, style = MaterialTheme.typography.bodyMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

@Composable
private fun EmptyState(icon: androidx.compose.ui.graphics.vector.ImageVector, title: String, detail: String) {
    Column(Modifier.fillMaxSize().padding(32.dp), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
        Icon(icon, null, tint = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.size(36.dp))
        Spacer(Modifier.height(14.dp))
        Text(title, style = MaterialTheme.typography.titleLarge)
        Text(detail, color = MaterialTheme.colorScheme.onSurfaceVariant, style = MaterialTheme.typography.bodyMedium, modifier = Modifier.padding(top = 6.dp))
    }
}

@Composable
private fun ErrorPanel(error: String?, modifier: Modifier = Modifier) {
    if (error.isNullOrBlank()) return
    Surface(color = Color(0xFFFFDAD6), shape = Square, modifier = modifier.fillMaxWidth().padding(vertical = 8.dp)) {
        Text(error, color = Color(0xFF410002), style = MaterialTheme.typography.bodySmall, modifier = Modifier.padding(horizontal = 12.dp, vertical = 10.dp))
    }
}

@Composable
private fun StatusBanner(text: String, color: Color, modifier: Modifier = Modifier) {
    Surface(color = color, contentColor = Color.White, shape = Square, modifier = modifier) {
        Text(text, style = MaterialTheme.typography.labelMedium, modifier = Modifier.padding(horizontal = 10.dp, vertical = 7.dp))
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun SpawnSheet(model: MuxlaneViewModel) {
    val state by model.state.collectAsState()
    ModalBottomSheet(onDismissRequest = model::closeSpawn, shape = Square, dragHandle = null) {
        Text(stringResource(R.string.spawn_title), style = MaterialTheme.typography.titleLarge, modifier = Modifier.padding(horizontal = 20.dp, vertical = 16.dp))
        if (state.presets.isEmpty()) Text(stringResource(R.string.no_presets), color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(20.dp))
        state.presets.forEach { preset ->
            ListItem(
                headlineContent = { Text(preset.label) },
                supportingContent = { Text(preset.program.ifBlank { "Shell" }, fontFamily = MonoFamily) },
                leadingContent = { Icon(Icons.Outlined.Terminal, null, tint = MaterialTheme.colorScheme.primary) },
                trailingContent = { Icon(Icons.Outlined.Add, stringResource(R.string.new_session)) },
                modifier = Modifier.clickable { model.spawn(preset) },
            )
        }
        Spacer(Modifier.navigationBarsPadding().height(16.dp))
    }
}

@Composable
private fun DeleteConfirm(model: MuxlaneViewModel, agentId: String) {
    val title = model.state.collectAsState().value.snapshot?.agents?.find { it.id == agentId }?.title ?: agentId
    AlertDialog(
        onDismissRequest = model::cancelDelete,
        shape = Square,
        title = { Text(stringResource(R.string.delete_session_title)) },
        text = { Text(stringResource(R.string.delete_session_body, title)) },
        confirmButton = {
            Button(
                onClick = model::confirmDelete,
                shape = Square,
                colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error),
            ) { Text(stringResource(R.string.delete)) }
        },
        dismissButton = { TextButton(onClick = model::cancelDelete, shape = Square) { Text(stringResource(R.string.cancel)) } },
    )
}

@Composable
private fun RemoveMachineConfirm(model: MuxlaneViewModel, hostId: String) {
    val pairing = model.state.collectAsState().value.pairings.find { it.hostId == hostId }
    val name = pairing?.machineName ?: hostId
    AlertDialog(
        onDismissRequest = model::cancelRemoveMachine,
        shape = Square,
        title = { Text(stringResource(R.string.remove_machine_title)) },
        text = { Text(stringResource(R.string.remove_machine_body, name)) },
        confirmButton = {
            Button(
                onClick = model::confirmRemoveMachine,
                shape = Square,
                colors = ButtonDefaults.buttonColors(containerColor = MaterialTheme.colorScheme.error),
            ) { Text(stringResource(R.string.delete)) }
        },
        dismissButton = { TextButton(onClick = model::cancelRemoveMachine, shape = Square) { Text(stringResource(R.string.cancel)) } },
    )
}

internal fun terminalInputDelta(previous: String, next: String): String? {
    if (previous == next) return null
    val prefix = previous.zip(next).indexOfFirst { it.first != it.second }.let { if (it < 0) minOf(previous.length, next.length) else it }
    return when {
        next.length < previous.length && prefix == next.length -> "\u007f".repeat(previous.length - next.length)
        next.length > prefix -> next.substring(prefix).replace("\n", "\r")
        else -> null
    }
}

internal fun applyTerminalModifiers(value: String, ctrl: Boolean, alt: Boolean): String {
    val controlled = if (!ctrl) value else buildString {
        value.forEach { ch -> append(if (ch.code in 64..127) (ch.code and 31).toChar() else ch) }
    }
    return if (alt && controlled.isNotEmpty()) "\u001b$controlled" else controlled
}

@StringRes
private fun statusLabelRes(status: AgentStatus): Int = when (status) {
    AgentStatus.WORKING -> R.string.status_working
    AgentStatus.BLOCKED -> R.string.status_blocked
    AgentStatus.DONE -> R.string.status_done
    AgentStatus.FAILED -> R.string.status_failed
    else -> R.string.status_idle
}

private fun statusPriority(status: AgentStatus): Int = when (status) {
    AgentStatus.BLOCKED -> 0
    AgentStatus.FAILED -> 1
    AgentStatus.WORKING -> 2
    AgentStatus.DONE -> 3
    else -> 4
}

private fun statusColor(status: AgentStatus): Color = when (status) {
    AgentStatus.WORKING -> MuxAccent
    AgentStatus.BLOCKED -> MuxYellow
    AgentStatus.DONE -> MuxGreen
    AgentStatus.FAILED -> MuxRed
    else -> MuxFg2
}

private fun applyPairText(model: MuxlaneViewModel, text: String) {
    val value = text.trim()
    val uri = runCatching { android.net.Uri.parse(value) }.getOrNull()
    if (uri?.scheme == "muxlane" && uri.host == "pair") {
        uri.getQueryParameter("relay")?.let(model::setRelay)
        uri.getQueryParameter("id")?.orEmpty()?.ifEmpty { uri.getQueryParameter("host").orEmpty() }?.let(model::setHostId)
        return
    }
    Regex("""wss?://\S+""").find(value)?.value?.trimEnd(',', ';')?.let(model::setRelay)
    Regex("""machine_[A-Za-z0-9]+""").find(value)?.value?.let(model::setHostId)
}

private fun relayHost(value: String): String = runCatching { java.net.URI(value).authority }.getOrNull() ?: value
