package dev.shellpp.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.rule.GrantPermissionRule
import androidx.test.uiautomator.By
import androidx.test.uiautomator.Until
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import java.io.File
import java.util.UUID
import java.util.regex.Pattern

/**
 * M4 on the emulator against OpenSSH in Docker: a key generated on the
 * phone (Keychain), its public key put on the server through a session of
 * the key user with the run's client key (imported), a host that signs in
 * with the phone's key (no password asked), then SFTP to that host:
 * the home folder listed, a file uploaded and downloaded back through the
 * transfer queue, the listing showing it.
 */
@RunWith(AndroidJUnit4::class)
class KeysSftpTest {
    @get:Rule
    val permissions: GrantPermissionRule = GrantPermissionRule.grant(
        TerminalActivity.ACCESS_LOCAL_NETWORK,
        android.Manifest.permission.POST_NOTIFICATIONS,
    )

    private fun vault() = Core.array("vaults").getJSONObject(0).getString("vault_id")

    private fun acceptHostKeyIfAsked() {
        E2e.device.wait(Until.findObject(By.text(Pattern.compile("(?i)accept and save"))), 15_000)?.click()
    }

    @Test
    fun aKeyMadeOnThePhoneSignsInAndSftpMovesAFileBothWays() {
        E2e.ensureUnlocked()
        E2e.launch()
        val label = "phone-key-" + UUID.randomUUID().toString().take(6)

        // Keychain ▸ + ▸ Generate.
        E2e.click("tab_keychain", 60_000)
        E2e.click("key_add")
        E2e.clickText("Generate")
        E2e.type("key_label", label)
        E2e.hideKeyboard()
        E2e.clickExact("Generate")
        E2e.click("key_$label", 30_000)
        val deadline = System.currentTimeMillis() + 20_000
        var public = ""
        while (System.currentTimeMillis() < deadline) {
            public = E2e.find("key_public").text ?: ""
            if (public.startsWith("ssh-ed25519 ")) break
            Thread.sleep(250)
        }
        assertTrue("the public key in OpenSSH format: $public", public.startsWith("ssh-ed25519 "))
        E2e.click("page_back")

        // The public key goes on the server through the key user's session
        // with the run's client key, imported into the Keychain (the test
        // server's key user takes keys only: tests/openssh/README.md).
        val key = (0 until Core.array("keys_list").length()).map { Core.array("keys_list").getJSONObject(it) }
            .first { it.optString("label") == label }
        val clientKey = String(android.util.Base64.decode(E2e.arg("sshClientKey"), android.util.Base64.DEFAULT))
        val imported = Core.obj(
            "key_import",
            JSONObject().put("vault_id", vault()).put("label", "client-$label").put("private_key", clientKey),
        ).getJSONObject("key")
        val keyUser = E2e.arg("sshKeyUser")
        val passwordHost = Core.obj(
            "record_save",
            JSONObject().put("vault_id", vault()).put("type", "host").put(
                "data",
                JSONObject().put("address", E2e.sshHost).put("label", "client-$label")
                    .put("ssh", JSONObject().put("port", E2e.sshPort).put("username", keyUser).put("key_id", imported.getString("record_id"))),
            ),
        )
        E2e.click("tab_hosts")
        E2e.type("search", "client-$label")
        E2e.click("host_client-$label")
        acceptHostKeyIfAsked()
        assertTrue("a prompt", E2e.waitForTerminal(60_000) { lines -> lines.any { it.contains("$") } })
        E2e.typeInTerminal("echo '$public' >> ~/.ssh/authorized_keys && echo key-added-$label\r")
        assertTrue("the key added", E2e.waitForTerminal(30_000) { lines -> lines.count { it.contains("key-added-$label") } >= 2 })
        E2e.typeInTerminal("exit\r")
        E2e.device.pressBack()

        // A host with that key and no password: it signs in with the key.
        val keyHost = Core.obj(
            "record_save",
            JSONObject().put("vault_id", vault()).put("type", "host").put(
                "data",
                JSONObject().put("address", E2e.sshHost).put("label", "key-$label")
                    .put("ssh", JSONObject().put("port", E2e.sshPort).put("username", keyUser).put("key_id", key.getString("record_id"))),
            ),
        )
        E2e.launch()
        E2e.click("tab_hosts", 30_000)
        E2e.type("search", "key-$label")
        E2e.click("host_key-$label")
        acceptHostKeyIfAsked()
        val signedIn = E2e.waitForTerminal(60_000) { lines -> lines.any { it.contains("$") } }
        if (!signedIn) E2e.shot("key-no-prompt")
        assertTrue("signed in with the key: ${E2e.terminalLines().filter { it.isNotBlank() }}", signedIn)
        assertFalse("no password was asked", E2e.hasText("Password for", 1_000))
        E2e.device.pressBack()

        // SFTP to the key host: its home folder, then a file up and back down.
        E2e.launch()
        E2e.click("tab_hosts", 30_000)
        E2e.type("search", "key-$label")
        E2e.find("host_key-$label").longClick()
        E2e.clickText("Open SFTP")
        acceptHostKeyIfAsked()
        E2e.find("sftp_add", 60_000)
        val app = E2e.app
        val session = app.sftpSessions.first()
        val home = Core.answer(Native.sftpCall(session, "home", "{}")) as String
        val name = "shellpp-m4-" + UUID.randomUUID().toString().take(8) + ".txt"
        val content = "uploaded from the phone " + UUID.randomUUID()
        val dir = File(E2e.instrumentation.targetContext.cacheDir, "e2e").apply { mkdirs() }
        val upload = File(dir, "up").apply { writeText(content) }
        val up = Native.sftpUpload(session, upload.absolutePath, remoteJoin(home, name))
        assertTrue("uploaded", waitTransfer(app, up))
        val download = File(dir, "down")
        val down = Native.sftpDownload(session, remoteJoin(home, name), download.absolutePath)
        assertTrue("downloaded", waitTransfer(app, down))
        assertEquals(content, download.readText())
        // The listing shows it (pull to refresh reads the folder again).
        val listed = Core.answer(Native.sftpCall(session, "list", JSONObject().put("path", home).toString())) as JSONArray
        assertTrue((0 until listed.length()).any { listed.getJSONObject(it).optString("name") == name })
        Core.answer(Native.sftpCall(session, "remove", JSONObject().put("path", remoteJoin(home, name)).toString()))

        // An upload through the app's own queue, with a lock while it goes
        // (review of run 14): it is not dropped with the screen, it ends
        // on the server whole, and its cache file goes.
        val big = File(dir, "big").apply {
            outputStream().use { out ->
                val chunk = ByteArray(1 shl 20) { it.toByte() }
                repeat(24) { out.write(chunk) }
            }
        }
        val folder = remoteJoin(home, "shellpp-m4-" + UUID.randomUUID().toString().take(8))
        Core.answer(Native.sftpCall(session, "mkdir", JSONObject().put("path", folder).toString()))
        val transfersBefore = app.transfers.value.keys.toSet()
        val problem = java.util.concurrent.atomic.AtomicReference<String?>(null)
        app.upload(session, folder, listOf(android.net.Uri.fromFile(big))) { problem.set(it) }
        Core.call("lock")
        app.refreshLock()
        val queued = waitNewTransfer(app, transfersBefore)
        assertTrue("the upload was queued (${problem.get()})", queued != null)
        assertTrue("the upload ended while locked", waitTransfer(app, queued!!, 120_000))
        val uploaded = Core.answer(Native.sftpCall(session, "list", JSONObject().put("path", folder).toString())) as JSONArray
        assertEquals(1, uploaded.length())
        assertEquals(big.length(), uploaded.getJSONObject(0).optLong("size"))
        val cache = File(E2e.instrumentation.targetContext.cacheDir, "transfers")
        val emptied = System.currentTimeMillis() + 10_000
        while (cache.listFiles().orEmpty().isNotEmpty() && System.currentTimeMillis() < emptied) Thread.sleep(200)
        assertTrue("no cache file left: ${cache.listFiles().orEmpty().map { it.name }}", cache.listFiles().orEmpty().isEmpty())
        Core.answer(Native.sftpCall(session, "remove", JSONObject().put("path", remoteJoin(folder, uploaded.getJSONObject(0).optString("name"))).toString()))
        Core.answer(Native.sftpCall(session, "remove", JSONObject().put("path", folder).put("dir", true).toString()))
        E2e.ensureUnlocked()

        // What the test made goes: its hosts, its keys, its files.
        Core.call(
            "record_delete",
            JSONObject().put("vault_id", vault()).put(
                "record_ids",
                JSONArray().put(passwordHost.getString("record_id")).put(keyHost.getString("record_id"))
                    .put(key.getString("record_id")).put(imported.getString("record_id")),
            ),
        )
        dir.deleteRecursively()
    }

    /** The id of a transfer that was not in [before], once the app has one. */
    private fun waitNewTransfer(app: App, before: Set<Long>): Long? {
        val deadline = System.currentTimeMillis() + 30_000
        while (System.currentTimeMillis() < deadline) {
            app.transfers.value.keys.firstOrNull { it !in before }?.let { return it }
            Thread.sleep(100)
        }
        return null
    }

    private fun waitTransfer(app: App, id: Long, timeoutMs: Long = 60_000): Boolean {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            when (app.transfers.value[id]?.optString("state")) {
                "done" -> return true
                "failed", "cancelled" -> return false
            }
            Thread.sleep(200)
        }
        return false
    }
}
