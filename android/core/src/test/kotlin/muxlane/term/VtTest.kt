package muxlane.term

import kotlin.test.Test
import kotlin.test.assertEquals
import kotlin.test.assertTrue

class VtTest {
    @Test
    fun sgrColorsAndText() {
        val vt = VirtualTerminal(20, 4)
        vt.write("\u001b[31mred\u001b[0m plain".toByteArray())
        assertEquals("red plain", vt.visibleLine(0))
        assertEquals(0xFFE06C75.toInt(), vt.screen()[0][0].fg)
    }

    @Test
    fun trueColorAndIndexed() {
        val vt = VirtualTerminal(10, 2)
        vt.write("\u001b[38;2;1;2;3mA\u001b[38;5;196mB".toByteArray())
        assertEquals(0xFF010203.toInt(), vt.screen()[0][0].fg)
        assertTrue(vt.screen()[0][1].fg != DEFAULT_FG)
    }

    @Test
    fun cjkWideAndNewline() {
        val vt = VirtualTerminal(4, 2)
        vt.write("你\n好".toByteArray(Charsets.UTF_8))
        assertEquals("你", vt.visibleLine(0).trim())
        assertEquals("好", vt.visibleLine(1).trim())
    }

    @Test
    fun resetClearsScreen() {
        val vt = VirtualTerminal(8, 2)
        vt.write("hello".toByteArray())
        vt.reset()
        assertEquals("", vt.visibleLine(0))
        assertEquals(0, vt.cursor.col)
    }
}
