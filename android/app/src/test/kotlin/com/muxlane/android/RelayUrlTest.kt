package com.muxlane.android

import com.google.zxing.BarcodeFormat
import com.google.zxing.BinaryBitmap
import com.google.zxing.RGBLuminanceSource
import com.google.zxing.common.HybridBinarizer
import com.google.zxing.qrcode.QRCodeReader
import com.google.zxing.qrcode.QRCodeWriter
import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertFailsWith
import kotlin.test.assertNull

class RelayUrlTest {
    @Test
    fun embeddedDecoderReadsPairingQrAndRejectsUnrelatedCodes() {
        val text = "muxlane://pair?relay=ws%3A%2F%2F192.168.1.8%3A9843&id=machine_01ABC"
        val bits = QRCodeWriter().encode(text, BarcodeFormat.QR_CODE, 256, 256)
        val pixels = IntArray(bits.width * bits.height) { index ->
            if (bits[index % bits.width, index / bits.width]) 0xFF000000.toInt() else 0xFFFFFFFF.toInt()
        }
        val source = RGBLuminanceSource(bits.width, bits.height, pixels)
        val decoded = QRCodeReader().decode(BinaryBitmap(HybridBinarizer(source))).text
        assertEquals("ws://192.168.1.8:9843" to "machine_01ABC", parsePairQr(decoded))
        assertEquals("ws://192.168.1.8:9843" to "machine_01ABC", parsePairQr(text.replace("id=", "host=")))
        for (invalid in listOf(
            "https://example.com",
            "muxlane://pair?relay=ws%3A%2F%2F192.168.1.8%3A9843",
            "muxlane://pair?relay=ws%3A%2F%2Frelay.example.com&id=machine_01ABC",
            text.replace("machine_01ABC", "machine%2Fother"),
            text.replace("machine_01ABC", "x".repeat(65)),
        )) assertNull(parsePairQr(invalid), invalid)
    }

    @Test
    fun pairingLinkDecodesQrPayloadAndLegacyHost() {
        val relay = "wss://relay.example.com"
        assertEquals(
            relay to "machine_01ABC",
            parsePairLink("muxlane://pair?relay=wss%3A%2F%2Frelay.example.com&id=machine_01ABC"),
        )
        for (id in listOf("host=machine_01ABC", "id=&host=machine_01ABC")) {
            assertEquals(relay to "machine_01ABC", parsePairLink("muxlane://pair?relay=$relay&$id"))
        }
        assertEquals(
            "ws://muxlane.local:9843/a+b" to "机器_01",
            parsePairLink("muxlane://pair?relay=ws%3A%2F%2Fmuxlane.local%3A9843%2Fa%2Bb&id=%E6%9C%BA%E5%99%A8_01"),
        )
        assertNull(parsePairLink("https://pair?relay=$relay&id=machine_01ABC"))
        assertNull(parsePairLink("muxlane://other?relay=$relay&id=machine_01ABC"))
        assertNull(parsePairLink("muxlane://pair?relay=%ZZ&id=machine_01ABC"))
    }

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
