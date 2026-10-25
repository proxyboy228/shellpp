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
    }
}
