package dev.shellpp.android

import android.content.Context
import org.json.JSONArray

/**
 * A key of the extra-keys row (reference-2 7.3, 9.2; D220). Its [id] is
 * what Settings → Customize keyboard stores; [label] is drawn on its chip.
 */
sealed class XKey(val id: String, val label: String) {
    /** The host's saved password, after a confirmation (D220). */
    data object Password : XKey("password", "")

    /** Moves the cursor by dragging on the terminal while on. */
    data object Cursor : XKey("cursor", "")

    /** A file from the system picker, uploaded by SFTP (D220 item 6). */
    data object Attach : XKey("attach", "")

    /** Sticky Ctrl and Alt (UI spec 9.2). */
    data object Ctrl : XKey("ctrl", "ctrl")
    data object Alt : XKey("alt", "alt")

    /** A key the core maps ([Native] key codes), repeating while held when [repeats]. */
    class Code(id: String, label: String, val code: Int, val repeats: Boolean = false) : XKey(id, label)

    /** Text as typed, [ctrl] for a Ctrl combination; [times] for one sent twice (^X^X). */
    class Text(id: String, label: String, val text: String, val ctrl: Boolean = false, val times: Int = 1) : XKey(id, label)

    /** A sequence of key presses (tmux: the prefix, then a key). */
    class Sequence(id: String, label: String, val steps: List<Text>) : XKey(id, label)

    val isIcon: Boolean get() = this is Password || this is Cursor || this is Attach

    /** What TalkBack says for the key (plan of run 16, 3.3): words, not the glyph drawn. */
    val spoken: String get() = spokenName(id, label)

    companion object {
        private const val ESC = "\u001b"

        private fun ctrl(letter: Char) = Text("^${letter.uppercaseChar()}", "^${letter.uppercaseChar()}", letter.toString(), ctrl = true)

        private fun tmux(id: String, key: String) =
            Sequence(id, "^B $key", listOf(Text("", "", "b", ctrl = true), Text("", "", key)))

        /** Every key, by id. */
        val ALL: Map<String, XKey> = buildList {
            add(Password)
            add(Cursor)
            add(Attach)
            // The sequences come from the shared table (D224): armed Ctrl and
            // Alt apply to these keys too.
            add(Code("shift_tab", "shift+tab", Native.KEY_BACKTAB))
            add(Ctrl)
            add(Code("esc", "esc", Native.KEY_ESC))
            add(Code("left", "←", Native.KEY_LEFT, repeats = true))
            add(Code("up", "↑", Native.KEY_UP, repeats = true))
            add(Code("down", "↓", Native.KEY_DOWN, repeats = true))
            add(Code("right", "→", Native.KEY_RIGHT, repeats = true))
            add(tmux("tmux_split_h", "%"))
            add(tmux("tmux_split_v", "\""))
            add(tmux("tmux_next", "o"))
            add(tmux("tmux_close", "x"))
            add(Alt)
            add(Code("tab", "tab", Native.KEY_TAB))
            add(Code("ins", "ins", Native.KEY_INSERT))
            add(Code("del", "del", Native.KEY_DELETE, repeats = true))
            add(Code("home", "home", Native.KEY_HOME))
            add(Code("pgup", "pgUp", Native.KEY_PAGE_UP, repeats = true))
            add(Code("pgdn", "pgDn", Native.KEY_PAGE_DOWN, repeats = true))
            add(Code("end", "end", Native.KEY_END))
            for (sign in listOf("|", "\\", "?", "-", "/", ":", ";", "!", "~", "@", "$", "*", "^", "%", "=", "`", "<", ">", "(", ")", "{", "}", "[", "]")) {
                add(Text(sign, sign, sign))
            }
            for (n in 1..12) add(Code("f$n", "F$n", Native.KEY_F1 + n - 1))
            add(Text("^_", "^_", "_", ctrl = true))
            for (letter in "wrclszxgnp") add(ctrl(letter))
            add(Text("^X^X", "^X^X", "x", ctrl = true, times = 2))
        }.associateBy { it.id }

        /**
         * The default row: nineteen groups of reference-2 9.2, without the
         * AI key (D220).
         */
        val DEFAULT: List<List<String>> = listOf(
            listOf("password", "cursor", "attach"),
            listOf("shift_tab", "ctrl", "esc"),
            listOf("left", "up", "down", "right"),
            listOf("tmux_split_h", "tmux_split_v"),
            listOf("tmux_next", "tmux_close"),
            listOf("alt", "tab", "ins", "del"),
            listOf("home", "pgup", "pgdn", "end"),
            listOf("|", "\\", "?", "-"),
            listOf("/", ":", ";", "!"),
            listOf("~", "@", "$", "*"),
            listOf("^", "%", "=", "`"),
            listOf("<", ">", "(", ")"),
            listOf("{", "}", "[", "]"),
            listOf("f1", "f2", "f3", "f4"),
            listOf("f5", "f6", "f7", "f8"),
            listOf("f9", "f10", "f11", "f12"),
            listOf("^_", "^W", "^R", "^X^X"),
            listOf("^C", "^L", "^S", "^Z"),
            listOf("^X", "^G", "^N", "^P"),
        )

        /** The groups of this device (Customize keyboard; a device setting, D220 [decision]). */
        fun groups(context: Context): List<List<XKey>> {
            val stored = TerminalPrefs.of(context).getString(TerminalPrefs.KEYS, null)
            val ids = stored?.let { text ->
                runCatching {
                    val outer = JSONArray(text)
                    (0 until outer.length()).map { index ->
                        val inner = outer.getJSONArray(index)
                        (0 until inner.length()).map { inner.getString(it) }
                    }
                }.getOrNull()
            } ?: DEFAULT
            return ids.map { group -> group.mapNotNull { ALL[it] } }.filter { it.isNotEmpty() }
        }

        fun save(context: Context, groups: List<List<String>>?) {
            val prefs = TerminalPrefs.of(context).edit()
            if (groups == null) {
                prefs.remove(TerminalPrefs.KEYS)
            } else {
                prefs.putString(TerminalPrefs.KEYS, JSONArray(groups.map { JSONArray(it) }).toString())
            }
            prefs.apply()
        }
    }
}
