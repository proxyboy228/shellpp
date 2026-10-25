package dev.shellpp.android

/**
 * A sticky modifier of the extra keys (Ctrl or Alt; ui-spec 9.2): a tap
 * arms it for the next key, a double tap locks it until it is tapped again.
 */
class StickyModifier(private val doubleTapMs: Long = 300) {
    enum class State { OFF, ARMED, LOCKED }

    var state = State.OFF
        private set
    private var armedAt = 0L

    val active: Boolean get() = state != State.OFF

    fun tap(now: Long) {
        state = when (state) {
            State.OFF -> {
                armedAt = now
                State.ARMED
            }
            State.ARMED -> if (now - armedAt <= doubleTapMs) State.LOCKED else State.OFF
            State.LOCKED -> State.OFF
        }
    }

    /** A key went out with the modifier: an armed one is used up. */
    fun consume() {
        if (state == State.ARMED) state = State.OFF
    }
}
