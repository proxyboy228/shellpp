package dev.shellpp.android

/** The names of the signs of the keys row, as TalkBack says them. */
private val SIGNS = mapOf(
    "|" to "pipe", "\\" to "backslash", "?" to "question mark", "-" to "minus", "/" to "slash",
    ":" to "colon", ";" to "semicolon", "!" to "exclamation mark", "~" to "tilde", "@" to "at",
    "$" to "dollar", "*" to "asterisk", "^" to "caret", "%" to "percent", "=" to "equals",
    "`" to "backtick", "<" to "less than", ">" to "greater than", "(" to "left parenthesis",
    ")" to "right parenthesis", "{" to "left brace", "}" to "right brace", "[" to "left bracket",
    "]" to "right bracket", "_" to "underscore",
)

/**
 * The spoken name of the key [id] drawn as [label] (the terminal's
 * accessibility, plan of run 16, 3.3): arrows and page keys in words,
 * Ctrl combinations as "Control C", tmux keys by what they do.
 */
fun spokenName(id: String, label: String): String = when (id) {
    "password" -> "Send the saved password"
    "cursor" -> "Move the cursor by dragging"
    "attach" -> "Send a file to the server"
    "ctrl" -> "Control"
    "alt" -> "Alt"
    "shift_tab" -> "Shift Tab"
    "esc" -> "Escape"
    "tab" -> "Tab"
    "left" -> "Left arrow"
    "right" -> "Right arrow"
    "up" -> "Up arrow"
    "down" -> "Down arrow"
    "ins" -> "Insert"
    "del" -> "Delete"
    "home" -> "Home"
    "end" -> "End"
    "pgup" -> "Page up"
    "pgdn" -> "Page down"
    "tmux_split_h" -> "tmux: split side by side"
    "tmux_split_v" -> "tmux: split one above the other"
    "tmux_next" -> "tmux: next pane"
    "tmux_close" -> "tmux: close the pane"
    "^X^X" -> "Control X twice"
    else -> when {
        id.matches(Regex("f\\d+")) -> id.uppercase()
        label.startsWith("^") && label.length == 2 -> "Control " + (SIGNS[label.substring(1)] ?: label.substring(1))
        else -> SIGNS[label] ?: label
    }
}
