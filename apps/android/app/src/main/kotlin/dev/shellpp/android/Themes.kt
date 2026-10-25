package dev.shellpp.android

import androidx.compose.ui.graphics.Color

/**
 * The built-in terminal themes (`crates/core/src/model/fields.rs`
 * `BUILTIN_THEMES`, colors from `apps/desktop/src/theme/terminal/`, one JSON file each):
 * the background, the foreground and the eight normal ANSI colors, for the
 * theme tiles of the pickers (reference-2 3.9, 9.3).
 */
data class TerminalTheme(val id: String, val name: String, val background: Long, val foreground: Long, val ansi: LongArray) {
    val bg: Color get() = Color(background)
    val fg: Color get() = Color(foreground)
    fun color(index: Int): Color = Color(ansi[index])
}

val TERMINAL_THEMES = listOf(
    TerminalTheme("graphite", "Graphite", 0xFF121211, 0xFFDCDCD5, longArrayOf(0xFF1E1E1C, 0xFFE06C64, 0xFF8FC46A, 0xFFE3B65A, 0xFF6EA2D9, 0xFFC08BD6, 0xFF4FC9B4, 0xFFCFCFC8)),
    TerminalTheme("paper", "Paper", 0xFFFAF9F6, 0xFF2A2A27, longArrayOf(0xFF2A2A27, 0xFFB3403A, 0xFF3F7D2E, 0xFF8E6A12, 0xFF2F62A0, 0xFF8A4AA0, 0xFF157A6A, 0xFF8A8982)),
    TerminalTheme("dracula", "Dracula", 0xFF282A36, 0xFFF8F8F2, longArrayOf(0xFF21222C, 0xFFFF5555, 0xFF50FA7B, 0xFFF1FA8C, 0xFFBD93F9, 0xFFFF79C6, 0xFF8BE9FD, 0xFFF8F8F2)),
    TerminalTheme("nord", "Nord", 0xFF2E3440, 0xFFD8DEE9, longArrayOf(0xFF3B4252, 0xFFBF616A, 0xFFA3BE8C, 0xFFEBCB8B, 0xFF81A1C1, 0xFFB48EAD, 0xFF88C0D0, 0xFFE5E9F0)),
    TerminalTheme("solarized-dark", "Solarized Dark", 0xFF002B36, 0xFF839496, longArrayOf(0xFF073642, 0xFFDC322F, 0xFF859900, 0xFFB58900, 0xFF268BD2, 0xFFD33682, 0xFF2AA198, 0xFFEEE8D5)),
    TerminalTheme("solarized-light", "Solarized Light", 0xFFFDF6E3, 0xFF657B83, longArrayOf(0xFF073642, 0xFFDC322F, 0xFF859900, 0xFFB58900, 0xFF268BD2, 0xFFD33682, 0xFF2AA198, 0xFFEEE8D5)),
    TerminalTheme("gruvbox-dark", "Gruvbox Dark", 0xFF282828, 0xFFEBDBB2, longArrayOf(0xFF282828, 0xFFCC241D, 0xFF98971A, 0xFFD79921, 0xFF458588, 0xFFB16286, 0xFF689D6A, 0xFFA89984)),
    TerminalTheme("tokyo-night", "Tokyo Night", 0xFF1A1B26, 0xFFC0CAF5, longArrayOf(0xFF15161E, 0xFFF7768E, 0xFF9ECE6A, 0xFFE0AF68, 0xFF7AA2F7, 0xFFBB9AF7, 0xFF7DCFFF, 0xFFA9B1D6)),
    TerminalTheme("catppuccin-mocha", "Catppuccin Mocha", 0xFF1E1E2E, 0xFFCDD6F4, longArrayOf(0xFF45475A, 0xFFF38BA8, 0xFFA6E3A1, 0xFFF9E2AF, 0xFF89B4FA, 0xFFF5C2E7, 0xFF94E2D5, 0xFFA6ADC8)),
    TerminalTheme("catppuccin-latte", "Catppuccin Latte", 0xFFEFF1F5, 0xFF4C4F69, longArrayOf(0xFF5C5F77, 0xFFD20F39, 0xFF40A02B, 0xFFDF8E1D, 0xFF1E66F5, 0xFFEA76CB, 0xFF179299, 0xFFACB0BE)),
)

fun terminalTheme(id: String?): TerminalTheme? = TERMINAL_THEMES.firstOrNull { it.id == id }
