package com.asa.chat.ui

import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Density

val AsaBackground = Color(0xFF0F1115)
val AsaSurface = Color(0xFF171A21)
val AsaSurfaceVariant = Color(0xFF20242D)
val AsaPrimary = Color(0xFF4DD0E1)
val AsaOnPrimary = Color(0xFF00201F)
val AsaPrimaryContainer = Color(0xFF123C42)
val AsaOnPrimaryContainer = Color(0xFFD6F7FA)
val AsaText = Color(0xFFE6E8EC)
val AsaMuted = Color(0xFF9AA3B2)
val AsaWarning = Color(0xFFFFC857)
val AsaError = Color(0xFFFF6B6B)

private val AsaColorScheme = darkColorScheme(
    primary = AsaPrimary,
    onPrimary = AsaOnPrimary,
    primaryContainer = AsaPrimaryContainer,
    onPrimaryContainer = AsaOnPrimaryContainer,
    secondary = AsaMuted,
    onSecondary = AsaBackground,
    background = AsaBackground,
    onBackground = AsaText,
    surface = AsaSurface,
    onSurface = AsaText,
    surfaceVariant = AsaSurfaceVariant,
    onSurfaceVariant = AsaMuted,
    error = AsaError,
    onError = AsaBackground,
    outline = Color(0xFF353B46),
)

private val AsaLightScheme = lightColorScheme(
    primary = Color(0xFF007A87),
    onPrimary = Color.White,
    primaryContainer = Color(0xFFC9F2F6),
    onPrimaryContainer = Color(0xFF00363B),
    secondary = Color(0xFF5A6270),
    onSecondary = Color.White,
    background = Color(0xFFF5F7FA),
    onBackground = Color(0xFF1A1C20),
    surface = Color.White,
    onSurface = Color(0xFF1A1C20),
    surfaceVariant = Color(0xFFE4E8EE),
    onSurfaceVariant = Color(0xFF5A6270),
    error = Color(0xFFB3261E),
    onError = Color.White,
    outline = Color(0xFFB8C0CC),
)

private val AsaHighContrast = darkColorScheme(
    primary = Color(0xFF00E5FF),
    onPrimary = Color.Black,
    primaryContainer = Color(0xFF003A40),
    onPrimaryContainer = Color.White,
    secondary = Color(0xFFFFD166),
    onSecondary = Color.Black,
    background = Color.Black,
    onBackground = Color.White,
    surface = Color(0xFF080A0C),
    onSurface = Color.White,
    surfaceVariant = Color(0xFF14181D),
    onSurfaceVariant = Color(0xFFE0E0E0),
    error = Color(0xFFFF5252),
    onError = Color.Black,
    outline = Color(0xFF8A8A8A),
)

@Composable
fun AsaTheme(
    theme: String = "dark",
    fontSize: Int = 15,
    content: @Composable () -> Unit,
) {
    val scheme = when (theme) {
        "light" -> AsaLightScheme
        "high-contrast" -> AsaHighContrast
        else -> AsaColorScheme
    }
    val density = LocalDensity.current
    val scale = fontSize.coerceIn(10, 30) / 15f
    CompositionLocalProvider(
        LocalDensity provides Density(density.density, scale),
    ) {
        MaterialTheme(colorScheme = scheme, content = content)
    }
}
