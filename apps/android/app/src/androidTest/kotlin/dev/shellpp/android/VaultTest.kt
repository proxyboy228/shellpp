package dev.shellpp.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.rule.GrantPermissionRule
import androidx.test.uiautomator.By
import androidx.test.uiautomator.Until
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.util.UUID
import java.util.regex.Pattern

private const val GROUP = "E2E servers"
private const val HOST = "E2E host"

/**
 * M1 on the emulator, first process: a new vault at the first run, a group
 * with the SSH defaults of the run's OpenSSH server, a host in it with the
 * server's password, and a session from the host's row that runs a command.
 */
@RunWith(AndroidJUnit4::class)
class VaultFirstRunTest {
    @get:Rule
    val permissions: GrantPermissionRule = GrantPermissionRule.grant(
        TerminalActivity.ACCESS_LOCAL_NETWORK,
        android.Manifest.permission.POST_NOTIFICATIONS,
    )

    @Test
    fun aNewVaultGetsAGroupAndAHostThatConnects() {
        E2e.launch()
        E2e.click("create_vault_start", 30_000)
        E2e.type("first_password", E2e.vaultPassword)
        E2e.type("first_repeat", E2e.vaultPassword)
        E2e.click("create_vault")
        E2e.find("add", 60_000)

        // A group whose SSH defaults the host inherits.
        E2e.click("add")
        E2e.clickText("New group")
        E2e.type("group_name", GROUP)
        E2e.type("group_username", E2e.sshUser)
        E2e.type("group_port", E2e.sshPort.toString())
        Thread.sleep(1_500)
        E2e.click("editor_close")
        E2e.click("group_$GROUP")

        // A host in it: address, label, password; the username comes from the group.
        E2e.click("add")
        E2e.clickText("New host")
        E2e.type("host_address", E2e.sshHost)
        E2e.type("host_label", HOST)
        E2e.type("host_password", E2e.sshPassword)
        Thread.sleep(1_500)
        assertTrue("the group's username shown as inherited", E2e.hasText("from group $GROUP", 10_000))
        E2e.click("editor_close")

        // Tap the host: a session; the first key is accepted and saved.
        E2e.click("host_$HOST")
        // The dialog's buttons may show in capitals.
        E2e.device.wait(Until.findObject(By.text(Pattern.compile("(?i)accept and save"))), 60_000)?.click()
        val prompted = E2e.waitForTerminal(60_000) { lines -> lines.any { it.contains("$") || it.contains("#") } }
        if (!prompted) E2e.shot("host-no-prompt")
        assertTrue("a prompt: ${E2e.terminalLines()}", prompted)
        val marker = "shellpp-m1-" + UUID.randomUUID().toString().take(8)
        E2e.typeInTerminal("echo $marker\r")
        assertTrue("the command's output", E2e.waitForTerminal(30_000) { lines -> lines.count { it.contains(marker) } >= 2 })
    }
}

/**
 * M1 on the emulator, a second process after the first: the vault is
 * locked at start, a wrong password is refused, the right one opens it with
 * the group and the host still there, and Lock asks for the password again.
 */
@RunWith(AndroidJUnit4::class)
class VaultAfterRestartTest {
    @Test
    fun theVaultIsThereAfterARestartAndLockAsksForThePassword() {
        E2e.launch()
        E2e.type("unlock_password", "not-the-password-" + UUID.randomUUID().toString().take(6))
        E2e.click("unlock")
        assertTrue("a wrong password is refused", E2e.hasText("Wrong password", 30_000))
        E2e.type("unlock_password", E2e.vaultPassword)
        E2e.click("unlock")
        E2e.click("group_$GROUP", 60_000)
        E2e.find("host_$HOST")

        // Lock from More: the unlock screen, and the password opens it again.
        E2e.click("tab_more")
        E2e.click("more_lock")
        E2e.type("unlock_password", E2e.vaultPassword)
        E2e.click("unlock")
        E2e.find("tab_hosts", 60_000)
    }
}

/**
 * The host editor writes only what was changed in it (review of run 14):
 * a change that comes in while it is open (written here through the core,
 * as a sync's pull writes it) stays after the editor's next saves, in a
 * field it does not show and in one it shows but nobody touched.
 */
@RunWith(AndroidJUnit4::class)
class EditorKeepsOtherChangesTest {
    @Test
    fun aChangeThatComesInWhileTheEditorIsOpenStays() {
        E2e.ensureUnlocked()
        E2e.launch()
        val vault = Core.array("vaults").getJSONObject(0).getString("vault_id")
        val label = "editor-" + UUID.randomUUID().toString().take(6)
        val id = Core.obj(
            "record_save",
            JSONObject().put("vault_id", vault).put("type", "host")
                .put("data", JSONObject().put("address", "editor.example.org").put("label", label)),
        ).getString("record_id")
        fun data() = Core.obj("record_get", JSONObject().put("vault_id", vault).put("record_id", id)).getJSONObject("data")

        E2e.click("tab_hosts", 60_000)
        E2e.type("search", label)
        E2e.find("host_$label").longClick()
        E2e.clickText("Edit")
        E2e.type("host_tags", "one")
        // Saved half a second after the last change.
        Thread.sleep(1_500)
        assertEquals("one", data().optJSONArray("tags")?.optString(0))

        // Another device's change arrives: the notes and the port.
        val incoming = data().put("notes", "from the desktop")
        incoming.put("ssh", (incoming.optJSONObject("ssh") ?: JSONObject()).put("port", 2222))
        Core.call("record_save", JSONObject().put("vault_id", vault).put("record_id", id).put("type", "host").put("data", incoming))

        E2e.type("host_tags", "one, two")
        Thread.sleep(1_500)
        E2e.click("editor_close")
        val after = data()
        assertEquals("the incoming notes stay", "from the desktop", after.optString("notes"))
        assertEquals("the incoming port stays", 2222, after.optJSONObject("ssh")?.optInt("port"))
        assertEquals("the editor's change is saved", 2, after.optJSONArray("tags")?.length())
        Core.call("record_delete", JSONObject().put("vault_id", vault).put("record_ids", JSONArray().put(id)))
    }
}

