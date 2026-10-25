package dev.shellpp.android

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.res.colorResource
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.sp

/**
 * The design system's tokens on the phone (UI spec 2): the colors of
 * res/values(-night)/colors.xml, body text 16 sp, dense lists 14 sp,
 * captions 12 sp, section headings 18 sp, dialog titles 22 sp.
 */
object Tokens {
    val bgBase: Color @Composable get() = colorResource(R.color.bg_base)
    val bgSurface: Color @Composable get() = colorResource(R.color.bg_surface)
    val bgRaised: Color @Composable get() = colorResource(R.color.bg_raised)
    val bgInput: Color @Composable get() = colorResource(R.color.bg_input)
    val border: Color @Composable get() = colorResource(R.color.border_strong)
    val text: Color @Composable get() = colorResource(R.color.text_primary)
    val textSecondary: Color @Composable get() = colorResource(R.color.text_secondary)
    val textMuted: Color @Composable get() = colorResource(R.color.text_muted)
    val accent: Color @Composable get() = colorResource(R.color.accent)
    val onAccent: Color @Composable get() = colorResource(R.color.text_on_accent)
    val accentSubtle: Color @Composable get() = colorResource(R.color.accent_subtle)
    val danger: Color @Composable get() = colorResource(R.color.danger)
}

@Composable
fun ShellTheme(content: @Composable () -> Unit) {
    val dark = isSystemInDarkTheme()
    val base = if (dark) darkColorScheme() else lightColorScheme()
    val colors = base.copy(
        primary = Tokens.accent,
        onPrimary = Tokens.onAccent,
        // The floating button and the bottom bar's selection: the accent,
        // not Material's own purples (design-system tokens only).
        primaryContainer = Tokens.accent,
        onPrimaryContainer = Tokens.onAccent,
        secondary = Tokens.accent,
        onSecondary = Tokens.onAccent,
        secondaryContainer = Tokens.accentSubtle,
        onSecondaryContainer = Tokens.text,
        tertiary = Tokens.accent,
        onTertiary = Tokens.onAccent,
        background = Tokens.bgBase,
        onBackground = Tokens.text,
        surface = Tokens.bgSurface,
        onSurface = Tokens.text,
        surfaceVariant = Tokens.bgInput,
        onSurfaceVariant = Tokens.textSecondary,
        surfaceContainer = Tokens.bgSurface,
        surfaceContainerHigh = Tokens.bgRaised,
        surfaceContainerHighest = Tokens.bgRaised,
        surfaceContainerLow = Tokens.bgSurface,
        outline = Tokens.border,
        error = Tokens.danger,
    )
    val typography = Typography(
        bodyLarge = TextStyle(fontSize = 16.sp),
        bodyMedium = TextStyle(fontSize = 14.sp),
        bodySmall = TextStyle(fontSize = 12.sp),
        labelLarge = TextStyle(fontSize = 16.sp, fontWeight = FontWeight.Medium),
        titleMedium = TextStyle(fontSize = 18.sp, fontWeight = FontWeight.Medium),
        titleLarge = TextStyle(fontSize = 22.sp, fontWeight = FontWeight.Medium),
        headlineSmall = TextStyle(fontSize = 24.sp, fontWeight = FontWeight.Medium),
    )
    MaterialTheme(colorScheme = colors, typography = typography, content = content)
}
