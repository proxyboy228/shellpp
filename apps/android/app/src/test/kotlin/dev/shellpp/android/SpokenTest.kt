package dev.shellpp.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Test

/** What TalkBack says for the keys row (plan of run 16, 3.3): words, never a bare glyph. */
class SpokenTest {
    @Test
    fun keysAreSpokenInWords() {
        assertEquals("Up arrow", spokenName("up", "↑"))
        assertEquals("Page down", spokenName("pgdn", "pgDn"))
        assertEquals("Control C", spokenName("^C", "^C"))
        assertEquals("Control underscore", spokenName("^_", "^_"))
        assertEquals("Control X twice", spokenName("^X^X", "^X^X"))
        assertEquals("F12", spokenName("f12", "F12"))
        assertEquals("pipe", spokenName("|", "|"))
        assertEquals("tmux: next pane", spokenName("tmux_next", "^B o"))
    }

    @Test
    fun everyKeyOfTheRowHasAName() {
        for (key in XKey.ALL.values) {
            val spoken = key.spoken
            assertFalse("${key.id}: $spoken", spoken.isBlank())
            // A glyph alone (an arrow, a sign) would be read as a symbol or not at all.
            assertFalse("${key.id}: $spoken", spoken.length == 1)
        }
    }
}
