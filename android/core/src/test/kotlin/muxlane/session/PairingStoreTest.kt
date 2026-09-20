package muxlane.session

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class PairingStoreTest {
    @Test
    fun roundTripAndClear() {
        val store = MemoryPairingStore()
        assertNull(store.load())
        store.save(Pairing("ws://127.0.0.1:9843", "machine_1", "v1:token", "box"))
        assertEquals("machine_1", store.load()?.hostId)
        store.clear()
        assertNull(store.load())
    }
}
