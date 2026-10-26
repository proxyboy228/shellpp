package dev.shellpp.android

import android.accessibilityservice.AccessibilityServiceInfo
import android.content.Intent
import android.view.View
import android.view.accessibility.AccessibilityManager
import android.view.accessibility.AccessibilityNodeInfo
import androidx.test.core.app.ActivityScenario
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The terminal with touch exploration on, as TalkBack has it (review of run
 * 16): the terminal and each row take accessibility focus, a row reads its
 * text, the cursor's row says so, and a double tap on a row is taken. The
 * test's own accessibility connection asks for touch exploration; the
 * emulator needs no TalkBack.
 */
@RunWith(AndroidJUnit4::class)
class TalkBackTest {
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

    private fun waitFor(timeoutMs: Long, test: () -> Boolean): Boolean {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            if (test()) return true
            Thread.sleep(200)
        }
        return false
    }

    @Test
    fun theTerminalAndItsRowsTakeTalkBacksFocus() {
        ActivityScenario.launch<TerminalActivity>(quickConnect()).use { s ->
            val marker = "talkback-row-${System.currentTimeMillis()}"
            assertTrue("a prompt", E2e.waitForTerminal(60_000) { lines -> lines.any { it.trimEnd().endsWith("$") } })
            E2e.typeInTerminal("echo $marker\r")
            assertTrue("the marker", E2e.waitForTerminal(20_000) { lines -> lines.any { it.trimEnd() == marker } })

            val ui = instrumentation.uiAutomation
            val info = ui.serviceInfo
            val flags = info.flags
            info.flags = flags or AccessibilityServiceInfo.FLAG_REQUEST_TOUCH_EXPLORATION_MODE
            ui.serviceInfo = info
            try {
                val manager = instrumentation.targetContext.getSystemService(AccessibilityManager::class.java)
                assertTrue("touch exploration", waitFor(10_000) { manager.isTouchExplorationEnabled })
                s.onActivity { activity ->
                    val view = activity.terminal
                    val screen = view.screen!!
                    val provider = view.accessibilityNodeProvider!!
                    val host = provider.createAccessibilityNodeInfo(View.NO_ID)!!
                    assertEquals(activity.getString(R.string.terminal_description), host.contentDescription)
                    assertTrue("rows", host.childCount > 0)

                    assertTrue("the terminal takes focus", provider.performAction(View.NO_ID, AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS, null))
                    val row = (0 until screen.rows).first { screen.text(0, it, screen.cols - 1, it).trimEnd() == marker }
                    assertTrue("a row takes focus", provider.performAction(row, AccessibilityNodeInfo.ACTION_ACCESSIBILITY_FOCUS, null))
                    val node = provider.createAccessibilityNodeInfo(row)!!
                    assertTrue("focused", node.isAccessibilityFocused)
                    assertEquals(marker, node.text.toString())
                    assertTrue("visible", node.isVisibleToUser)

                    val cursor = provider.createAccessibilityNodeInfo(screen.cursorRow)!!
                    assertEquals(activity.getString(R.string.terminal_cursor_row), cursor.stateDescription?.toString())
                    // A double tap on a row: the keyboard, as a tap.
                    assertTrue("a double tap", provider.performAction(row, AccessibilityNodeInfo.ACTION_CLICK, null))
                }
            } finally {
                info.flags = flags
                ui.serviceInfo = info
            }
        }
    }
}
