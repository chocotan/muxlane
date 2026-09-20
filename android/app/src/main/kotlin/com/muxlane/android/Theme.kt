package com.muxlane.android

import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.RectangleShape
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/** OLED 画布。铬（顶栏、列表）走 One Dark，终端再暗一档。 */
val MuxCanvas = Color(0xFF12141A)
val MuxBg0 = Color(0xFF1B1E24)
val MuxBg1 = Color(0xFF21252B)
val MuxBg2 = Color(0xFF2C313A)
val MuxLine = Color(0xFF3E4451)
val MuxFg0 = Color(0xFFE6E9EF)
val MuxFg1 = Color(0xFFC5CAD3)
val MuxFg2 = Color(0xFF9AA1AD)
val MuxAccent = Color(0xFF61AFEF)
val MuxGreen = Color(0xFF98C379)
val MuxYellow = Color(0xFFE5C07B)
val MuxRed = Color(0xFFE06C75)
val MuxOnAccent = Color(0xFF0F1419)

private val colors: ColorScheme = darkColorScheme(
    primary = MuxAccent,
    onPrimary = MuxOnAccent,
    background = MuxBg0,
    onBackground = MuxFg0,
    surface = MuxBg1,
    onSurface = MuxFg0,
    surfaceVariant = MuxBg2,
    outline = MuxLine,
    error = MuxRed,
)

@Composable
fun MuxlaneTheme(content: @Composable () -> Unit) {
    MaterialTheme(
        colorScheme = colors,
        typography = Typography(
            bodyLarge = androidx.compose.ui.text.TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 15.sp,
                lineHeight = 24.sp,
                color = MuxFg1,
            ),
            titleLarge = androidx.compose.ui.text.TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Medium,
                fontSize = 22.sp,
                lineHeight = 28.sp,
                color = MuxFg0,
            ),
            labelMedium = androidx.compose.ui.text.TextStyle(
                fontFamily = FontFamily.Default,
                fontWeight = FontWeight.Normal,
                fontSize = 13.sp,
                lineHeight = 20.sp,
                color = MuxFg2,
            ),
        ),
        content = content,
    )
}

val Square = RectangleShape
