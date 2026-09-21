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
        assertEquals(0xFFA3333F.toInt(), vt.screen()[0][0].fg)
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

    @Test
    fun viewportCanReadScrollback() {
        val vt = VirtualTerminal(4, 2)
        vt.write("one\ntwo\nthree".toByteArray())
        assertTrue(vt.maxScrollRows > 0)
        val oldest = vt.viewport(vt.maxScrollRows).first().joinToString("") { it.ch }
        assertTrue(oldest.contains("one"))
    }

    @Test
    fun iso2022CharsetDoesNotPrintB() {
        val vt = VirtualTerminal(20, 2)
        vt.write("ok\u001b(Btext".toByteArray())
        assertEquals("oktext", vt.visibleLine(0))
    }

    @Test
    fun kittyApcAndPlaceholderAreIgnored() {
        val vt = VirtualTerminal(20, 2)
        vt.write("A\u001b_Ga=T,f=100,i=1;xxxx\u001b\\B".toByteArray())
        assertEquals("AB", vt.visibleLine(0))
        vt.reset()
        vt.write(byteArrayOf(0x41, 0xF4.toByte(), 0x8E.toByte(), 0xBB.toByte(), 0xAE.toByte(), 0x42))
        assertEquals("AB", vt.visibleLine(0))
    }

    @Test
    fun supplementaryUnicodeGlyphsArePreserved() {
        val vt = VirtualTerminal(20, 2)
        vt.write("📁⚡".toByteArray())
        assertEquals("📁 ⚡", vt.visibleLine(0))
    }
}
