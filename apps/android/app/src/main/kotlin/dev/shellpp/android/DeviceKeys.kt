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

    /** A strong biometric is enrolled and usable. */
    fun available(): Boolean {
        if (Build.VERSION.SDK_INT < 30) return false
        val manager = context.getSystemService(BiometricManager::class.java) ?: return false
        return try {
            manager.canAuthenticate(BiometricManager.Authenticators.BIOMETRIC_STRONG) ==
                BiometricManager.BIOMETRIC_SUCCESS
        } catch (error: SecurityException) {
            false
        }
    }

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

    /**
     * The cipher for the biometric prompt that reads a key, or null when the
     * Keystore key is gone or became invalid (a fingerprint was added).
     */
    fun decryptCipher(): Cipher? {
        val key = try {
            privateKey()
        } catch (error: Exception) {
            null
        } ?: return null
        return try {
            Cipher.getInstance(TRANSFORMATION).apply { init(Cipher.DECRYPT_MODE, key, OAEP) }
        } catch (error: KeyPermanentlyInvalidatedException) {
            null
        }
    }

    /** The Keystore key exists but became invalid (a fingerprint was added). */
    fun invalidated(): Boolean {
        if (privateKey() == null) return false
        return decryptCipher() == null
    }

    /**
     * After the prompt passed: decrypts the item of [account] with the
     * authorized [cipher] and holds it for the step that asked.
     */
    fun hold(account: String, cipher: Cipher) {
        val plain = cipher.doFinal(file(account).readBytes())
        synchronized(held) {
            held.put(account, plain)?.fill(0)
        }
    }

    /** Wipes every held key (the step is over). */
    fun release() {
        synchronized(held) {
            for (key in held.values) key.fill(0)
            held.clear()
        }
    }

    /**
     * The core's read ([Platform.secretGet]): the held key, else 0 (no
     * usable key: none stored, or the Keystore key is invalid), else 2 (the
     * prompt was not passed for this step).
     */
    fun read(account: String, out: ByteArray): Int {
        synchronized(held) {
            held[account]?.let { key ->
                key.copyInto(out)
                return 1
            }
        }
        if (!has(account)) return 0
        if (decryptCipher() == null) return 0
        return 2
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
        if (privateKey() == null || invalidated()) {
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
