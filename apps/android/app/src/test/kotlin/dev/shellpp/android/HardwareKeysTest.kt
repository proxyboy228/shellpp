package dev.shellpp.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class HardwareKeysTest {
    @Test
    fun altGrTypesTheLayoutsCharacter() {
        // A German keyboard: AltGr+Q is @ (ssh user@host), not Esc q.
        assertEquals(HardwareChar('@'.code, false), hardwareChar('q'.code, '@'.code, rightAlt = true, alt = true))
        assertEquals(HardwareChar('{'.code, false), hardwareChar('7'.code, '{'.code, rightAlt = true, alt = true))
    }

    @Test
    fun altWithoutALayoutCharacterIsEscBeforeTheKey() {
        // A US layout has nothing on right Alt: Alt+q.
        assertEquals(HardwareChar('q'.code, true), hardwareChar('q'.code, 0, rightAlt = true, alt = true))
        assertEquals(HardwareChar('q'.code, true), hardwareChar('q'.code, 'q'.code, rightAlt = true, alt = true))
        // Left Alt is always Alt, whatever the layout puts on it.
        assertEquals(HardwareChar('x'.code, true), hardwareChar('x'.code, '\u2248'.code, rightAlt = false, alt = true))
        assertEquals(HardwareChar('x'.code, false), hardwareChar('x'.code, 'x'.code, rightAlt = false, alt = false))
    }

    @Test
    fun aDeadKeyOrNoCharacterTypesNothing() {
        assertNull(hardwareChar(0, 0, rightAlt = false, alt = false))
        assertNull(hardwareChar(Int.MIN_VALUE or '`'.code, 0, rightAlt = false, alt = false))
    }
}
