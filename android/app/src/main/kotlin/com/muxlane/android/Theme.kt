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

// DeepSeek Harness (dsh) Cool-Neutral Light palette with strict 0px square geometry
val MuxCanvas = Color(0xFFF1F3F5)
val MuxBg0 = Color(0xFFF9FAFB)       // Sidebar / workspace browser surface (--dsw-static-neutral-bluish-50)
val MuxBg1 = Color(0xFFFFFFFF)       // Pure white main canvas / cards (--dsw-static-neutral-bluish-00)
val MuxBg2 = Color(0xFFF5F6F7)       // Subtle chip / header / dock surface (--dsw-static-neutral-bluish-60)
val MuxBg3 = Color(0xFFEDF0F4)       // Selected row highlight (--dsw-static-neutral-bluish-90)
val MuxLine = Color(0xFFEAECEF)      // Hairline structural divider
val MuxControlLine = Color(0xFFD9DFE8) // Interactive control hairline border
val MuxFg0 = Color(0xFF0F1115)       // Primary ink (--dsw-static-neutral-bluish-1000)
val MuxFg1 = Color(0xFF43454A)       // Secondary ink (--dsw-static-neutral-bluish-750)
val MuxFg2 = Color(0xFF61666B)       // Tertiary / muted ink (--dsw-static-neutral-bluish-700)

val MuxAccent = Color(0xFF0F1115)    // Crisp obsidian primary action fill (--dsw-alias-button-primary-fill)
val MuxCobalt = Color(0xFF4176E6)    // DeepSeek Blue active accent (--dsw-static-deepseek-500)
val MuxAccentBg = Color(0xFFEDF3FE)  // Soft DeepSeek Blue tint (--dsw-static-deepseek-50)
val MuxUserSurface = Color(0xFFEDF3FE) // dsh signature light user message fill (--dsw-specific-bubble)
val MuxCodeBg = Color(0xFFF9FAFB)
val MuxCodeHeader = Color(0xFFF1F3F5)
val MuxGreen = Color(0xFF16A34A)
val MuxYellow = Color(0xFFD97706)
val MuxRed = Color(0xFFDC2626)
val MuxOnAccent = Color.White

// Pure Light terminal surface aligned with dsh light aesthetic
val TerminalSurface = Color(0xFFFFFFFF)
val TerminalKeycap = Color(0xFFFFFFFF)
val TerminalBorder = Color(0xFFD9DFE8)
val TerminalDivider = Color(0xFFEAECEF)
val TerminalForeground = Color(0xFF0F1115)
val TerminalMuted = Color(0xFF61666B)
val TerminalAccent = Color(0xFF4176E6)

val MuxErrorBg = Color(0xFFFEF2F2)
val MuxErrorFg = Color(0xFFB91C1C)
val MuxErrorLine = Color(0xFFFECACA)

val MuxWarningBg = Color(0xFFFEF5E7)
val MuxWarningFg = Color(0xFFB45309)
val MuxWarningLine = Color(0xFFFDE68A)

private val colors = lightColorScheme(
    primary = MuxAccent,
    onPrimary = Color.White,
    primaryContainer = MuxAccentBg,
    onPrimaryContainer = MuxFg0,
    secondary = MuxCobalt,
    secondaryContainer = MuxBg2,
    background = MuxBg1,
    surface = MuxBg1,
    surfaceVariant = MuxBg0,
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
