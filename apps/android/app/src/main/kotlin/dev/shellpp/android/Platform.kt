package dev.shellpp.android

import android.content.ClipData
import android.content.ClipDescription
import android.content.ClipboardManager
import android.content.Context
import android.content.Intent
import android.net.Uri
import android.os.Build
import android.os.PersistableBundle

/**
 * The core's platform services on Android (apps/android/native/src/android_platform.rs
 * calls these methods from Rust threads): the device key in the Keystore
 * ([DeviceKeys], D218), the clipboard and the browser.
 */
class Platform(private val context: Context, val deviceKeys: DeviceKeys) {
    /** The app has a window with focus: Android 10+ reads the clipboard only then. */
    @Volatile
    var focused = false

    private val clipboard: ClipboardManager
        get() = context.getSystemService(ClipboardManager::class.java)

    fun secretAvailable(): Boolean = deviceKeys.available()

    /** 0: no key, 1: the key is in [out], 2: the fingerprint prompt was not passed. */
    fun secretGet(account: String, out: ByteArray): Int = deviceKeys.read(account, out)

    fun secretSet(account: String, key: ByteArray) {
        try {
            deviceKeys.write(account, key)
        } finally {
            key.fill(0)
        }
    }

    fun secretDelete(account: String) = deviceKeys.delete(account)

    /** Throws without focus, so the core does not take an empty read for "cleared" (architecture 4). */
    fun clipboardRead(): String? {
        check(focused) { "no focus" }
        val clip = clipboard.primaryClip ?: return null
        if (clip.itemCount == 0) return null
        return clip.getItemAt(0).coerceToText(context)?.toString()
    }

    fun clipboardWrite(text: String) {
        clipboard.setPrimaryClip(ClipData.newPlainText("Shell++", text))
    }

    fun clipboardWriteSensitive(text: ByteArray) {
        val clip = ClipData.newPlainText("Shell++", String(text, Charsets.UTF_8))
        if (Build.VERSION.SDK_INT >= 33) {
            clip.description.extras = PersistableBundle().apply {
                putBoolean(ClipDescription.EXTRA_IS_SENSITIVE, true)
            }
        }
        clipboard.setPrimaryClip(clip)
    }

    fun clipboardClear() {
        if (Build.VERSION.SDK_INT >= 28) clipboard.clearPrimaryClip()
    }

    fun openUrl(url: String) {
        context.startActivity(Intent(Intent.ACTION_VIEW, Uri.parse(url)).addFlags(Intent.FLAG_ACTIVITY_NEW_TASK))
    }
}
