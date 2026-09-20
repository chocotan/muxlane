package com.muxlane.android

import android.content.Intent
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.navigationBarsPadding
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.wrapContentHeight
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import muxlane.protocol.AgentInstance
import muxlane.protocol.AgentStatus
import muxlane.term.VirtualTerminal
import kotlin.math.floor

@Composable
fun MuxlaneRoot(model: MuxlaneViewModel) {
    val state by model.state.collectAsState()
    val context = LocalContext.current
    LaunchedEffect(state.pairing) {
        if (state.pairing != null) {
            context.startForegroundService(Intent(context, RelayService::class.java))
        }
    }
    Box(Modifier.fillMaxSize().background(MuxBg0)) {
        when {
            state.selectedAgent != null -> TerminalScreen(model)
            state.pairing != null -> SessionTree(model)
            else -> PairScreen(model)
        }
        state.confirmDelete?.let { DeleteConfirm(model, it) }
        state.spawnProject?.let { SpawnSheet(model) }
    }
}

@Composable
private fun PairScreen(model: MuxlaneViewModel) {
    val state by model.state.collectAsState()
    var showRelay by remember { mutableStateOf(true) }
    val codeFocus = remember { FocusRequester() }
    LaunchedEffect(Unit) { codeFocus.requestFocus() }
    Column(
        Modifier
            .fillMaxSize()
            .safeDrawingPadding()
            .imePadding()
            .verticalScroll(rememberScrollState())
            .padding(horizontal = 24.dp, vertical = 20.dp),
        verticalArrangement = Arrangement.spacedBy(16.dp),
    ) {
        Text("把手机接到电脑", color = MuxFg0, fontSize = 22.sp, fontWeight = FontWeight.Medium, lineHeight = 28.sp)
        Text(
            "在桌面打开「配对手机」，把 8 位码打在这里。",
            color = MuxFg2,
            fontSize = 15.sp,
            lineHeight = 24.sp,
        )
        Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text("配对码", color = MuxFg2, fontSize = 13.sp, lineHeight = 20.sp)
            BasicTextField(
                value = state.pairCode,
                onValueChange = { model.setCode(it) },
                textStyle = TextStyle(
                    color = MuxFg0,
                    fontFamily = FontFamily.Monospace,
                    fontSize = 32.sp,
                    fontWeight = FontWeight.Medium,
                    letterSpacing = 4.sp,
                ),
                cursorBrush = SolidColor(MuxAccent),
                keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Number, imeAction = ImeAction.Go),
                keyboardActions = KeyboardActions(onGo = { model.pair() }),
                singleLine = true,
                modifier = Modifier
                    .focusRequester(codeFocus)
                    .fillMaxWidth()
                    .heightIn(min = 72.dp)
                    .border(1.dp, MuxLine, Square)
                    .background(MuxBg1, Square)
                    .padding(horizontal = 16.dp, vertical = 18.dp),
                decorationBox = { inner ->
                    Box(contentAlignment = Alignment.CenterStart) {
                        if (state.pairCode.isEmpty()) {
                            Text(
                                "00000000",
                                color = MuxFg2.copy(alpha = 0.45f),
                                fontFamily = FontFamily.Monospace,
                                fontSize = 32.sp,
                                fontWeight = FontWeight.Medium,
                                letterSpacing = 4.sp,
                            )
                        }
                        inner()
                    }
                },
            )
        }
        Text(
            if (showRelay) "收起中继地址" else "中继不是本机时，改地址",
            color = MuxFg1,
            fontSize = 15.sp,
            lineHeight = 24.sp,
            modifier = Modifier.touchText { showRelay = !showRelay }.padding(0.dp),
        )
        if (showRelay) {
            Field("中继地址", state.relayUrl, KeyboardType.Uri) { model.setRelay(it) }
        }
        SquareButton(
            if (state.connecting) "正在配对…" else "配对",
            enabled = !state.connecting,
            filled = true,
            wide = true,
        ) { model.pair() }
        state.error?.let { Text(it, color = MuxRed, fontSize = 15.sp, lineHeight = 24.sp) }
    }
}

@Composable
private fun SessionTree(model: MuxlaneViewModel) {
    val state by model.state.collectAsState()
    val snapshot = state.snapshot
    Column(Modifier.fillMaxSize().safeDrawingPadding()) {
        Row(
            Modifier.fillMaxWidth().background(MuxBg1).padding(horizontal = 16.dp, vertical = 12.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text(
                snapshot?.machine?.name ?: state.pairing?.machineName ?: "Muxlane",
                color = MuxFg0,
                fontSize = 20.sp,
                fontWeight = FontWeight.Medium,
            )
            Text("解除配对", color = MuxFg1, modifier = Modifier.touchText { model.unpair() })
        }
        if (snapshot == null) {
            Column(Modifier.padding(20.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                if (state.error != null) {
                    Text("连不上那台机器", color = MuxFg0, fontSize = 18.sp, fontWeight = FontWeight.Medium)
                    Text(
                        "检查中继是否在线、桌面端是否运行。凭证过期就解除配对，再打一次码。",
                        color = MuxFg2,
                        fontSize = 15.sp,
                        lineHeight = 24.sp,
                    )
                    SquareButton("重试", filled = true, wide = true) { model.reconnect() }
                    QuietButton("解除配对", wide = true) { model.unpair() }
                } else {
                    Text("正在接到那台机器…", color = MuxFg2, fontSize = 15.sp, lineHeight = 24.sp)
                }
            }
            return
        }
        val blocked = snapshot.agents.count { it.status == AgentStatus.BLOCKED }
        if (blocked > 0) {
            Text(
                if (blocked == 1) "1 个会话等你确认" else "$blocked 个会话等你确认",
                color = MuxYellow,
                fontSize = 15.sp,
                fontWeight = FontWeight.Medium,
                modifier = Modifier.fillMaxWidth().background(MuxBg2).padding(horizontal = 16.dp, vertical = 12.dp),
            )
        }
        if (snapshot.projects.isEmpty()) {
            Text(
                "电脑上还没有项目。打开一个工程后会出现在这里。",
                color = MuxFg2,
                fontSize = 15.sp,
                lineHeight = 24.sp,
                modifier = Modifier.padding(20.dp),
            )
            return
        }
        LazyColumn(Modifier.fillMaxSize()) {
            items(snapshot.projects, key = { it.id }) { project ->
                val projectAgents = snapshot.agents.filter { it.project == project.id }
                Row(
                    Modifier.fillMaxWidth().padding(start = 16.dp, end = 8.dp, top = 18.dp, bottom = 4.dp),
                    horizontalArrangement = Arrangement.SpaceBetween,
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Text(
                        project.displayName(),
                        color = MuxFg2,
                        fontSize = 13.sp,
                        fontWeight = FontWeight.Medium,
                    )
                    Text("新建", color = MuxFg1, modifier = Modifier.touchText { model.loadPresets(project.id) })
                }
                if (projectAgents.isEmpty()) {
                    Text(
                        "这个项目还没有会话",
                        color = MuxFg2,
                        fontSize = 13.sp,
                        lineHeight = 20.sp,
                        modifier = Modifier.padding(start = 16.dp, end = 16.dp, bottom = 8.dp),
                    )
                }
                projectAgents.sortedBy { statusPriority(it.status) }.forEach { agent ->
                    AgentRow(agent, onOpen = { model.openAgent(agent.id) }, onDelete = { model.requestDelete(agent.id) })
                }
            }
        }
        state.error?.let {
            Text(it, color = MuxRed, fontSize = 15.sp, lineHeight = 24.sp, modifier = Modifier.padding(16.dp))
        }
    }
}

@Composable
private fun AgentRow(agent: AgentInstance, onOpen: () -> Unit, onDelete: () -> Unit) {
    val waiting = agent.status == AgentStatus.BLOCKED
    Row(
        Modifier
            .fillMaxWidth()
            .background(if (waiting) MuxBg2 else Color.Transparent)
            .clickable(onClick = onOpen)
            .padding(start = 16.dp, end = 8.dp)
            .heightIn(min = 56.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Box(Modifier.size(8.dp).background(statusColor(agent.status), Square))
        Spacer(Modifier.width(12.dp))
        Column(Modifier.weight(1f).padding(vertical = 10.dp)) {
            Text(agent.title, color = MuxFg0, fontSize = 16.sp, fontWeight = FontWeight.Medium, lineHeight = 22.sp)
            Text(
                statusLabel(agent.status),
                color = statusColor(agent.status),
                fontSize = 13.sp,
                fontWeight = FontWeight.Medium,
                lineHeight = 20.sp,
            )
        }
        Text("删除", color = MuxFg2, modifier = Modifier.touchText(onDelete))
    }
}

@Composable
private fun TerminalScreen(model: MuxlaneViewModel) {
    val state by model.state.collectAsState()
    var input by remember { mutableStateOf("") }
    val agent = state.snapshot?.agents?.find { it.id == state.selectedAgent }
    val waiting = agent?.status == AgentStatus.BLOCKED
    Column(Modifier.fillMaxSize().background(MuxCanvas).statusBarsPadding()) {
        Row(
            Modifier.fillMaxWidth().background(MuxBg1).padding(horizontal = 8.dp, vertical = 8.dp),
            horizontalArrangement = Arrangement.SpaceBetween,
            verticalAlignment = Alignment.CenterVertically,
        ) {
            Text("返回", color = MuxFg1, modifier = Modifier.touchText { model.closeAgent() })
            Text(
                agent?.title ?: "终端",
                color = MuxFg0,
                fontSize = 16.sp,
                fontWeight = FontWeight.Medium,
                modifier = Modifier.weight(1f).padding(horizontal = 8.dp),
            )
            if (agent != null) {
                Text(
                    statusLabel(agent.status),
                    color = statusColor(agent.status),
                    fontSize = 13.sp,
                    fontWeight = FontWeight.Medium,
                )
            }
        }
        Box(Modifier.weight(1f).fillMaxWidth()) {
            TerminalCanvas(
                model.terminal,
                Modifier.fillMaxSize(),
                onResize = { cols, rows -> model.resize(cols, rows) },
            )
            if (waiting) {
                Text(
                    "等你确认",
                    color = MuxOnAccent,
                    fontSize = 13.sp,
                    fontWeight = FontWeight.Medium,
                    modifier = Modifier
                        .align(Alignment.TopEnd)
                        .padding(12.dp)
                        .background(MuxYellow)
                        .padding(horizontal = 8.dp, vertical = 4.dp),
                )
            }
        }
        val send = {
            if (input.isNotBlank()) {
                model.sendInput(input + "\n")
                input = ""
            }
        }
        Box(
            Modifier
                .fillMaxWidth()
                .background(MuxCanvas)
                .navigationBarsPadding()
                .imePadding()
                .padding(horizontal = 12.dp, vertical = 8.dp),
        ) {
            BasicTextField(
                value = input,
                onValueChange = { input = it },
                textStyle = TextStyle(
                    color = MuxFg0,
                    fontFamily = FontFamily.Monospace,
                    fontSize = 16.sp,
                ),
                cursorBrush = SolidColor(MuxAccent),
                modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send),
                keyboardActions = KeyboardActions(onSend = { send() }),
                decorationBox = { inner ->
                    Row(verticalAlignment = Alignment.CenterVertically) {
                        Text("› ", color = MuxAccent, fontFamily = FontFamily.Monospace, fontSize = 16.sp)
                        Box(Modifier.weight(1f), contentAlignment = Alignment.CenterStart) {
                            if (input.isEmpty()) {
                                Text("回车发送", color = MuxFg2, fontFamily = FontFamily.Monospace, fontSize = 16.sp)
                            }
                            inner()
                        }
                    }
                },
            )
        }
    }
}

@Composable
private fun TerminalCanvas(
    terminal: VirtualTerminal,
    modifier: Modifier,
    onResize: (Int, Int) -> Unit,
) {
    val density = LocalDensity.current
    val fontPx = 14f * density.density
    val cellW = fontPx * 0.62f
    val cellH = fontPx * 1.35f
    Canvas(
        modifier
            .background(MuxCanvas)
            .onSizeChanged { size ->
                val cols = floor(size.width / cellW).toInt().coerceIn(20, 200)
                val rows = floor(size.height / cellH).toInt().coerceIn(8, 80)
                onResize(cols, rows)
            },
    ) {
        val screen = terminal.screen()
        val paint = android.graphics.Paint().apply {
            textSize = fontPx
            typeface = android.graphics.Typeface.MONOSPACE
            isAntiAlias = true
            isSubpixelText = true
        }
        for (r in 0 until terminal.rows.coerceAtMost(screen.size)) {
            val line = screen[r]
            for (c in 0 until terminal.cols.coerceAtMost(line.size)) {
                val cell = line[c]
                val fg = Color(if (cell.inverse) cell.bg else cell.fg)
                val back = Color(if (cell.inverse) cell.fg else cell.bg)
                if (back != MuxCanvas) {
                    drawRect(back, Offset(c * cellW, r * cellH), Size(cellW + 0.5f, cellH + 0.5f))
                }
                if (cell.ch != ' ') {
                    drawIntoCanvas { canvas ->
                        paint.color = fg.toArgb()
                        canvas.nativeCanvas.drawText(
                            cell.ch.toString(),
                            c * cellW,
                            r * cellH + cellH * 0.78f,
                            paint,
                        )
                    }
                }
            }
        }
        val cur = terminal.cursor
        drawRect(
            MuxAccent.copy(alpha = 0.75f),
            Offset(cur.col * cellW, cur.row * cellH),
            Size(2f * density.density, cellH),
        )
    }
}

@Composable
private fun SpawnSheet(model: MuxlaneViewModel) {
    val state by model.state.collectAsState()
    Box(Modifier.fillMaxSize().background(Color(0x99000000)).clickable { model.closeSpawn() }) {
        Column(
            Modifier
                .align(Alignment.BottomCenter)
                .fillMaxWidth()
                .background(MuxBg1)
                .border(1.dp, MuxLine, Square)
                .navigationBarsPadding()
                .consumeClicks()
                .padding(20.dp),
        ) {
            Text("新建会话", color = MuxFg0, fontSize = 18.sp, fontWeight = FontWeight.Medium)
            Spacer(Modifier.height(12.dp))
            if (state.presets.isEmpty()) {
                Text("没有可用的终端预设", color = MuxFg2, fontSize = 15.sp, lineHeight = 24.sp, modifier = Modifier.padding(vertical = 8.dp))
            }
            state.presets.forEach { preset ->
                Text(
                    preset.label,
                    color = MuxFg0,
                    fontSize = 16.sp,
                    modifier = Modifier.fillMaxWidth().clickable { model.spawn(preset) }.heightIn(min = 48.dp).wrapContentHeight(Alignment.CenterVertically),
                )
            }
            Spacer(Modifier.height(8.dp))
            QuietButton("取消", wide = true) { model.closeSpawn() }
        }
    }
}

@Composable
private fun DeleteConfirm(model: MuxlaneViewModel, agent: String) {
    val title = model.state.collectAsState().value.snapshot?.agents?.find { it.id == agent }?.title ?: agent
    Box(Modifier.fillMaxSize().background(Color(0x99000000)).clickable { model.cancelDelete() }) {
        Column(
            Modifier
                .align(Alignment.Center)
                .background(MuxBg1)
                .border(1.dp, MuxLine, Square)
                .consumeClicks()
                .padding(20.dp)
                .width(280.dp),
        ) {
            Text("删除会话？", color = MuxFg0, fontSize = 18.sp, fontWeight = FontWeight.Medium)
            Text(
                "将结束 $title。",
                color = MuxFg2,
                fontSize = 15.sp,
                lineHeight = 24.sp,
                modifier = Modifier.padding(top = 8.dp, bottom = 16.dp),
            )
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                QuietButton("取消") { model.cancelDelete() }
                SquareButton("删除", filled = true, color = MuxRed, wide = false) { model.confirmDelete() }
            }
        }
    }
}

@Composable
private fun Field(label: String, value: String, type: KeyboardType, onChange: (String) -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(label, color = MuxFg2, fontSize = 13.sp, lineHeight = 20.sp)
        BasicTextField(
            value = value,
            onValueChange = onChange,
            textStyle = TextStyle(color = MuxFg0, fontSize = 16.sp, lineHeight = 24.sp),
            cursorBrush = SolidColor(MuxAccent),
            keyboardOptions = KeyboardOptions(keyboardType = type),
            modifier = Modifier
                .fillMaxWidth()
                .heightIn(min = 48.dp)
                .border(1.dp, MuxLine, Square)
                .padding(horizontal = 12.dp, vertical = 12.dp),
        )
    }
}

@Composable
private fun SquareButton(
    label: String,
    enabled: Boolean = true,
    filled: Boolean = false,
    wide: Boolean = false,
    color: Color = MuxFg0,
    onClick: () -> Unit,
) {
    val bg = when {
        !enabled -> MuxBg2
        filled -> color
        else -> Color.Transparent
    }
    val fg = when {
        !enabled -> MuxFg2
        filled -> MuxOnAccent
        else -> MuxFg0
    }
    Box(
        Modifier
            .then(if (wide) Modifier.fillMaxWidth() else Modifier)
            .background(bg, Square)
            .then(if (filled) Modifier else Modifier.border(1.dp, MuxLine, Square))
            .clickable(enabled = enabled, onClick = onClick)
            .heightIn(min = 48.dp)
            .padding(horizontal = 16.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(label, color = fg, fontSize = 15.sp, fontWeight = FontWeight.Medium)
    }
}

@Composable
private fun QuietButton(label: String, wide: Boolean = false, onClick: () -> Unit) {
    Box(
        Modifier
            .then(if (wide) Modifier.fillMaxWidth() else Modifier)
            .clickable(onClick = onClick)
            .heightIn(min = 48.dp)
            .padding(horizontal = 12.dp),
        contentAlignment = Alignment.Center,
    ) {
        Text(label, color = MuxFg2, fontSize = 15.sp)
    }
}

@Composable
private fun Modifier.consumeClicks(): Modifier {
    val source = remember { MutableInteractionSource() }
    return clickable(indication = null, interactionSource = source) {}
}

private fun Modifier.touchText(onClick: () -> Unit): Modifier =
    clickable(onClick = onClick)
        .heightIn(min = 48.dp)
        .wrapContentHeight(Alignment.CenterVertically)
        .padding(horizontal = 12.dp)

private fun statusLabel(status: AgentStatus): String = when (status) {
    AgentStatus.WORKING -> "进行中"
    AgentStatus.BLOCKED -> "等待确认"
    AgentStatus.DONE -> "已完成"
    AgentStatus.FAILED -> "失败"
    else -> "空闲"
}

private fun statusPriority(status: AgentStatus): Int = when (status) {
    AgentStatus.BLOCKED -> 0
    AgentStatus.FAILED -> 1
    AgentStatus.WORKING -> 2
    AgentStatus.DONE -> 3
    else -> 4
}

private fun statusColor(status: AgentStatus): Color = when (status) {
    AgentStatus.WORKING -> MuxYellow
    AgentStatus.BLOCKED -> MuxYellow
    AgentStatus.DONE -> MuxGreen
    AgentStatus.FAILED -> MuxRed
    else -> MuxFg2
}
