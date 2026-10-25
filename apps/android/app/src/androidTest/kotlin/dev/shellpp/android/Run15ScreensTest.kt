package dev.shellpp.android

import android.graphics.Bitmap
import android.os.SystemClock
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import androidx.test.rule.GrantPermissionRule
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assume.assumeTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * The screens of run 15 for the owner (plan 3.5): runs only with the
 * instrumentation argument `screens15=<theme>` on a fresh install; the
 * PNGs go to the app's external files (`screens15/<theme>-<state>.png`).
 * With the server arguments of [ConnectTest] it also shows the terminal,
 * two sessions and SFTP.
 */
@RunWith(AndroidJUnit4::class)
class Run15ScreensTest {
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
        val dir = File(E2e.instrumentation.targetContext.getExternalFilesDir(null), "screens15").apply { mkdirs() }
        File(dir, "$theme-$state.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }

    private fun vault(): String = Core.array("vaults").getJSONObject(0).getString("vault_id")

    private fun save(type: String, data: JSONObject, secrets: JSONObject? = null): String {
        val input = JSONObject().put("vault_id", vault()).put("type", type).put("data", data)
        if (secrets != null) input.put("secrets", secrets)
        return Core.obj("record_save", input).getString("record_id")
    }

    /** Example data (addresses from the documentation ranges, no real host). */
    private fun seed() {
        val production = save("group", JSONObject().put("name", "Production").put("ssh", JSONObject().put("username", "deploy")))
        save("group", JSONObject().put("name", "Home lab"))
        save("group", JSONObject().put("name", "Databases").put("parent_id", production))
        save("host", JSONObject().put("address", "web1.example.org").put("label", "Web 1").put("group_id", production))
        save("host", JSONObject().put("address", "web2.example.org").put("label", "Web 2").put("group_id", production))
        save("host", JSONObject().put("address", "203.0.113.7").put("label", "Router").put("ssh", JSONObject().put("username", "admin")))
        save("host", JSONObject().put("address", "198.51.100.20").put("label", "Backup").put("tags", JSONArray().put("nightly")))
        save("host", JSONObject().put("address", "192.0.2.15").put("ssh", JSONObject().put("username", "root")), JSONObject().put("ssh.password", "example-only"))
    }

    @Test
    fun theScreensOfRun15() {
        val theme = args.getString("screens15")
        assumeTrue("screenshots only when asked", theme != null)
        E2e.ensureUnlocked()
        if (Core.array("records_query", JSONObject().put("types", JSONArray().put("host"))).length() == 0) seed()
        E2e.launch()
        E2e.find("vault_hosts", 30_000)
        shot(theme!!, "01-vault-home")
        E2e.click("vault_switcher")
        shot(theme, "02-vault-switcher")
        E2e.device.pressBack()
        E2e.click("vault_hosts")
        E2e.find("host_Router")
        shot(theme, "03-hosts")
        E2e.click("sort")
        shot(theme, "04-sort")
        E2e.device.pressBack()
        E2e.click("search_open")
        E2e.type("search", "web")
        E2e.hideKeyboard()
        shot(theme, "05-search")
        E2e.click("search_close")
        E2e.find("host_Router").longClick()
        E2e.find("host_Backup").click()
        shot(theme, "06-selection")
        E2e.click("selection_close")
        E2e.click("add")
        shot(theme, "07-new-sheet")
        E2e.device.pressBack()
        E2e.click("group_Production")
        shot(theme, "08-group")
        E2e.find("host_Web 1").longClick()
        E2e.click("selection_edit")
        E2e.find("host_label")
        E2e.hideKeyboard()
        shot(theme, "09-host-editor-top")
        E2e.device.findObject(androidx.test.uiautomator.By.scrollable(true))?.scroll(androidx.test.uiautomator.Direction.DOWN, 1.0f)
        shot(theme, "10-host-editor-bottom")
        E2e.click("editor_close")
        E2e.click("tab_settings")
        shot(theme, "20-settings")
        val port = args.getString("sshPort")
        if (port != null) terminalStates(theme, port.toInt())
        vaultStates(theme)
    }

    /** Keychain, Known hosts and the two sub-pages of Settings (plan 3.4). */
    private fun vaultStates(theme: String) {
        if (Core.array("keys_list").length() == 0) {
            generateKey(JSONObject().put("label", "laptop").put("kind", "ed25519").put("save_passphrase", false), CharArray(0))
            Core.call("identity_save", JSONObject().put("label", "deploy").put("username", "deploy"))
        }
        E2e.openVaultSection("vault_keychain")
        E2e.find("key_laptop")
        shot(theme, "40-keychain")
        E2e.click("key_add")
        shot(theme, "41-keychain-sheet")
        E2e.device.pressBack()
        E2e.click("tab_vaults")
        E2e.click("tab_vaults")
        E2e.click("vault_known_hosts")
        shot(theme, "42-known-hosts")
        E2e.openSettings()
        E2e.click("settings_keyboard")
        shot(theme, "43-customize-keyboard")
        E2e.device.pressBack()
        E2e.click("settings_font")
        shot(theme, "44-font-colours")
        E2e.device.pressBack()
    }

    /** The terminal, two sessions, SFTP and Connections, against the run's OpenSSH server. */
    private fun terminalStates(theme: String, port: Int) {
        E2e.click("tab_connections")
        shot(theme, "30-connections-empty")
        val title = "Lab server"
        if (Core.array("records_query", JSONObject().put("types", JSONArray().put("host")).put("text", title)).length() == 0) {
            save(
                "host",
                JSONObject().put("address", E2e.sshHost).put("label", title)
                    .put("ssh", JSONObject().put("port", port).put("username", E2e.sshUser)),
                JSONObject().put("ssh.password", E2e.sshPassword),
            )
        }
        E2e.openHosts()
        E2e.click("host_$title")
        E2e.device.wait(androidx.test.uiautomator.Until.findObject(androidx.test.uiautomator.By.text(java.util.regex.Pattern.compile("(?i)accept and save"))), 20_000)?.click()
        E2e.waitForTerminal(60_000) { lines -> lines.any { it.contains("$") } }
        E2e.typeInTerminal("ls -la /etc | head -24\r")
        E2e.waitForTerminal(10_000) { lines -> lines.any { it.contains("passwd") || it.contains("total") } }
        shot(theme, "31-terminal-keyboard")
        E2e.hideKeyboard()
        shot(theme, "32-terminal")
        val width = E2e.device.displayWidth
        E2e.device.swipe(width / 3, 420, width / 3, 420, 120)
        shot(theme, "33-selection")
        E2e.device.pressBack()
        E2e.click("key_password")
        shot(theme, "34-password-confirm")
        E2e.clickText("Cancel")
        E2e.click("session_plus")
        shot(theme, "35-plus-menu")
        E2e.clickText("Connect via SFTP")
        E2e.find("sftp_add", 60_000)
        shot(theme, "36-sftp")
        E2e.click("chip_$title")
        shot(theme, "37-two-sessions")
        E2e.click("session_back")
        E2e.click("tab_connections", 30_000)
        shot(theme, "38-connections-sessions")
    }
}
