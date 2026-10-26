package dev.shellpp.android

import android.graphics.Rect
import android.os.Build
import android.view.View
import android.view.accessibility.AccessibilityNodeInfo
import android.view.accessibility.AccessibilityNodeProvider

/**
 * The terminal for TalkBack (architecture 6.1; plan of run 16, 3.3): a
 * Canvas view has no text of its own, so each row of the screen is a
 * virtual node with the row's text, and the cursor's row says that it is.
 * [TerminalView] announces the cursor's row when the output settles.
 */
class TerminalNodes(private val view: TerminalView) : AccessibilityNodeProvider() {
    @Suppress("DEPRECATION")
    override fun createAccessibilityNodeInfo(virtualViewId: Int): AccessibilityNodeInfo? {
        val screen = view.screen
        if (virtualViewId == View.NO_ID) {
            val info = AccessibilityNodeInfo.obtain(view)
            view.onInitializeAccessibilityNodeInfo(info)
            info.contentDescription = view.context.getString(R.string.terminal_description)
            if (screen != null) for (row in 0 until screen.rows) info.addChild(view, row)
            return info
        }
        if (screen == null || virtualViewId !in 0 until screen.rows) return null
        val row = virtualViewId
        val info = AccessibilityNodeInfo.obtain(view, row)
        info.setParent(view)
        info.setSource(view, row)
        info.packageName = view.context.packageName
        info.className = "android.widget.TextView"
        val text = screen.text(0, row, screen.cols - 1, row).trimEnd()
        info.text = text.ifEmpty { " " }
        val cursorRow = screen.cursorVisible && row == screen.cursorRow
        if (cursorRow) {
            if (Build.VERSION.SDK_INT >= 30) info.stateDescription = view.context.getString(R.string.terminal_cursor_row)
            else info.contentDescription = view.context.getString(R.string.terminal_cursor_row) + ", " + text
        }
        val bounds = view.rowBounds(row)
        info.setBoundsInParent(bounds)
        val origin = IntArray(2).also { view.getLocationOnScreen(it) }
        bounds.offset(origin[0], origin[1])
        info.setBoundsInScreen(bounds)
        info.isVisibleToUser = true
        info.isEnabled = true
        info.isFocusable = true
        return info
    }
}

/** A row's rectangle in the view, for the virtual nodes. */
fun Rect.ofRow(row: Int, width: Int, rowHeight: Float): Rect {
    set(0, (row * rowHeight).toInt(), width, ((row + 1) * rowHeight).toInt())
    return this
}
