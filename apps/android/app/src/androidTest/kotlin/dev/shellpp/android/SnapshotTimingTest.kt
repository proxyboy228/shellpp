package dev.shellpp.android

import android.content.Intent
import android.os.Bundle
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The screen snapshot's profile (plan of run 16, 3.3; D224): the terminal
 * floods with output (`seq`, a big file through `cat`) while the session
 * screen is seen, and the snapshot's and the drawing's times per frame are
 * reported as instrumentation status (`INSTRUMENTATION_STATUS: ...`) and held
 * to the frame budget on average. The numbers are the emulator's (x86_64 on
 * the computer), not the S25's.
 * `scripts\verify-android.ps1 -Emulator -Programs` runs it.
 */
@RunWith(AndroidJUnit4::class)
class SnapshotTimingTest {
    @get:Rule
    val permissions: GrantPermissionRule = GrantPermissionRule.grant(
        TerminalActivity.ACCESS_LOCAL_NETWORK,
        android.Manifest.permission.POST_NOTIFICATIONS,
    )

    private val instrumentation = InstrumentationRegistry.getInstrumentation()

    @Before
    fun unlocked() = E2e.ensureUnlocked()

    private fun quickConnect(): Intent {
        val args = InstrumentationRegistry.getArguments()
        return Intent(instrumentation.targetContext, TerminalActivity::class.java)
            .putExtra(TerminalActivity.EXTRA_ADDRESS, args.getString("sshHost") ?: "10.0.2.2")
            .putExtra(TerminalActivity.EXTRA_PORT, args.getString("sshPort")?.toInt() ?: error("sshPort argument"))
            .putExtra(TerminalActivity.EXTRA_USERNAME, args.getString("sshUser") ?: error("sshUser argument"))
            .putExtra(TerminalActivity.EXTRA_TEST_PASSWORD, (args.getString("sshPassword") ?: error("sshPassword argument")).toByteArray())
            .putExtra(TerminalActivity.EXTRA_TEST_ACCEPT_HOST_KEY, true)
    }

    private fun lines(s: ActivityScenario<TerminalActivity>): List<String> {
        var lines = emptyList<String>()
        s.onActivity { lines = it.terminal.screen?.logicalLines() ?: emptyList() }
        return lines
    }

    private fun waitFor(s: ActivityScenario<TerminalActivity>, timeoutMs: Long, test: (List<String>) -> Boolean): Boolean {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            if (test(lines(s))) return true
            Thread.sleep(200)
        }
        return false
    }

    private fun flood(s: ActivityScenario<TerminalActivity>, name: String, command: String) {
        val marker = "flood-$name-done"
        FrameTiming.reset()
        val started = System.currentTimeMillis()
        s.onActivity { it.terminal.typeText("$command; echo $marker\r") }
        // The marker's own line (the command line may have scrolled away).
        assertTrue("$name ends", waitFor(s, 600_000) { lines -> lines.any { it.trimEnd() == marker } })
        val report = FrameTiming.report()
        val status = Bundle()
        for ((key, value) in report) status.putLong("$name.$key", value)
        status.putLong("$name.seconds", (System.currentTimeMillis() - started) / 1000)
        instrumentation.sendStatus(0, status)
        // The frame budget (D224): half a 60 Hz frame each, on average.
        assertTrue("$name: frames drawn ($report)", report.getValue("snapshots") > 0 && report.getValue("draws") > 0)
        assertTrue("$name: the snapshot within 2 ms on average ($report)", report.getValue("snapshotAvgUs") <= SNAPSHOT_BUDGET_US)
        assertTrue("$name: the drawing within 8 ms on average ($report)", report.getValue("drawAvgUs") <= DRAW_BUDGET_US)
    }

    @Test
    fun theSnapshotAndTheDrawingKeepUpWithAFlood() {
        ActivityScenario.launch<TerminalActivity>(quickConnect()).use { s ->
            assertTrue("a prompt", waitFor(s, 60_000) { lines -> lines.any { it.trimEnd().endsWith("$") || it.trimEnd().endsWith("#") } })
            flood(s, "seq", "seq 1 300000")
            s.onActivity { it.terminal.typeText("seq 1 200000 | sed 's/$/ the quick brown fox jumps over the lazy dog 0123456789/' > /tmp/shellpp-big.txt\r") }
            Thread.sleep(5_000)
            flood(s, "cat", "cat /tmp/shellpp-big.txt")
        }
    }

    private companion object {
        const val SNAPSHOT_BUDGET_US = 2_000L
        const val DRAW_BUDGET_US = 8_000L
    }
}
