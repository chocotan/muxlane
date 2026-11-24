package com.muxlane.android

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.outlined.Add
import androidx.compose.material.icons.outlined.Close
import androidx.compose.material.icons.outlined.Dashboard
import androidx.compose.material.icons.outlined.Folder
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.outlined.Refresh
import androidx.compose.material.icons.outlined.Search
import androidx.compose.material.icons.outlined.SmartToy
import androidx.compose.material.icons.outlined.Terminal
import androidx.compose.material.icons.outlined.Tune
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.SliderDefaults
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import kotlinx.coroutines.delay
import muxlane.protocol.AgentStatus
import kotlin.math.roundToInt

@Composable
internal fun MuxBottomTabBar(
    activeTab: HubTab,
    needsYouCount: Int,
    botAttentionCount: Int,
    onSelect: (HubTab) -> Unit,
) {
    Surface(color = MuxBg1, contentColor = MuxFg0) {
        Column(
            Modifier
                .fillMaxWidth()
                .navigationBarsPadding(),
        ) {
            HorizontalDivider(color = MuxLine)
            Row(
                Modifier
                    .fillMaxWidth()
                    .height(58.dp),
            ) {
                HubNavButton(
                    icon = Icons.Outlined.Dashboard,
                    label = stringResource(R.string.tab_feed),
                    selected = activeTab == HubTab.FEED,
                    badgeCount = 0,
                    badgeColor = MuxAccent,
                    onClick = { onSelect(HubTab.FEED) },
                )
                HubNavButton(
                    icon = Icons.Outlined.Folder,
                    label = stringResource(R.string.tab_workspaces),
                    selected = activeTab == HubTab.WORKSPACES,
                    badgeCount = 0,
                    badgeColor = MuxAccent,
                    onClick = { onSelect(HubTab.WORKSPACES) },
                )
                HubNavButton(
                    icon = Icons.Outlined.Terminal,
                    label = stringResource(R.string.tab_agents),
                    selected = activeTab == HubTab.AGENTS,
                    badgeCount = needsYouCount,
                    badgeColor = MuxRed,
                    onClick = { onSelect(HubTab.AGENTS) },
                )
                HubNavButton(
                    icon = Icons.Outlined.SmartToy,
                    label = stringResource(R.string.tab_bots),
                    selected = activeTab == HubTab.BOTS,
                    badgeCount = botAttentionCount,
                    badgeColor = MuxYellow,
                    onClick = { onSelect(HubTab.BOTS) },
                )
            }
        }
    }
}

@Composable
private fun RowScope.HubNavButton(
    icon: ImageVector,
    label: String,
    selected: Boolean,
    badgeCount: Int,
    badgeColor: Color,
    onClick: () -> Unit,
) {
    val tint = if (selected) MuxAccent else MuxFg2
    Box(
        Modifier
            .weight(1f)
            .height(58.dp)
            .background(if (selected) MuxAccentBg.copy(alpha = 0.55f) else MuxBg1, Square)
            .clickable(onClick = onClick),
    ) {
        if (selected) {
            Box(
                Modifier
                    .fillMaxWidth()
                    .height(2.dp)
                    .background(MuxAccent, Square)
                    .align(Alignment.TopCenter),
            )
        }
        Column(
            Modifier
                .align(Alignment.Center)
                .padding(horizontal = 4.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(2.dp),
        ) {
            Box {
                Icon(icon, label, tint = tint, modifier = Modifier.size(21.dp))
                if (badgeCount > 0) {
                    Box(
                        Modifier
                            .align(Alignment.TopEnd)
                            .background(badgeColor, Square)
                            .padding(horizontal = 4.dp, vertical = 1.dp),
                    ) {
                        Text(
                            text = if (badgeCount > 99) "99+" else badgeCount.toString(),
                            color = MuxOnAccent,
                            fontSize = 9.sp,
                            fontWeight = FontWeight.Bold,
                            fontFamily = MonoFamily,
                        )
                    }
                }
            }
            Text(
                text = label,
                style = MaterialTheme.typography.labelSmall,
                color = tint,
                fontWeight = if (selected) FontWeight.Bold else FontWeight.Medium,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

@Composable
internal fun MuxBottomSearchBar(
    value: String,
    onValueChange: (String) -> Unit,
    placeholder: String,
    filterActive: Boolean = false,
    onToggleFilter: (() -> Unit)? = null,
    primaryEnabled: Boolean = true,
    onPrimaryClick: (() -> Unit)? = null,
    primaryContentDescription: String = stringResource(R.string.new_session),
) {
    Surface(color = MuxBg1, contentColor = MuxFg0) {
        Column(Modifier.fillMaxWidth()) {
            HorizontalDivider(color = MuxLine)
            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 12.dp, vertical = 8.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(8.dp),
            ) {
                if (onToggleFilter != null) {
                    Box(
                        Modifier
                            .size(40.dp)
                            .background(if (filterActive) MuxAccent else MuxBg2, Square)
                            .border(BorderStroke(1.dp, if (filterActive) MuxAccent else MuxControlLine), Square)
                            .clickable(onClick = onToggleFilter),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(
                            Icons.Outlined.Tune,
                            stringResource(R.string.filter_needs_you_only),
                            tint = if (filterActive) MuxOnAccent else MuxFg1,
                            modifier = Modifier.size(19.dp),
                        )
                    }
                }
                Row(
                    Modifier
                        .weight(1f)
                        .height(40.dp)
                        .background(MuxBg0, Square)
                        .border(BorderStroke(1.dp, MuxControlLine), Square)
                        .padding(horizontal = 10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(8.dp),
                ) {
                    Icon(Icons.Outlined.Search, null, tint = MuxFg2, modifier = Modifier.size(18.dp))
                    Box(Modifier.weight(1f), contentAlignment = Alignment.CenterStart) {
                        if (value.isEmpty()) {
                            Text(
                                placeholder,
                                style = MaterialTheme.typography.bodyMedium,
                                color = MuxFg2,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        BasicTextField(
                            value = value,
                            onValueChange = onValueChange,
                            singleLine = true,
                            textStyle = MaterialTheme.typography.bodyMedium.copy(color = MuxFg0),
                            cursorBrush = SolidColor(MuxAccent),
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                    if (value.isNotEmpty()) {
                        Icon(
                            Icons.Outlined.Close,
                            stringResource(R.string.cancel),
                            tint = MuxFg2,
                            modifier = Modifier
                                .size(18.dp)
                                .clickable { onValueChange("") },
                        )
                    }
                }
                if (onPrimaryClick != null) {
                    Box(
                        Modifier
                            .size(40.dp)
                            .background(if (primaryEnabled) MuxAccent else MuxBg2, Square)
                            .border(
                                BorderStroke(1.dp, if (primaryEnabled) MuxAccent else MuxControlLine),
                                Square,
                            )
                            .clickable(enabled = primaryEnabled, onClick = onPrimaryClick),
                        contentAlignment = Alignment.Center,
                    ) {
                        Icon(
                            Icons.Outlined.Add,
                            primaryContentDescription,
                            tint = if (primaryEnabled) MuxOnAccent else MuxFg2,
                            modifier = Modifier.size(22.dp),
                        )
                    }
                }
            }
        }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
internal fun MuxSheetFrame(
    onDismiss: () -> Unit,
    title: String,
    subtitle: String? = null,
    trailing: @Composable (RowScope.() -> Unit)? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    val sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true)
    ModalBottomSheet(
        onDismissRequest = onDismiss,
        sheetState = sheetState,
        shape = Square,
        dragHandle = null,
        containerColor = MuxBg1,
        contentColor = MuxFg0,
    ) {
        Column(
            Modifier
                .fillMaxWidth()
                .background(MuxBg1, Square)
                .border(BorderStroke(1.dp, MuxControlLine), Square)
                .imePadding()
                .navigationBarsPadding(),
        ) {
            MuxSheetHeader(
                title = title,
                subtitle = subtitle,
                trailing = trailing,
                onClose = onDismiss,
            )
            HorizontalDivider(color = MuxLine)
            Column(
                Modifier
                    .fillMaxWidth()
                    .padding(horizontal = 16.dp, vertical = 14.dp),
                content = content,
            )
        }
    }
}

@Composable
internal fun MuxSheetHeader(
    title: String,
    subtitle: String? = null,
    trailing: @Composable (RowScope.() -> Unit)? = null,
    onClose: () -> Unit,
) {
    Column(Modifier.fillMaxWidth().background(MuxBg0)) {
        Box(
            Modifier
                .padding(top = 8.dp)
                .size(width = 36.dp, height = 3.dp)
                .background(MuxControlLine, Square)
                .align(Alignment.CenterHorizontally),
        )
        Row(
            Modifier
                .fillMaxWidth()
                .padding(start = 16.dp, end = 8.dp, top = 8.dp, bottom = 10.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Column(Modifier.weight(1f)) {
                Text(
                    title,
                    style = MaterialTheme.typography.titleMedium,
                    color = MuxFg0,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (!subtitle.isNullOrBlank()) {
                    Text(
                        subtitle,
                        style = MaterialTheme.typography.labelMedium,
                        color = MuxFg2,
                        fontFamily = MonoFamily,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                    )
                }
            }
            if (trailing != null) {
                Row(
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                    content = trailing,
                )
            }
            IconButton(onClick = onClose, modifier = Modifier.size(36.dp)) {
                Icon(Icons.Outlined.Close, stringResource(R.string.cancel), tint = MuxFg1, modifier = Modifier.size(19.dp))
            }
        }
    }
}

@Composable
internal fun <T> MuxSegmentedSwitch(
    options: List<Pair<T, String>>,
    selected: T,
    onSelect: (T) -> Unit,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier
            .border(BorderStroke(1.dp, MuxControlLine), Square)
            .background(MuxBg0, Square),
    ) {
        options.forEachIndexed { index, (value, label) ->
            if (index > 0) {
                Box(Modifier.width(1.dp).height(34.dp).background(MuxControlLine))
            }
            val isSelected = value == selected
            Box(
                Modifier
                    .weight(1f)
                    .height(34.dp)
                    .background(if (isSelected) MuxAccent else MuxBg0, Square)
                    .clickable { onSelect(value) },
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    label,
                    style = MaterialTheme.typography.labelMedium,
                    fontWeight = if (isSelected) FontWeight.Bold else FontWeight.Medium,
                    color = if (isSelected) MuxOnAccent else MuxFg1,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            }
        }
    }
}

@Composable
internal fun MuxSectionHeading(
    title: String,
    count: Int? = null,
    collapsed: Boolean? = null,
    onToggleCollapse: (() -> Unit)? = null,
    aside: String? = null,
    onAsideClick: (() -> Unit)? = null,
) {
    Row(
        Modifier
            .fillMaxWidth()
            .then(
                if (onToggleCollapse != null) {
                    Modifier.clickable(onClick = onToggleCollapse)
                } else {
                    Modifier
                },
            )
            .padding(vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        if (collapsed != null) {
            Icon(
                if (collapsed) Icons.AutoMirrored.Outlined.KeyboardArrowRight else Icons.Outlined.KeyboardArrowDown,
                null,
                tint = MuxFg2,
                modifier = Modifier.size(18.dp),
            )
        }
        Text(
            title,
            style = MaterialTheme.typography.labelLarge,
            color = MuxFg1,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
        )
        if (count != null) {
            Box(
                Modifier
                    .background(MuxBg2, Square)
                    .border(BorderStroke(1.dp, MuxLine), Square)
                    .padding(horizontal = 6.dp, vertical = 1.dp),
            ) {
                Text(
                    count.toString(),
                    style = MaterialTheme.typography.labelSmall,
                    fontFamily = MonoFamily,
                    color = MuxFg2,
                )
            }
        }
        Spacer(Modifier.weight(1f))
        if (!aside.isNullOrBlank() && onAsideClick != null) {
            Text(
                aside,
                style = MaterialTheme.typography.labelMedium,
                color = MuxAccent,
                modifier = Modifier
                    .clickable(onClick = onAsideClick)
                    .padding(horizontal = 4.dp, vertical = 2.dp),
            )
        }
    }
}

@Composable
internal fun ConnectionBadgesRow(
    connected: Boolean,
    connecting: Boolean,
    e2eeActive: Boolean,
    usingLan: Boolean,
) {
    Row(
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(6.dp),
    ) {
        val (dotColor, statusText) = when {
            connected -> MuxGreen to if (usingLan) stringResource(R.string.connection_lan) else stringResource(R.string.connection_relay)
            connecting -> MuxYellow to "连接中"
            else -> MuxRed to "离线"
        }
        Row(
            Modifier
                .background(MuxBg2, Square)
                .border(BorderStroke(1.dp, MuxControlLine), Square)
                .padding(horizontal = 6.dp, vertical = 2.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(5.dp),
        ) {
            Box(Modifier.size(7.dp).background(dotColor, Square))
            Text(
                statusText,
                style = MaterialTheme.typography.labelSmall,
                color = MuxFg0,
                fontFamily = MonoFamily,
            )
        }
        if (connected && e2eeActive) {
            Box(
                Modifier
                    .background(MuxAccentBg, Square)
                    .border(BorderStroke(1.dp, MuxAccent.copy(alpha = 0.45f)), Square)
                    .padding(horizontal = 6.dp, vertical = 2.dp),
            ) {
                Text(
                    stringResource(R.string.connection_e2ee),
                    style = MaterialTheme.typography.labelSmall,
                    color = MuxAccent,
                    fontFamily = MonoFamily,
                )
            }
        }
    }
}

@Composable
internal fun AgentStatusSquareBadge(status: AgentStatus) {
    val fg = statusColor(status)
    Row(
        Modifier
            .border(BorderStroke(1.dp, fg.copy(alpha = 0.45f)), Square)
            .background(fg.copy(alpha = 0.08f), Square)
            .padding(horizontal = 7.dp, vertical = 3.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(5.dp),
    ) {
        Box(Modifier.size(7.dp).background(fg, Square))
        Text(
            stringResource(statusLabelRes(status)),
            style = MaterialTheme.typography.labelSmall,
            color = fg,
        )
    }
}

@Composable
internal fun SquareStepIndicator(
    color: Color = MuxAccent,
    modifier: Modifier = Modifier,
    blockSize: androidx.compose.ui.unit.Dp = 7.dp,
    gap: androidx.compose.ui.unit.Dp = 4.dp,
    blockCount: Int = 3,
) {
    var active by remember { mutableIntStateOf(0) }
    LaunchedEffect(Unit) {
        while (true) {
            delay(180)
            active = (active + 1) % blockCount.coerceAtLeast(1)
        }
    }
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(gap)) {
        repeat(blockCount) { idx ->
            Box(Modifier.size(blockSize).background(if (idx == active) color else color.copy(alpha = 0.25f), Square))
        }
    }
}

@Composable
internal fun ErrorPanel(error: String?, modifier: Modifier = Modifier) {
    if (error.isNullOrBlank()) return
    StatusNoticeBanner(
        text = error,
        bg = MuxErrorBg,
        border = MuxErrorLine,
        fg = MuxErrorFg,
        modifier = modifier,
    )
}

@Composable
internal fun StatusNoticeBanner(
    text: String,
    bg: Color = MuxWarningBg,
    border: Color = MuxWarningLine,
    fg: Color = MuxWarningFg,
    modifier: Modifier = Modifier,
) {
    Row(
        modifier
            .fillMaxWidth()
            .background(bg, Square)
            .border(BorderStroke(1.dp, border), Square)
            .padding(horizontal = 12.dp, vertical = 8.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        Box(Modifier.size(6.dp).background(fg, Square))
        Text(
            text = text,
            color = fg,
            style = MaterialTheme.typography.bodySmall,
            modifier = Modifier.weight(1f),
        )
    }
}

@Composable
internal fun EmptyStateBox(
    icon: ImageVector,
    title: String,
    subtitle: String? = null,
    modifier: Modifier = Modifier,
) {
    Box(modifier.fillMaxWidth().padding(32.dp), contentAlignment = Alignment.Center) {
        Column(
            horizontalAlignment = Alignment.CenterHorizontally,
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            Icon(icon, null, tint = MuxFg2, modifier = Modifier.size(34.dp))
            Text(title, style = MaterialTheme.typography.bodyMedium, color = MuxFg1)
            if (!subtitle.isNullOrBlank()) {
                Text(subtitle, style = MaterialTheme.typography.bodySmall, color = MuxFg2)
            }
        }
    }
}

@Composable
internal fun DiagnosticsSettingsSheet(
    state: UiState,
    onReconnect: () -> Unit,
    onAddMachine: () -> Unit,
    onSwitchMachine: (String) -> Unit,
    onRemoveMachine: (String) -> Unit,
    onChatTextScale: (Float) -> Unit,
    onTerminalTextScale: (Float) -> Unit,
    onDismiss: () -> Unit,
) {
    val active = state.pairing
    MuxSheetFrame(
        onDismiss = onDismiss,
        title = stringResource(R.string.host_settings),
        subtitle = active?.machineName ?: active?.hostId,
    ) {
        Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
            if (active != null) {
                Surface(shape = Square, color = MuxBg0, border = BorderStroke(1.dp, MuxLine)) {
                    Column(Modifier.fillMaxWidth().padding(12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        Row(
                            Modifier.fillMaxWidth(),
                            horizontalArrangement = Arrangement.SpaceBetween,
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            Column(Modifier.weight(1f)) {
                                Text(active.machineName ?: active.hostId, style = MaterialTheme.typography.titleMedium)
                                Text(
                                    active.hostId,
                                    style = MaterialTheme.typography.labelSmall,
                                    color = MuxFg2,
                                    fontFamily = MonoFamily,
                                )
                            }
                            ConnectionBadgesRow(
                                connected = state.connected,
                                connecting = state.connecting,
                                e2eeActive = state.e2eeActive,
                                usingLan = state.usingLan,
                            )
                        }
                        Text(
                            "中继: ${active.relayUrl}",
                            style = MaterialTheme.typography.labelSmall,
                            color = MuxFg2,
                            fontFamily = MonoFamily,
                            maxLines = 1,
                            overflow = TextOverflow.Ellipsis,
                        )
                        if (!active.lanUrl.isNullOrBlank()) {
                            Text(
                                "局域网: ${active.lanUrl}",
                                style = MaterialTheme.typography.labelSmall,
                                color = MuxFg2,
                                fontFamily = MonoFamily,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                            OutlinedButton(
                                onClick = {
                                    onReconnect()
                                    onDismiss()
                                },
                                shape = Square,
                                border = BorderStroke(1.dp, MuxControlLine),
                                modifier = Modifier.weight(1f),
                            ) {
                                Icon(Icons.Outlined.Refresh, null, modifier = Modifier.size(16.dp))
                                Spacer(Modifier.width(6.dp))
                                Text("重连当前主机")
                            }
                            OutlinedButton(
                                onClick = {
                                    onRemoveMachine(active.hostId)
                                    onDismiss()
                                },
                                shape = Square,
                                border = BorderStroke(1.dp, MuxRed.copy(alpha = 0.5f)),
                            ) { Text("解绑", color = MuxRed) }
                        }
                    }
                }
            }

            if (state.pairings.size > 1) {
                MuxSectionHeading(title = "切换已配对主机", count = state.pairings.size)
                Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    state.pairings.forEach { pairing ->
                        val isCurrent = pairing.hostId == active?.hostId
                        Row(
                            Modifier
                                .fillMaxWidth()
                                .background(if (isCurrent) MuxAccentBg else MuxBg0, Square)
                                .border(BorderStroke(1.dp, if (isCurrent) MuxAccent else MuxLine), Square)
                                .clickable {
                                    if (!isCurrent) onSwitchMachine(pairing.hostId)
                                    onDismiss()
                                }
                                .padding(horizontal = 12.dp, vertical = 10.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                        ) {
                            Box(Modifier.size(8.dp).background(if (isCurrent && state.connected) MuxGreen else MuxFg2, Square))
                            Column(Modifier.weight(1f)) {
                                Text(pairing.machineName ?: pairing.hostId, style = MaterialTheme.typography.titleMedium)
                                Text(pairing.hostId, style = MaterialTheme.typography.labelSmall, color = MuxFg2, fontFamily = MonoFamily)
                            }
                            if (isCurrent) {
                                Text("当前", style = MaterialTheme.typography.labelSmall, color = MuxAccent)
                            }
                        }
                    }
                }
            }

            MuxSectionHeading(title = stringResource(R.string.text_size_settings))
            TextScaleRow(
                label = "对话字体 (${(state.chatTextScale * 100).roundToInt()}%)",
                value = state.chatTextScale,
                range = 0.85f..1.4f,
                onChange = onChatTextScale,
            )
            TextScaleRow(
                label = "终端字体 (${(state.terminalTextScale * 100).roundToInt()}%)",
                value = state.terminalTextScale,
                range = 0.7f..1.65f,
                onChange = onTerminalTextScale,
            )

            Button(
                onClick = {
                    onDismiss()
                    onAddMachine()
                },
                shape = Square,
                colors = ButtonDefaults.buttonColors(containerColor = MuxAccent),
                contentPadding = PaddingValues(vertical = 10.dp),
                modifier = Modifier.fillMaxWidth().heightIn(min = 44.dp),
            ) {
                Icon(Icons.Outlined.Add, null, modifier = Modifier.size(18.dp))
                Spacer(Modifier.width(6.dp))
                Text(stringResource(R.string.add_machine))
            }
        }
    }
}

@Composable
internal fun TextScaleRow(
    label: String,
    value: Float,
    range: ClosedFloatingPointRange<Float>,
    onChange: (Float) -> Unit,
) {
    Column(Modifier.fillMaxWidth()) {
        Row(Modifier.fillMaxWidth(), horizontalArrangement = Arrangement.SpaceBetween) {
            Text(label, style = MaterialTheme.typography.bodySmall, color = MuxFg1, fontFamily = MonoFamily)
            Text(
                "重置",
                style = MaterialTheme.typography.labelSmall,
                color = MuxAccent,
                modifier = Modifier.clickable { onChange(1.0f) },
            )
        }
        Slider(
            value = value,
            onValueChange = onChange,
            valueRange = range,
            colors = SliderDefaults.colors(
                thumbColor = MuxAccent,
                activeTrackColor = MuxAccent,
                inactiveTrackColor = MuxBg2,
            ),
        )
    }
}
