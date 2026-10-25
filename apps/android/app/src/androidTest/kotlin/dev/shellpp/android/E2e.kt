package dev.shellpp.android

import android.content.Intent
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry
import androidx.test.runner.lifecycle.Stage
import androidx.test.uiautomator.By
import androidx.test.uiautomator.Direction
import androidx.test.uiautomator.UiDevice
import androidx.test.uiautomator.UiObject2
import androidx.test.uiautomator.Until
import org.junit.Assert.assertNotNull

/**
 * Helpers of the end-to-end tests on the emulator (M1):
 * `scripts\verify-android.ps1 -Emulator` passes the OpenSSH server's port,
 * user and password and the vault's master password made for the run as
 * instrumentation arguments, and runs the classes one by one, each in a
 * new process, so that "after a restart" is a real one.
 */
object E2e {
    val instrumentation get() = InstrumentationRegistry.getInstrumentation()
    val device: UiDevice get() = UiDevice.getInstance(instrumentation)
    val app get() = instrumentation.targetContext.applicationContext as App

    fun arg(name: String): String =
        InstrumentationRegistry.getArguments().getString(name) ?: error("$name argument")

    val vaultPassword get() = arg("vaultPassword")
    val sshHost get() = InstrumentationRegistry.getArguments().getString("sshHost") ?: "10.0.2.2"
    val sshPort get() = arg("sshPort").toInt()
    val sshUser get() = arg("sshUser")
    val sshPassword get() = arg("sshPassword")

    /** Starts the app's main screen in a task of its own. */
    fun launch() {
        val context = instrumentation.targetContext
        context.startActivity(
            Intent(context, MainActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK or Intent.FLAG_ACTIVITY_CLEAR_TASK),
        )
    }

    /** The control tagged [tag], scrolling the screen down to it when it is below (the keyboard may hide it). */
    fun find(tag: String, timeoutMs: Long = 20_000): UiObject2 {
        var found = device.wait(Until.findObject(By.res(tag)), minOf(timeoutMs, 3_000))
        var scrolls = 0
        while (found == null && scrolls < 6) {
            device.findObject(By.scrollable(true))?.scroll(Direction.DOWN, 0.6f)
            scrolls += 1
            found = device.wait(Until.findObject(By.res(tag)), 1_000)
        }
        if (found == null) found = device.wait(Until.findObject(By.res(tag)), timeoutMs)
        if (found == null) shot("no-$tag")
        assertNotNull("no control $tag on the screen", found)
        return found!!
    }

    /** Popups and dialogs are windows of their own, without the screen's tags as ids: by their text. */
    fun clickText(text: String, timeoutMs: Long = 20_000) = text(text, timeoutMs).click()

    fun has(tag: String, timeoutMs: Long = 5_000): Boolean =
        device.wait(Until.hasObject(By.res(tag)), timeoutMs)

    fun click(tag: String, timeoutMs: Long = 20_000) = find(tag, timeoutMs).click()

    fun type(tag: String, text: String) {
        val field = find(tag)
        field.click()
        field.text = text
    }

    /** A screenshot for the failure's log (`adb pull` from the app's external files). */
    fun shot(name: String) {
        val dir = instrumentation.targetContext.getExternalFilesDir(null) ?: return
        device.takeScreenshot(java.io.File(dir, name.replace(Regex("[^A-Za-z0-9_-]"), "_") + ".png"))
    }

    fun text(text: String, timeoutMs: Long = 20_000): UiObject2 {
        val found = device.wait(Until.findObject(By.textContains(text)), timeoutMs)
        if (found == null) shot("no-text-$text")
        assertNotNull("no text \"$text\" on the screen", found)
        return found!!
    }

    fun hasText(text: String, timeoutMs: Long = 5_000): Boolean =
        device.wait(Until.hasObject(By.textContains(text)), timeoutMs)

    /** The engine runs (the core opens its database off the main thread). */
    fun awaitEngine() {
        val deadline = System.currentTimeMillis() + 30_000
        while (!app.ready && System.currentTimeMillis() < deadline) Thread.sleep(100)
        check(app.ready) { "the engine did not start: ${app.startError}" }
    }

    /** The vault exists and is unlocked, made or opened with the run's password. */
    fun ensureUnlocked() {
        awaitEngine()
        val status = Core.lockStatus()
        when {
            status.firstRun -> Core.withPassword("first_run_setup", "password", vaultPassword.toCharArray())
            !status.unlocked -> Core.withPassword("unlock", "password", vaultPassword.toCharArray())
        }
        app.refreshLock()
    }

    /** The terminal screen in front (call on the main thread). */
    private fun resumedTerminal(): TerminalActivity? =
        ActivityLifecycleMonitorRegistry.getInstance()
            .getActivitiesInStage(Stage.RESUMED)
            .filterIsInstance<TerminalActivity>()
            .firstOrNull()

    /** The lines on the terminal's screen, empty while no terminal is shown. */
    fun terminalLines(): List<String> {
        var lines = emptyList<String>()
        instrumentation.runOnMainSync {
            lines = resumedTerminal()?.terminal?.screen?.logicalLines() ?: emptyList()
        }
        return lines
    }

    fun typeInTerminal(text: String) {
        instrumentation.runOnMainSync { resumedTerminal()?.terminal?.typeText(text) }
    }

    fun waitForTerminal(timeoutMs: Long, test: (List<String>) -> Boolean): Boolean {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            if (test(terminalLines())) return true
            Thread.sleep(250)
        }
        return false
    }
}
