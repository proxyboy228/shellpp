package dev.shellpp.android

/** A cell of the visible screen. */
data class Cell(val col: Int, val row: Int) : Comparable<Cell> {
    override fun compareTo(other: Cell): Int =
        if (row != other.row) row.compareTo(other.row) else col.compareTo(other.col)
}

/**
 * A selection on the visible screen (docs/mobile/ui-spec.md 9.4): from
 * [start] to [end], both included, in reading order.
 */
data class Selection(val start: Cell, val end: Cell) {
    fun contains(col: Int, row: Int): Boolean {
        val cell = Cell(col, row)
        return cell >= start && cell <= end
    }

    fun text(screen: Screen): String = screen.text(start.col, start.row, end.col, end.row)

    /** The selection with one end moved to [to] (a handle dragged). */
    fun withEnd(moveStart: Boolean, to: Cell): Selection {
        val fixed = if (moveStart) end else start
        return if (to <= fixed) Selection(to, fixed) else Selection(fixed, to)
    }

    companion object {
        /**
         * The word at [at] with the desktop's rule: it ends at a space or a
         * word-end character and runs across wrapped rows. A cell that is a
         * word end selects itself.
         */
        fun word(screen: Screen, at: Cell): Selection {
            if (screen.isWordEnd(at.col, at.row)) return Selection(at, at)
            var start = at
            while (true) {
                val prev = previous(screen, start) ?: break
                if (screen.isWordEnd(prev.col, prev.row)) break
                start = prev
            }
            var end = at
            while (true) {
                val next = next(screen, end) ?: break
                if (screen.isWordEnd(next.col, next.row)) break
                end = next
            }
            return Selection(start, end)
        }

        /** The logical line at [row]: the rows joined by wraps. */
        fun line(screen: Screen, row: Int): Selection {
            var first = row
            while (first > 0 && screen.wraps[first - 1]) first--
            var last = row
            while (last < screen.rows - 1 && screen.wraps[last]) last++
            return Selection(Cell(0, first), Cell(screen.cols - 1, last))
        }

        /** Everything visible. */
        fun all(screen: Screen) = Selection(Cell(0, 0), Cell(screen.cols - 1, screen.rows - 1))

        private fun previous(screen: Screen, cell: Cell): Cell? = when {
            cell.col > 0 -> Cell(cell.col - 1, cell.row)
            cell.row > 0 && screen.wraps[cell.row - 1] -> Cell(screen.cols - 1, cell.row - 1)
            else -> null
        }

        private fun next(screen: Screen, cell: Cell): Cell? = when {
            cell.col < screen.cols - 1 -> Cell(cell.col + 1, cell.row)
            cell.row < screen.rows - 1 && screen.wraps[cell.row] -> Cell(0, cell.row + 1)
            else -> null
        }
    }
}
