package com.muxlane.android

import android.content.Context
import android.security.keystore.KeyGenParameterSpec
import android.security.keystore.KeyProperties
import android.util.Base64
import muxlane.session.Pairing
import org.json.JSONArray
import org.json.JSONObject
import java.security.KeyStore
import javax.crypto.Cipher
import javax.crypto.KeyGenerator
import javax.crypto.SecretKey
import javax.crypto.spec.GCMParameterSpec

class PrefsStore(context: Context) {
    private val prefs = context.getSharedPreferences("muxlane", Context.MODE_PRIVATE)

    fun loadAll(): List<Pairing> {
        val stored = prefs.getString(PAIRINGS, null)
        if (stored == null) return migrateLegacy()
        return runCatching {
            val array = JSONArray(stored)
            buildList {
                for (index in 0 until array.length()) {
                    val item = array.getJSONObject(index)
                    val token = runCatching { decrypt(item.getString("token")) }.getOrNull() ?: continue
                    add(
                        Pairing(
                            relayUrl = item.getString("relay_url"),
                            hostId = item.getString("host_id"),
                            token = token,
                            machineName = item.optString("machine_name", item.getString("host_id")),
                        ),
                    )
                }
            }
        }.getOrElse { emptyList() }
    }

    fun save(pairing: Pairing) {
        writeAll(loadAll().filterNot { it.hostId == pairing.hostId } + pairing)
    }

    fun remove(hostId: String) {
        writeAll(loadAll().filterNot { it.hostId == hostId })
        if (activeHostId() == hostId) setActiveHost(null)
    }

    fun clear() {
        prefs.edit().clear().apply()
    }

    fun activeHostId(): String? = prefs.getString(ACTIVE_HOST, null)

    fun setActiveHost(hostId: String?) {
        prefs.edit().apply {
            if (hostId == null) remove(ACTIVE_HOST) else putString(ACTIVE_HOST, hostId)
        }.apply()
    }

    private fun writeAll(pairings: List<Pairing>) {
        val array = JSONArray()
        pairings.forEach { pairing ->
            array.put(
                JSONObject()
                    .put("relay_url", pairing.relayUrl)
                    .put("host_id", pairing.hostId)
                    .put("machine_name", pairing.machineName)
                    .put("token", encrypt(pairing.token)),
            )
        }
        prefs.edit()
            .putString(PAIRINGS, array.toString())
            .remove("relay_url")
            .remove("host_id")
            .remove("machine_name")
            .remove("token")
            .remove("token_enc")
            .apply()
    }

    private fun migrateLegacy(): List<Pairing> {
        val relayUrl = prefs.getString("relay_url", null) ?: return emptyList()
        val hostId = prefs.getString("host_id", null) ?: return emptyList()
        val encrypted = prefs.getString("token_enc", null)
        val token = when {
            encrypted != null -> runCatching { decrypt(encrypted) }.getOrNull()
            else -> prefs.getString("token", null)
        } ?: return emptyList()
        val pairing = Pairing(
            relayUrl = relayUrl,
            hostId = hostId,
            token = token,
            machineName = prefs.getString("machine_name", hostId) ?: hostId,
        )
        writeAll(listOf(pairing))
        setActiveHost(hostId)
        return listOf(pairing)
    }

    private fun encrypt(value: String): String {
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(Cipher.ENCRYPT_MODE, key())
        val iv = Base64.encodeToString(cipher.iv, Base64.NO_WRAP)
        val body = Base64.encodeToString(cipher.doFinal(value.toByteArray()), Base64.NO_WRAP)
        return "$iv.$body"
    }

    private fun decrypt(value: String): String {
        val parts = value.split('.', limit = 2)
        require(parts.size == 2) { "invalid encrypted token" }
        val cipher = Cipher.getInstance(TRANSFORMATION)
        cipher.init(
            Cipher.DECRYPT_MODE,
            key(),
            GCMParameterSpec(128, Base64.decode(parts[0], Base64.NO_WRAP)),
        )
        return cipher.doFinal(Base64.decode(parts[1], Base64.NO_WRAP)).toString(Charsets.UTF_8)
    }

    private fun key(): SecretKey {
        val store = KeyStore.getInstance("AndroidKeyStore").apply { load(null) }
        (store.getKey(KEY_ALIAS, null) as? SecretKey)?.let { return it }
        return KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore").run {
            init(
                KeyGenParameterSpec.Builder(
                    KEY_ALIAS,
                    KeyProperties.PURPOSE_ENCRYPT or KeyProperties.PURPOSE_DECRYPT,
                )
                    .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                    .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                    .build(),
            )
            generateKey()
        }
    }

    private companion object {
        const val PAIRINGS = "pairings_v2"
        const val ACTIVE_HOST = "active_host_id"
        const val KEY_ALIAS = "muxlane-pairing-token"
        const val TRANSFORMATION = "AES/GCM/NoPadding"
    }
}
