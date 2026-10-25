package dev.shellpp.android

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Fingerprint unlock on the emulator (D218). `scripts\verify-android.ps1
 * -Emulator` gives the emulator a PIN and enrolls finger 1 first, and
 * touches the sensor with finger 1 every two seconds while this class runs,
 * so every biometric prompt passes: turning the option on reads the key
 * back once, and after Lock the prompt opens the vault.
 */
@RunWith(AndroidJUnit4::class)
class FingerprintOnTest {
    @Test
    fun fingerprintUnlockTurnsOnAndOpensTheVaultAfterALock() {
        E2e.launch()
        E2e.type("unlock_password", E2e.vaultPassword)
        E2e.click("unlock")
        E2e.click("tab_more", 60_000)
        E2e.click("more_settings")
        val toggle = E2e.find("settings_fingerprint")
        assertTrue("fingerprint unlock is offered", toggle.isEnabled)
        toggle.click()
        // The master password once before the prompt (UI spec 12).
        E2e.type("fingerprint_password", E2e.vaultPassword)
        E2e.clickText("Continue")
        val deadline = System.currentTimeMillis() + 60_000
        while (!E2e.find("settings_fingerprint").isChecked && System.currentTimeMillis() < deadline) Thread.sleep(500)
        if (!E2e.find("settings_fingerprint").isChecked) E2e.shot("fingerprint-not-on")
        assertTrue("fingerprint unlock is on", E2e.find("settings_fingerprint").isChecked)
        E2e.click("screen_close")

        // Lock: the unlock screen opens the prompt by itself, the finger opens the vault.
        E2e.click("tab_more")
        E2e.click("more_lock")
        assertTrue("unlocked with the fingerprint", E2e.has("tab_hosts", 60_000))
        assertTrue(E2e.app.lock.value.unlocked)
    }
}

/**
 * After the script enrolled finger 2 (the Keystore key became invalid,
 * D218): no prompt, the master password opens the vault, and Settings
 * says why fingerprint unlock went off. Runs without the sensor touches.
 */
@RunWith(AndroidJUnit4::class)
class FingerprintInvalidatedTest {
    @Test
    fun aNewFingerprintTurnsFingerprintUnlockOffWithItsReason() {
        E2e.launch()
        E2e.find("unlock_password", 30_000)
        assertFalse("no fingerprint button", E2e.has("unlock_fingerprint", 3_000))
        E2e.type("unlock_password", E2e.vaultPassword)
        E2e.click("unlock")
        E2e.click("tab_more", 60_000)
        E2e.click("more_settings")
        assertFalse("fingerprint unlock is off", E2e.find("settings_fingerprint").isChecked)
        assertTrue("Settings says why", E2e.hasText("a new fingerprint was added", 10_000))
    }
}
