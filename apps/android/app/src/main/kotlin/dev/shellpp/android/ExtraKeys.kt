package dev.shellpp.android

import android.annotation.SuppressLint
import android.content.Context
import android.graphics.Rect
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.view.Gravity
import android.view.MotionEvent
import android.widget.LinearLayout
import android.widget.TextView

/**
 * The extra keys (docs/mobile/ui-spec.md 9.2): two rows without scrolling,
 * 48 dp high; Ctrl and Alt sticky; a held key repeats; the rows are kept
 * out of the system's edge gestures.
 */
@SuppressLint("ViewConstructor")
class ExtraKeys(context: Context, private val sink: Sink) : LinearLayout(context), TerminalView.Modifiers {
    /** Where the keys go. */
    interface Sink {
        fun key(code: Int)
        fun text(text: String)
        fun toggleKeyboard()
    }

    private val ctrlState = StickyModifier()
    private val altState = StickyModifier()
    private lateinit var ctrlView: TextView
    private lateinit var altView: TextView
    private val handler = Handler(Looper.getMainLooper())

    override val ctrl: Boolean get() = ctrlState.active
    override val alt: Boolean get() = altState.active

    override fun consume() {
        ctrlState.consume()
        altState.consume()
        paintModifiers()
    }

    private sealed class Key(val label: String) {
        class Code(label: String, val code: Int) : Key(label)
        class Text(label: String, val text: String) : Key(label)
        class Ctrl : Key("Ctrl")
        class Alt : Key("Alt")
        class Keyboard : Key("⌨")
    }

    init {
        orientation = VERTICAL
        setBackgroundColor(context.color(R.color.bg_surface))
        addRow(
            listOf(
                Key.Code("Esc", Native.KEY_ESC), Key.Code("Tab", Native.KEY_TAB), Key.Ctrl(), Key.Alt(),
                Key.Code("←", Native.KEY_LEFT), Key.Code("↓", Native.KEY_DOWN),
                Key.Code("↑", Native.KEY_UP), Key.Code("→", Native.KEY_RIGHT),
            ),
        )
        addRow(
            listOf(
                Key.Keyboard(), Key.Text("|", "|"), Key.Text("/", "/"), Key.Text("~", "~"), Key.Text("-", "-"),
                Key.Code("Home", Native.KEY_HOME), Key.Code("End", Native.KEY_END),
                Key.Code("PgUp", Native.KEY_PAGE_UP), Key.Code("PgDn", Native.KEY_PAGE_DOWN),
            ),
        )
    }

    private fun addRow(keys: List<Key>) {
        val row = LinearLayout(context).apply { orientation = HORIZONTAL }
        for (key in keys) {
            val view = keyView(key)
            row.addView(view, LayoutParams(0, context.dp(48f), 1f))
        }
        addView(row, LayoutParams(LayoutParams.MATCH_PARENT, LayoutParams.WRAP_CONTENT))
    }

    @SuppressLint("ClickableViewAccessibility")
    private fun keyView(key: Key): TextView = TextView(context).apply {
        text = key.label
        textSize = 14f
        gravity = Gravity.CENTER
        setTextColor(context.color(R.color.text_primary))
        contentDescription = key.label
        when (key) {
            is Key.Ctrl -> {
                ctrlView = this
                setOnClickListener {
                    ctrlState.tap(System.currentTimeMillis())
                    paintModifiers()
                }
            }
            is Key.Alt -> {
                altView = this
                setOnClickListener {
                    altState.tap(System.currentTimeMillis())
                    paintModifiers()
                }
            }
            is Key.Keyboard -> setOnClickListener { sink.toggleKeyboard() }
            is Key.Code, is Key.Text -> {
                val send = {
                    when (key) {
                        is Key.Code -> sink.key(key.code)
                        is Key.Text -> sink.text(key.text)
                        else -> {}
                    }
                }
                // A held key repeats after 400 ms, every 50 ms (ui-spec 9.2).
                val repeat = object : Runnable {
                    override fun run() {
                        send()
                        handler.postDelayed(this, 50)
                    }
                }
                setOnTouchListener { view, event ->
                    when (event.actionMasked) {
                        MotionEvent.ACTION_DOWN -> {
                            view.isPressed = true
                            send()
                            handler.postDelayed(repeat, 400)
                        }
                        MotionEvent.ACTION_UP, MotionEvent.ACTION_CANCEL -> {
                            view.isPressed = false
                            handler.removeCallbacks(repeat)
                        }
                    }
                    true
                }
            }
        }
        background = context.rounded(context.color(R.color.bg_raised), radius = 6f)
        (layoutParams as? MarginLayoutParams)?.setMargins(2, 2, 2, 2)
    }

    private fun paintModifiers() {
        for ((view, state) in listOf(ctrlView to ctrlState, altView to altState)) {
            when (state.state) {
                StickyModifier.State.OFF -> {
                    view.setTextColor(context.color(R.color.text_primary))
                    view.background = context.rounded(context.color(R.color.bg_raised), radius = 6f)
                }
                StickyModifier.State.ARMED -> {
                    view.setTextColor(context.color(R.color.accent))
                    view.background = context.rounded(context.color(R.color.accent_subtle), radius = 6f)
                }
                StickyModifier.State.LOCKED -> {
                    view.setTextColor(context.color(R.color.text_on_accent))
                    view.background = context.rounded(context.color(R.color.accent), radius = 6f)
                }
            }
        }
    }

    override fun onLayout(changed: Boolean, l: Int, t: Int, r: Int, b: Int) {
        super.onLayout(changed, l, t, r, b)
        // A press near the screen's edge is a key, not "back" (ui-spec 9.2).
        if (Build.VERSION.SDK_INT >= 29) systemGestureExclusionRects = listOf(Rect(0, 0, width, height))
    }
}
