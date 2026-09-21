package com.muxlane.android

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

val MuxCanvas = Color(0xFFF4EBDD)
val MuxBg0 = Color(0xFFF6F8F7)
val MuxBg1 = Color(0xFFFFFFFF)
val MuxBg2 = Color(0xFFE6ECE9)
val MuxLine = Color(0xFFDFE6E3)
val MuxControlLine = Color(0xFFC3CECA)
val MuxFg0 = Color(0xFF18201E)
val MuxFg1 = Color(0xFF394541)
val MuxFg2 = Color(0xFF596662)
val MuxAccent = Color(0xFF007A68)
val MuxGreen = Color(0xFF007A68)
val MuxYellow = Color(0xFF8A5100)
val MuxRed = Color(0xFFBA1A1A)
val MuxOnAccent = Color.White

val TerminalSurface = Color(0xFFFAF4E8)
val TerminalKeycap = Color(0xFFE8DCC7)
val TerminalBorder = Color(0xFF8E7D63)
val TerminalDivider = Color(0xFFCBBDA4)
val TerminalForeground = Color(0xFF39342C)
val TerminalMuted = Color(0xFF6A6257)
val TerminalAccent = Color(0xFF006D5D)

private val colors = lightColorScheme(
    primary = MuxAccent,
    onPrimary = Color.White,
    primaryContainer = Color(0xFFB9F2E3),
    onPrimaryContainer = Color(0xFF00201A),
    secondary = Color(0xFF4C635D),
    secondaryContainer = Color(0xFFCFE8E0),
    background = MuxBg0,
    surface = MuxBg1,
    surfaceVariant = MuxBg2,
    onSurface = MuxFg0,
    onSurfaceVariant = MuxFg2,
    outline = MuxControlLine,
    outlineVariant = MuxLine,
    error = MuxRed,
)

private val typography = Typography(
    headlineSmall = TextStyle(fontSize = 22.sp, lineHeight = 28.sp, fontWeight = FontWeight.Bold),
    titleLarge = TextStyle(fontSize = 18.sp, lineHeight = 24.sp, fontWeight = FontWeight.SemiBold),
    titleMedium = TextStyle(fontSize = 15.sp, lineHeight = 20.sp, fontWeight = FontWeight.SemiBold),
    bodyLarge = TextStyle(fontSize = 15.sp, lineHeight = 22.sp),
    bodyMedium = TextStyle(fontSize = 14.sp, lineHeight = 20.sp),
    bodySmall = TextStyle(fontSize = 12.sp, lineHeight = 16.sp),
    labelLarge = TextStyle(fontSize = 14.sp, lineHeight = 20.sp, fontWeight = FontWeight.SemiBold),
    labelMedium = TextStyle(fontSize = 12.sp, lineHeight = 16.sp, fontWeight = FontWeight.Medium),
)

@Composable
fun MuxlaneTheme(content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = colors, typography = typography, content = content)
}

val Square = RectangleShape
val MonoFamily = FontFamily.Monospace
