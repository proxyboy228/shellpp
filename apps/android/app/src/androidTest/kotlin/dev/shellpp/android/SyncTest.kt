package dev.shellpp.android

import android.util.Base64
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.json.JSONArray
import org.json.JSONObject
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Sync on the emulator (M3) against the real server behind a TLS front of
 * the computer (`server/tests/android_peer.rs`, the "desktop" device of the
 * same account). `scripts\verify-android.ps1 -Emulator` passes the peer's
 * origin (`https://10.0.2.2:<port>`), account, CA and certificate
 * fingerprint, and runs the classes in order with the desktop's steps
 * between them.
 */
private object Peer {
    private val args get() = InstrumentationRegistry.getArguments()
    val origin: String get() = args.getString("syncOrigin") ?: error("syncOrigin argument")
    val email: String get() = args.getString("syncEmail") ?: error("syncEmail argument")
    val password: String get() = args.getString("syncPassword") ?: error("syncPassword argument")
    val fingerprint: String get() = args.getString("syncSha256") ?: error("syncSha256 argument")
    val caPem: String get() = String(Base64.decode(args.getString("syncCa") ?: error("syncCa argument"), Base64.DEFAULT))
    const val DESKTOP_HOST = "desktop-host.example.com"
    const val DESKTOP_SECOND = "desktop-second.example.com"
    const val PHONE_HOST = "phone-host.example.org"

    fun addresses(): Set<String> {
        val list = Core.array("records_query", JSONObject().put("types", JSONArray().put("host")))
        return (0 until list.length()).map { list.getJSONObject(it).getJSONObject("data").optString("address") }.toSet()
    }

    fun waitForAddress(address: String, timeoutMs: Long): Boolean {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (System.currentTimeMillis() < deadline) {
            if (address in addresses()) return true
            Thread.sleep(500)
        }
        return false
    }
}

/**
 * A phone with its own vault signs in to the desktop's account through the
 * wizard: the system does not trust the certificate, the fingerprint shown
 * is the server's and is pinned; the desktop's host arrives; the phone's
 * own host goes up with Sync now.
 */
@RunWith(AndroidJUnit4::class)
class SyncSignInTest {
    @Test
    fun theWizardPinsTheCertificateSignsInAndSyncs() {
        E2e.launch()
        E2e.type("unlock_password", E2e.vaultPassword)
        E2e.click("unlock")
        E2e.click("tab_more", 60_000)
        E2e.click("more_account")
        E2e.click("account_connect")
        E2e.type("sync_origin", Peer.origin)
        E2e.click("sync_continue")
        // Not trusted by the system: the certificate's fingerprint, then "Trust this certificate".
        val shown = E2e.find("cert_fingerprint", 60_000).text
        assertEquals(Peer.fingerprint, shown)
        E2e.click("trust_pin")
        E2e.type("sync_email", Peer.email)
        E2e.type("sync_password", Peer.password)
        E2e.click("sync_sign_in")
        E2e.find("account_sync_now", 120_000)
        assertTrue("the desktop's host arrived", Peer.waitForAddress(Peer.DESKTOP_HOST, 60_000))

        val vault = Core.array("vaults").getJSONObject(0).getString("vault_id")
        Core.call(
            "record_save",
            JSONObject().put("vault_id", vault).put("type", "host").put("data", JSONObject().put("address", Peer.PHONE_HOST)),
        )
        E2e.click("account_sync_now")
        assertTrue("synced", E2e.hasText("Synced:", 60_000))
    }
}

/**
 * A new process after the desktop added a second host: the unlock with the
 * account's password (the phone's master password since the sign-in) syncs
 * by itself, and the second host arrives.
 */
@RunWith(AndroidJUnit4::class)
class SyncAfterUnlockTest {
    @Test
    fun anUnlockSyncsAndTheDesktopsNewHostArrives() {
        E2e.launch()
        E2e.type("unlock_password", Peer.password)
        E2e.click("unlock")
        E2e.find("tab_hosts", 60_000)
        assertTrue("the desktop's second host arrived", Peer.waitForAddress(Peer.DESKTOP_SECOND, 90_000))
    }
}

/**
 * A clean install: the CA of the server is added (as "Add its CA…" would,
 * through the core), then "Restore from sync" on the first run brings every
 * host, and the account's password unlocks the phone from then on.
 */
@RunWith(AndroidJUnit4::class)
class SyncRestoreTest {
    @Test
    fun restoreFromSyncOnAFreshInstallBringsEverything() {
        E2e.awaitEngine()
        val ca = Core.obj(
            "ca_file_info",
            JSONObject().put("file", Base64.encodeToString(Peer.caPem.toByteArray(), Base64.NO_WRAP)),
        )
        val check = Core.obj(
            "sync_server_trust_verified",
            JSONObject().put("origin", Peer.origin).put("choice", JSONObject().put("kind", "ca").put("der", ca.getString("der"))),
        )
        assertEquals("the CA vouches for the server", "ready", check.optString("kind"))

        E2e.launch()
        E2e.click("restore_from_sync", 30_000)
        E2e.type("sync_origin", Peer.origin)
        E2e.click("sync_continue")
        E2e.type("sync_email", Peer.email)
        E2e.type("sync_password", Peer.password)
        E2e.click("sync_sign_in")
        E2e.find("tab_hosts", 120_000)
        for (address in listOf(Peer.DESKTOP_HOST, Peer.PHONE_HOST, Peer.DESKTOP_SECOND)) {
            assertTrue("$address restored", Peer.waitForAddress(address, 90_000))
        }
    }
}
