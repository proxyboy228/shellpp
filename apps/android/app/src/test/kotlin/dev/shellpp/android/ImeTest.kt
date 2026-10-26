package dev.shellpp.android

import org.junit.Assert.assertEquals
import org.junit.Test

/** The keyboard's steps (architecture 6.2) and the paste rule on them (D223; review of run 16). */
class ImeTest {
    @Test
    fun aWordCommittedWithItsLineBreakIsAnEnter() {
        // Composing "ls", then the keyboard commits "ls" and Enter.
        assertEquals(ImeStep(0, "\n", paste = false), imeStep("ls", "ls\n"))
    }

    @Test
    fun aClipboardOverComposedTextTakesItBackThenPastes() {
        assertEquals(ImeStep(2, "a\nb", paste = true), imeStep("rm", "a\nb"))
        assertEquals(ImeStep(0, "a\nb", paste = true), imeStep("", "a\nb"))
    }

    @Test
    fun typingAndCorrectionsAreNoPaste() {
        assertEquals(ImeStep(0, "s", paste = false), imeStep("l", "ls"))
        assertEquals(ImeStep(1, "", paste = false), imeStep("ls", "l"))
        assertEquals(ImeStep(0, "\r", paste = false), imeStep("", "\r"))
    }
}
