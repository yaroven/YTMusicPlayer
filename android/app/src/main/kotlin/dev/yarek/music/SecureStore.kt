package dev.yarek.music

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import org.json.JSONObject
import java.io.File
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

/**
 * The youtube.com cookies, encrypted with an AES key that never leaves the
 * Android Keystore (the iPhone app keeps them in the Keychain). Backups are
 * off, so they stay on this device.
 */
class SecureStore(context: Context) {
    private val file = File(context.noBackupFilesDir, "youtube-session")

    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getEntry(ALIAS, null) as? KeyStore.SecretKeyEntry)?.let { return it.secretKey }
        val generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        generator.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .build(),
        )
        return generator.generateKey()
    }

    fun cookies(): Map<String, String> = runCatching {
        val (iv, data) = file.readText().split(':').map { Base64.decode(it, Base64.NO_WRAP) }
        val cipher = Cipher.getInstance(TRANSFORM)
        cipher.init(Cipher.DECRYPT_MODE, key(), GCMParameterSpec(128, iv))
        val o = JSONObject(cipher.doFinal(data).toString(Charsets.UTF_8))
        o.keys().asSequence().associateWith { o.getString(it) }
    }.getOrDefault(emptyMap())

    @Synchronized
    fun save(cookies: Map<String, String>) {
        runCatching {
            val cipher = Cipher.getInstance(TRANSFORM)
            cipher.init(Cipher.ENCRYPT_MODE, key())
            val data = cipher.doFinal(JSONObject(cookies).toString().toByteArray())
            val text = Base64.encodeToString(cipher.iv, Base64.NO_WRAP) + ":" + Base64.encodeToString(data, Base64.NO_WRAP)
            val tmp = File(file.parentFile, file.name + ".tmp")
            tmp.writeText(text)
            tmp.renameTo(file)
        }
    }

    fun delete() {
        file.delete()
    }

    private companion object {
        const val ALIAS = "youtube-session"
        const val TRANSFORM = "AES/GCM/NoPadding"
    }
}
