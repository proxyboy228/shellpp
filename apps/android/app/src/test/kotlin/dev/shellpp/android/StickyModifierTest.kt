package dev.shellpp.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class StickyModifierTest {
    @Test
    fun aTapArmsItForOneKey() {
        val ctrl = StickyModifier()
        ctrl.tap(now = 1000)
        assertEquals(StickyModifier.State.ARMED, ctrl.state)
        assertTrue(ctrl.active)
        ctrl.consume()
        assertFalse(ctrl.active)
    }

    @Test
    fun aDoubleTapLocksItUntilTappedAgain() {
        val alt = StickyModifier()
        alt.tap(now = 1000)
        alt.tap(now = 1200)
        assertEquals(StickyModifier.State.LOCKED, alt.state)
        alt.consume()
        alt.consume()
        assertTrue(alt.active)
        alt.tap(now = 5000)
        assertFalse(alt.active)
    }

    @Test
    fun aSlowSecondTapDisarms() {
        val ctrl = StickyModifier()
        ctrl.tap(now = 1000)
        ctrl.tap(now = 2000)
        assertEquals(StickyModifier.State.OFF, ctrl.state)
    }
}
