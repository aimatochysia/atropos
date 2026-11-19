package io.github.aimatochysia.atropos

import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyPermanentlyInvalidatedException
import android.security.keystore.KeyProperties
import android.security.keystore.StrongBoxUnavailableException
import androidx.biometric.BiometricManager
import androidx.biometric.BiometricManager.Authenticators.BIOMETRIC_STRONG
import androidx.biometric.BiometricPrompt
import androidx.core.content.ContextCompat
import androidx.fragment.app.FragmentActivity
import io.flutter.plugin.common.MethodCall
import io.flutter.plugin.common.MethodChannel.Result
import java.io.File
import java.security.KeyStore
import java.util.Arrays
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * Keeps one 32-byte vault secret per vault, encrypted by an AES-256 key in the
 * Android Keystore (StrongBox when the device has it). The key can only be
 * used right after a strong biometric check (class 3 fingerprint or face) and
 * is destroyed by Android when a new fingerprint or face is enrolled.
 *
 * The secret itself is made by the Rust core and only passes through here.
 * Files live in noBackupFilesDir, so they are never copied to cloud backups.
 */
class BiometricVault(private val activity: FragmentActivity) {
    private val dir = File(activity.noBackupFilesDir, "biometric").apply { mkdirs() }

    private fun alias(vaultId: String) = "atropos-vault-$vaultId"
    private fun file(vaultId: String) = File(dir, "$vaultId.bin")

    fun handle(call: MethodCall, result: Result) {
        val vaultId = call.argument<String>("vaultId")
        if (call.method != "status" && (vaultId == null || !vaultId.matches(Regex("[0-9a-f]{32}")))) {
            result.error("args", "vaultId missing", null)
            return
        }
        when (call.method) {
            "status" -> result.success(status())
            "has" -> result.success(file(vaultId!!).exists())
            "remove" -> {
                remove(vaultId!!)
                result.success(null)
            }
            "enroll" -> {
                val secret = call.argument<ByteArray>("secret")
                if (secret == null || secret.size != 32) {
                    result.error("args", "secret missing", null)
                } else {
                    enroll(vaultId!!, secret, call.argument<String>("title") ?: "Turn on fingerprint unlock", result)
                }
            }
            "unlock" -> unlock(vaultId!!, call.argument<String>("title") ?: "Unlock vault", result)
            else -> result.notImplemented()
        }
    }

    /** "ready", "none_enrolled" (no fingerprint/face set up) or "unavailable". */
    private fun status(): String =
        when (BiometricManager.from(activity).canAuthenticate(BIOMETRIC_STRONG)) {
            BiometricManager.BIOMETRIC_SUCCESS -> "ready"
            BiometricManager.BIOMETRIC_ERROR_NONE_ENROLLED -> "none_enrolled"
            else -> "unavailable"
        }

    private fun remove(vaultId: String) {
        file(vaultId).delete()
        try {
            val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
            ks.deleteEntry(alias(vaultId))
        } catch (_: Exception) {
        }
    }

    private fun newKey(vaultId: String): SecretKey {
        fun spec(strongBox: Boolean): KeyGenParameterSpec {
            val b = KeyGenParameterSpec.Builder(
                alias(vaultId),
                KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
            )
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .setUserAuthenticationRequired(true)
                .setInvalidatedByBiometricEnrollment(true)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
                b.setUserAuthenticationParameters(0, KeyProperties.AUTH_BIOMETRIC_STRONG)
            }
            if (strongBox && Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
                b.setIsStrongBoxBacked(true)
            }
            return b.build()
        }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P) {
            try {
                gen.init(spec(true))
                return gen.generateKey()
            } catch (_: StrongBoxUnavailableException) {
                // No StrongBox chip: use the normal hardware-backed Keystore.
            } catch (_: java.security.ProviderException) {
            }
        }
        gen.init(spec(false))
        return gen.generateKey()
    }

    private fun key(vaultId: String): SecretKey? {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        return ks.getKey(alias(vaultId), null) as SecretKey?
    }

    private fun prompt(
        title: String,
        subtitle: String,
        cipher: Cipher,
        result: Result,
        onCipher: (Cipher) -> Unit,
    ) {
        val info = BiometricPrompt.PromptInfo.Builder()
            .setTitle(title)
            .setSubtitle(subtitle)
            .setNegativeButtonText("Use password")
            .setAllowedAuthenticators(BIOMETRIC_STRONG)
            .setConfirmationRequired(false)
            .build()
        val callback = object : BiometricPrompt.AuthenticationCallback() {
            override fun onAuthenticationSucceeded(r: BiometricPrompt.AuthenticationResult) {
                val c = r.cryptoObject?.cipher
                if (c == null) {
                    result.error("failed", "No cipher after authentication.", null)
                    return
                }
                try {
                    onCipher(c)
                } catch (e: Exception) {
                    result.error("failed", e.message, null)
                }
            }

            override fun onAuthenticationError(code: Int, msg: CharSequence) {
                val cancelled = code == BiometricPrompt.ERROR_NEGATIVE_BUTTON ||
                    code == BiometricPrompt.ERROR_USER_CANCELED ||
                    code == BiometricPrompt.ERROR_CANCELED
                if (cancelled) result.success(null) else result.error("biometric", msg.toString(), null)
            }
        }
        BiometricPrompt(activity, ContextCompat.getMainExecutor(activity), callback)
            .authenticate(info, BiometricPrompt.CryptoObject(cipher))
    }

    /** Asks for a fingerprint, then stores `secret` encrypted. Answers true, or null if cancelled. */
    private fun enroll(vaultId: String, secret: ByteArray, title: String, result: Result) {
        try {
            remove(vaultId)
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.ENCRYPT_MODE, newKey(vaultId))
            prompt(title, "Confirm with your fingerprint or face", cipher, result) { c ->
                try {
                    val ct = c.doFinal(secret)
                    val out = c.iv + ct
                    val tmp = File(dir, "$vaultId.tmp")
                    tmp.writeBytes(out)
                    if (!tmp.renameTo(file(vaultId))) throw IllegalStateException("could not save")
                    result.success(true)
                } finally {
                    Arrays.fill(secret, 0)
                }
            }
        } catch (e: Exception) {
            Arrays.fill(secret, 0)
            remove(vaultId)
            result.error("failed", e.message, null)
        }
    }

    /**
     * Asks for a fingerprint and answers the secret, null if cancelled, or the
     * error "invalidated" when the fingerprints changed and the key is gone.
     */
    private fun unlock(vaultId: String, title: String, result: Result) {
        val f = file(vaultId)
        if (!f.exists()) {
            result.error("missing", "Fingerprint unlock is not set up for this vault.", null)
            return
        }
        try {
            val data = f.readBytes()
            if (data.size < 12 + 16 + 32) throw IllegalStateException("damaged key file")
            val k = key(vaultId) ?: throw KeyPermanentlyInvalidatedException()
            val cipher = Cipher.getInstance("AES/GCM/NoPadding")
            cipher.init(Cipher.DECRYPT_MODE, k, GCMParameterSpec(128, data, 0, 12))
            prompt(title, "Use your fingerprint or face", cipher, result) { c ->
                result.success(c.doFinal(data, 12, data.size - 12))
            }
        } catch (e: KeyPermanentlyInvalidatedException) {
            remove(vaultId)
            result.error(
                "invalidated",
                "The fingerprints or faces on this device changed, so fingerprint unlock was turned off. Unlock with the password and turn it on again.",
                null,
            )
        } catch (e: Exception) {
            result.error("failed", e.message, null)
        }
    }
}
