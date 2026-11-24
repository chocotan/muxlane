package com.muxlane.android

import androidx.compose.animation.core.RepeatMode
import androidx.compose.animation.core.animateFloat
import androidx.compose.animation.core.infiniteRepeatable
import androidx.compose.animation.core.keyframes
import androidx.compose.animation.core.rememberInfiniteTransition
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.detectTransformGestures
import androidx.compose.foundation.gestures.detectVerticalDragGestures
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.WindowInsets
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
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowLeft
import androidx.compose.material.icons.automirrored.outlined.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.outlined.KeyboardReturn
import androidx.compose.material.icons.outlined.Keyboard
import androidx.compose.material.icons.outlined.KeyboardArrowDown
import androidx.compose.material.icons.outlined.KeyboardArrowUp
import androidx.compose.material.icons.outlined.KeyboardHide
import androidx.compose.material.icons.outlined.MoreHoriz
import androidx.compose.material3.HorizontalDivider
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
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
import androidx.compose.ui.draw.clipToBounds
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.drawscope.drawIntoCanvas
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.nativeCanvas
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.hapticfeedback.HapticFeedbackType
import androidx.compose.ui.input.pointer.pointerInput
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalHapticFeedback
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import muxlane.term.VirtualTerminal
import kotlin.math.floor

@Composable
internal fun TerminalPane(
    model: MuxlaneViewModel,
    agentId: String,
    state: UiState,
    modifier: Modifier = Modifier,
) {
    val revision by model.terminalRevision.collectAsState()
    val terminalFocusRequester = remember { FocusRequester() }
    val liveInputFocusRequester = remember { FocusRequester() }
    val localView = LocalView.current
    val softwareKeyboard = LocalSoftwareKeyboardController.current

    DisposableEffect(localView) {
        val listener = android.view.ViewTreeObserver.OnWindowFocusChangeListener { focused ->
            if (focused) model.claimTerminalSize()
        }
        localView.viewTreeObserver.addOnWindowFocusChangeListener(listener)
        onDispose { localView.viewTreeObserver.removeOnWindowFocusChangeListener(listener) }
    }
    LaunchedEffect(agentId) {
        terminalFocusRequester.requestFocus()
        model.claimTerminalSize()
    }

    var ctrl by remember(agentId) { mutableStateOf(false) }
    var alt by remember(agentId) { mutableStateOf(false) }
    var showMoreKeys by remember(agentId) { mutableStateOf(false) }
    var imeBuffer by remember(agentId) { mutableStateOf("") }
    var dockVisible by remember(agentId) { mutableStateOf(true) }

    fun focusSystemKeyboard() {
        liveInputFocusRequester.requestFocus()
        softwareKeyboard?.show()
    }

    Column(modifier.fillMaxSize().background(MuxCanvas)) {
        Box(
            Modifier
                .weight(1f)
                .fillMaxWidth()
                .focusRequester(terminalFocusRequester)
                .focusable()
                .onFocusChanged { if (it.isFocused) model.claimTerminalSize() }
                .clickable(
                    indication = null,
                    interactionSource = remember { MutableInteractionSource() },
                    onClick = ::focusSystemKeyboard,
                ),
        ) {
            StableTerminalViewport(
                model = model,
                revision = revision,
                textScale = state.terminalTextScale,
                onPinchScale = { zoom -> model.setTerminalTextScale(state.terminalTextScale * zoom) },
                modifier = Modifier.fillMaxSize(),
            )
            if (!state.connected) {
                StatusNoticeBanner(
                    text = stringResource(R.string.terminal_disconnected),
                    bg = MuxErrorBg,
                    border = MuxErrorLine,
                    fg = MuxErrorFg,
                    modifier = Modifier.align(Alignment.TopCenter).padding(10.dp),
                )
            }
        }

        if (dockVisible) {
            TerminalCommandDock(
                enabled = state.connected,
                ctrl = ctrl,
                alt = alt,
                showMoreKeys = showMoreKeys,
                imeBuffer = imeBuffer,
                focusRequester = liveInputFocusRequester,
                onToggleCtrl = { ctrl = !ctrl },
                onToggleAlt = { alt = !alt },
                onToggleMoreKeys = { showMoreKeys = !showMoreKeys },
                onSendRaw = { seq -> model.sendInput(seq) },
                onImeBufferChange = { next ->
                    terminalInputDelta(imeBuffer, next)?.let { delta ->
                        model.sendInput(applyTerminalModifiers(delta, ctrl, alt))
                        if (ctrl) ctrl = false
                        if (alt) alt = false
                    }
                    imeBuffer = if (next.length > 256) next.takeLast(64) else next
                },
                onSubmitEnter = {
                    model.sendInput("\r")
                },
                onHideDock = {
                    dockVisible = false
                    softwareKeyboard?.hide()
                },
            )
        } else {
            Surface(color = TerminalSurface, contentColor = TerminalForeground) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .navigationBarsPadding()
                        .border(BorderStroke(1.dp, TerminalDivider), Square)
                        .clickable {
                            dockVisible = true
                            focusSystemKeyboard()
                        }
                        .padding(horizontal = 12.dp, vertical = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text("终端输入已折叠 · 点击展开命令栏", style = MaterialTheme.typography.labelMedium, color = TerminalForeground)
                    Icon(Icons.Outlined.Keyboard, stringResource(R.string.terminal_show_keyboard), tint = TerminalAccent, modifier = Modifier.size(18.dp))
                }
            }
        }
    }
}

@Composable
private fun StableTerminalViewport(
    model: MuxlaneViewModel,
    revision: Long,
    textScale: Float,
    onPinchScale: (Float) -> Unit,
    modifier: Modifier = Modifier,
) {
    val density = LocalDensity.current
    val imeVisible = WindowInsets.ime.getBottom(density) > 0
    var visibleHeight by remember { mutableIntStateOf(0) }
    var normalHeight by remember { mutableIntStateOf(0) }
    var scrollRows by remember { mutableIntStateOf(0) }
    val fontPx = (14f * textScale.coerceIn(0.7f, 1.65f)) * density.density
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
            fontPx = fontPx,
            scrollRows = scrollRows,
            startRow = 0,
            suppressResize = imeVisible,
            onScroll = model::scrollTerminal,
            onPinchScale = onPinchScale,
            onResize = model::resize,
            modifier = Modifier
                .fillMaxWidth()
                .requiredHeight(with(density) { terminalHeight.toDp() })
                .graphicsLayer { translationY = -offsetRows * cellHeight },
        )
        if (scrollRows > 0) {
            TextButton(
                onClick = { scrollRows = 0 },
                shape = Square,
                modifier = Modifier.align(Alignment.TopStart).heightIn(min = 42.dp),
            ) {
                Text(
                    stringResource(R.string.terminal_history, scrollRows),
                    color = TerminalForeground,
                    fontSize = 12.sp,
                )
            }
        }
    }
}

@Composable
private fun TerminalCanvas(
    terminal: VirtualTerminal,
    revision: Long,
    fontPx: Float,
    scrollRows: Int,
    startRow: Int,
    suppressResize: Boolean,
    onScroll: (Int) -> Unit,
    onPinchScale: (Float) -> Unit,
    onResize: (Int, Int) -> Unit,
    modifier: Modifier,
) {
    val density = LocalDensity.current
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
    val currentOnPinchScale by rememberUpdatedState(onPinchScale)
    val infiniteTransition = rememberInfiniteTransition(label = "cursor_blink")
    val cursorVisible by infiniteTransition.animateFloat(
        initialValue = 1f,
        targetValue = 0f,
        animationSpec = infiniteRepeatable(
            animation = keyframes {
                durationMillis = 1000
                1f at 0
                1f at 500
                0f at 501
                0f at 1000
            },
            repeatMode = RepeatMode.Restart,
        ),
        label = "cursor_visible",
    )
    Canvas(
        modifier
            .background(MuxCanvas)
            .pointerInput(Unit) {
                detectTransformGestures { _, _, zoom, _ ->
                    if (kotlin.math.abs(zoom - 1f) > 0.02f) {
                        currentOnPinchScale(zoom)
                    }
                }
            }
            .pointerInput(cellH) {
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
        val drawnRows = floor(size.height / cellH).toInt().coerceAtMost((screen.size - startRow).coerceAtLeast(0))
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
        if (scrollRows == 0 && cursorVisible > 0.5f) {
            val cursor = terminal.cursor
            val cursorRow = cursor.row - startRow
            if (cursorRow in 0 until drawnRows) {
                drawRect(TerminalAccent.copy(alpha = 0.3f), Offset(cursor.col * cellW, cursorRow * cellH), Size(cellW, cellH))
                drawRect(TerminalAccent, Offset(cursor.col * cellW, cursorRow * cellH), Size(2.5f * density.density, cellH))
            }
        }
    }
}

@Composable
private fun TerminalCommandDock(
    enabled: Boolean,
    ctrl: Boolean,
    alt: Boolean,
    showMoreKeys: Boolean,
    imeBuffer: String,
    focusRequester: FocusRequester,
    onToggleCtrl: () -> Unit,
    onToggleAlt: () -> Unit,
    onToggleMoreKeys: () -> Unit,
    onSendRaw: (String) -> Unit,
    onImeBufferChange: (String) -> Unit,
    onSubmitEnter: () -> Unit,
    onHideDock: () -> Unit,
) {
    val haptic = LocalHapticFeedback.current
    val softwareKeyboard = LocalSoftwareKeyboardController.current

    Surface(color = TerminalSurface, contentColor = TerminalForeground) {
        Column(
            Modifier
                .fillMaxWidth()
                .imePadding()
                .navigationBarsPadding(),
        ) {
            HorizontalDivider(color = TerminalDivider)
            if (showMoreKeys) {
                Row(
                    Modifier
                        .fillMaxWidth()
                        .horizontalScroll(rememberScrollState())
                        .padding(horizontal = 6.dp, vertical = 5.dp),
                    horizontalArrangement = Arrangement.spacedBy(4.dp),
                ) {
                    val extras = listOf(
                        "Home" to "\u001b[H",
                        "End" to "\u001b[F",
                        "PgUp" to "\u001b[5~",
                        "PgDn" to "\u001b[6~",
                        "Del" to "\u001b[3~",
                        "/" to "/",
                        "-" to "-",
                        "|" to "|",
                        "~" to "~",
                        "&" to "&",
                    )
                    extras.forEach { (label, seq) ->
                        CompactKeycap(
                            label = label,
                            enabled = enabled,
                            onClick = {
                                haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                                onSendRaw(seq)
                            },
                        )
                    }
                }
                HorizontalDivider(color = TerminalDivider)
            }

            Row(
                Modifier.fillMaxWidth().padding(horizontal = 6.dp, vertical = 5.dp),
                horizontalArrangement = Arrangement.spacedBy(4.dp),
            ) {
                TacticalKeycap("^C", 1.1f, enabled = enabled, isDanger = true) {
                    haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                    onSendRaw("\u0003")
                }
                TacticalKeycap("Esc", 1f, enabled = enabled) {
                    haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                    onSendRaw("\u001b")
                }
                TacticalKeycap("Tab", 1f, enabled = enabled) {
                    haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                    onSendRaw("\t")
                }
                TacticalKeycap("Ctrl", 1.05f, active = ctrl, enabled = enabled, onClick = onToggleCtrl)
                TacticalKeycap("Alt", 0.95f, active = alt, enabled = enabled, onClick = onToggleAlt)
                TacticalIconKey(Icons.AutoMirrored.Outlined.KeyboardArrowLeft, "左", 1f, enabled) {
                    onSendRaw("\u001b[D")
                }
                TacticalIconKey(Icons.Outlined.KeyboardArrowDown, "下", 1f, enabled) {
                    onSendRaw("\u001b[B")
                }
                TacticalIconKey(Icons.Outlined.KeyboardArrowUp, "上", 1f, enabled) {
                    onSendRaw("\u001b[A")
                }
                TacticalIconKey(Icons.AutoMirrored.Outlined.KeyboardArrowRight, "右", 1f, enabled) {
                    onSendRaw("\u001b[C")
                }
                Box(
                    Modifier
                        .weight(0.95f)
                        .height(38.dp)
                        .background(if (showMoreKeys) TerminalAccent else TerminalKeycap, Square)
                        .border(BorderStroke(1.dp, if (showMoreKeys) TerminalAccent else TerminalBorder), Square)
                        .clickable(onClick = onToggleMoreKeys),
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(
                        Icons.Outlined.MoreHoriz,
                        stringResource(R.string.terminal_more_keys),
                        tint = if (showMoreKeys) MuxOnAccent else TerminalForeground,
                        modifier = Modifier.size(18.dp),
                    )
                }
            }

            Row(
                Modifier
                    .fillMaxWidth()
                    .padding(start = 6.dp, end = 6.dp, bottom = 6.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(5.dp),
            ) {
                Row(
                    Modifier
                        .weight(1f)
                        .height(38.dp)
                        .background(MuxBg1, Square)
                        .border(BorderStroke(1.dp, TerminalBorder), Square)
                        .padding(horizontal = 8.dp),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(6.dp),
                ) {
                    Icon(
                        Icons.Outlined.Keyboard,
                        null,
                        tint = if (ctrl || alt) TerminalAccent else TerminalMuted,
                        modifier = Modifier.size(16.dp),
                    )
                    if (ctrl || alt) {
                        Box(Modifier.background(TerminalAccent, Square).padding(horizontal = 4.dp, vertical = 1.dp)) {
                            Text(
                                buildString {
                                    if (ctrl) append("CTRL")
                                    if (ctrl && alt) append("+")
                                    if (alt) append("ALT")
                                },
                                color = MuxOnAccent,
                                fontSize = 9.sp,
                                fontFamily = MonoFamily,
                            )
                        }
                    }
                    Box(Modifier.weight(1f), contentAlignment = Alignment.CenterStart) {
                        if (imeBuffer.isEmpty()) {
                            Text(
                                stringResource(R.string.terminal_live_input_hint),
                                style = MaterialTheme.typography.labelMedium,
                                color = TerminalMuted,
                                maxLines = 1,
                                overflow = TextOverflow.Ellipsis,
                            )
                        }
                        BasicTextField(
                            value = imeBuffer,
                            onValueChange = onImeBufferChange,
                            enabled = enabled,
                            singleLine = true,
                            textStyle = MaterialTheme.typography.bodySmall.copy(
                                color = TerminalForeground,
                                fontFamily = MonoFamily,
                            ),
                            cursorBrush = SolidColor(TerminalAccent),
                            keyboardOptions = KeyboardOptions(keyboardType = KeyboardType.Text, imeAction = ImeAction.Send),
                            keyboardActions = KeyboardActions(onSend = { onSubmitEnter() }),
                            modifier = Modifier
                                .fillMaxWidth()
                                .focusRequester(focusRequester)
                                .semantics { contentDescription = "终端输入" },
                        )
                    }
                }
                Box(
                    Modifier
                        .size(width = 44.dp, height = 38.dp)
                        .background(TerminalAccent, Square)
                        .border(BorderStroke(1.dp, TerminalAccent), Square)
                        .clickable(enabled = enabled) {
                            haptic.performHapticFeedback(HapticFeedbackType.TextHandleMove)
                            onSubmitEnter()
                        },
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(Icons.AutoMirrored.Outlined.KeyboardReturn, "回车", tint = MuxOnAccent, modifier = Modifier.size(18.dp))
                }
                Box(
                    Modifier
                        .size(width = 38.dp, height = 38.dp)
                        .background(TerminalKeycap, Square)
                        .border(BorderStroke(1.dp, TerminalBorder), Square)
                        .clickable {
                            softwareKeyboard?.hide()
                            onHideDock()
                        },
                    contentAlignment = Alignment.Center,
                ) {
                    Icon(Icons.Outlined.KeyboardHide, stringResource(R.string.terminal_hide_keyboard), tint = TerminalForeground, modifier = Modifier.size(18.dp))
                }
            }
        }
    }
}

@Composable
private fun CompactKeycap(
    label: String,
    enabled: Boolean,
    onClick: () -> Unit,
) {
    Surface(
        modifier = Modifier.height(34.dp).clickable(enabled = enabled, onClick = onClick),
        shape = Square,
        color = TerminalKeycap,
        border = BorderStroke(1.dp, TerminalBorder),
    ) {
        Box(Modifier.padding(horizontal = 10.dp), contentAlignment = Alignment.Center) {
            Text(
                label,
                color = if (enabled) TerminalForeground else TerminalMuted,
                fontFamily = MonoFamily,
                fontSize = 11.sp,
                fontWeight = FontWeight.Medium,
            )
        }
    }
}

@Composable
private fun RowScope.TacticalKeycap(
    label: String,
    weight: Float,
    active: Boolean = false,
    enabled: Boolean,
    isDanger: Boolean = false,
    onClick: () -> Unit,
) {
    val bg = when {
        active -> TerminalAccent
        isDanger -> Color(0xFFFDE8E8)
        else -> TerminalKeycap
    }
    val border = when {
        active -> TerminalAccent
        isDanger -> MuxRed
        else -> TerminalBorder
    }
    val fg = when {
        active -> MuxOnAccent
        isDanger -> MuxRed
        !enabled -> TerminalMuted
        else -> TerminalForeground
    }

    Surface(
        modifier = Modifier
            .weight(weight)
            .height(38.dp)
            .clickable(enabled = enabled, onClick = onClick),
        shape = Square,
        color = bg,
        border = BorderStroke(1.dp, border),
    ) {
        Box(contentAlignment = Alignment.Center) {
            Text(
                label,
                color = fg,
                fontFamily = MonoFamily,
                fontWeight = if (active || isDanger) FontWeight.Bold else FontWeight.Medium,
                fontSize = if (label.length > 3) 10.sp else 11.sp,
                letterSpacing = 0.02.em,
                maxLines = 1,
            )
        }
    }
}

@Composable
private fun RowScope.TacticalIconKey(
    icon: androidx.compose.ui.graphics.vector.ImageVector,
    description: String,
    weight: Float,
    enabled: Boolean,
    onClick: () -> Unit,
) {
    Surface(
        modifier = Modifier
            .weight(weight)
            .height(38.dp)
            .clickable(enabled = enabled, onClick = onClick),
        shape = Square,
        color = TerminalKeycap,
        border = BorderStroke(1.dp, TerminalBorder),
    ) {
        Box(contentAlignment = Alignment.Center) {
            Icon(
                icon,
                description,
                tint = if (enabled) TerminalForeground else TerminalMuted,
                modifier = Modifier.size(18.dp),
            )
        }
    }
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
