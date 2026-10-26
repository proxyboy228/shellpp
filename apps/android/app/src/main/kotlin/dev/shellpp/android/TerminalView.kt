package dev.shellpp.android

import android.content.ClipData
import android.content.ClipboardManager
import android.content.Context
import android.graphics.Canvas
import android.graphics.Paint
import android.graphics.Rect
import android.graphics.Typeface
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.text.InputType
import android.view.ActionMode
import android.view.GestureDetector
import android.view.KeyEvent
import android.view.Menu
import android.view.MenuItem
import android.view.MotionEvent
import android.view.ScaleGestureDetector
import android.view.View
import android.view.inputmethod.BaseInputConnection
import android.view.inputmethod.EditorInfo
import android.view.inputmethod.InputConnection
import android.view.inputmethod.InputMethodManager
import android.widget.Magnifier
import android.widget.Toast
import org.json.JSONObject
import java.util.concurrent.Executors
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min

/**
 * The terminal of the prototype (docs/mobile/ui-spec.md 9): draws the Rust
 * model's snapshot, sends the keyboard's input, and does the gestures of the
 * gesture table: a tap puts the cursor (a click or a walk with feedback, in
 * Rust), a handle under the cursor moves it precisely with the magnifier, a
 * long press selects a word with handles and a floating toolbar, a vertical
 * swipe scrolls, a pinch zooms the font.
 */
class TerminalView(context: Context, private val modifiers: Modifiers) : View(context) {
    /** The armed or locked Ctrl and Alt of the extra keys. */
    interface Modifiers {
        val ctrl: Boolean
        val alt: Boolean
        fun consume()
    }

    var session: Long = 0
        private set

    /** Let go by its activity ([release]); its pump ends. */
    @Volatile
    private var released = false

    /** The screen drawn last (tests read it). */
    @Volatile
    var screen: Screen? = null
        private set

    /** The size the terminal has now, once laid out. */
    var onSize: ((cols: Int, rows: Int) -> Unit)? = null

    /**
     * The cursor key is on (reference-2 7.3; plan 3.3): a drag on the
     * terminal moves the cursor with the arrow keys, at Settings' cursor
     * speed.
     */
    var cursorPad = false

    /** The screen is not seen: wake-ups are taken, no snapshot is made (M2). */
    @Volatile
    var paused = false
        set(value) {
            val resumed = field && !value
            field = value
            // The app left the screen: the paste question goes with it.
            if (value) dismissPaste()
            if (resumed && session != 0L && !released) Thread({ refresh() }, "terminal-resume").start()
        }

    /** The screen's rows as nodes for TalkBack (plan of run 16, 3.3). */
    private val nodes by lazy { TerminalNodes(this) }

    override fun getAccessibilityNodeProvider(): android.view.accessibility.AccessibilityNodeProvider = nodes

    /** The rectangle of [row] in this view (the accessibility nodes). */
    fun rowBounds(row: Int): Rect = Rect().ofRow(row, width, cellH)

    /** The cursor's row last announced to TalkBack. */
    private var spokenCursorRow: String? = null

    /** Announces the cursor's row once the output settled, when TalkBack is on. */
    private val announceCursorRow = Runnable {
        val s = screen ?: return@Runnable
        if (!s.cursorVisible || s.cursorRow !in 0 until s.rows) return@Runnable
        val line = s.text(0, s.cursorRow, s.cols - 1, s.cursorRow).trimEnd()
        if (line.isNotEmpty() && line != spokenCursorRow) {
            spokenCursorRow = line
            @Suppress("DEPRECATION")
            announceForAccessibility(line)
        }
    }

    private fun talkBackOn(): Boolean =
        context.getSystemService(android.view.accessibility.AccessibilityManager::class.java)?.isTouchExplorationEnabled == true

    /** Shown in the activity (the session in front): its bell rings, its paste question stays. */
    @Volatile private var onScreen = false

    override fun onAttachedToWindow() {
        super.onAttachedToWindow()
        onScreen = true
        bellArmed = false
    }

    override fun onDetachedFromWindow() {
        // Another session came to the front: its question goes (review of run 16).
        onScreen = false
        dismissPaste()
        super.onDetachedFromWindow()
    }

    private var padRestX = 0f
    private var padRestY = 0f

    private val main = Handler(Looper.getMainLooper())
    private val worker = Executors.newSingleThreadExecutor()
    private val paint = Paint(Paint.ANTI_ALIAS_FLAG).apply {
        typeface = Typeface.createFromAsset(context.assets, "fonts/JetBrainsMono-Regular.ttf")
    }
    private val fill = Paint()
    // Pinch changes it for this view only; Settings keeps the device's size.
    private var fontSp = TerminalPrefs.of(context).getInt(TerminalPrefs.FONT, TerminalPrefs.DEFAULT_FONT_SP).toFloat()
    private var cellW = 1f
    private var cellH = 1f
    private var ascent = 0f
    private var cols = 0
    private var rows = 0
    private var pendingResize: Runnable? = null
    private val bell = Bell(context)
    /** False until the first snapshot since the session was seen again. */
    @Volatile private var bellArmed = false

    private var selection: Selection? = null
    private var actionMode: ActionMode? = null
    private var dragging: Drag = Drag.None
    private var cursorHandleUntil = 0L
    private var cursorMark: Cell? = null
    private var magnifier: Magnifier? = null
    private var scrollRest = 0f

    private enum class Drag { None, SelectionStart, SelectionEnd, Cursor }

    init {
        isFocusable = true
        isFocusableInTouchMode = true
        updateMetrics()
    }

    // ----- session --------------------------------------------------------

    /** Starts drawing session [id]: a pump thread waits for the model's wake-ups. */
    fun attach(id: Long) {
        session = id
        // A size laid out before the session existed reaches the server now.
        if (cols > 0 && rows > 0) worker.execute { runCatching { Native.resize(id, cols, rows) } }
        Thread({
            refresh()
            while (try { Native.awaitWake(id) } catch (e: RuntimeException) { false }) {
                if (released) {
                    // One wake channel per session: a wake this let-go view
                    // took belongs to the screen that replaced it.
                    runCatching { Native.wake(id) }
                    break
                }
                refresh()
            }
        }, "terminal-pump-$id").apply { isDaemon = true }.start()
    }

    private fun refresh() {
        if (paused) {
            // A bell while the session is not seen does not ring later.
            bellArmed = false
            return
        }
        val started = System.nanoTime()
        val bytes = try {
            Native.snapshot(session, true)
        } catch (e: RuntimeException) {
            return
        }
        val parsed = Screen.parse(bytes)
        FrameTiming.snapshot(System.nanoTime() - started, parsed.cols * parsed.rows)
        // Only the session on the screen rings, and a bell from while it was
        // not there does not ring later.
        if (parsed.bell && bellArmed && onScreen) bell.ring()
        bellArmed = onScreen
        main.post {
            val before = screen
            screen = parsed
            val shown = (0xFF000000.toInt() or parsed.background) to (0xFF000000.toInt() or parsed.foreground)
            if (colors[session] != shown) colors[session] = shown
            // Another program took the screen (nano left, a pager came):
            // what was selected and offered no longer holds.
            if (selection != null && before != null &&
                (before.alternateScreen != parsed.alternateScreen || before.mouse != parsed.mouse)
            ) {
                clearSelection()
            }
            invalidate()
            // TalkBack reads the rows again, and the cursor's row once the
            // output has been quiet for a moment.
            if (onScreen && talkBackOn()) {
                sendAccessibilityEvent(android.view.accessibility.AccessibilityEvent.TYPE_WINDOW_CONTENT_CHANGED)
                removeCallbacks(announceCursorRow)
                postDelayed(announceCursorRow, ANNOUNCE_DELAY_MS)
            }
        }
    }

    /** Types [text] as the soft keyboard would (tests). */
    fun typeText(text: String) = sendText(text)

    // ----- metrics and size --------------------------------------------------

    private fun updateMetrics() {
        paint.textSize = fontSp * resources.displayMetrics.scaledDensity
        val metrics = paint.fontMetrics
        cellW = paint.measureText("M")
        cellH = (metrics.descent - metrics.ascent) * 1.2f
        ascent = -metrics.ascent + (cellH - (metrics.descent - metrics.ascent)) / 2
    }

    override fun onSizeChanged(w: Int, h: Int, oldw: Int, oldh: Int) {
        super.onSizeChanged(w, h, oldw, oldh)
        sizeChanged()
    }

    private fun sizeChanged(settleMs: Long = 150) {
        val newCols = max(1, (width / cellW).toInt())
        val newRows = max(1, (height / cellH).toInt())
        if (newCols == cols && newRows == rows) return
        cols = newCols
        rows = newRows
        // Cells of the old size mean nothing on the new screen (review of run 13).
        if (selection != null) clearSelection()
        pendingResize?.let { main.removeCallbacks(it) }
        // Once, after the change settles (ui-spec 9.8).
        val resize = Runnable {
            onSize?.invoke(cols, rows)
            if (session != 0L && !worker.isShutdown) worker.execute { runCatching { Native.resize(session, cols, rows) } }
        }
        pendingResize = resize
        if (session == 0L) resize.run() else main.postDelayed(resize, settleMs)
    }

    /** Columns and rows of the laid-out view. */
    fun size(): Pair<Int, Int> = Pair(cols, rows)

    /** The center of a cell in screen coordinates (tests inject touches there). */
    fun cellOnScreen(col: Int, row: Int): Pair<Float, Float> {
        val origin = IntArray(2).also { getLocationOnScreen(it) }
        return Pair(origin[0] + col * cellW + cellW / 2, origin[1] + row * cellH + cellH / 2)
    }

    // ----- drawing -------------------------------------------------------

    override fun onDraw(canvas: Canvas) {
        val started = System.nanoTime()
        drawScreen(canvas)
        FrameTiming.drawn(System.nanoTime() - started)
    }

    private fun drawScreen(canvas: Canvas) {
        val s = screen
        if (s == null) {
            canvas.drawColor(context.color(R.color.bg_base))
            return
        }
        canvas.drawColor(0xFF000000.toInt() or s.background)
        val sel = selection
        for (row in 0 until s.rows) {
            val y = row * cellH
            var col = 0
            while (col < s.cols) {
                val i = s.index(col, row)
                val wide = s.flags[i] and Screen.FLAG_WIDE != 0
                val span = if (wide) 2 else 1
                val x = col * cellW
                if (s.bg[i] != s.background) {
                    fill.color = 0xFF000000.toInt() or s.bg[i]
                    canvas.drawRect(x, y, x + cellW * span, y + cellH, fill)
                }
                col += span
            }
            drawRowText(canvas, s, row, y)
            if (sel != null) {
                fill.color = s.selection
                for (c in 0 until s.cols) {
                    if (sel.contains(c, row)) canvas.drawRect(c * cellW, y, (c + 1) * cellW, y + cellH, fill)
                }
            }
        }
        drawCursor(canvas, s)
        drawHandles(canvas, s)
    }

    /** Runs of cells with the same color and style go out as one drawText. */
    private fun drawRowText(canvas: Canvas, s: Screen, row: Int, y: Float) {
        val run = StringBuilder()
        var runStart = 0
        var runFg = 0
        var runFlags = 0
        fun flush() {
            if (run.isEmpty()) return
            paint.color = 0xFF000000.toInt() or runFg
            paint.isFakeBoldText = runFlags and Screen.FLAG_BOLD != 0
            paint.textSkewX = if (runFlags and Screen.FLAG_ITALIC != 0) -0.2f else 0f
            paint.isUnderlineText = runFlags and Screen.FLAG_UNDERLINE != 0
            paint.isStrikeThruText = runFlags and Screen.FLAG_STRIKEOUT != 0
            canvas.drawText(run.toString(), runStart * cellW, y + ascent, paint)
            run.setLength(0)
        }
        val styleMask = Screen.FLAG_BOLD or Screen.FLAG_ITALIC or Screen.FLAG_UNDERLINE or Screen.FLAG_STRIKEOUT
        for (col in 0 until s.cols) {
            val i = s.index(col, row)
            val flags = s.flags[i]
            if (flags and Screen.FLAG_WIDE_SPACER != 0) continue
            val single = flags and (Screen.FLAG_WIDE or Screen.FLAG_COMBINED) == 0
            if (!single || s.fg[i] != runFg || flags and styleMask != runFlags and styleMask) {
                flush()
                runStart = col
                runFg = s.fg[i]
                runFlags = flags
            }
            val text = s.cellText(col, row)
            if (single) {
                run.append(if (text.isEmpty()) " " else text)
            } else {
                run.append(text)
                flush()
                runStart = col + if (flags and Screen.FLAG_WIDE != 0) 2 else 1
            }
        }
        flush()
    }

    private fun drawCursor(canvas: Canvas, s: Screen) {
        if (!s.cursorVisible || s.scrollOffset != 0) return
        val x = s.cursorCol * cellW
        val y = s.cursorRow * cellH
        fill.color = 0xFF000000.toInt() or s.cursorColor
        when (s.cursorShape) {
            1 -> canvas.drawRect(x, y + cellH - cellH / 8, x + cellW, y + cellH, fill)
            2 -> canvas.drawRect(x, y, x + max(2f, cellW / 8), y + cellH, fill)
            3 -> {
                fill.style = Paint.Style.STROKE
                fill.strokeWidth = max(1f, cellW / 10)
                canvas.drawRect(x, y, x + cellW, y + cellH, fill)
                fill.style = Paint.Style.FILL
            }
            else -> {
                canvas.drawRect(x, y, x + cellW, y + cellH, fill)
                val text = s.cellText(s.cursorCol, s.cursorRow)
                if (text.isNotEmpty()) {
                    paint.color = 0xFF000000.toInt() or s.cursorText
                    paint.isFakeBoldText = false
                    paint.textSkewX = 0f
                    paint.isUnderlineText = false
                    paint.isStrikeThruText = false
                    canvas.drawText(text, x, y + ascent, paint)
                }
            }
        }
    }

    private val handleRadius get() = context.dp(10f).toFloat()

    private fun handleCenter(cell: Cell): Pair<Float, Float> =
        Pair(cell.col * cellW + cellW / 2, (cell.row + 1) * cellH + handleRadius)

    private fun drawHandles(canvas: Canvas, s: Screen) {
        fill.color = context.color(R.color.accent)
        selection?.let { sel ->
            for (cell in listOf(sel.start, sel.end)) {
                val (x, y) = handleCenter(cell)
                canvas.drawCircle(x, y, handleRadius, fill)
            }
        }
        if (selection == null && System.currentTimeMillis() < cursorHandleUntil) {
            val cell = cursorMark ?: Cell(s.cursorCol, s.cursorRow)
            val (x, y) = handleCenter(cell)
            canvas.drawCircle(x, y, handleRadius, fill)
            if (cursorMark != null) {
                fill.style = Paint.Style.STROKE
                fill.strokeWidth = context.dp(2f).toFloat()
                canvas.drawRect(cell.col * cellW, cell.row * cellH, (cell.col + 1) * cellW, (cell.row + 1) * cellH, fill)
                fill.style = Paint.Style.FILL
            }
        }
    }

    // ----- input from the keyboard -----------------------------------------

    override fun onCheckIsTextEditor(): Boolean = true

    override fun onCreateInputConnection(outAttrs: EditorInfo): InputConnection {
        // Each key commits as it is typed: no predictions, no composing
        // (architecture 6.2), and the keyboard never takes the whole screen.
        outAttrs.inputType = InputType.TYPE_CLASS_TEXT or
            InputType.TYPE_TEXT_VARIATION_VISIBLE_PASSWORD or
            InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS
        outAttrs.imeOptions = EditorInfo.IME_FLAG_NO_EXTRACT_UI or
            EditorInfo.IME_FLAG_NO_FULLSCREEN or EditorInfo.IME_ACTION_NONE
        return object : BaseInputConnection(this, false) {
            private var composing = ""

            override fun commitText(text: CharSequence, newCursorPosition: Int): Boolean {
                sendComposingDiff(text.toString())
                composing = ""
                return true
            }

            override fun setComposingText(text: CharSequence, newCursorPosition: Int): Boolean {
                sendComposingDiff(text.toString())
                composing = text.toString()
                return true
            }

            override fun finishComposingText(): Boolean {
                composing = ""
                return true
            }

            /**
             * What a keyboard still composes is sent as the difference to what
             * went out ([imeStep]): a keyboard's clipboard commits its text, and
             * added text with a line break is a paste (D223), not typing.
             */
            private fun sendComposingDiff(next: String) {
                val step = imeStep(composing, next)
                repeat(step.backspaces) { sendKey(Native.KEY_BACKSPACE) }
                when {
                    step.paste -> pasteText(step.added)
                    step.added.isNotEmpty() -> sendText(step.added)
                }
            }

            override fun deleteSurroundingText(beforeLength: Int, afterLength: Int): Boolean {
                repeat(max(1, beforeLength)) { sendKey(Native.KEY_BACKSPACE) }
                return true
            }

            override fun sendKeyEvent(event: KeyEvent): Boolean {
                if (event.action == KeyEvent.ACTION_DOWN) onKeyDown(event.keyCode, event)
                return true
            }
        }
    }

    override fun onKeyDown(keyCode: Int, event: KeyEvent): Boolean {
        val key = when (keyCode) {
            KeyEvent.KEYCODE_DEL -> Native.KEY_BACKSPACE
            KeyEvent.KEYCODE_FORWARD_DEL -> Native.KEY_DELETE
            KeyEvent.KEYCODE_ENTER, KeyEvent.KEYCODE_NUMPAD_ENTER -> Native.KEY_ENTER
            KeyEvent.KEYCODE_TAB -> if (event.isShiftPressed) Native.KEY_BACKTAB else Native.KEY_TAB
            KeyEvent.KEYCODE_INSERT -> Native.KEY_INSERT
            in KeyEvent.KEYCODE_F1..KeyEvent.KEYCODE_F12 -> Native.KEY_F1 + (keyCode - KeyEvent.KEYCODE_F1)
            KeyEvent.KEYCODE_ESCAPE -> Native.KEY_ESC
            KeyEvent.KEYCODE_DPAD_UP -> Native.KEY_UP
            KeyEvent.KEYCODE_DPAD_DOWN -> Native.KEY_DOWN
            KeyEvent.KEYCODE_DPAD_LEFT -> Native.KEY_LEFT
            KeyEvent.KEYCODE_DPAD_RIGHT -> Native.KEY_RIGHT
            KeyEvent.KEYCODE_MOVE_HOME -> Native.KEY_HOME
            KeyEvent.KEYCODE_MOVE_END -> Native.KEY_END
            KeyEvent.KEYCODE_PAGE_UP -> Native.KEY_PAGE_UP
            KeyEvent.KEYCODE_PAGE_DOWN -> Native.KEY_PAGE_DOWN
            else -> 0
        }
        if (key != 0) {
            sendKey(key, event.isCtrlPressed, event.isAltPressed)
            return true
        }
        val unicode = event.getUnicodeChar(event.metaState and (KeyEvent.META_CTRL_MASK or KeyEvent.META_ALT_MASK).inv())
        if (unicode != 0) {
            sendText(String(Character.toChars(unicode)), event.isCtrlPressed, event.isAltPressed)
            return true
        }
        return super.onKeyDown(keyCode, event)
    }

    fun sendKey(code: Int, ctrl: Boolean = false, alt: Boolean = false) {
        if (session == 0L) return
        // Keys of the soft keyboard count as input for the auto-lock (UI spec 5).
        (context.applicationContext as? App)?.noteActivity()
        val withCtrl = ctrl || modifiers.ctrl
        val withAlt = alt || modifiers.alt
        modifiers.consume()
        if (selection != null) clearSelection()
        if (worker.isShutdown) return
        worker.execute { runCatching { Native.key(session, code, withCtrl, withAlt) } }
    }

    /** [raw]: as it is, without the armed Ctrl and Alt (a tmux sequence). */
    fun sendText(text: String, ctrl: Boolean = false, alt: Boolean = false, raw: Boolean = false) {
        if (session == 0L || text.isEmpty()) return
        (context.applicationContext as? App)?.noteActivity()
        val withCtrl = ctrl || (!raw && modifiers.ctrl)
        val withAlt = alt || (!raw && modifiers.alt)
        if (!raw) modifiers.consume()
        if (selection != null) clearSelection()
        if (worker.isShutdown) return
        worker.execute { runCatching { Native.text(session, text, withCtrl, withAlt) } }
    }

    fun toggleKeyboard() {
        val imm = context.getSystemService(InputMethodManager::class.java)
        requestFocus()
        if (Build.VERSION.SDK_INT >= 30 && rootWindowInsets?.isVisible(android.view.WindowInsets.Type.ime()) == true) {
            imm.hideSoftInputFromWindow(windowToken, 0)
        } else {
            imm.showSoftInput(this, InputMethodManager.SHOW_IMPLICIT)
        }
    }

    fun showKeyboard() {
        requestFocus()
        context.getSystemService(InputMethodManager::class.java).showSoftInput(this, InputMethodManager.SHOW_IMPLICIT)
    }

    // ----- gestures --------------------------------------------------------

    private fun cellAt(x: Float, y: Float): Cell? {
        val s = screen ?: return null
        val col = min(s.cols - 1, max(0, (x / cellW).toInt()))
        val row = min(s.rows - 1, max(0, (y / cellH).toInt()))
        return Cell(col, row)
    }

    /** Puts the cursor on [cell] off the main thread (a walk blocks). */
    private fun putCursor(cell: Cell) {
        if (worker.isShutdown) return
        worker.execute {
            runCatching { Native.putCursor(session, cell.col, cell.row) }
            main.post { invalidate() }
        }
    }

    private val gestures = GestureDetector(context, object : GestureDetector.SimpleOnGestureListener() {
        override fun onDown(e: MotionEvent) = true

        override fun onSingleTapUp(e: MotionEvent): Boolean {
            if (selection != null) {
                clearSelection()
                return true
            }
            showKeyboard()
            val cell = cellAt(e.x, e.y) ?: return true
            putCursor(cell)
            cursorHandleUntil = System.currentTimeMillis() + 4000
            main.postDelayed({ invalidate() }, 4100)
            invalidate()
            return true
        }

        override fun onLongPress(e: MotionEvent) {
            val s = screen ?: return
            val cell = cellAt(e.x, e.y) ?: return
            selection = Selection.word(s, cell)
            performHapticFeedback(HAPTIC_FEEDBACK_ENABLED)
            startSelectionToolbar()
            invalidate()
        }

        override fun onScroll(e1: MotionEvent?, e2: MotionEvent, dx: Float, dy: Float): Boolean {
            if (dragging != Drag.None || abs(dy) < abs(dx)) return false
            val s = screen ?: return false
            scrollRest += dy
            val lines = (scrollRest / cellH).toInt()
            if (lines == 0) return true
            scrollRest -= lines * cellH
            if (s.alternateScreen) {
                // ↑/↓ on the alternate screen (xterm's alternate scroll, ui-spec 9.7).
                val key = if (lines < 0) Native.KEY_UP else Native.KEY_DOWN
                repeat(abs(lines)) { sendKey(key) }
            } else {
                worker.execute { runCatching { Native.scroll(session, -lines) } }
            }
            return true
        }
    })

    private val scale = ScaleGestureDetector(context, object : ScaleGestureDetector.SimpleOnScaleGestureListener() {
        override fun onScale(detector: ScaleGestureDetector): Boolean {
            // Pinch to zoom is a setting (reference-1 4).
            if (!TerminalPrefs.of(context).getBoolean(TerminalPrefs.PINCH, true)) return true
            val next = min(28f, max(8f, fontSp * detector.scaleFactor))
            if (abs(next - fontSp) >= 0.5f) {
                fontSp = Math.round(next).toFloat()
                updateMetrics()
                invalidate()
            }
            return true
        }

        override fun onScaleEnd(detector: ScaleGestureDetector) {
            // The new size goes to the server when the pinch ends (ui-spec 9.8).
            sizeChanged(settleMs = 0)
        }
    })

    private fun nearHandle(x: Float, y: Float, cell: Cell): Boolean {
        val (hx, hy) = handleCenter(cell)
        val reach = handleRadius * 2.5f
        return abs(x - hx) < reach && abs(y - hy) < reach
    }

    override fun onTouchEvent(event: MotionEvent): Boolean {
        val s = screen
        if (cursorPad && selection == null && padDrag(event)) return true
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                val sel = selection
                dragging = when {
                    sel != null && nearHandle(event.x, event.y, sel.start) -> Drag.SelectionStart
                    sel != null && nearHandle(event.x, event.y, sel.end) -> Drag.SelectionEnd
                    sel == null && s != null && System.currentTimeMillis() < cursorHandleUntil &&
                        nearHandle(event.x, event.y, Cell(s.cursorCol, s.cursorRow)) -> Drag.Cursor
                    else -> Drag.None
                }
                if (dragging != Drag.None) {
                    actionMode?.hide(Long.MAX_VALUE)
                    return true
                }
            }
            MotionEvent.ACTION_MOVE -> if (dragging != Drag.None) {
                // The finger is under the handle, below its cell: aim above it.
                val cell = cellAt(event.x, event.y - cellH - handleRadius) ?: return true
                when (dragging) {
                    Drag.SelectionStart -> selection = selection?.withEnd(true, cell)
                    Drag.SelectionEnd -> selection = selection?.withEnd(false, cell)
                    Drag.Cursor -> {
                        cursorMark = cell
                        cursorHandleUntil = System.currentTimeMillis() + 4000
                    }
                    Drag.None -> {}
                }
                showMagnifier(cell)
                invalidate()
                return true
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> if (dragging != Drag.None) {
                magnifier?.dismiss()
                if (dragging == Drag.Cursor) {
                    cursorMark?.let { putCursor(it) }
                    cursorMark = null
                    main.postDelayed({ invalidate() }, 4100)
                } else {
                    // The items follow the new selection (Delete, Cut in nano).
                    actionMode?.invalidate()
                    actionMode?.invalidateContentRect()
                    actionMode?.hide(0)
                }
                dragging = Drag.None
                invalidate()
                return true
            }
        }
        scale.onTouchEvent(event)
        if (!scale.isInProgress) gestures.onTouchEvent(event)
        return true
    }

    private var padStartX = 0f
    private var padStartY = 0f
    private var padMoved = false

    /** A second finger came down in this gesture: a pinch, no arrows until all fingers are up. */
    private var padPinch = false

    /**
     * The cursor key's drag: every [step] of the finger sends one arrow key
     * the way it moved. A tap still puts the cursor; a drag never does.
     */
    private fun padDrag(event: MotionEvent): Boolean {
        val speed = TerminalPrefs.of(context).getInt(TerminalPrefs.CURSOR_SPEED, 2)
        val factor = when (speed) {
            1 -> 1.6f
            3 -> 0.6f
            else -> 1f
        }
        if (event.actionMasked == MotionEvent.ACTION_POINTER_DOWN || (event.pointerCount > 1 && !padPinch)) {
            // Two fingers: a pinch (scale), and no tap afterwards.
            padPinch = true
            padMoved = true
            val cancel = MotionEvent.obtain(event).apply { action = MotionEvent.ACTION_CANCEL }
            gestures.onTouchEvent(cancel)
            cancel.recycle()
        }
        if (padPinch) {
            scale.onTouchEvent(event)
            if (event.actionMasked == MotionEvent.ACTION_UP || event.actionMasked == MotionEvent.ACTION_CANCEL) padPinch = false
            return true
        }
        when (event.actionMasked) {
            MotionEvent.ACTION_DOWN -> {
                padStartX = event.x
                padStartY = event.y
                padRestX = 0f
                padRestY = 0f
                padMoved = false
                padPinch = false
                gestures.onTouchEvent(event)
                return true
            }
            MotionEvent.ACTION_MOVE -> {
                val dx = event.x - padStartX
                val dy = event.y - padStartY
                padStartX = event.x
                padStartY = event.y
                padRestX += dx
                padRestY += dy
                val stepX = cellW * factor
                val stepY = cellH * factor
                if (!padMoved && abs(padRestX) < stepX && abs(padRestY) < stepY) return true
                if (!padMoved) {
                    // A drag, not a tap or a long press.
                    val cancel = MotionEvent.obtain(event).apply { action = MotionEvent.ACTION_CANCEL }
                    gestures.onTouchEvent(cancel)
                    cancel.recycle()
                }
                padMoved = true
                while (abs(padRestX) >= stepX) {
                    sendKey(if (padRestX > 0) Native.KEY_RIGHT else Native.KEY_LEFT)
                    padRestX -= if (padRestX > 0) stepX else -stepX
                }
                while (abs(padRestY) >= stepY) {
                    sendKey(if (padRestY > 0) Native.KEY_DOWN else Native.KEY_UP)
                    padRestY -= if (padRestY > 0) stepY else -stepY
                }
                return true
            }
            MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                if (!padMoved) gestures.onTouchEvent(event)
                return true
            }
        }
        return false
    }

    private fun showMagnifier(cell: Cell) {
        if (Build.VERSION.SDK_INT < 28) return
        val lens = magnifier ?: Magnifier(this).also { magnifier = it }
        lens.show(cell.col * cellW + cellW / 2, cell.row * cellH + cellH / 2)
    }

    // ----- selection toolbar ------------------------------------------------

    private fun clearSelection() {
        selection = null
        actionMode?.finish()
        actionMode = null
        invalidate()
    }

    /** Back with a selection clears it and is used up (D220); false without one. */
    fun clearSelectionIfAny(): Boolean {
        if (selection == null) return false
        clearSelection()
        return true
    }

    private fun startSelectionToolbar() {
        actionMode?.finish()
        actionMode = startActionMode(object : ActionMode.Callback2() {
            override fun onCreateActionMode(mode: ActionMode, menu: Menu): Boolean {
                fillMenu(menu)
                return true
            }

            // Built again whenever the selection changes (Select all).
            override fun onPrepareActionMode(mode: ActionMode, menu: Menu): Boolean {
                menu.clear()
                fillMenu(menu)
                return true
            }

            override fun onActionItemClicked(mode: ActionMode, item: MenuItem): Boolean {
                val s = screen ?: return false
                val sel = selection ?: return false
                when (item.itemId) {
                    COPY -> {
                        copy(sel.text(s))
                        clearSelection()
                    }
                    PASTE -> {
                        clearSelection()
                        paste()
                    }
                    // Checked again on the screen of now: nothing is sent
                    // to a program the item was not offered for.
                    CUT_NANO -> {
                        clearSelection()
                        if (canCutInNano(sel, s)) cutInNano(sel, s)
                    }
                    DELETE_LINE -> {
                        clearSelection()
                        if (canDeleteOnLine(sel, s)) deleteOnLine(sel)
                    }
                    SELECT_SCREEN -> {
                        selection = Selection.all(s)
                        mode.invalidate()
                        mode.invalidateContentRect()
                        invalidate()
                    }
                }
                return true
            }

            override fun onDestroyActionMode(mode: ActionMode) {
                if (actionMode == mode) actionMode = null
            }

            override fun onGetContentRect(mode: ActionMode, view: View, outRect: Rect) {
                val sel = selection ?: return
                outRect.set(
                    0, (sel.start.row * cellH).toInt(),
                    width, ((sel.end.row + 1) * cellH).toInt(),
                )
            }
        }, ActionMode.TYPE_FLOATING)
    }

    /**
     * The selection toolbar: the platform's own Copy, Paste, Select all
     * (reference-2 11.1), and what really deletes on the server (UI spec
     * 9.6): nano's own cut where its shortcut rows show and it does not take
     * the mouse (a click is not a walk that can be checked), Backspaces on
     * the line being typed; never in the scrollback.
     */
    private fun fillMenu(menu: Menu) {
        menu.add(0, COPY, 0, android.R.string.copy)
        menu.add(0, PASTE, 1, android.R.string.paste)
        menu.add(0, SELECT_SCREEN, 2, android.R.string.selectAll)
        val s = screen ?: return
        val sel = selection ?: return
        if (canCutInNano(sel, s)) menu.add(0, CUT_NANO, 3, R.string.cut_in_nano)
        if (canDeleteOnLine(sel, s)) menu.add(0, DELETE_LINE, 4, R.string.delete_selection)
    }

    private fun canCutInNano(sel: Selection, s: Screen): Boolean =
        s.scrollOffset == 0 && isNano(s) && !s.mouse && !isWholeScreen(sel, s)

    private fun canDeleteOnLine(sel: Selection, s: Screen): Boolean =
        s.scrollOffset == 0 && !s.alternateScreen && sel.start.row == sel.end.row && sel.start.row == s.cursorRow

    private fun isWholeScreen(sel: Selection, s: Screen): Boolean =
        sel.start == Cell(0, 0) && sel.end == Cell(s.cols - 1, s.rows - 1)

    /** nano's shortcut rows on the last two rows (`^G` and `^X`). */
    private fun isNano(s: Screen): Boolean {
        if (s.rows < 3) return false
        val bottom = s.text(0, s.rows - 2, s.cols - 1, s.rows - 1)
        return bottom.contains("^G") && bottom.contains("^X")
    }

    /** The cell just past [cell] on the screen (the next row's first one at a row's end). */
    private fun after(cell: Cell, s: Screen): Cell =
        if (cell.col + 1 < s.cols) Cell(cell.col + 1, cell.row) else Cell(0, min(s.rows - 1, cell.row + 1))

    private fun cannotReach() = main.post {
        Toast.makeText(context, R.string.cursor_not_reached, Toast.LENGTH_LONG).show()
    }

    /**
     * Cut in nano (UI spec 9.6): the cursor on the first character, nano's
     * mark (Alt+A), the cursor just past the last one, Ctrl+K. Nothing is
     * cut unless both walks landed; a mark it set is unset again.
     */
    private fun cutInNano(sel: Selection, s: Screen) {
        if (worker.isShutdown) return
        val id = session
        worker.execute {
            runCatching {
                if (!Native.putCursor(id, sel.start.col, sel.start.row)) {
                    cannotReach()
                    return@runCatching
                }
                Native.text(id, "a", false, true)
                val past = after(sel.end, s)
                if (!Native.putCursor(id, past.col, past.row)) {
                    Native.text(id, "a", false, true)
                    cannotReach()
                    return@runCatching
                }
                Native.text(id, "k", true, false)
            }
        }
    }

    /**
     * Selects from [start] to [end] and runs Cut in nano as the toolbar's item
     * does (the walks of ProgramsTest); false when the toolbar would not offer it.
     */
    fun cutInNanoForTest(start: Cell, end: Cell): Boolean {
        val s = screen ?: return false
        val sel = Selection(start, end)
        if (!canCutInNano(sel, s)) return false
        cutInNano(sel, s)
        return true
    }

    /** Delete on the command line (UI spec 9.6): the cursor past the selection, one Backspace per character. */
    private fun deleteOnLine(sel: Selection) {
        if (worker.isShutdown) return
        val id = session
        val s = screen ?: return
        val count = (sel.start.col..sel.end.col).count { col -> s.cellText(col, sel.start.row).isNotEmpty() }
        worker.execute {
            runCatching {
                if (!Native.putCursor(id, sel.end.col + 1, sel.end.row)) {
                    cannotReach()
                    return@runCatching
                }
                repeat(count) { Native.key(id, Native.KEY_BACKSPACE, false, false) }
            }
        }
    }

    private fun copy(text: String) {
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        clipboard.setPrimaryClip(ClipData.newPlainText("terminal", text))
        // From Android 13 the system shows its own notice (ui-spec 9.4).
        if (Build.VERSION.SDK_INT < 33) Toast.makeText(context, R.string.copied, Toast.LENGTH_SHORT).show()
    }

    fun paste() {
        val clipboard = context.getSystemService(ClipboardManager::class.java)
        val text = clipboard.primaryClip?.takeIf { it.itemCount > 0 }?.getItemAt(0)?.coerceToText(context)?.toString()
        if (text != null) pasteText(text)
    }

    /**
     * A paste by the desktop's rules (ui-spec 9.4, UI 4.3; D223): several
     * lines ask first while `terminal.confirm_multiline_paste` is on; the
     * text goes cleaned and bracketed when the program asked for it.
     */
    fun pasteText(text: String) {
        val target = session
        if (target == 0L || text.isEmpty() || worker.isShutdown) return
        worker.execute {
            val confirm = coreSettings().optBoolean("terminal.confirm_multiline_paste", true)
            // A failure pastes nothing (it never skips the question).
            val preview = try {
                Native.pastePreview(text, confirm)
            } catch (e: RuntimeException) {
                return@execute
            }
            if (preview == null) {
                if (session == target) runCatching { Native.paste(target, text) }
            } else {
                main.post { confirmPaste(target, text, JSONObject(preview)) }
            }
        }
    }

    private var pasteDialog: android.app.AlertDialog? = null

    private fun dismissPaste() {
        main.post {
            pasteDialog?.dismiss()
            pasteDialog = null
        }
    }

    /** The multi-line paste's question: its first lines, then Paste or Cancel. */
    private fun confirmPaste(target: Long, text: String, preview: JSONObject) {
        if (released || session != target || !onScreen || paused) return
        val lines = preview.optJSONArray("lines")
        val shown = buildString {
            if (lines != null) for (i in 0 until lines.length()) appendLine(lines.optString(i))
            val more = preview.optInt("more")
            if (more > 0) append(context.resources.getQuantityString(R.plurals.paste_more_lines, more, more))
        }.trimEnd()
        pasteDialog?.dismiss()
        pasteDialog = android.app.AlertDialog.Builder(context)
            .setTitle(R.string.paste_lines_title)
            .setMessage(shown)
            .setPositiveButton(R.string.paste) { _, _ ->
                // Only to the session it was asked for, while it is in front.
                if (session == target && onScreen && !worker.isShutdown) worker.execute { runCatching { Native.paste(target, text) } }
            }
            .setNegativeButton(R.string.cancel, null)
            .setOnDismissListener { pasteDialog = null }
            .show()
    }

    fun release() {
        released = true
        // Unblocks this view's pump, which hands the wake on and ends.
        if (session != 0L) runCatching { Native.wake(session) }
        main.removeCallbacksAndMessages(null)
        worker.shutdown()
        magnifier?.dismiss()
    }

    companion object {
        /** How long the output stays quiet before TalkBack hears the cursor's row. */
        const val ANNOUNCE_DELAY_MS = 700L

        /**
         * The background and foreground each session's terminal shows (its
         * host's theme first, D220): the session bar and the cards of
         * Connections paint with them, not with the Settings theme.
         */
        val colors = androidx.compose.runtime.mutableStateMapOf<Long, Pair<Int, Int>>()

        private const val COPY = 1
        private const val PASTE = 2
        private const val SELECT_SCREEN = 4
        private const val CUT_NANO = 5
        private const val DELETE_LINE = 6
    }
}
