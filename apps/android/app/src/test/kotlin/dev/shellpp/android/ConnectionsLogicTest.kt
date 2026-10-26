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
    fun oneWordMayNameASavedHost() {
        assertEquals("web", singleTarget("ssh web"))
        assertEquals("web", singleTarget(" web "))
        assertEquals("10.0.0.5", singleTarget("ssh 10.0.0.5"))
        assertNull(singleTarget("ssh root@web"))
        assertNull(singleTarget("ssh web -p 22"))
        assertNull(singleTarget("ssh"))
        assertNull(singleTarget("two words"))
    }

    private fun host(id: String, label: String?, address: String) = Record(
        org.json.JSONObject().put("record_id", id).put("vault_id", "v").put("type", "host")
            .put("data", org.json.JSONObject().put("label", label ?: org.json.JSONObject.NULL).put("address", address)),
    )

    @Test
    fun theDesktopsExactMatchFindsTheSavedHost() {
        val hosts = listOf(host("1", "web", "a.lan"), host("2", "Web", "b.lan"), host("3", null, "c.lan"))
        // The exact spelling first, then without case (UI 2.1, D223 item 12).
        assertEquals(listOf("2"), exactHosts("Web", hosts).map { it.id })
        assertEquals(listOf("1", "2"), exactHosts("WEB", hosts).map { it.id })
        assertEquals(listOf("2"), exactHosts("b.lan", hosts).map { it.id })
        assertEquals(listOf("3"), exactHosts("C.LAN", hosts).map { it.id })
        assertEquals(emptyList<String>(), exactHosts("we", hosts).map { it.id })
    }

    @Test
    fun anSftpCardSaysItsLastTransfer() {
        val transfer = { id: Long, direction: String, name: String, state: String ->
            org.json.JSONObject().put("id", id).put("direction", direction).put("name", name).put("state", state)
        }
        assertEquals("No transfers", lastTransfer(emptyList()))
        assertEquals(
            "↑ notes.txt · done",
            lastTransfer(listOf(transfer(1, "download", "old.log", "failed"), transfer(2, "upload", "notes.txt", "done"))),
        )
        assertEquals("↓ big.iso · waiting", lastTransfer(listOf(transfer(7, "download", "big.iso", "queued"))))
    }

    @Test
    fun aRecentRowShowsTheFullCommand() {
        assertEquals("ssh root@203.0.113.7 -p 22", sshCommand("root", "203.0.113.7", 22))
        assertEquals("ssh host.example.org -p 2222", sshCommand(null, "host.example.org", 2222))
    }
}
