package com.muxlane.android

import android.content.Context
import muxlane.session.Pairing
import muxlane.session.PairingStore

class PrefsStore(context: Context) : PairingStore {
    private val prefs = context.getSharedPreferences("muxlane", Context.MODE_PRIVATE)

    override fun load(): Pairing? {
        val url = prefs.getString("relay_url", null) ?: return null
        val host = prefs.getString("host_id", null) ?: return null
        val token = prefs.getString("token", null) ?: return null
        val name = prefs.getString("machine_name", host) ?: host
        return Pairing(url, host, token, name)
    }

    override fun save(pairing: Pairing) {
        prefs.edit()
            .putString("relay_url", pairing.relayUrl)
            .putString("host_id", pairing.hostId)
            .putString("token", pairing.token)
            .putString("machine_name", pairing.machineName)
            .apply()
    }

    override fun clear() {
        prefs.edit().clear().apply()
    }
}
