package dev.shellpp.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SelectionTest {
    private fun screen(vararg lines: String, cols: Int, wraps: List<Boolean>? = null) =
        Screen.parse(Snapshots.of(lines.toList(), cols, wraps ?: List(lines.size) { false }))

    @Test
    fun aLongPressSelectsAPathAsOneWord() {
        val s = screen("vi /etc/nginx/nginx.conf", cols = 30)
        val word = Selection.word(s, Cell(8, 0))
        assertEquals("/etc/nginx/nginx.conf", word.text(s))
    }

    @Test
    fun wordsEndAtTheDesktopsCharacters() {
        val s = screen("key=\"value\" (x)", cols = 20)
        assertEquals("key=", Selection.word(s, Cell(1, 0)).text(s))
        assertEquals("value", Selection.word(s, Cell(6, 0)).text(s))
        // A word-end character selects itself.
        assertEquals("(", Selection.word(s, Cell(12, 0)).text(s))
    }

    @Test
    fun aWordRunsAcrossAWrap() {
        val s = screen("abc12", "345 x", cols = 5, wraps = listOf(true, false))
        assertEquals("abc12345", Selection.word(s, Cell(1, 1)).text(s))
    }

    @Test
    fun aLineIsTheRowsItWrapsOver() {
        val s = screen("first", "wrapp", "ed ok", "other", cols = 5, wraps = listOf(false, true, false, false))
        val line = Selection.line(s, 2)
        assertEquals(Cell(0, 1), line.start)
        assertEquals("wrapped ok", line.text(s))
        assertEquals("first\nwrapped ok\nother", Selection.all(s).text(s))
    }

    @Test
    fun aHandleDraggedPastTheOtherEndSwapsThem() {
        val sel = Selection(Cell(2, 1), Cell(5, 1))
        val moved = sel.withEnd(moveStart = false, to = Cell(0, 1))
        assertEquals(Selection(Cell(0, 1), Cell(2, 1)), moved)
        assertTrue(moved.contains(1, 1))
        assertFalse(moved.contains(3, 1))
    }
}
