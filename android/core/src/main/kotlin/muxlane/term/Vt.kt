package muxlane.term

data class Cell(
    val ch: String = " ",
    val fg: Int = DEFAULT_FG,
    val bg: Int = DEFAULT_BG,
    val bold: Boolean = false,
    val underline: Boolean = false,
    val inverse: Boolean = false,
)

data class Cursor(var col: Int = 0, var row: Int = 0)

const val DEFAULT_FG: Int = 0xFF39342C.toInt()
const val DEFAULT_BG: Int = 0xFFF4EBDD.toInt()

private val ANSI16 = intArrayOf(
    0xFF2F2B25.toInt(), 0xFFA3333F.toInt(), 0xFF356A42.toInt(), 0xFF7A5B17.toInt(),
    0xFF355E8A.toInt(), 0xFF754D73.toInt(), 0xFF2C6868.toInt(), 0xFF5C554B.toInt(),
    0xFF72695D.toInt(), 0xFFA92F3A.toInt(), 0xFF2F7042.toInt(), 0xFF745711.toInt(),
    0xFF2F6098.toInt(), 0xFF7B4778.toInt(), 0xFF246A70.toInt(), 0xFF49433A.toInt(),
)

class VirtualTerminal(
    var cols: Int = 80,
    var rows: Int = 24,
) {
    var cursor = Cursor()
        private set
    var scrollback = ArrayDeque<List<Cell>>()
        private set
    private var grid = Array(rows) { Array(cols) { Cell() } }
    private var fg = DEFAULT_FG
    private var bg = DEFAULT_BG
    private var bold = false
    private var underline = false
    private var inverse = false
    private var parser = AnsiParser()
    val maxScrollback: Int = 1000

    fun screen(): Array<Array<Cell>> = Array(rows) { r -> grid[r].copyOf() }

    fun viewport(scrollRows: Int = 0): List<List<Cell>> {
        val offset = scrollRows.coerceIn(0, scrollback.size)
        val history = scrollback.toList()
        val all = history + grid.map { it.asList() }
        val end = (all.size - offset).coerceAtLeast(rows)
        val start = (end - rows).coerceAtLeast(0)
        return all.subList(start, end)
    }

    val maxScrollRows: Int
        get() = scrollback.size

    fun visibleLine(row: Int): String = grid[row].joinToString("") { it.ch }.trimEnd()

    fun resize(newCols: Int, newRows: Int) {
        val next = Array(newRows) { r ->
            Array(newCols) { c ->
                if (r < rows && c < cols) grid[r][c] else Cell()
            }
        }
        cols = newCols
        rows = newRows
        grid = next
        cursor.col = cursor.col.coerceIn(0, cols - 1)
        cursor.row = cursor.row.coerceIn(0, rows - 1)
    }

    fun reset() {
        grid = Array(rows) { Array(cols) { Cell() } }
        cursor = Cursor()
        fg = DEFAULT_FG
        bg = DEFAULT_BG
        bold = false
        underline = false
        inverse = false
        parser = AnsiParser()
        scrollback.clear()
    }

    fun write(bytes: ByteArray) {
        parser.feed(bytes) { action ->
            when (action) {
                is Action.Print -> printChar(action.ch)
                is Action.Execute -> execute(action.b)
                is Action.Csi -> csi(action)
                is Action.Osc -> {}
                is Action.Esc -> {}
            }
        }
    }

    private fun printChar(ch: Int) {
        if (ch == 0 || isKittyPlaceholder(ch) || isCombiningMark(ch)) return
        val width = if (isWide(ch)) 2 else 1
        val glyph = String(Character.toChars(ch))
        if (cursor.col + width > cols) newline()
        put(glyph)
        if (width == 2 && cursor.col < cols) {
            put(" ")
        }
    }

    private fun put(ch: String) {
        if (cursor.row !in 0 until rows || cursor.col !in 0 until cols) return
        grid[cursor.row][cursor.col] = Cell(ch, fg, bg, bold, underline, inverse)
        cursor.col += 1
        if (cursor.col >= cols) newline()
    }

    private fun newline() {
        cursor.col = 0
        if (cursor.row < rows - 1) {
            cursor.row += 1
        } else {
            scrollUp()
        }
    }

    private fun scrollUp() {
        scrollback.addLast(grid[0].toList())
        if (scrollback.size > maxScrollback) scrollback.removeFirst()
        for (r in 0 until rows - 1) {
            grid[r] = grid[r + 1]
        }
        grid[rows - 1] = Array(cols) { Cell() }
    }

    private fun execute(b: Int) {
        when (b) {
            0x08 -> cursor.col = (cursor.col - 1).coerceAtLeast(0)
            0x09 -> cursor.col = ((cursor.col / 8 + 1) * 8).coerceAtMost(cols - 1)
            0x0A, 0x0B, 0x0C -> newline()
            0x0D -> cursor.col = 0
        }
    }

    private fun csi(action: Action.Csi) {
        val p = action.params
        fun n(i: Int, default: Int = 1) = p.getOrNull(i)?.takeIf { it > 0 } ?: default
        when (action.final) {
            'A' -> cursor.row = (cursor.row - n(0)).coerceAtLeast(0)
            'B' -> cursor.row = (cursor.row + n(0)).coerceAtMost(rows - 1)
            'C' -> cursor.col = (cursor.col + n(0)).coerceAtMost(cols - 1)
            'D' -> cursor.col = (cursor.col - n(0)).coerceAtLeast(0)
            'H', 'f' -> {
                cursor.row = (n(0) - 1).coerceIn(0, rows - 1)
                cursor.col = (n(1, 1) - 1).coerceIn(0, cols - 1)
            }
            'J' -> eraseDisplay(p.getOrNull(0) ?: 0)
            'K' -> eraseLine(p.getOrNull(0) ?: 0)
            'm' -> sgr(p)
        }
    }

    private fun eraseDisplay(mode: Int) {
        when (mode) {
            2, 3 -> {
                grid = Array(rows) { Array(cols) { Cell() } }
                cursor = Cursor()
            }
            0 -> {
                eraseLine(0)
                for (r in cursor.row + 1 until rows) grid[r] = Array(cols) { Cell() }
            }
            1 -> {
                for (r in 0 until cursor.row) grid[r] = Array(cols) { Cell() }
                eraseLine(1)
            }
        }
    }

    private fun eraseLine(mode: Int) {
        val row = grid[cursor.row]
        val range = when (mode) {
            1 -> 0..cursor.col
            2 -> 0 until cols
            else -> cursor.col until cols
        }
        for (c in range) row[c] = Cell()
    }

    private fun sgr(params: List<Int>) {
        if (params.isEmpty()) {
            resetAttrs()
            return
        }
        var i = 0
        while (i < params.size) {
            when (val p = params[i]) {
                0 -> resetAttrs()
                1 -> bold = true
                4 -> underline = true
                7 -> inverse = true
                22 -> bold = false
                24 -> underline = false
                27 -> inverse = false
                in 30..37 -> fg = ANSI16[p - 30]
                39 -> fg = DEFAULT_FG
                in 40..47 -> bg = ANSI16[p - 40]
                49 -> bg = DEFAULT_BG
                in 90..97 -> fg = ANSI16[p - 90 + 8]
                in 100..107 -> bg = ANSI16[p - 100 + 8]
                38, 48 -> {
                    val targetFg = p == 38
                    val mode = params.getOrNull(i + 1)
                    if (mode == 5) {
                        val idx = params.getOrNull(i + 2) ?: 0
                        val color = indexedColor(idx)
                        if (targetFg) fg = color else bg = color
                        i += 2
                    } else if (mode == 2) {
                        val r = params.getOrNull(i + 2) ?: 0
                        val g = params.getOrNull(i + 3) ?: 0
                        val b = params.getOrNull(i + 4) ?: 0
                        val color = rgb(r, g, b)
                        if (targetFg) fg = color else bg = color
                        i += 4
                    }
                }
            }
            i += 1
        }
    }

    private fun resetAttrs() {
        fg = DEFAULT_FG
        bg = DEFAULT_BG
        bold = false
        underline = false
        inverse = false
    }
}

private fun rgb(r: Int, g: Int, b: Int): Int =
    (0xFF shl 24) or ((r and 0xFF) shl 16) or ((g and 0xFF) shl 8) or (b and 0xFF)

private fun indexedColor(index: Int): Int {
    if (index in 0..15) return ANSI16[index]
    if (index in 16..231) {
        val n = index - 16
        fun level(v: Int) = if (v == 0) 0 else 55 + v * 40
        return rgb(level(n / 36), level((n / 6) % 6), level(n % 6))
    }
    if (index in 232..255) {
        val v = 8 + (index - 232) * 10
        return rgb(v, v, v)
    }
    return DEFAULT_FG
}

private fun isKittyPlaceholder(code: Int): Boolean =
    code == 0x10EEEE || code in 0xEE00..0xEEFF

private fun isCombiningMark(code: Int): Boolean =
    code in 0x0300..0x036F || code in 0x1DC0..0x1DFF || code in 0x20D0..0x20FF

private fun isWide(code: Int): Boolean {
    return code in 0x1100..0x115F ||
        code in 0x2E80..0xA4CF ||
        code in 0xAC00..0xD7A3 ||
        code in 0xF900..0xFAFF ||
        code in 0xFE10..0xFE19 ||
        code in 0xFE30..0xFE6F ||
        code in 0xFF00..0xFF60 ||
        code in 0xFFE0..0xFFE6 ||
        code in 0x1F300..0x1FAFF
}

private sealed class Action {
    data class Print(val ch: Int) : Action()
    data class Execute(val b: Int) : Action()
    data class Csi(val params: List<Int>, val final: Char) : Action()
    data class Osc(val payload: String) : Action()
    data class Esc(val ch: Char) : Action()
}

private class AnsiParser {
    private enum class State { GROUND, ESC, CSI, OSC, OSC_ESC, APC, APC_ESC, CHARSET }
    private var state = State.GROUND
    private val params = StringBuilder()
    private val osc = StringBuilder()
    private val utf8 = ByteArray(4)
    private var utf8Need = 0
    private var utf8Got = 0

    fun feed(bytes: ByteArray, emit: (Action) -> Unit) {
        for (b in bytes) {
            val v = b.toInt() and 0xFF
            when (state) {
                State.GROUND -> when {
                    v == 0x1B -> state = State.ESC
                    v < 0x20 -> emit(Action.Execute(v))
                    v < 0x80 -> emit(Action.Print(v))
                    else -> utf8Byte(v, emit)
                }
                State.ESC -> when (v.toChar()) {
                    '[' -> {
                        params.clear()
                        state = State.CSI
                    }
                    ']' -> {
                        osc.clear()
                        state = State.OSC
                    }
                    '_' -> state = State.APC
                    '(', ')', '*' , '+' -> state = State.CHARSET
                    else -> {
                        emit(Action.Esc(v.toChar()))
                        state = State.GROUND
                    }
                }
                State.CHARSET -> state = State.GROUND
                State.APC -> if (v == 0x1B) state = State.APC_ESC
                State.APC_ESC -> state = if (v == 0x5C) State.GROUND else State.APC
                State.CSI -> {
                    if (v in 0x40..0x7E) {
                        emit(Action.Csi(parseParams(params.toString()), v.toChar()))
                        state = State.GROUND
                    } else {
                        params.append(v.toChar())
                    }
                }
                State.OSC -> when (v) {
                    0x07 -> {
                        emit(Action.Osc(osc.toString()))
                        state = State.GROUND
                    }
                    0x1B -> state = State.OSC_ESC
                    else -> osc.append(v.toChar())
                }
                State.OSC_ESC -> {
                    if (v == 0x5C) emit(Action.Osc(osc.toString()))
                    state = State.GROUND
                }
            }
        }
    }

    private fun utf8Byte(v: Int, emit: (Action) -> Unit) {
        if (utf8Need == 0) {
            utf8Need = when {
                v and 0xE0 == 0xC0 -> 2
                v and 0xF0 == 0xE0 -> 3
                v and 0xF8 == 0xF0 -> 4
                else -> {
                    emit(Action.Print('?'.code))
                    return
                }
            }
            utf8Got = 0
        }
        utf8[utf8Got++] = v.toByte()
        if (utf8Got == utf8Need) {
            val text = String(utf8, 0, utf8Got, Charsets.UTF_8)
            if (text.isNotEmpty()) emit(Action.Print(text.codePointAt(0)))
            utf8Need = 0
            utf8Got = 0
        }
    }
}

private fun parseParams(raw: String): List<Int> {
    if (raw.isEmpty()) return emptyList()
    return raw.split(';').map { part ->
        part.filter { it.isDigit() }.toIntOrNull() ?: 0
    }
}
