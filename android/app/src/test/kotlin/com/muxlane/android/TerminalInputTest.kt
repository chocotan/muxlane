package com.muxlane.android

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertNull

class TerminalInputTest {
    @Test
    fun convertsImeEditsToTerminalInput() {
        assertEquals("hello", terminalInputDelta("", "hello"))
        assertEquals("\u007f", terminalInputDelta("hello", "hell"))
        assertEquals("\r", terminalInputDelta("hello", "hello\n"))
        assertNull(terminalInputDelta("same", "same"))
    }

    @Test
    fun appliesControlAndAltModifiers() {
        assertEquals("\u0003", applyTerminalModifiers("C", ctrl = true, alt = false))
        assertEquals("\u001bx", applyTerminalModifiers("x", ctrl = false, alt = true))
    }

    @Test
    fun encodesTmuxWheelReports() {
        assertEquals(listOf(0x1B, 0x5B, 0x4D, 0x60, 0x28, 0x29), tmuxWheelReport(true, 7, 8).map { it.toInt() and 0xFF })
        assertEquals(listOf(0x1B, 0x5B, 0x4D, 0x61, 0x21, 0x21), tmuxWheelReport(false, -1, -1).map { it.toInt() and 0xFF })
    }
}
