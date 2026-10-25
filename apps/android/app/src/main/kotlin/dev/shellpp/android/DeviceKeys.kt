package dev.shellpp.android

import android.content.Context
import android.hardware.biometrics.BiometricManager
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import java.io.File
import java.security.KeyFactory
import java.security.KeyPairGenerator
import java.security.KeyStore
import java.security.PrivateKey
import java.security.spec.MGF1ParameterSpec
import java.security.spec.X509EncodedKeySpec
import javax.crypto.Cipher
import javax.crypto.spec.OAEPParameterSpec
import javax.crypto.spec.PSource

/**
 * The device key of fingerprint unlock in the Android Keystore (D218,
 * architecture 4): each 32-byte key the core stores is encrypted with the
 * public half of an RSA key pair whose private half needs a strong
 * biometric for every use and becomes invalid when a fingerprint is added
 * (StrongBox where the phone has one). Encrypting needs no prompt; reading
 * needs the [android.hardware.biometrics.BiometricPrompt] with
 * [decryptCipher], after which the app [hold]s the key for the one step
 * that asked and [release]s it.
 *
 * Fingerprint unlock needs Android 11 (API 30) for strong biometrics only
 * [decision]; below it the option is not offered.
 */
class DeviceKeys(private val context: Context) {
    private val dir = File(context.noBackupFilesDir, "device-keys")
    private val held = HashMap<String, ByteArray>()

    /** What the system says about strong biometrics now. */
    enum class Sensor {
        /** Enrolled and usable. */
        Ready,

        /** Enrolled, but not usable this moment (the sensor busy, an update due): nothing is lost. */
        Busy,

        /** No fingerprint enrolled, no sensor, or Android older than 11. */
        None,
    }

    fun sensor(): Sensor {
        if (Build.VERSION.SDK_INT < 30) return Sensor.None
        val manager = context.getSystemService(BiometricManager::class.java) ?: return Sensor.None
        val answer = try {
            manager.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG)
        } catch (error: SecurityException) {
            return Sensor.None
        }
        return when (answer) {
            BiometricManager.BIOMETRIC_SUCCESS -> Sensor.Ready
            BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED,
            BiometricManager.BIOMETRIC_ERROR_NO_HARDWARE,
            -> Sensor.None
            else -> Sensor.Busy
        }
    }

    /** A strong biometric is enrolled and usable now. */
    fun available(): Boolean = sensor() == Sensor.Ready

    /**
     * The core's `SecretStore` availability: a sensor that is only busy
     * counts as there, so the core never takes a moment of it for a lost key
     * (review of run 14).
     */
    fun storeAvailable(): Boolean = sensor() != Sensor.None

    private fun file(account: String) = File(dir, account.replace(Regex("[^A-Za-z0-9-]"), "_") + ".bin")

    /** The stored item of [account], if any. */
    fun has(account: String): Boolean = file(account).isFile

    /** The account of the one stored item (one vault per app), if any. */
    fun storedAccount(): String? = dir.listFiles()?.firstOrNull { it.name.endsWith(".bin") }?.let { stored ->
        storedAccounts[stored.name]
    }

    /** Account names by file name, kept beside the items. */
    private val storedAccounts: Map<String, String>
        get() = File(dir, "accounts").takeIf { it.isFile }?.readLines()
            ?.mapNotNull { line -> line.split('\t').takeIf { it.size == 2 }?.let { it[0] to it[1] } }
            ?.toMap() ?: emptyMap()

    private fun keyStore(): KeyStore = KeyStore.getInstance(KEYSTORE).apply { load(null) }

    private fun privateKey(): PrivateKey? = keyStore().getKey(ALIAS, null) as? PrivateKey

    /** The Keystore key as the prompt would use it. */
    enum class KeyState {
        /** Usable after a passed prompt. */
        Usable,

        /** Became invalid: a fingerprint was added (or all were removed). */
        Invalidated,

        /** The Keystore has no such key (the screen lock was removed, data cleared). */
        Missing,

        /** The Keystore did not answer this time: nothing is concluded. */
        Unknown,
    }

    private fun cipherOrState(): Pair<Cipher?, KeyState> {
        val key = try {
            privateKey()
        } catch (error: Exception) {
            return null to KeyState.Unknown
        } ?: return null to KeyState.Missing
        return try {
            Cipher.getInstance(TRANSFORMATION).apply { init(Cipher.DECRYPT_MODE, key, OAEP) } to KeyState.Usable
        } catch (error: KeyPermanentlyInvalidatedException) {
            null to KeyState.Invalidated
        } catch (error: Exception) {
            null to KeyState.Unknown
        }
    }

    fun keyState(): KeyState = cipherOrState().second

    /** The cipher for the biometric prompt that reads a key, or null when the key cannot be used now. */
    fun decryptCipher(): Cipher? = cipherOrState().first

    /** The Keystore key is gone for good (invalid or missing): its items can never be read. */
    fun lost(): Boolean = keyState().let { it == KeyState.Invalidated || it == KeyState.Missing }

    /**
     * After the prompt passed: decrypts the item of [account] with the
     * authorized [cipher] and holds it for the step that asked. False when
     * it could not be read (a damaged item, a Keystore error): the step goes
     * on without the key, and the password is asked.
     */
    fun hold(account: String, cipher: Cipher): Boolean {
        val plain = try {
            cipher.doFinal(file(account).readBytes())
        } catch (error: Exception) {
            return false
        }
        if (plain.size != 32) {
            plain.fill(0)
            return false
        }
        synchronized(held) {
            held.put(account, plain)?.fill(0)
        }
        return true
    }

    /** Wipes every held key (the step is over). */
    fun release() {
        synchronized(held) {
            for (key in held.values) key.fill(0)
            held.clear()
        }
    }

    /**
     * The core's read ([Platform.secretGet]): the held key, else 0 only when
     * the key is gone for good (no item, or its Keystore key invalid or
     * missing), else 2: the prompt was not passed for this step, or the
     * phone could not tell now. A key is never declared lost by a moment's
     * failure (D218; review of run 14).
     */
    fun read(account: String, out: ByteArray): Int {
        synchronized(held) {
            held[account]?.let { key ->
                key.copyInto(out)
                return 1
            }
        }
        return try {
            if (!has(account) || lost()) 0 else 2
        } catch (error: Exception) {
            2
        }
    }

    /** Encrypts [key] for [account] with the public key (no prompt). */
    fun write(account: String, key: ByteArray) {
        val public = publicKey()
        val cipher = Cipher.getInstance(TRANSFORMATION).apply { init(Cipher.ENCRYPT_MODE, public, OAEP) }
        val sealed = cipher.doFinal(key)
        dir.mkdirs()
        val target = file(account)
        val temporary = File(dir, target.name + ".tmp")
        temporary.writeBytes(sealed)
        if (!temporary.renameTo(target)) {
            target.delete()
            check(temporary.renameTo(target)) { "device key not stored" }
        }
        val accounts = storedAccounts.toMutableMap()
        accounts[target.name] = account
        File(dir, "accounts").writeText(accounts.entries.joinToString("") { "${it.key}\t${it.value}\n" })
        synchronized(held) { held.remove(account)?.fill(0) }
    }

    /** Deletes the item of [account]; with no item left, the Keystore key pair too. */
    fun delete(account: String) {
        file(account).delete()
        synchronized(held) { held.remove(account)?.fill(0) }
        if (dir.listFiles()?.none { it.name.endsWith(".bin") } != false) {
            File(dir, "accounts").delete()
            try {
                keyStore().deleteEntry(ALIAS)
            } catch (error: Exception) {
                // Nothing to delete.
            }
        }
    }

    /**
     * The public key, made with a new key pair when there is none or the
     * old one became invalid (its items cannot be read any more).
     */
    private fun publicKey(): java.security.PublicKey {
        if (lost()) {
            try {
                keyStore().deleteEntry(ALIAS)
            } catch (error: Exception) {
                // Nothing to delete.
            }
            generate()
        }
        val certificate = keyStore().getCertificate(ALIAS) ?: error("no device key pair")
        // A copy outside the Keystore: encrypting with it needs no
        // authentication (only the private key is bound to the fingerprint).
        return KeyFactory.getInstance(certificate.publicKey.algorithm)
            .generatePublic(X509EncodedKeySpec(certificate.publicKey.encoded))
    }

    private fun generate() {
        check(Build.VERSION.SDK_INT >= 30) { "fingerprint unlock needs Android 11" }
        fun spec(strongBox: Boolean) = KeyGenParameterSpec.Builder(
            ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
        )
            .setKeySize(2048)
            .setDigests(KeyProperties.DIGEST_SHA256)
            .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_RSA_OAEP)
            .setUserAuthenticationRequired(true)
            .setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
            .setInvalidatedByBiometricEnrollment(true)
            .setIsStrongBoxBacked(strongBox)
            .build()
        val generator = KeyPairGenerator.getInstance(KeyProperties.KEY_ALGORITHM_RSA, KEYSTORE)
        try {
            generator.initialize(spec(strongBox = true))
            generator.generateKeyPair()
        } catch (error: java.security.ProviderException) {
            // No StrongBox (StrongBoxUnavailableException) or one that does
            // not take this key: the phone's other secure hardware.
            generator.initialize(spec(strongBox = false))
            generator.generateKeyPair()
        }
    }

    companion object {
        private const val KEYSTORE = "AndroidKeyStore"
        private const val ALIAS = "shellpp-device-key"
        private const val TRANSFORMATION = "RSA/ECB/OAEPWithSHA-256AndMGF1Padding"

        /** OAEP with SHA-256 and MGF1 with SHA-1, which every Keystore takes. */
        private val OAEP = OAEPParameterSpec("SHA-256", "MGF1", MGF1ParameterSpec.SHA1, PSource.PSpecified.DEFAULT)
    }
}
