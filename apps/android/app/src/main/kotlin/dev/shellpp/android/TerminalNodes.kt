package dev.shellpp.android

import android.graphics.Rect
import android.os.Bundle
import androidx.core.view.accessibility.AccessibilityNodeInfoCompat
import androidx.customview.widget.ExploreByTouchHelper

/**
 * The terminal for TalkBack (architecture 6.1; plan of run 16, 3.3): a
 * Canvas view has no text of its own, so each row of the screen is a
 * virtual node with the row's text, and the cursor's row says that it is.
 * The helper takes TalkBack's focus, touch exploration (a finger over a row
 * reads it) and its double tap (the keyboard, as a tap on the terminal);
 * [TerminalView] announces the cursor's row when the output settles.
 */
class TerminalNodes(private val view: TerminalView) : ExploreByTouchHelper(view) {
    override fun getVirtualViewAt(x: Float, y: Float): Int {
        val s = view.screen ?: return INVALID_ID
        val row = view.rowAt(y)
        return if (row in 0 until s.rows && shown(row)) row else INVALID_ID
    }

    override fun getVisibleVirtualViews(virtualViewIds: MutableList<Int>) {
        val s = view.screen ?: return
        for (row in 0 until s.rows) if (shown(row)) virtualViewIds.add(row)
    }

    /** Whether [row] is on the view: after a pinch or with the keyboard
     * opening, the screen can have rows below its bottom edge. */
    private fun shown(row: Int): Boolean = view.rowBounds(row).top < view.height

    override fun onPopulateNodeForHost(node: AccessibilityNodeInfoCompat) {
        node.contentDescription = view.context.getString(R.string.terminal_description)
    }

    @Suppress("DEPRECATION")
    override fun onPopulateNodeForVirtualView(virtualViewId: Int, node: AccessibilityNodeInfoCompat) {
        val s = view.screen
        if (s == null || virtualViewId !in 0 until s.rows) {
            // A row the screen no longer has (it shrank): an empty node.
            node.text = ""
            node.setBoundsInParent(Rect())
            return
        }
        val row = virtualViewId
        node.className = "android.widget.TextView"
        val text = s.text(0, row, s.cols - 1, row).trimEnd()
        node.text = text.ifEmpty { " " }
        if (s.cursorVisible && row == s.cursorRow) {
            node.stateDescription = view.context.getString(R.string.terminal_cursor_row)
        }
        node.setBoundsInParent(view.rowBounds(row))
        node.addAction(AccessibilityNodeInfoCompat.AccessibilityActionCompat.ACTION_CLICK)
    }

    override fun onPerformActionForVirtualView(virtualViewId: Int, action: Int, arguments: Bundle?): Boolean {
        if (action != AccessibilityNodeInfoCompat.ACTION_CLICK) return false
        view.showKeyboard()
        return true
    }
}

/** A row's rectangle in the view, for the virtual nodes. */
fun Rect.ofRow(row: Int, width: Int, rowHeight: Float): Rect {
    set(0, (row * rowHeight).toInt(), width, ((row + 1) * rowHeight).toInt())
    return this
}
