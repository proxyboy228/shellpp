package dev.shellpp.android

import android.content.Intent
import android.graphics.Bitmap
import android.os.SystemClock
import android.view.MotionEvent
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assume.assumeTrue
import androidx.test.rule.GrantPermissionRule
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * The terminal's screenshots (run 13, plan 3.7): the terminal with
 * the keyboard and the extra keys, a selection, the cursor put by a tap
 * with its handle. Runs only with the instrumentation argument
 * `screenshots=<theme>` and the server arguments of [ConnectTest]; the PNGs
 * go to the app's external files (`screens/<theme>-<state>.png`), where the
 * run pulls them from.
 */
@RunWith(AndroidJUnit4::class)
class ScreenshotsTest {
    /** The local network (10.0.2.2) and the service's notification, granted for the test. */
    @get:Rule
    val permissions: GrantPermissionRule = GrantPermissionRule.grant(
        TerminalActivity.ACCESS_LOCAL_NETWORK,
        android.Manifest.permission.POST_NOTIFICATIONS,
    )

    private val instrumentation = InstrumentationRegistry.getInstrumentation()
    private val args = InstrumentationRegistry.getArguments()

    private fun shot(theme: String, state: String) {
        SystemClock.sleep(800)
        instrumentation.waitForIdleSync()
        val bitmap = instrumentation.uiAutomation.takeScreenshot() ?: return
        val dir = File(instrumentation.targetContext.getExternalFilesDir(null), "screens").apply { mkdirs() }
        File(dir, "$theme-$state.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }

    private fun touch(x: Float, y: Float, holdMs: Long) {
        val down = SystemClock.uptimeMillis()
        instrumentation.sendPointerSync(MotionEvent.obtain(down, down, MotionEvent.ACTION_DOWN, x, y, 0))
        SystemClock.sleep(holdMs)
        val up = SystemClock.uptimeMillis()
        instrumentation.sendPointerSync(MotionEvent.obtain(down, up, MotionEvent.ACTION_UP, x, y, 0))
    }

    private fun waitForLines(scenario: ActivityScenario<TerminalActivity>, test: (List<String>) -> Boolean) {
        val deadline = System.currentTimeMillis() + 60_000
        while (System.currentTimeMillis() < deadline) {
            var lines = emptyList<String>()
            scenario.onActivity { lines = it.terminal.screen?.logicalLines() ?: emptyList() }
            if (test(lines)) return
            SystemClock.sleep(250)
        }
        error("the screen never showed what was waited for")
    }

    @Test
    fun theScreensOfTheTerminal() {
        val theme = args.getString("screenshots")
        assumeTrue("screenshots only when asked", theme != null)
        val context = instrumentation.targetContext

        // Sessions need the vault (M1).
        E2e.ensureUnlocked()

        val intent = Intent(context, TerminalActivity::class.java)
            .putExtra(TerminalActivity.EXTRA_ADDRESS, args.getString("sshHost") ?: "10.0.2.2")
            .putExtra(TerminalActivity.EXTRA_PORT, args.getString("sshPort")!!.toInt())
            .putExtra(TerminalActivity.EXTRA_USERNAME, args.getString("sshUser"))
            .putExtra(TerminalActivity.EXTRA_TEST_PASSWORD, args.getString("sshPassword")!!.toByteArray())
            .putExtra(TerminalActivity.EXTRA_TEST_ACCEPT_HOST_KEY, true)
        ActivityScenario.launch<TerminalActivity>(intent).use { scenario ->
            waitForLines(scenario) { lines -> lines.any { it.trimEnd().endsWith("$") } }
            // A short prompt, so the typed line below stays on one row.
            scenario.onActivity { it.terminal.typeText("PS1='$ '; clear\r") }
            waitForLines(scenario) { lines -> lines.firstOrNull()?.trimEnd() == "$" }
            scenario.onActivity {
                it.terminal.typeText(
                    "printf '\\033[1;32mok\\033[0m \\033[33mwarn\\033[0m \\033[31merror\\033[0m\\n'; ls -la /etc | head -8\r",
                )
            }
            waitForLines(scenario) { lines -> lines.any { it.contains("passwd") || it.contains("total") } }
            scenario.onActivity { it.terminal.showKeyboard() }
            shot(theme!!, "terminal")

            // A long press on a word of the listing selects it.
            var at: Pair<Float, Float>? = null
            scenario.onActivity { activity ->
                val screen = activity.terminal.screen!!
                val row = screen.lines().indexOfFirst { it.contains("total") }
                at = activity.terminal.cellOnScreen(1, row)
            }
            touch(at!!.first, at!!.second, holdMs = 900)
            shot(theme, "selection")
            touch(at!!.first, at!!.second, holdMs = 50)

            // The typed line, then a tap on its first word puts the cursor there.
            scenario.onActivity { it.terminal.typeText("echo hello world") }
            waitForLines(scenario) { lines -> lines.any { it.contains("echo hello world") } }
            scenario.onActivity { activity ->
                val screen = activity.terminal.screen!!
                val row = screen.lines().indexOfLast { it.contains("echo hello world") }
                val col = screen.lines()[row].indexOf("hello")
                at = activity.terminal.cellOnScreen(col, row)
            }
            touch(at!!.first, at!!.second, holdMs = 50)
            SystemClock.sleep(1500)
            shot(theme, "cursor")
        }
    }
}
