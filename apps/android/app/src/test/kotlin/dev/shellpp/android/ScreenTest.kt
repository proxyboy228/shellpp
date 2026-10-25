package dev.shellpp.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Builds snapshots in the layout of apps/android/native/src/screen.rs. */
object Snapshots {
    fun of(
        lines: List<String>,
        cols: Int,
        wraps: List<Boolean> = List(lines.size) { false },
        cursor: Pair<Int, Int> = Pair(0, 0),
        modes: Int = 0,
        combined: Map<Pair<Int, Int>, String> = emptyMap(),
        wide: Set<Pair<Int, Int>> = emptySet(),
    ): ByteArray {
        val rows = lines.size
        val extra = combined.entries.sumOf { 6 + it.value.toByteArray().size }
        val b = ByteBuffer.allocate(Screen.HEADER_BYTES + rows + rows * cols * Screen.CELL_BYTES + 4 + extra)
            .order(ByteOrder.LITTLE_ENDIAN)
        b.putShort(cols.toShort()).putShort(rows.toShort())
        b.putShort(cursor.first.toShort()).putShort(cursor.second.toShort())
        b.put(1).put(0).put(modes.toByte()).put(0)
        b.putInt(0x101010).putInt(0xDDDDDD).putInt(0x2EC4A6).putInt(0x101010)
        b.putInt(0x4D2EC4A6).putInt(0).putInt(0).putInt(0)
        for (w in wraps) b.put(if (w) 1 else 0)
        for ((row, line) in lines.withIndex()) {
            val points = line.codePoints().toArray()
            for (col in 0 until cols) {
                val c = points.getOrElse(col) { 0 }
                b.putInt(if (c == ' '.code) 0 else c).putInt(0xDDDDDD).putInt(0x101010)
                var flags = 0
                if (Pair(col, row) in wide) flags = flags or Screen.FLAG_WIDE
                if (Pair(col - 1, row) in wide) flags = flags or Screen.FLAG_WIDE_SPACER
                if (Pair(col, row) in combined) flags = flags or Screen.FLAG_COMBINED
                b.putShort(flags.toShort())
            }
        }
        b.putInt(combined.size)
        for ((cell, text) in combined) {
            val bytes = text.toByteArray()
            b.putShort(cell.first.toShort()).putShort(cell.second.toShort()).putShort(bytes.size.toShort())
            b.put(bytes)
        }
        return b.array()
    }
}

class ScreenTest {
    @Test
    fun aSnapshotReadsBack() {
        val bytes = Snapshots.of(
            listOf("ab  ", "e   "), cols = 4, cursor = Pair(1, 1),
            modes = Screen.MODE_ALT_SCREEN or Screen.MODE_MOUSE,
            combined = mapOf(Pair(0, 1) to "é"),
        )
        val screen = Screen.parse(bytes)
        assertEquals(4, screen.cols)
        assertEquals(2, screen.rows)
        assertEquals(Pair(1, 1), Pair(screen.cursorCol, screen.cursorRow))
        assertTrue(screen.alternateScreen)
        assertTrue(screen.mouse)
        assertEquals(0x4D2EC4A6, screen.selection)
        assertEquals("a", screen.cellText(0, 0))
        assertEquals("", screen.cellText(2, 0))
        assertEquals("é", screen.cellText(0, 1))
        assertEquals(listOf("ab", "é"), screen.lines())
    }

    @Test
    fun textJoinsWrappedRowsAndDropsTrailingBlanks() {
        val screen = Screen.parse(
            Snapshots.of(listOf("echo", " ok ", "next"), cols = 4, wraps = listOf(true, false, false)),
        )
        assertEquals("echo ok\nnext", screen.text(0, 0, 3, 2))
        assertEquals(listOf("echo ok", "next"), screen.logicalLines())
        assertEquals("ho ok", screen.text(2, 0, 3, 1))
    }

    @Test
    fun aWideCharacterIsOneCharacterOfText() {
        val screen = Screen.parse(Snapshots.of(listOf("漢 x"), cols = 4, wide = setOf(Pair(0, 0))))
        assertEquals("漢", screen.cellText(0, 0))
        assertEquals("", screen.cellText(1, 0))
        assertFalse(screen.isWordEnd(0, 0))
    }
}
