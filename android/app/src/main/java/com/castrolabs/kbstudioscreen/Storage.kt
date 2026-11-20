package com.castrolabs.kbstudioscreen

import android.content.Context
import android.os.Build
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import org.json.JSONArray
import org.json.JSONObject
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

data class SavedHost(val name: String, val host: String, val port: Int, val fp: String)

/**
 * Identidad del dispositivo y PCs emparejadas.
 * La clave privada se guarda cifrada con una clave AES-GCM de Android
 * Keystore (no exportable). Nada se respalda en la nube (ver data_extraction_rules).
 */
class Storage(ctx: Context) {
    private val prefs = ctx.getSharedPreferences("kbss", Context.MODE_PRIVATE)

    private fun wrapKey(): SecretKey {
        val ks = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (ks.getKey(ALIAS, null) as? SecretKey)?.let { return it }
        val gen = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore")
        gen.init(
            KeyGenParameterSpec.Builder(ALIAS, KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setKeySize(256)
                .build()
        )
        return gen.generateKey()
    }

    /** [cert, clave] — se generan la primera vez. */
    fun identity(): Pair<ByteArray, ByteArray> {
        val c = prefs.getString("cert", null)
        val k = prefs.getString("key_enc", null)
        val iv = prefs.getString("key_iv", null)
        if (c != null && k != null && iv != null) {
            try {
                val cipher = Cipher.getInstance("AES/GCM/NoPadding")
                cipher.init(Cipher.DECRYPT_MODE, wrapKey(), GCMParameterSpec(128, b64d(iv)))
                return b64d(c) to cipher.doFinal(b64d(k))
            } catch (_: Exception) {
                // Keystore reseteado (p. ej. cambio de bloqueo de pantalla): se regenera.
            }
        }
        val id = NativeCore.generateIdentity()
        val cipher = Cipher.getInstance("AES/GCM/NoPadding")
        cipher.init(Cipher.ENCRYPT_MODE, wrapKey())
        val enc = cipher.doFinal(id[1])
        prefs.edit()
            .putString("cert", b64e(id[0]))
            .putString("key_enc", b64e(enc))
            .putString("key_iv", b64e(cipher.iv))
            .remove("hosts") // identidad nueva => los emparejamientos viejos ya no sirven
            .apply()
        return id[0] to id[1]
    }

    fun hosts(): List<SavedHost> {
        val arr = JSONArray(prefs.getString("hosts", "[]"))
        return (0 until arr.length()).map {
            val o = arr.getJSONObject(it)
            SavedHost(o.getString("name"), o.getString("host"), o.getInt("port"), o.getString("fp"))
        }
    }

    fun saveHost(h: SavedHost) {
        val list = hosts().filter { it.fp != h.fp } + h
        val arr = JSONArray()
        list.forEach { arr.put(JSONObject().put("name", it.name).put("host", it.host).put("port", it.port).put("fp", it.fp)) }
        prefs.edit().putString("hosts", arr.toString()).apply()
    }

    fun forget(fp: String) {
        val arr = JSONArray()
        hosts().filter { it.fp != fp }.forEach { arr.put(JSONObject().put("name", it.name).put("host", it.host).put("port", it.port).put("fp", it.fp)) }
        prefs.edit().putString("hosts", arr.toString()).apply()
    }

    var deviceName: String
        get() = prefs.getString("device_name", null) ?: defaultName()
        set(v) = prefs.edit().putString("device_name", v.take(40)).apply()

    private fun defaultName(): String {
        val m = Build.MODEL ?: "Android"
        val b = Build.MANUFACTURER ?: ""
        return if (m.startsWith(b, ignoreCase = true)) m else "$b $m".trim()
    }

    companion object {
        private const val ALIAS = "kbss_wrap_v1"
        fun b64e(b: ByteArray): String = Base64.encodeToString(b, Base64.NO_WRAP)
        fun b64d(s: String): ByteArray = Base64.decode(s, Base64.NO_WRAP)
    }
}
