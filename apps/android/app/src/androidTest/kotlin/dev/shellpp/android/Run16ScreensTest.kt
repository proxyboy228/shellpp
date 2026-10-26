package dev.shellpp.android

import android.content.ClipData
import android.content.ClipboardManager
import android.graphics.Bitmap
import android.os.SystemClock
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import androidx.test.uiautomator.By
import androidx.test.uiautomator.Direction
import androidx.test.uiautomator.Until
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.regex.Pattern

/**
 * The screens that changed in run 16, for the owner (plan 3.4): runs only
 * with the instrumentation argument `screens16=<theme>` and the server
 * arguments of [ConnectTest]; the PNGs go to the app's external files
 * (`screens16/<theme>-<state>.png`).
 */
@RunWith(AndroidJUnit4::class)
class Run16ScreensTest {
    @get:Rule
    val permissions: GrantPermissionRule = GrantPermissionRule.grant(
        TerminalActivity.ACCESS_LOCAL_NETWORK,
        android.Manifest.permission.POST_NOTIFICATIONS,
    )

    private val args = InstrumentationRegistry.getArguments()

    private fun shot(theme: String, state: String) {
        SystemClock.sleep(900)
        E2e.instrumentation.waitForIdleSync()
        val bitmap = E2e.instrumentation.uiAutomation.takeScreenshot() ?: return
        val dir = File(E2e.instrumentation.targetContext.getExternalFilesDir(null), "screens16").apply { mkdirs() }
        File(dir, "$theme-$state.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }

    private fun vault(): String = Core.array("vaults").getJSONObject(0).getString("vault_id")

    private fun save(type: String, data: JSONObject, secrets: JSONObject? = null): String {
        val input = JSONObject().put("vault_id", vault()).put("type", type).put("data", data)
        if (secrets != null) input.put("secrets", secrets)
        return Core.obj("record_save", input).getString("record_id")
    }

    private fun scrollDown() {
        E2e.device.findObject(By.scrollable(true))?.scroll(Direction.DOWN, 1.0f)
    }

    @Test
    fun theScreensOfRun16() {
        val theme = args.getString("screens16")
        assumeTrue("screenshots only when asked", theme != null)
        val port = args.getString("sshPort")?.toInt() ?: error("sshPort argument")
        E2e.ensureUnlocked()
        val title = "Lab server"
        if (Core.array("records_query", JSONObject().put("types", JSONArray().put("host")).put("text", title)).length() == 0) {
            val group = save("group", JSONObject().put("name", "Production").put("notes", "Owned by the ops team.\nPatch window: Sunday 02:00."))
            save(
                "host",
                // Its output is recorded: the password key shows why it cannot send.
                JSONObject().put("address", E2e.sshHost).put("label", title).put("group_id", group).put("record_output", "on")
                    .put("notes", "The lab box under the desk.\nReboot only after the backup.")
                    .put("ssh", JSONObject().put("port", port).put("username", E2e.sshUser)),
                JSONObject().put("ssh.password", E2e.sshPassword),
            )
        }
        E2e.launch()

        // The editors' notes (D223 item 4), at the bottom of each editor.
        E2e.openHosts()
        E2e.find("group_Production").longClick()
        E2e.click("selection_edit")
        E2e.find("group_name")
        E2e.hideKeyboard()
        scrollDown()
        shot(theme!!, "01-group-editor-notes")
        E2e.click("editor_close")
        if (E2e.has("selection_close", 2_000)) E2e.click("selection_close")
        E2e.click("group_Production")
        E2e.find("host_$title").longClick()
        E2e.click("selection_edit")
        E2e.find("host_label")
        E2e.hideKeyboard()
        scrollDown()
        scrollDown()
        shot(theme, "02-host-editor-notes")
        E2e.click("editor_close")
        if (E2e.has("selection_close", 2_000)) E2e.click("selection_close")

        // The terminal at 11 sp (D223 item 7).
        E2e.click("host_$title")
        E2e.device.wait(Until.findObject(By.text(Pattern.compile("(?i)accept and save"))), 20_000)?.click()
        E2e.waitForTerminal(60_000) { lines -> lines.any { it.contains("$") } }
        E2e.typeInTerminal("ls -la /etc | head -30\r")
        E2e.waitForTerminal(10_000) { lines -> lines.any { it.contains("passwd") || it.contains("total") } }
        E2e.hideKeyboard()
        shot(theme, "03-terminal-11sp")
        E2e.click("key_password")
        shot(theme, "04-password-key-recorded")

        // The multi-line paste question (ui-spec 9.4).
        E2e.instrumentation.runOnMainSync {
            val clipboard = E2e.instrumentation.targetContext.getSystemService(ClipboardManager::class.java)
            clipboard.setPrimaryClip(ClipData.newPlainText("lines", "uptime\ndf -h\nfree -m\n"))
        }
        E2e.instrumentation.runOnMainSync {
            val activity = androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry.getInstance()
                .getActivitiesInStage(androidx.test.runner.lifecycle.Stage.RESUMED).filterIsInstance<TerminalActivity>().firstOrNull()
            activity?.terminal?.paste()
        }
        E2e.text("Paste several lines?", 10_000)
        shot(theme, "05-paste-question")
        E2e.clickText("Cancel")

        // An SFTP session beside it: its card in Connections (D223 item 9).
        E2e.click("session_plus")
        E2e.clickText("Connect via SFTP")
        E2e.find("sftp_add", 60_000)
        E2e.click("session_back")
        E2e.click("tab_connections", 30_000)
        E2e.find("sftp_card_folder", 30_000)
        shot(theme, "06-connections-sftp-card")

        // Settings: the bell and the paste rows, About, the licenses (D223 item 5).
        E2e.click("tab_settings")
        E2e.find("settings_bell", 20_000)
        shot(theme, "07-settings-terminal")
        scrollDown()
        scrollDown()
        E2e.find("settings_licenses", 20_000)
        shot(theme, "08-settings-about")
        E2e.click("settings_licenses")
        E2e.find("licenses", 20_000)
        shot(theme, "09-licenses")
        E2e.device.pressBack()
    }
}
