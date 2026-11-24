package com.muxlane.android

import android.annotation.SuppressLint
import androidx.compose.animation.AnimatedContent
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.animation.togetherWith
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.width
import androidx.compose.material3.AlertDialog
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp

private enum class RootTarget {
    PAIR,
    SPLIT_WIDE,
    HUB,
    SESSION,
}

@SuppressLint("UnusedBoxWithConstraintsScope")
@Composable
fun MuxlaneRoot(model: MuxlaneViewModel) {
    val state by model.state.collectAsState()
    var showDiagnostics by remember { mutableStateOf(false) }
    var wideSidebarOpen by remember { mutableStateOf(true) }

    val openNewTab: (String?) -> Unit = { projectId ->
        val fallbackProject = projectId ?: state.snapshot?.projects?.firstOrNull()?.id
        if (fallbackProject != null) model.loadPresets(fallbackProject)
    }

    Surface(Modifier.fillMaxSize(), shape = Square, color = MaterialTheme.colorScheme.background) {
        BoxWithConstraints(Modifier.fillMaxSize()) {
            val isWideLayout = maxWidth >= 720.dp
            val target = when {
                state.addingMachine || state.pairings.isEmpty() -> RootTarget.PAIR
                state.pairing != null && isWideLayout -> RootTarget.SPLIT_WIDE
                state.selectedAgent != null -> RootTarget.SESSION
                else -> RootTarget.HUB
            }

            AnimatedContent(
                targetState = target,
                transitionSpec = { fadeIn() togetherWith fadeOut() },
                label = "muxlane_root_screen",
            ) { screen ->
                when (screen) {
                    RootTarget.PAIR -> PairOnboardingScreen(model = model, state = state)
                    RootTarget.SPLIT_WIDE -> Row(Modifier.fillMaxSize()) {
                        if (wideSidebarOpen) {
                            Box(Modifier.width(320.dp).fillMaxHeight()) {
                                WorkspacesPane(
                                    model = model,
                                    state = state,
                                    showSidebarChrome = true,
                                    onOpenNewTab = openNewTab,
                                    onOpenDiagnostics = { showDiagnostics = true },
                                    onHideSidebar = { wideSidebarOpen = false },
                                )
                            }
                            Box(Modifier.width(1.dp).fillMaxHeight().background(MuxLine))
                        }
                        Box(Modifier.weight(1f).fillMaxHeight()) {
                            SessionWorkspace(
                                model = model,
                                state = state,
                                wideSplitMode = true,
                                sidebarVisible = wideSidebarOpen,
                                onToggleSidebar = { wideSidebarOpen = !wideSidebarOpen },
                                onOpenNewTab = openNewTab,
                                onOpenDiagnostics = { showDiagnostics = true },
                            )
                        }
                    }
                    RootTarget.HUB -> HomeHubScaffold(
                        model = model,
                        state = state,
                        onOpenNewTab = openNewTab,
                        onOpenDiagnostics = { showDiagnostics = true },
                    )
                    RootTarget.SESSION -> SessionWorkspace(
                        model = model,
                        state = state,
                        wideSplitMode = false,
                        sidebarVisible = false,
                        onToggleSidebar = null,
                        onOpenNewTab = openNewTab,
                        onOpenDiagnostics = { showDiagnostics = true },
                    )
                }
            }
        }

        if (state.spawnProject != null) {
            NewTabSheet(model = model, state = state)
        }

        if (showDiagnostics) {
            DiagnosticsSettingsSheet(
                state = state,
                onReconnect = model::reconnect,
                onAddMachine = { model.beginAddMachine() },
                onSwitchMachine = model::selectMachine,
                onRemoveMachine = model::requestRemoveMachine,
                onChatTextScale = model::setChatTextScale,
                onTerminalTextScale = model::setTerminalTextScale,
                onDismiss = { showDiagnostics = false },
            )
        }

        state.confirmDelete?.let { agentId ->
            DeleteConfirmDialog(
                title = state.snapshot?.agents?.find { it.id == agentId }?.title ?: agentId,
                onConfirm = model::confirmDelete,
                onDismiss = model::cancelDelete,
            )
        }

        state.confirmRemoveHost?.let { hostId ->
            val label = state.pairings.find { it.hostId == hostId }?.machineName ?: hostId
            RemoveMachineConfirmDialog(
                name = label,
                onConfirm = model::confirmRemoveMachine,
                onDismiss = model::cancelRemoveMachine,
            )
        }
    }
}

@Composable
private fun DeleteConfirmDialog(
    title: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        shape = Square,
        title = { Text(stringResource(R.string.delete_session_title)) },
        text = { Text("确定销毁会话「$title」？远端运行中的代理将被终止。") },
        confirmButton = {
            TextButton(onClick = onConfirm, shape = Square) {
                Text(stringResource(R.string.delete), color = MuxRed)
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss, shape = Square) {
                Text(stringResource(R.string.cancel))
            }
        },
    )
}

@Composable
private fun RemoveMachineConfirmDialog(
    name: String,
    onConfirm: () -> Unit,
    onDismiss: () -> Unit,
) {
    AlertDialog(
        onDismissRequest = onDismiss,
        shape = Square,
        title = { Text(stringResource(R.string.remove_machine_title)) },
        text = { Text("将移除主机「$name」的本地配对凭据。") },
        confirmButton = {
            TextButton(onClick = onConfirm, shape = Square) {
                Text(stringResource(R.string.delete), color = MuxRed)
            }
        },
        dismissButton = {
            TextButton(onClick = onDismiss, shape = Square) {
                Text(stringResource(R.string.cancel))
            }
        },
    )
}
