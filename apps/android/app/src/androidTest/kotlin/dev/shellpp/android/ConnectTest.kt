package dev.shellpp.android

import android.content.Intent
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertTrue
import androidx.test.rule.GrantPermissionRule
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.util.UUID

/**
 * End to end on the emulator (plan 3.7): the app connects to OpenSSH in
 * Docker on the computer (the emulator sees it at 10.0.2.2), runs a
 * command, and its output is on the terminal's screen.
 * `scripts\verify-android.ps1 -Emulator` passes the port, the user and the
 * password made for the run as instrumentation arguments.
 */
@RunWith(AndroidJUnit4::class)
class ConnectTest {
    /** The local network (10.0.2.2) and the service's notification, granted for the test. */
    @get:Rule
    val permissions: GrantPermissionRule = GrantPermissionRule.grant(
        TerminalActivity.ACCESS_LOCAL_NETWORK,
        android.Manifest.permission.POST_NOTIFICATIONS,
    )

    @Test
    fun aQuickConnectRunsACommandAndShowsItsOutput() {
        val args = InstrumentationRegistry.getArguments()
        val host = args.getString("sshHost") ?: "10.0.2.2"
        val port = args.getString("sshPort")?.toInt() ?: error("sshPort argument")
        val user = args.getString("sshUser") ?: error("sshUser argument")
        val password = args.getString("sshPassword") ?: error("sshPassword argument")
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        val intent = Intent(context, TerminalActivity::class.java)
            .putExtra(TerminalActivity.EXTRA_ADDRESS, host)
            .putExtra(TerminalActivity.EXTRA_PORT, port)
            .putExtra(TerminalActivity.EXTRA_USERNAME, user)
            .putExtra(TerminalActivity.EXTRA_TEST_PASSWORD, password.toByteArray())
            .putExtra(TerminalActivity.EXTRA_TEST_ACCEPT_HOST_KEY, true)
        val marker = "shellpp-e2e-" + UUID.randomUUID().toString().take(8)
        ActivityScenario.launch<TerminalActivity>(intent).use { scenario ->
            assertTrue("a prompt", waitFor(scenario, 60_000) { lines -> lines.any { it.contains("$") || it.contains("#") } })
            scenario.onActivity { it.terminal.typeText("echo $marker\r") }
            assertTrue(
                "the command's output",
                waitFor(scenario, 30_000) { lines -> lines.count { it.contains(marker) } >= 2 },
            )
        }
    }

    private fun waitFor(
        scenario: ActivityScenario<TerminalActivity>,
        timeoutMs: Long,
        test: (List<String>) -> Boolean,
    ): Boolean {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            var lines = emptyList<String>()
            scenario.onActivity { lines = it.terminal.screen?.logicalLines() ?: emptyList() }
            if (test(lines)) return true
            Thread.sleep(250)
        }
        return false
    }
}
