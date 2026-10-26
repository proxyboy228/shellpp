package dev.shellpp.android

import android.content.Intent
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The walks through full-screen programs (plan of run 16, 3.3; ui-spec 9):
 * nano with the owner's example (open a config, select lines, Cut in nano,
 * save), vim (insert, Esc, the arrows, `:wq`), less (a page down, `q`),
 * htop and mc (F10 from the keys row), tmux (`^B %`, `^B "`, `^B o`, `^B x`
 * from the keys row), each step checked on the screen. Against OpenSSH in
 * Docker with the terminal programs (`tests/openssh/programs.Dockerfile`):
 * `scripts\verify-android.ps1 -Emulator -Programs` (with SnapshotTimingTest).
 */
@RunWith(AndroidJUnit4::class)
class ProgramsTest {
    @get:Rule
    val permissions: GrantPermissionRule = GrantPermissionRule.grant(
        TerminalActivity.ACCESS_LOCAL_NETWORK,
        android.Manifest.permission.POST_NOTIFICATIONS,
    )

    private val context = InstrumentationRegistry.getInstrumentation().targetContext
    private var markers = 0

    @Before
    fun unlocked() = E2e.ensureUnlocked()

    private fun quickConnect(): Intent {
        val args = InstrumentationRegistry.getArguments()
        return Intent(context, TerminalActivity::class.java)
            .putExtra(TerminalActivity.EXTRA_ADDRESS, args.getString("sshHost") ?: "10.0.2.2")
            .putExtra(TerminalActivity.EXTRA_PORT, args.getString("sshPort")?.toInt() ?: error("sshPort argument"))
            .putExtra(TerminalActivity.EXTRA_USERNAME, args.getString("sshUser") ?: error("sshUser argument"))
            .putExtra(TerminalActivity.EXTRA_TEST_PASSWORD, (args.getString("sshPassword") ?: error("sshPassword argument")).toByteArray())
            .putExtra(TerminalActivity.EXTRA_TEST_ACCEPT_HOST_KEY, true)
    }

    private fun ActivityScenario<TerminalActivity>.rows(): List<String> {
        var rows = emptyList<String>()
        onActivity { activity ->
            val screen = activity.terminal.screen ?: return@onActivity
            rows = (0 until screen.rows).map { screen.text(0, it, screen.cols - 1, it) }
        }
        return rows
    }

    /** Waits until [test] holds for the screen's rows; fails with a screenshot. */
    private fun ActivityScenario<TerminalActivity>.waitFor(what: String, timeoutMs: Long = 30_000, test: (List<String>) -> Boolean) {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            if (test(rows())) return
            Thread.sleep(200)
        }
        E2e.shot("programs-${what.replace(' ', '-')}")
        fail("$what: ${rows()}")
    }

    private fun ActivityScenario<TerminalActivity>.type(text: String) = onActivity { it.terminal.typeText(text) }

    private fun ActivityScenario<TerminalActivity>.ctrl(letter: String) = onActivity { it.terminal.sendText(letter, ctrl = true) }

    /** A key of the extra-keys row, as a tap sends it. */
    private fun ActivityScenario<TerminalActivity>.key(id: String) = onActivity { it.key(XKey.ALL.getValue(id), it.target()) }

    /** Back at the shell: the program gave the screen back and the prompt waits; then a marker echoed. */
    private fun ActivityScenario<TerminalActivity>.shell(what: String) {
        // Typed while the program still reads its input, the marker would be
        // the program's (vim takes what comes after :wq).
        val deadline = System.currentTimeMillis() + 30_000
        while (System.currentTimeMillis() < deadline) {
            var ready = false
            onActivity { activity ->
                val screen = activity.terminal.screen ?: return@onActivity
                val last = (0 until screen.rows).map { screen.text(0, it, screen.cols - 1, it).trimEnd() }.lastOrNull { it.isNotEmpty() }
                ready = !screen.alternateScreen && last != null && (last.endsWith("$") || last.endsWith("#"))
            }
            if (ready) break
            Thread.sleep(200)
        }
        markers += 1
        val marker = "back-$markers-at-the-shell"
        type("echo $marker\r")
        // The echo's own line: the command line holds "echo" before it.
        waitFor("the shell after $what") { rows -> rows.any { it.trimEnd() == marker } }
    }

    @Test
    fun fullScreenProgramsWorkFromTheTerminalAndItsKeysRow() {
        ActivityScenario.launch<TerminalActivity>(quickConnect()).use { s ->
            s.waitFor("a prompt", 60_000) { rows -> rows.any { it.trimEnd().endsWith("$") || it.trimEnd().endsWith("#") } }
            s.type("export TERM=xterm-256color; clear\r")
            s.shell("the start")

            // nano: open a config, select two lines, Cut in nano, save (ui-spec 9.6).
            s.type("printf 'one\\ntwo\\nthree\\nfour\\n' > /tmp/shellpp-run16.conf; nano /tmp/shellpp-run16.conf\r")
            s.waitFor("nano") { rows -> rows.any { "^G" in it } && rows.any { it.trimEnd() == "two" } }
            var offered = false
            s.onActivity { activity ->
                val screen = activity.terminal.screen ?: return@onActivity
                val rows = (0 until screen.rows).map { screen.text(0, it, screen.cols - 1, it).trimEnd() }
                val two = rows.indexOf("two")
                val three = rows.indexOf("three")
                if (two >= 0 && three > two) offered = activity.terminal.cutInNanoForTest(Cell(0, two), Cell(screen.cols - 1, three))
            }
            assertTrue("Cut in nano offered for two lines", offered)
            s.waitFor("the lines cut") { rows -> rows.none { it.trimEnd() == "two" || it.trimEnd() == "three" } && rows.any { it.trimEnd() == "four" } }
            s.ctrl("o")
            // nano 8 asks "Write to File:", older ones "File Name to Write:".
            s.waitFor("nano asks the file name") { rows -> rows.any { "Write to File" in it || "File Name to Write" in it } }
            s.type("\r")
            s.waitFor("nano wrote") { rows -> rows.any { "Wrote" in it } }
            s.ctrl("x")
            s.shell("nano")
            s.type("tr '\\n' , < /tmp/shellpp-run16.conf; echo\r")
            s.waitFor("the file saved without the two lines") { rows -> rows.any { it.trimEnd() == "one,four," } }

            // vim: insert, Esc and the arrows from the row, x, :wq.
            s.type("vim -u NONE -N /tmp/shellpp-run16.vim\r")
            s.waitFor("vim") { rows -> rows.count { it.startsWith("~") } >= 3 }
            s.type("ihello")
            s.waitFor("vim inserted") { rows -> rows.any { it.trimEnd() == "hello" } }
            // A pause after Esc, so vim does not take Esc and the arrow for one key.
            s.key("esc")
            Thread.sleep(400)
            s.key("left")
            Thread.sleep(200)
            s.key("left")
            Thread.sleep(200)
            s.type("x")
            s.waitFor("vim deleted a character") { rows -> rows.any { it.trimEnd() == "helo" } }
            s.type(":wq\r")
            s.shell("vim")
            s.type("cat /tmp/shellpp-run16.vim\r")
            s.waitFor("the file vim wrote") { rows -> rows.any { it.trimEnd() == "helo" } }

            // less: a page down from the row, q.
            s.type("seq 1 400 | less\r")
            s.waitFor("less") { rows -> rows.firstOrNull()?.trimEnd() == "1" }
            s.key("pgdn")
            s.waitFor("less paged down") { rows -> rows.firstOrNull()?.trimEnd() != "1" && rows.any { it.trimEnd().toIntOrNull()?.let { n -> n > 30 } == true } }
            s.type("q")
            s.shell("less")

            // htop: F10 from the row quits.
            s.type("htop\r")
            s.waitFor("htop") { rows -> rows.any { "PID" in it } }
            s.key("f10")
            s.shell("htop")

            // mc: F10 from the row; its question, if it asks, is answered.
            s.type("mc\r")
            // Its menu bar (at 60 columns the function key bar ends before "10Quit").
            s.waitFor("mc") { rows -> rows.any { "Left" in it && "File" in it && "Command" in it } }
            s.key("f10")
            Thread.sleep(1_500)
            if (s.rows().any { "really want to quit" in it }) s.type("\r")
            s.shell("mc")

            // tmux: the prefix combinations of the row.
            s.type("tmux new -s run16\r")
            s.waitFor("tmux") { rows -> rows.any { "[run16]" in it } }
            s.key("tmux_split_h")
            s.type("tmux list-panes | wc -l | sed s/^/panes=/\r")
            s.waitFor("tmux split sideways") { rows -> rows.any { "panes=2" in it } }
            s.key("tmux_split_v")
            s.key("tmux_next")
            s.type("tmux list-panes | wc -l | sed s/^/panes=/\r")
            s.waitFor("tmux split again") { rows -> rows.any { "panes=3" in it } }
            s.key("tmux_close")
            s.waitFor("tmux asks to kill the pane") { rows -> rows.any { "kill-pane" in it } }
            s.type("y")
            s.type("tmux list-panes | wc -l | sed s/^/panes=/\r")
            s.waitFor("tmux closed a pane") { rows -> rows.any { "panes=2" in it } }
            s.type("tmux kill-server\r")
            s.shell("tmux")
        }
    }
}
