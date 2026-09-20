package muxlane.session

data class Pairing(
    val relayUrl: String,
    val hostId: String,
    val token: String,
    val machineName: String,
)

interface PairingStore {
    fun load(): Pairing?
    fun save(pairing: Pairing)
    fun clear()
}

class MemoryPairingStore : PairingStore {
    private var value: Pairing? = null
    override fun load(): Pairing? = value
    override fun save(pairing: Pairing) {
        value = pairing
    }
    override fun clear() {
        value = null
    }
}
