package dev.shellpp.android

import android.graphics.Bitmap
import android.os.SystemClock
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assume.assumeTrue
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File

/**
 * The screens of M1 for review (UI spec 4-6, 12): first run, the hosts
 * list at the top and in a group, the host editor with an inherited value,
 * Settings, the unlock screen. Runs only with `screenshots=<theme>` on a
 * fresh install (the first run is the first screen); the PNGs go to the
 * app's external files (`screens/<theme>-<state>.png`).
 */
@RunWith(AndroidJUnit4::class)
class VaultScreensTest {
    private fun shot(theme: String, state: String) {
        SystemClock.sleep(1000)
        E2e.instrumentation.waitForIdleSync()
        val bitmap = E2e.instrumentation.uiAutomation.takeScreenshot() ?: return
        val dir = File(E2e.instrumentation.targetContext.getExternalFilesDir(null), "screens").apply { mkdirs() }
        File(dir, "$theme-$state.png").outputStream().use { bitmap.compress(Bitmap.CompressFormat.PNG, 100, it) }
    }

    private fun save(type: String, data: JSONObject, secrets: JSONObject? = null): String {
        val vault = Core.array("vaults").getJSONObject(0).getString("vault_id")
        val input = JSONObject().put("vault_id", vault).put("type", type).put("data", data)
        if (secrets != null) input.put("secrets", secrets)
        return Core.obj("record_save", input).getString("record_id")
    }

    @Test
    fun theScreensOfTheVault() {
        val theme = InstrumentationRegistry.getArguments().getString("screenshots")
        assumeTrue("screenshots only when asked", theme != null)
        E2e.launch()
        E2e.find("create_vault_start", 30_000)
        shot(theme!!, "first-run")
        E2e.click("create_vault_start")
        E2e.type("first_password", "Correct-Horse-9")
        shot(theme, "first-run-password")

        E2e.ensureUnlocked()
        val servers = save("group", JSONObject().put("name", "Servers").put("color", "ocean").put("ssh", JSONObject().put("username", "deploy")))
        save("group", JSONObject().put("name", "Home lab").put("color", "jade"))
        save("host", JSONObject().put("address", "web1.example.org").put("label", "Web 1").put("group_id", servers).put("color", "ocean"))
        save("host", JSONObject().put("address", "db.example.org").put("label", "Database").put("group_id", servers)
            .put("tags", JSONArray().put("prod")), JSONObject().put("ssh.password", "example-only"))
        save("host", JSONObject().put("address", "203.0.113.7").put("ssh", JSONObject().put("username", "root")))
        E2e.find("tab_hosts", 30_000)
        shot(theme, "hosts")
        E2e.click("group_Servers")
        shot(theme, "hosts-group")
        E2e.find("host_Database").longClick()
        shot(theme, "host-sheet")
        E2e.clickText("Edit")
        E2e.find("host_address")
        shot(theme, "host-editor")
        E2e.click("editor_close")
        E2e.click("tab_more")
        shot(theme, "more")
        E2e.click("more_settings")
        shot(theme, "settings")
        E2e.click("screen_close")
        E2e.click("more_lock")
        E2e.find("unlock_password")
        shot(theme, "unlock")
    }
}
