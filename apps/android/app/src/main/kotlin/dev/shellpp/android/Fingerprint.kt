package dev.shellpp.android

import android.app.Activity
import android.hardware.biometrics.BiometricManager
import android.hardware.biometrics.BiometricPrompt
import android.os.Build
import android.os.CancellationSignal
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.suspendCancellableCoroutine
import kotlinx.coroutines.withContext
import javax.crypto.Cipher
import kotlin.coroutines.resume

/** What the biometric prompt ended with. */
sealed interface PromptResult {
    data class Passed(val cipher: Cipher) : PromptResult

    /** "Use password", a cancel or a back gesture. */
    data object Cancelled : PromptResult

    /** The system locked biometrics out after failed attempts: the password is asked. */
    data object LockedOut : PromptResult

    data class Failed(val message: String) : PromptResult
}

/**
 * Fingerprint unlock (D218, architecture 4, UI spec 5 and 12): the system's
 * biometric prompt with the cipher of the device key's Keystore key, and the
 * steps that read the device key behind it.
 */
object Fingerprint {
    const val PREF_ON = "fingerprint"
    const val PREF_LAST_PASSWORD = "last_password_ms"
    const val PREF_OFF_REASON = "fingerprint_off_reason"
    const val REASON_NEW_FINGERPRINT = "new_fingerprint"
    const val REASON_NONE_ENROLLED = "none_enrolled"
    const val REASON_KEY_GONE = "key_gone"
    const val REASON_FAILED = "failed"

    /** The master password is asked once a week at least (D218). */
    const val PASSWORD_EVERY_MS = 7L * 24 * 60 * 60 * 1000

    suspend fun prompt(activity: Activity, title: String, cipher: Cipher): PromptResult {
        if (Build.VERSION.SDK_INT < 30) return PromptResult.Failed("needs Android 11")
        return suspendCancellableCoroutine { continuation ->
            val cancel = CancellationSignal()
            val prompt = BiometricPrompt.Builder(activity)
                .setTitle(title)
                .setAllowedAuthenticators(BiometricManager.Authenticators.BIOMETRIC_STRONG)
                .setNegativeButton(activity.getString(R.string.use_password), activity.mainExecutor) { _, _ ->
                    if (continuation.isActive) continuation.resume(PromptResult.Cancelled)
                }
                .build()
            continuation.invokeOnCancellation { cancel.cancel() }
            prompt.authenticate(
                BiometricPrompt.CryptoObject(cipher),
                cancel,
                activity.mainExecutor,
                object : BiometricPrompt.AuthenticationCallback() {
                    override fun onAuthenticationSucceeded(result: BiometricPrompt.AuthenticationResult) {
                        val passed = result.cryptoObject?.cipher
                        if (!continuation.isActive) return
                        continuation.resume(if (passed != null) PromptResult.Passed(passed) else PromptResult.Failed(""))
                    }

                    override fun onAuthenticationError(code: Int, message: CharSequence) {
                        if (!continuation.isActive) return
                        continuation.resume(
                            when (code) {
                                BiometricPrompt.BIOMETRIC_ERROR_LOCKOUT,
                                BiometricPrompt.BIOMETRIC_ERROR_LOCKOUT_PERMANENT,
                                -> PromptResult.LockedOut
                                BiometricPrompt.BIOMETRIC_ERROR_USER_CANCELED,
                                BiometricPrompt.BIOMETRIC_ERROR_CANCELED,
                                -> PromptResult.Cancelled
                                else -> PromptResult.Failed(message.toString())
                            },
                        )
                    }
                },
            )
        }
    }

    /** Fingerprint unlock is on and may be offered now, or why not (null: offer it). */
    fun blocker(app: App, lock: LockStatus): String? {
        val prefs = app.unlockPrefs
        if (!prefs.getBoolean(PREF_ON, false)) return "off"
        val keys = app.platform.deviceKeys
        return try {
            when {
                keys.sensor() == DeviceKeys.Sensor.None -> REASON_NONE_ENROLLED
                keys.sensor() == DeviceKeys.Sensor.Busy -> "busy"
                keys.keyState() == DeviceKeys.KeyState.Invalidated || keys.wasInvalidated() -> REASON_NEW_FINGERPRINT
                keys.keyState() == DeviceKeys.KeyState.Missing -> REASON_KEY_GONE
                keys.keyState() == DeviceKeys.KeyState.Unknown -> "busy"
                !lock.rememberEnabled || keys.storedAccount() == null -> "refresh"
                passwordDue(prefs.getLong(PREF_LAST_PASSWORD, 0L), System.currentTimeMillis()) -> "weekly"
                else -> null
            }
        } catch (error: Exception) {
            "busy"
        }
    }

    /**
     * The weekly master password is due: a week since the last one, or a
     * clock that went back past it (D218; review of run 14).
     */
    fun passwordDue(last: Long, now: Long): Boolean = now < last || now - last > PASSWORD_EVERY_MS

    /**
     * Holds the device key behind the prompt for [step] (an unlock, a
     * sign-in, a recovery, a key change), then wipes it. A prompt not passed
     * runs [step] without the key: the core keeps the item (D218).
     */
    suspend fun <T> withDeviceKey(activity: Activity, app: App, title: String, step: () -> T): Pair<PromptResult, T> {
        val keys = app.platform.deviceKeys
        val account = keys.storedAccount()
        val cipher = keys.decryptCipher()
        val result = try {
            if (account != null && cipher != null) prompt(activity, title, cipher) else PromptResult.Cancelled
        } catch (error: Exception) {
            PromptResult.Failed(error.message ?: "The fingerprint prompt did not open.")
        }
        return withContext(Dispatchers.IO) {
            try {
                // A key that cannot be read after a passed prompt (a damaged
                // item, a Keystore error): the step goes on without it.
                val read = result is PromptResult.Passed && account != null && keys.hold(account, result.cipher)
                val outcome = if (result is PromptResult.Passed && !read) PromptResult.Failed("The fingerprint key could not be read.") else result
                outcome to step()
            } finally {
                keys.release()
            }
        }
    }

    /**
     * After an unlock with the master password: the weekly count starts
     * again, and fingerprint unlock is kept working: turned off with its
     * reason when its Keystore key is gone, or given a new device key when
     * the vault's key changed while the prompt was not passed (D218).
     */
    fun afterPasswordUnlock(app: App) {
        val prefs = app.unlockPrefs
        prefs.edit().putLong(PREF_LAST_PASSWORD, System.currentTimeMillis()).apply()
        if (!prefs.getBoolean(PREF_ON, false)) return
        val keys = app.platform.deviceKeys
        // Only a key gone for good turns the option off (D218): a sensor busy
        // for a moment or a Keystore that did not answer changes nothing.
        val reason = try {
            when {
                keys.sensor() == DeviceKeys.Sensor.None -> REASON_NONE_ENROLLED
                keys.keyState() == DeviceKeys.KeyState.Invalidated || keys.wasInvalidated() -> REASON_NEW_FINGERPRINT
                keys.keyState() == DeviceKeys.KeyState.Missing -> REASON_KEY_GONE
                keys.sensor() == DeviceKeys.Sensor.Busy || keys.keyState() == DeviceKeys.KeyState.Unknown -> return
                else -> null
            }
        } catch (error: Exception) {
            return
        }
        try {
            if (reason != null) {
                app.setLock(LockStatus.from(Core.obj("remember_device_set", org.json.JSONObject().put("enabled", false))))
                prefs.edit().putBoolean(PREF_ON, false).putString(PREF_OFF_REASON, reason).apply()
            } else if (!Core.lockStatus().rememberEnabled) {
                app.setLock(LockStatus.from(Core.obj("remember_device_set", org.json.JSONObject().put("enabled", true))))
            }
        } catch (error: Exception) {
            // The core refused (a store error): the option stays as it is and
            // the next unlock tries again.
        }
    }
}
