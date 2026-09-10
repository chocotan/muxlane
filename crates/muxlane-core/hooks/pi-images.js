
// ---------------------------------------------------------------------------
// 内联图片：让 pi 在 muxlane 的 tmux pane 里也能显示图片，不需要额外安装插件。
//
// pi 自己的 detectCapabilities() 一见 $TMUX 就返回 images:null，永远不发图。这里绕过它：
// 图片用 Kitty 图形协议的 Unicode Placeholder 模式（U=1）发，APC 序列用 tmux DCS
// passthrough 包一层穿过 tmux，占位符是普通文本+SGR，tmux 按文字搬动，muxlane 侧
// （muxlane-term/kitty_graphics.rs + term_view.rs）负责解码与渲染。
//
// 只在 muxlane 明确设置 MUXLANE_KITTY_GRAPHICS=1 时启用；别的终端里此扩展只做状态上报。
// ---------------------------------------------------------------------------

const KITTY_ESC = "\x1b"
const KITTY_GLYPH = "\u{10eeee}"
// rowcolumn-diacritics.txt（Unicode 6.0.0 冻结表）前 224 项；muxlane 侧支持全部 297 项，
// 这里只需要覆盖行/列上限（80 列 × 24 行）即可。
const KITTY_DIACRITICS = [
  0x305, 0x30D, 0x30E, 0x310, 0x312, 0x33D, 0x33E, 0x33F, 0x346, 0x34A, 0x34B, 0x34C, 0x350, 0x351, 0x352, 0x357,
  0x35B, 0x363, 0x364, 0x365, 0x366, 0x367, 0x368, 0x369, 0x36A, 0x36B, 0x36C, 0x36D, 0x36E, 0x36F, 0x483, 0x484,
  0x485, 0x486, 0x487, 0x592, 0x593, 0x594, 0x595, 0x597, 0x598, 0x599, 0x59C, 0x59D, 0x59E, 0x59F, 0x5A0, 0x5A1,
  0x5A8, 0x5A9, 0x5AB, 0x5AC, 0x5AF, 0x5C4, 0x610, 0x611, 0x612, 0x613, 0x614, 0x615, 0x616, 0x617, 0x657, 0x658,
  0x659, 0x65A, 0x65B, 0x65D, 0x65E, 0x6D6, 0x6D7, 0x6D8, 0x6D9, 0x6DA, 0x6DB, 0x6DC, 0x6DF, 0x6E0, 0x6E1, 0x6E2,
  0x6E4, 0x6E7, 0x6E8, 0x6EB, 0x6EC, 0x730, 0x732, 0x733, 0x735, 0x736, 0x73A, 0x73D, 0x73F, 0x740, 0x741, 0x743,
  0x745, 0x747, 0x749, 0x74A, 0x7EB, 0x7EC, 0x7ED, 0x7EE, 0x7EF, 0x7F0, 0x7F1, 0x7F3, 0x816, 0x817, 0x818, 0x819,
  0x81B, 0x81C, 0x81D, 0x81E, 0x81F, 0x820, 0x821, 0x822, 0x823, 0x825, 0x826, 0x827, 0x829, 0x82A, 0x82B, 0x82C,
  0x82D, 0x951, 0x953, 0x954, 0xF82, 0xF83, 0xF86, 0xF87, 0x135D, 0x135E, 0x135F, 0x17DD, 0x193A, 0x1A17, 0x1A75, 0x1A76,
  0x1A77, 0x1A78, 0x1A79, 0x1A7A, 0x1A7B, 0x1A7C, 0x1B6B, 0x1B6D, 0x1B6E, 0x1B6F, 0x1B70, 0x1B71, 0x1B72, 0x1B73, 0x1CD0, 0x1CD1,
  0x1CD2, 0x1CDA, 0x1CDB, 0x1CE0, 0x1DC0, 0x1DC1, 0x1DC3, 0x1DC4, 0x1DC5, 0x1DC6, 0x1DC7, 0x1DC8, 0x1DC9, 0x1DCB, 0x1DCC, 0x1DD1,
  0x1DD2, 0x1DD3, 0x1DD4, 0x1DD5, 0x1DD6, 0x1DD7, 0x1DD8, 0x1DD9, 0x1DDA, 0x1DDB, 0x1DDC, 0x1DDD, 0x1DDE, 0x1DDF, 0x1DE0, 0x1DE1,
  0x1DE2, 0x1DE3, 0x1DE4, 0x1DE5, 0x1DE6, 0x1DFE, 0x20D0, 0x20D1, 0x20D4, 0x20D5, 0x20D6, 0x20D7, 0x20DB, 0x20DC, 0x20E1, 0x20E7,
  0x20E9, 0x20F0, 0x2CEF, 0x2CF0, 0x2CF1, 0x2DE0, 0x2DE1, 0x2DE2, 0x2DE3, 0x2DE4, 0x2DE5, 0x2DE6, 0x2DE7, 0x2DE8, 0x2DE9, 0x2DEA,
].map((cp) => String.fromCodePoint(cp))
const KITTY_MAX_COLS = 48
const KITTY_MAX_ROWS = 14
const KITTY_CELL = { w: 9, h: 18 } // 只用于算长宽比；muxlane 按真实 cell 尺寸画。

export function kittyEnabled(env = process.env) {
  return env.MUXLANE_KITTY_GRAPHICS === "1" && Boolean(env.TMUX)
}

function kittyTmuxWrap(sequence) {
  return `${KITTY_ESC}Ptmux;${sequence.replaceAll(KITTY_ESC, KITTY_ESC + KITTY_ESC)}${KITTY_ESC}\\`
}
function kittyApc(command) {
  return kittyTmuxWrap(`${KITTY_ESC}_G${command}${KITTY_ESC}\\`)
}
export function kittyUpload(base64, imageId) {
  const chunks = base64.match(/.{1,4096}/gu) ?? [""]
  return chunks.map((chunk, i) =>
    kittyApc(`${i ? "" : `a=t,f=100,i=${imageId},q=2,`}m=${i + 1 < chunks.length ? 1 : 0};${chunk}`))
}
export function kittyPlacement(imageId, columns, rows) {
  return kittyApc(`a=p,i=${imageId},U=1,c=${columns},r=${rows},q=2;`)
}
function kittyRgb(code, value) {
  return `${KITTY_ESC}[${code};2;${(value >>> 16) & 255};${(value >>> 8) & 255};${value & 255}m`
}
export function kittyCell(column, row, imageId) {
  const high = imageId >>> 24
  const mark = (i) => KITTY_DIACRITICS[i] ?? ""
  return `${kittyRgb(38, imageId & 0xffffff)}${KITTY_GLYPH}${mark(row)}${mark(column)}${high ? mark(high) : ""}${KITTY_ESC}[39m`
}
export function kittyGrid(columns, rows, imageId) {
  const lines = []
  for (let row = 0; row < rows; row++) {
    let line = ""
    for (let column = 0; column < columns; column++) line += kittyCell(column, row, imageId)
    lines.push(line)
  }
  return lines
}
export function kittyGeometry(widthPx, heightPx, maxColumns = KITTY_MAX_COLS, maxRows = KITTY_MAX_ROWS) {
  const scale = Math.min((maxColumns * KITTY_CELL.w) / widthPx, (maxRows * KITTY_CELL.h) / heightPx, 1)
  return {
    columns: Math.max(1, Math.ceil((widthPx * scale) / KITTY_CELL.w)),
    rows: Math.max(1, Math.ceil((heightPx * scale) / KITTY_CELL.h)),
  }
}

/// 读图片头部拿像素尺寸（PNG/JPEG/GIF/WebP），不解码像素。拿不到就按 4:3 兜底。
export function imageDimensions(buf) {
  if (buf.length >= 24 && buf[0] === 0x89 && buf.toString("ascii", 1, 4) === "PNG") {
    return { w: buf.readUInt32BE(16), h: buf.readUInt32BE(20) }
  }
  if (buf.length >= 10 && buf.toString("ascii", 0, 4) === "GIF8") {
    return { w: buf.readUInt16LE(6), h: buf.readUInt16LE(8) }
  }
  if (buf.length >= 30 && buf.toString("ascii", 0, 4) === "RIFF" && buf.toString("ascii", 8, 12) === "WEBP") {
    const chunk = buf.toString("ascii", 12, 16)
    if (chunk === "VP8 ") return { w: buf.readUInt16LE(26) & 0x3fff, h: buf.readUInt16LE(28) & 0x3fff }
    if (chunk === "VP8L") {
      const b = buf.readUInt32LE(21)
      return { w: (b & 0x3fff) + 1, h: ((b >>> 14) & 0x3fff) + 1 }
    }
    if (chunk === "VP8X") return { w: (buf.readUIntLE(24, 3)) + 1, h: (buf.readUIntLE(27, 3)) + 1 }
  }
  if (buf.length >= 4 && buf[0] === 0xff && buf[1] === 0xd8) {
    let i = 2
    while (i + 9 < buf.length) {
      if (buf[i] !== 0xff) { i++; continue }
      const marker = buf[i + 1]
      if (marker === 0xd8 || marker === 0x01 || (marker >= 0xd0 && marker <= 0xd7)) { i += 2; continue }
      const len = buf.readUInt16BE(i + 2)
      const sof = (marker >= 0xc0 && marker <= 0xcf) && marker !== 0xc4 && marker !== 0xc8 && marker !== 0xcc
      if (sof) return { w: buf.readUInt16BE(i + 7), h: buf.readUInt16BE(i + 5) }
      i += 2 + len
    }
  }
  return { w: 4, h: 3 }
}

export function imageBlocks(message) {
  const content = message?.content
  if (!Array.isArray(content)) return []
  return content.flatMap((v, index) =>
    v && v.type === "image" && typeof v.mimeType === "string" && typeof v.data === "string"
      ? [{ block: v, index }] : [])
}

const KITTY_ENTRY_TYPE = "muxlane-image"

export function registerInlineImages(pi, deps = {}) {
  const env = deps.env ?? process.env
  if (!kittyEnabled(env)) return
  const write = deps.write ?? ((s) => process.stdout.write(s))
  let nextId = (deps.seed ?? (Date.now() & 0x7fffffff)) || 1
  const images = new Map() // logicalId -> { base64, columns, rows, terminalId, uploaded, placed }
  const seen = new Set() // 已经产出过 entry 的 (message, blockIndex)

  function ensure(entry) {
    let img = images.get(entry.logicalId)
    if (!img) {
      const buf = Buffer.from(entry.data, "base64")
      const { w, h } = imageDimensions(buf)
      const { columns, rows } = kittyGeometry(w, h)
      nextId = (nextId + 1) >>> 0 || 1
      img = { base64: entry.data, columns, rows, terminalId: nextId, uploaded: false, placed: false }
      images.set(entry.logicalId, img)
    }
    if (!img.uploaded) {
      for (const s of kittyUpload(img.base64, img.terminalId)) write(s)
      img.uploaded = true
    }
    if (!img.placed) {
      write(kittyPlacement(img.terminalId, img.columns, img.rows))
      img.placed = true
    }
    return img
  }

  pi.registerEntryRenderer(KITTY_ENTRY_TYPE, (entry) => {
    const data = entry?.data
    if (!data || typeof data.data !== "string" || typeof data.logicalId !== "string") {
      return { render: () => ["[image] invalid muxlane-image entry"], invalidate() {} }
    }
    return {
      render: () => {
        try {
          const img = ensure(data)
          return kittyGrid(img.columns, img.rows, img.terminalId)
        } catch (error) {
          return [`[image] ${error instanceof Error ? error.message : String(error)}`]
        }
      },
      invalidate() {},
    }
  })

  pi.on("session_start", () => { images.clear(); seen.clear() })
  pi.on("message_end", (event) => {
    const message = event?.message
    if (!message || (message.role !== "user" && message.role !== "toolResult")) return
    for (const { block, index } of imageBlocks(message)) {
      const key = `${message.role}:${message.toolCallId ?? ""}:${message.timestamp ?? ""}:${index}:${block.data.length}`
      if (seen.has(key)) continue
      seen.add(key)
      pi.appendEntry(KITTY_ENTRY_TYPE, {
        logicalId: key,
        data: block.data,
        mimeType: block.mimeType,
      })
    }
  })
}
