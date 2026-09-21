package com.muxlane.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith

class RelayUrlTest {
    @Test
    fun publicRelayRequiresTls() {
        assertFailsWith<IllegalArgumentException> { normalizeRelayUrl("ws://relay.example.com") }
        assertEquals("wss://relay.example.com", normalizeRelayUrl(" wss://relay.example.com/ "))
    }

    @Test
    fun localRelayMayUseCleartext() {
        assertEquals("ws://192.168.1.8:9843", normalizeRelayUrl("ws://192.168.1.8:9843"))
        assertEquals("ws://muxlane.local:9843", normalizeRelayUrl("ws://muxlane.local:9843"))
    }
}
