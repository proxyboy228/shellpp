package dev.shellpp.android

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

/** Quick connect's command line and the recent rows' command (reference-1 3, reference-2 11.4). */
class ConnectionsLogicTest {
    @Test
    fun sshCommandLinesAndShortFormsAreRead() {
        assertEquals(QuickTarget("root", "203.0.113.7", 2222), parseSsh("ssh root@203.0.113.7 -p 2222"))
        assertEquals(QuickTarget("root", "203.0.113.7", 2222), parseSsh("ssh -p 2222 root@203.0.113.7"))
        assertEquals(QuickTarget("admin", "host.example.org", null), parseSsh("admin@host.example.org"))
        assertEquals(QuickTarget("admin", "host.example.org", 22), parseSsh("admin@host.example.org:22"))
        assertEquals(QuickTarget("", "host.example.org", null), parseSsh("ssh host.example.org"))
    }

    @Test
    fun searchTextAndOtherOptionsAreNoCommand() {
        assertNull(parseSsh("web"))
        assertNull(parseSsh("ssh -i key root@host"))
        assertNull(parseSsh("ssh root@host -p 70000"))
        assertNull(parseSsh("ssh a@b c@d"))
        assertNull(parseSsh(""))
    }

    @Test
    fun aRecentRowShowsTheFullCommand() {
        assertEquals("ssh root@203.0.113.7 -p 22", sshCommand("root", "203.0.113.7", 22))
        assertEquals("ssh host.example.org -p 2222", sshCommand(null, "host.example.org", 2222))
    }
}
