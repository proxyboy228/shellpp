package dev.shellpp.android

import android.content.pm.ApplicationInfo
import androidx.test.ext.junit.runners.AndroidJUnit4
import androidx.test.platform.app.InstrumentationRegistry
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test
import org.junit.runner.RunWith

/**
 * The release build on the emulator (D223 item 15; `scripts\verify-android.ps1
 * -Release`, after VaultFirstRunTest made the vault, a host and a session on
 * it): the app is not debuggable, so the test-only answers of the debug build
 * are off, and fingerprint unlock is off by default (D211).
 */
@RunWith(AndroidJUnit4::class)
class ReleaseBuildTest {
    @Test
    fun theReleaseBuildIsNotDebuggableAndFingerprintUnlockIsOff() {
        val context = InstrumentationRegistry.getInstrumentation().targetContext
        assertEquals("not debuggable", 0, context.applicationInfo.flags and ApplicationInfo.FLAG_DEBUGGABLE)
        val app = context.applicationContext as App
        assertFalse("fingerprint unlock off by default", app.unlockPrefs.getBoolean(Fingerprint.PREF_ON, false))
        assertTrue("the version of the first version", BuildConfig.VERSION_NAME.startsWith("1.0.0"))
    }
}
