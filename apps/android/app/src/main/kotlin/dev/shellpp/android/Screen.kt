package dev.shellpp.android

import java.nio.ByteBuffer
import java.nio.ByteOrder

/**
 * The visible screen from the Rust model's snapshot
 * (apps/android/native/src/screen.rs describes the layout).
 */
class Screen(
    val cols: Int,
    val rows: Int,
    val cursorCol: Int,
    val cursorRow: Int,
    val cursorVisible: Boolean,
    val cursorShape: Int,
    val modes: Int,
    val background: Int,
    val foreground: Int,
    val cursorColor: Int,
    val cursorText: Int,
    /** ARGB. */
    val selection: Int,
    val scrollOffset: Int,
    val history: Int,
    val wraps: BooleanArray,
    val chars: IntArray,
    val fg: IntArray,
    val bg: IntArray,
    val flags: IntArray,
    val combined: Map<Int, String>,
) {
    val alternateScreen: Boolean get() = modes and MODE_ALT_SCREEN != 0
    val mouse: Boolean get() = modes and MODE_MOUSE != 0

    fun index(col: Int, row: Int) = row * cols + col

    /** The text of one cell: "" for an empty cell or the right half of a wide one. */
    fun cellText(col: Int, row: Int): String {
        val i = index(col, row)
        if (flags[i] and FLAG_WIDE_SPACER != 0) return ""
        combined[i]?.let { return it }
        val c = chars[i]
        return if (c == 0) "" else String(Character.toChars(c))
    }

    /** Whether a cell holds a character that ends a word for a double tap. */
    fun isWordEnd(col: Int, row: Int): Boolean {
        val text = cellText(col, row)
        return text.isEmpty() || (text.length == 1 && SEMANTIC_ESCAPE_CHARS.indexOf(text[0]) >= 0)
    }

    /**
     * The text between two cells in reading order: rows joined, a line
     * break after a row that does not wrap, trailing blanks of a row
     * dropped (the desktop's copy rules).
     */
    fun text(fromCol: Int, fromRow: Int, toCol: Int, toRow: Int): String {
        // A selection made on a screen of another size stays inside this one.
        if (cols == 0 || rows == 0) return ""
        val firstRow = fromRow.coerceIn(0, rows - 1)
        val lastRow = toRow.coerceIn(firstRow, rows - 1)
        val firstCol = fromCol.coerceIn(0, cols - 1)
        val lastCol = toCol.coerceIn(0, cols - 1)
        val out = StringBuilder()
        for (row in firstRow..lastRow) {
            val start = if (row == firstRow) firstCol else 0
            val end = if (row == lastRow) lastCol else cols - 1
            val line = StringBuilder()
            for (col in start..end) {
                val i = index(col, row)
                if (flags[i] and FLAG_WIDE_SPACER != 0) continue
                val t = cellText(col, row)
                line.append(if (t.isEmpty()) " " else t)
            }
            val wraps = row < lastRow && this.wraps[row]
            out.append(if (wraps) line else line.trimEnd())
            if (row < lastRow && !wraps) out.append('\n')
        }
        return out.toString()
    }

    /** Every row's text (tests, the end-to-end check). */
    fun lines(): List<String> = (0 until rows).map { text(0, it, cols - 1, it) }

    /** The screen's logical lines: rows a line wraps over joined. */
    fun logicalLines(): List<String> = text(0, 0, cols - 1, rows - 1).split('\n')

    companion object {
        const val HEADER_BYTES = 44
        const val CELL_BYTES = 14

        const val MODE_APP_CURSOR = 1
        const val MODE_MOUSE = 2
        const val MODE_SGR_MOUSE = 4
        const val MODE_BRACKETED_PASTE = 8
        const val MODE_ALT_SCREEN = 16

        const val FLAG_BOLD = 1
        const val FLAG_ITALIC = 2
        const val FLAG_UNDERLINE = 4
        const val FLAG_STRIKEOUT = 8
        const val FLAG_WIDE = 16
        const val FLAG_WIDE_SPACER = 32
        const val FLAG_COMBINED = 64

        /** The desktop's word ends (crates/terminal/src/selection.rs). */
        const val SEMANTIC_ESCAPE_CHARS = ",│`|:\"' ()[]{}<>\t"

        fun parse(bytes: ByteArray): Screen {
            val b = ByteBuffer.wrap(bytes).order(ByteOrder.LITTLE_ENDIAN)
            val cols = b.short.toInt() and 0xFFFF
            val rows = b.short.toInt() and 0xFFFF
            val cursorCol = b.short.toInt() and 0xFFFF
            val cursorRow = b.short.toInt() and 0xFFFF
            val cursorVisible = b.get().toInt() != 0
            val cursorShape = b.get().toInt()
            val modes = b.get().toInt()
            b.get()
            val background = b.int
            val foreground = b.int
            val cursorColor = b.int
            val cursorText = b.int
            val selection = b.int
            val offset = b.int
            val history = b.int
            b.int
            val wraps = BooleanArray(rows) { b.get().toInt() != 0 }
            val count = rows * cols
            val chars = IntArray(count)
            val fg = IntArray(count)
            val bg = IntArray(count)
            val flags = IntArray(count)
            for (i in 0 until count) {
                chars[i] = b.int
                fg[i] = b.int
                bg[i] = b.int
                flags[i] = b.short.toInt() and 0xFFFF
            }
            val combined = HashMap<Int, String>()
            val entries = b.int
            repeat(entries) {
                val col = b.short.toInt() and 0xFFFF
                val row = b.short.toInt() and 0xFFFF
                val length = b.short.toInt() and 0xFFFF
                val text = ByteArray(length)
                b.get(text)
                combined[row * cols + col] = String(text, Charsets.UTF_8)
            }
            return Screen(
                cols, rows, cursorCol, cursorRow, cursorVisible, cursorShape, modes,
                background, foreground, cursorColor, cursorText, selection, offset, history,
                wraps, chars, fg, bg, flags, combined,
            )
        }
    }
}
