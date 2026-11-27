package com.muxlane.android

import androidx.compose.foundation.shape.CutCornerShape
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Shapes
import androidx.compose.material3.Typography
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.text.PlatformTextStyle
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.LineHeightStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

// Porcelain Editorial & Obsidian Ink palette (matching Desktop Orca aesthetic, 0px square geometry)
val MuxCanvas = Color(0xFFF4F2ED)
val MuxBg0 = Color(0xFFF9F8F5)
val MuxBg1 = Color(0xFFFFFFFF)
val MuxBg2 = Color(0xFFF2F0EB)
val MuxBg3 = Color(0xFFE7E4DC)
val MuxLine = Color(0xFFE6E3DC)
val MuxControlLine = Color(0xFFD0CCC2)
val MuxFg0 = Color(0xFF14171F)
val MuxFg1 = Color(0xFF3E4451)
val MuxFg2 = Color(0xFF707786)

val MuxAccent = Color(0xFF181C24)
val MuxCobalt = Color(0xFF2563EB)
val MuxAccentBg = Color(0xFFEFF4FF)
val MuxUserSurface = Color(0xFFF0EEE8)
val MuxCodeBg = Color(0xFFF5F4F0)
val MuxCodeHeader = Color(0xFFECEAE4)
val MuxGreen = Color(0xFF16803C)
val MuxYellow = Color(0xFFB45309)
val MuxRed = Color(0xFFDC2626)
val MuxOnAccent = Color.White

// Porcelain Light terminal surface (matching Desktop Porcelain Light theme)
val TerminalSurface = Color(0xFFFAF9F6)
val TerminalKeycap = Color(0xFFFFFFFF)
val TerminalBorder = Color(0xFFD0CCC2)
val TerminalDivider = Color(0xFFE6E3DC)
val TerminalForeground = Color(0xFF171A21)
val TerminalMuted = Color(0xFF6B7280)
val TerminalAccent = Color(0xFF2563EB)

val MuxErrorBg = Color(0xFFFEF2F2)
val MuxErrorFg = Color(0xFFB91C1C)
val MuxErrorLine = Color(0xFFFECACA)

val MuxWarningBg = Color(0xFFFFFBEB)
val MuxWarningFg = Color(0xFF92400E)
val MuxWarningLine = Color(0xFFFDE68A)

private val colors = lightColorScheme(
    primary = MuxAccent,
    onPrimary = Color.White,
    primaryContainer = MuxAccentBg,
    onPrimaryContainer = MuxFg0,
    secondary = MuxCobalt,
    secondaryContainer = MuxBg2,
    background = MuxBg0,
    surface = MuxBg1,
    surfaceVariant = MuxBg2,
    onSurface = MuxFg0,
    onSurfaceVariant = MuxFg2,
    outline = MuxControlLine,
    outlineVariant = MuxLine,
    error = MuxRed,
)

private val zeroRadius = CutCornerShape(0.dp)

private val squareShapes = Shapes(
    extraSmall = zeroRadius,
    small = zeroRadius,
    medium = zeroRadius,
    large = zeroRadius,
    extraLarge = zeroRadius,
)

private val cjkLineHeightStyle = LineHeightStyle(
    alignment = LineHeightStyle.Alignment.Center,
    trim = LineHeightStyle.Trim.None,
)

@Suppress("DEPRECATION")
private val cjkPlatformStyle = PlatformTextStyle(includeFontPadding = false)

private fun editorialStyle(
    fontSize: Float,
    lineHeight: Float,
    fontWeight: FontWeight = FontWeight.Normal,
    letterSpacing: Float = 0f,
): TextStyle = TextStyle(
    fontSize = fontSize.sp,
    lineHeight = lineHeight.sp,
    fontWeight = fontWeight,
    letterSpacing = letterSpacing.sp,
    lineHeightStyle = cjkLineHeightStyle,
    platformStyle = cjkPlatformStyle,
)

private val typography = Typography(
    headlineSmall = editorialStyle(22f, 32f, FontWeight.Bold, -0.2f),
    titleLarge = editorialStyle(18f, 26f, FontWeight.SemiBold, -0.15f),
    titleMedium = editorialStyle(15f, 23f, FontWeight.SemiBold),
    bodyLarge = editorialStyle(15f, 24f),
    bodyMedium = editorialStyle(14.5f, 23f),
    bodySmall = editorialStyle(13f, 21f),
    labelLarge = editorialStyle(14f, 21f, FontWeight.SemiBold),
    labelMedium = editorialStyle(12.5f, 19f, FontWeight.Medium),
    labelSmall = editorialStyle(11f, 17f, FontWeight.Medium, 0.2f),
)

@Composable
fun MuxlaneTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = colors,
        shapes = squareShapes,
        typography = typography,
        content = content,
    )
}

val Square = RectangleShape
val MonoFamily = FontFamily.Monospace
