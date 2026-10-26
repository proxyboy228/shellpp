package dev.shellpp.android

import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/** The logic behind the screens of run 15 (D220, D222): no device, no core. */
class ScreensLogicTest {
    @Test
    fun addressesSortWithTheirNumbersAsNumbers() {
        val sorted = listOf("109.0.0.1", "13.0.0.1", "13.0.0.10", "13.0.0.9", "[10.0.2.2]:31389", "alpha.example.org", "Beta.example.org")
            .sortedWith(NaturalOrder)
        assertEquals(
            listOf("13.0.0.1", "13.0.0.9", "13.0.0.10", "109.0.0.1", "[10.0.2.2]:31389", "alpha.example.org", "Beta.example.org"),
            sorted,
        )
    }

    @Test
    fun permissionsReadAsLsShowsThem() {
        assertEquals("drwxr-xr-x", permissionString(JSONObject().put("kind", "dir").put("permissions", 0b111_101_101)))
        assertEquals("-rw-r--r--", permissionString(JSONObject().put("kind", "file").put("permissions", 0b110_100_100)))
        assertEquals("lrwxrwxrwx", permissionString(JSONObject().put("kind", "link").put("permissions", 0b111_111_111)))
        assertNull(permissionString(JSONObject().put("kind", "file")))
    }

    @Test
    fun foldersComeFirstInEveryOrder() {
        val entries = listOf(
            JSONObject().put("name", "b.txt").put("size", 10).put("modified", 3),
            JSONObject().put("name", "a").put("dir", true).put("modified", 1),
            JSONObject().put("name", "c.log").put("size", 5).put("modified", 2),
        )
        assertEquals(listOf("a", "b.txt", "c.log"), sortEntries(entries, "name", true).map { it.getString("name") })
        assertEquals(listOf("a", "c.log", "b.txt"), sortEntries(entries, "size", true).map { it.getString("name") })
        assertEquals(listOf("a", "b.txt", "c.log"), sortEntries(entries, "date", false).map { it.getString("name") })
    }

    @Test
    fun theDefaultRowHasTheReferencesGroupsWithoutTheAiKey() {
        assertEquals(19, XKey.DEFAULT.size)
        assertEquals(listOf("password", "cursor", "attach"), XKey.DEFAULT[0])
        assertTrue(XKey.DEFAULT.flatten().all { it in XKey.ALL })
        assertFalse(XKey.ALL.keys.any { it.contains("ai", ignoreCase = true) })
        assertTrue(XKey.DEFAULT.all { it.size in 2..4 })
    }

    @Test
    fun theEditorsDataComparesAsTheCoreDoes() {
        assertTrue(sameData(JSONObject().put("label", ""), JSONObject()))
        assertTrue(sameData(JSONObject().put("ssh", JSONObject()), JSONObject()))
        assertTrue(sameData(JSONObject().put("ssh", JSONObject().put("agent_forwarding", false)), JSONObject()))
        assertFalse(sameData(JSONObject().put("ssh", JSONObject().put("jump_chain", JSONArray())), JSONObject()))
        assertFalse(sameData(JSONObject().put("label", "web"), JSONObject().put("label", "db")))
    }

    @Test
    fun aDottedPathSetsAndClearsANestedMember() {
        val data = JSONObject().with("ssh.port", 2222)
        assertEquals(2222, data.at("ssh.port"))
        assertNull(data.with("ssh.port", null).at("ssh.port"))
        assertNull(data.with("ssh.port", "").at("ssh.port"))
    }

    /** A server without case for [renameCase]: its names, and the steps that fail. */
    private class FakeServer(names: List<String>, val failing: Set<Pair<String, String>> = emptySet(), val keepsCase: Boolean = false) : RenameCalls {
        val files = names.toMutableList()
        val renames = mutableListOf<Pair<String, String>>()

        override fun rename(from: String, to: String) {
            renames += from to to
            if ((from to to) in failing) throw RuntimeException("refused")
            val name = from.substringAfterLast('/')
            val index = files.indexOfFirst { it.equals(name, ignoreCase = true) }
            check(index >= 0)
            val target = to.substringAfterLast('/')
            files[index] = if (keepsCase && target.equals(name, ignoreCase = true)) name else target
        }

        override fun exists(path: String) = files.any { it.equals(path.substringAfterLast('/'), ignoreCase = true) }

        override fun names(folder: String) = files.toList()
    }

    @Test
    fun aCaseOnlyRenameGoesAtOnceOrRoundAndNeverLosesTheFile() {
        // At once.
        val plain = FakeServer(listOf("readme"))
        assertNull(renameCase(plain, "/srv", "readme", "README", "x"))
        assertEquals(listOf("README"), plain.files)
        // The server keeps the old case: through a free name of its own.
        val keeps = FakeServer(listOf("readme"), keepsCase = true)
        assertNull(renameCase(keeps, "/srv", "readme", "README", "x"))
        assertEquals(listOf("README"), keeps.files)
        // The last step fails: the file gets its old name back.
        val failing = FakeServer(listOf("readme"), failing = setOf("/srv/readme" to "/srv/README", "/srv/readme.shellpp-renaming-x" to "/srv/README"))
        val said = renameCase(failing, "/srv", "readme", "README", "x")
        assertTrue(said!!.startsWith("Not renamed"))
        assertEquals(listOf("readme"), failing.files)
        // And when that fails too, the message says where the file is.
        val stuck = FakeServer(
            listOf("readme"),
            failing = setOf("/srv/readme" to "/srv/README", "/srv/readme.shellpp-renaming-x" to "/srv/README", "/srv/readme.shellpp-renaming-x" to "/srv/readme"),
        )
        assertEquals("Not renamed: the file is now “readme.shellpp-renaming-x”.", renameCase(stuck, "/srv", "readme", "README", "x"))
        assertEquals(listOf("readme.shellpp-renaming-x"), stuck.files)
    }

    @Test
    fun aRenameOfTheCaseOnlyGoesOnWhereTheServerHasNoCase() {
        // The server finds README, the folder lists only readme: the same file.
        assertEquals(RenameCheck.SAME_FILE, renameCheck("readme", "README", listOf("readme", "notes"), exists = true))
        // Both listed: two files on a server with case, never replaced.
        assertEquals(RenameCheck.TAKEN, renameCheck("readme", "README", listOf("readme", "README"), exists = true))
        // Another name the server finds is another file.
        assertEquals(RenameCheck.TAKEN, renameCheck("readme", "notes", listOf("readme", "notes"), exists = true))
        assertEquals(RenameCheck.FREE, renameCheck("readme", "README", listOf("readme"), exists = false))
    }

    @Test
    fun theLicensesListReadsTheNoticesSection() {
        val text = listOf(
            "## The Android app (apps/android)",
            "",
            "What the APK ships.",
            "",
            "### Android libraries",
            "",
            "<!-- android-libraries:start (generated) -->",
            "",
            "| Library | Version | License | Source |",
            "|---|---|---|---|",
            "| androidx.activity:activity | 1.13.0 | Apache-2.0 | <https://developer.android.com> |",
            "",
            "<!-- android-libraries:end -->",
        ).joinToString("\n")
        val items = parseNotices(text)
        assertEquals(NoticeItem.Paragraph("What the APK ships."), items[0])
        assertEquals(NoticeItem.Heading("Android libraries"), items[1])
        assertEquals(NoticeItem.Component("androidx.activity:activity", "1.13.0", "Apache-2.0", "https://developer.android.com"), items[2])
        assertEquals(3, items.size)
    }
}
