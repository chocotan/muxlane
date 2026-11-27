/**
 * Kitty Graphics Protocol scanner & Unicode Placeholder (U+10EEEE) decoder for xterm.js.
 *
 * Mirrors `crates/muxlane-term/src/kitty_graphics.rs` and `vterm.rs` so `pi`, `pie`,
 * `kitten icat`, and other Kitty-capable TUI apps render inline photos seamlessly
 * in Electron Desktop's TerminalPane — both locally and over remote SSH sessions.
 */

import type { Terminal } from "@xterm/xterm";

export const KITTY_PLACEHOLDER_CP = 0x10eeee;

/**
 * Frozen Unicode 6.0.0 `rowcolumn-diacritics.txt` (297 entries).
 * Index encodes row / col / MSB values in Kitty Unicode Placeholders.
 */
const ROWCOLUMN_DIACRITICS: readonly number[] = [
  0x305, 0x30d, 0x30e, 0x310, 0x312, 0x33d, 0x33e, 0x33f,
  0x346, 0x34a, 0x34b, 0x34c, 0x350, 0x351, 0x352, 0x357,
  0x35b, 0x363, 0x364, 0x365, 0x366, 0x367, 0x368, 0x369,
  0x36a, 0x36b, 0x36c, 0x36d, 0x36e, 0x36f, 0x483, 0x484,
  0x485, 0x486, 0x487, 0x592, 0x593, 0x594, 0x595, 0x597,
  0x598, 0x599, 0x59c, 0x59d, 0x59e, 0x59f, 0x5a0, 0x5a1,
  0x5a8, 0x5a9, 0x5ab, 0x5ac, 0x5af, 0x5c4, 0x610, 0x611,
  0x612, 0x613, 0x614, 0x615, 0x616, 0x617, 0x657, 0x658,
  0x659, 0x65a, 0x65b, 0x65d, 0x65e, 0x6d6, 0x6d7, 0x6d8,
  0x6d9, 0x6da, 0x6db, 0x6dc, 0x6df, 0x6e0, 0x6e1, 0x6e2,
  0x6e4, 0x6e7, 0x6e8, 0x6eb, 0x6ec, 0x730, 0x732, 0x733,
  0x735, 0x736, 0x73a, 0x73d, 0x73f, 0x740, 0x741, 0x743,
  0x745, 0x747, 0x749, 0x74a, 0x7eb, 0x7ec, 0x7ed, 0x7ee,
  0x7ef, 0x7f0, 0x7f1, 0x7f3, 0x816, 0x817, 0x818, 0x819,
  0x81b, 0x81c, 0x81d, 0x81e, 0x81f, 0x820, 0x821, 0x822,
  0x823, 0x825, 0x826, 0x827, 0x829, 0x82a, 0x82b, 0x82c,
  0x82d, 0x951, 0x953, 0x954, 0xf82, 0xf83, 0xf86, 0xf87,
  0x135d, 0x135e, 0x135f, 0x17dd, 0x193a, 0x1a17, 0x1a75, 0x1a76,
  0x1a77, 0x1a78, 0x1a79, 0x1a7a, 0x1a7b, 0x1a7c, 0x1b6b, 0x1b6d,
  0x1b6e, 0x1b6f, 0x1b70, 0x1b71, 0x1b72, 0x1b73, 0x1cd0, 0x1cd1,
  0x1cd2, 0x1cda, 0x1cdb, 0x1ce0, 0x1dc0, 0x1dc1, 0x1dc3, 0x1dc4,
  0x1dc5, 0x1dc6, 0x1dc7, 0x1dc8, 0x1dc9, 0x1dcb, 0x1dcc, 0x1dd1,
  0x1dd2, 0x1dd3, 0x1dd4, 0x1dd5, 0x1dd6, 0x1dd7, 0x1dd8, 0x1dd9,
  0x1dda, 0x1ddb, 0x1ddc, 0x1ddd, 0x1dde, 0x1ddf, 0x1de0, 0x1de1,
  0x1de2, 0x1de3, 0x1de4, 0x1de5, 0x1de6, 0x1dfe, 0x20d0, 0x20d1,
  0x20d4, 0x20d5, 0x20d6, 0x20d7, 0x20db, 0x20dc, 0x20e1, 0x20e7,
  0x20e9, 0x20f0, 0x2cef, 0x2cf0, 0x2cf1, 0x2de0, 0x2de1, 0x2de2,
  0x2de3, 0x2de4, 0x2de5, 0x2de6, 0x2de7, 0x2de8, 0x2de9, 0x2dea,
  0x2deb, 0x2dec, 0x2ded, 0x2dee, 0x2def, 0x2df0, 0x2df1, 0x2df2,
  0x2df3, 0x2df4, 0x2df5, 0x2df6, 0x2df7, 0x2df8, 0x2df9, 0x2dfa,
  0x2dfb, 0x2dfc, 0x2dfd, 0x2dfe, 0x2dff, 0xa66f, 0xa67c, 0xa67d,
  0xa6f0, 0xa6f1, 0xa8e0, 0xa8e1, 0xa8e2, 0xa8e3, 0xa8e4, 0xa8e5,
  0xa8e6, 0xa8e7, 0xa8e8, 0xa8e9, 0xa8ea, 0xa8eb, 0xa8ec, 0xa8ed,
  0xa8ee, 0xa8ef, 0xa8f0, 0xa8f1, 0xaab0, 0xaab2, 0xaab3, 0xaab7,
  0xaab8, 0xaabe, 0xaabf, 0xaac1, 0xfe20, 0xfe21, 0xfe22, 0xfe23,
  0xfe24, 0xfe25, 0xfe26, 0x10a0f, 0x10a38, 0x1d185, 0x1d186, 0x1d187,
  0x1d188, 0x1d189, 0x1d1aa, 0x1d1ab, 0x1d1ac, 0x1d1ad, 0x1d242, 0x1d243,
  0x1d244,
];

const DIACRITIC_INDEX_MAP = new Map<number, number>(
  ROWCOLUMN_DIACRITICS.map((cp, idx) => [cp, idx]),
);

export function diacriticIndex(codePoint: number): number | undefined {
  return DIACRITIC_INDEX_MAP.get(codePoint);
}

export interface StoredKittyImage {
  id: number;
  /** 24 = RGB, 32 = RGBA, 100 = PNG/encoded image */
  format: number;
  width?: number;
  height?: number;
  zlib: boolean;
  /** Raw decoded bytes (may still be zlib-compressed if `zlib === true`). */
  rawBytes: Uint8Array;
}

export interface KittyDirectPlacement {
  id: number;
  cols?: number;
  rows?: number;
}

export interface KittyProcessResult {
  cleanBytes: Uint8Array;
  replies: string[];
  updatedImageIds: number[];
  directPlacements: KittyDirectPlacement[];
  cleared: boolean;
}

interface PendingKittyImage {
  b64Chunks: string[];
  totalB64Len: number;
  format: number;
  width?: number;
  height?: number;
  zlib: boolean;
  directCols?: number;
  directRows?: number;
  isDirectDisplay: boolean;
}

const MAX_KITTY_B64_BYTES = 32 * 1024 * 1024; // 32 MiB safety cap per image
const MAX_STORED_IMAGES = 64;

type ScanState = "normal" | "esc" | "apc" | "apc_esc" | "apc_interleaved";

function decodeBase64ToUint8Array(b64: string): Uint8Array | null {
  const clean = b64.replace(/[\r\n\t ]+/g, "");
  if (!clean) return new Uint8Array(0);
  const rem = clean.length % 4;
  if (rem === 1) return null;
  const padded = rem === 0 ? clean : clean + "=".repeat(4 - rem);
  try {
    const bin = atob(padded);
    const out = new Uint8Array(bin.length);
    for (let i = 0; i < bin.length; i++) {
      out[i] = bin.charCodeAt(i);
    }
    return out;
  } catch {
    return null;
  }
}

export class KittyGraphicsScanner {
  private state: ScanState = "normal";
  private interleavedKind = 0;
  private apcBuf: number[] = [];
  private interleaveBuf: number[] = [];
  private pending = new Map<number, PendingKittyImage>();
  private currentId: number | null = null;
  private nextAutoId = 0x70000000;
  private images = new Map<number, StoredKittyImage>();
  private placements = new Map<number, { cols: number; rows: number }>();
  private lastPlacementId: number | null = null;

  reset(): void {
    this.state = "normal";
    this.apcBuf = [];
    this.interleaveBuf = [];
    this.pending.clear();
    this.currentId = null;
    this.images.clear();
    this.placements.clear();
    this.lastPlacementId = null;
  }

  getImage(id: number): StoredKittyImage | undefined {
    return this.images.get(id);
  }

  getPlacementSize(id: number): { cols: number; rows: number } | undefined {
    return this.placements.get(id);
  }

  getLatestPlacementId(): number | null {
    return this.lastPlacementId;
  }

  hasImages(): boolean {
    return this.images.size > 0;
  }

  process(input: Uint8Array): KittyProcessResult {
    const out: number[] = [];
    const replies: string[] = [];
    const updatedImageIds: number[] = [];
    const directPlacements: KittyDirectPlacement[] = [];
    let cleared = false;

    for (let i = 0; i < input.length; i++) {
      const b = input[i];
      switch (this.state) {
        case "normal":
          if (b === 0x1b) {
            this.state = "esc";
          } else {
            out.push(b);
          }
          break;
        case "esc":
          if (b === 0x5f) {
            // '_' -> APC start (ESC _)
            this.apcBuf = [];
            this.state = "apc";
          } else if (b === 0x1b) {
            out.push(0x1b);
          } else {
            out.push(0x1b, b);
            this.state = "normal";
          }
          break;
        case "apc":
          if (b === 0x1b) {
            this.state = "apc_esc";
          } else {
            this.apcBuf.push(b);
          }
          break;
        case "apc_esc":
          if (b === 0x5c) {
            // '\\' -> ST terminator (ESC \)
            const apcRes = this.handleApc();
            if (apcRes.reply) replies.push(apcRes.reply);
            if (apcRes.updatedId != null) updatedImageIds.push(apcRes.updatedId);
            if (apcRes.directPlacement) directPlacements.push(apcRes.directPlacement);
            if (apcRes.cleared) cleared = true;
            this.state = "normal";
          } else if (b === 0x5b || b === 0x5d || b === 0x50) {
            // Interleaved CSI '[' / OSC ']' / DCS 'P' emitted by tmux mid-passthrough
            this.interleavedKind = b;
            this.interleaveBuf = [0x1b, b];
            this.state = "apc_interleaved";
          } else {
            this.apcBuf.push(0x1b, b);
            this.state = "apc";
          }
          break;
        case "apc_interleaved": {
          this.interleaveBuf.push(b);
          const done =
            this.interleavedKind === 0x5b
              ? b >= 0x40 && b <= 0x7e
              : b === 0x07 ||
                (b === 0x5c &&
                  this.interleaveBuf.length >= 2 &&
                  this.interleaveBuf[this.interleaveBuf.length - 2] === 0x1b);
          if (done) {
            for (let j = 0; j < this.interleaveBuf.length; j++) {
              out.push(this.interleaveBuf[j]);
            }
            this.interleaveBuf = [];
            this.state = "apc";
          }
          break;
        }
      }
    }

    return {
      cleanBytes: new Uint8Array(out),
      replies,
      updatedImageIds,
      directPlacements,
      cleared,
    };
  }

  private handleApc(): {
    reply?: string;
    updatedId?: number;
    directPlacement?: KittyDirectPlacement;
    cleared?: boolean;
  } {
    const buf = this.apcBuf;
    this.apcBuf = [];
    if (buf.length === 0 || buf[0] !== 0x47 /* 'G' */) {
      return {};
    }

    let semiIdx = -1;
    for (let i = 1; i < buf.length; i++) {
      if (buf[i] === 0x3b /* ';' */) {
        semiIdx = i;
        break;
      }
    }

    const controlEnd = semiIdx >= 0 ? semiIdx : buf.length;
    let control = "";
    for (let i = 1; i < controlEnd; i++) {
      control += String.fromCharCode(buf[i]);
    }

    const fields = new Map<string, string>();
    for (const part of control.split(",")) {
      const eq = part.indexOf("=");
      if (eq > 0) {
        fields.set(part.slice(0, eq), part.slice(eq + 1));
      }
    }

    const action = fields.get("a") ?? "t";
    const quiet = Number(fields.get("q") ?? "0");
    const explicitIdRaw = fields.get("i");
    const explicitId =
      explicitIdRaw !== undefined && /^\d+$/.test(explicitIdRaw)
        ? Number.parseInt(explicitIdRaw, 10)
        : null;

    // Capability probe (`a=q`)
    if (action === "q") {
      if (quiet < 2 && explicitId !== null) {
        return { reply: `\x1b_Gi=${explicitId};OK\x1b\\` };
      }
      return {};
    }

    // Delete command (`a=d`)
    if (action === "d") {
      const delMode = fields.get("d") ?? "a";
      if (delMode === "a" || delMode === "A") {
        this.images.clear();
        this.placements.clear();
        this.lastPlacementId = null;
        return { cleared: true };
      }
      if ((delMode === "i" || delMode === "I") && explicitId !== null) {
        this.images.delete(explicitId);
        this.placements.delete(explicitId);
        if (this.lastPlacementId === explicitId) this.lastPlacementId = null;
        return { cleared: true };
      }
      return {};
    }

    let targetId = explicitId ?? this.currentId;
    if (targetId === null && (action === "t" || action === "T")) {
      targetId = this.nextAutoId++;
    }

    // Virtual placement (`U=1,c=...,r=...`)
    const colsRaw = fields.get("c");
    const rowsRaw = fields.get("r");
    const cols = colsRaw && /^\d+$/.test(colsRaw) ? Number.parseInt(colsRaw, 10) : undefined;
    const rows = rowsRaw && /^\d+$/.test(rowsRaw) ? Number.parseInt(rowsRaw, 10) : undefined;
    const isVirtual = fields.get("U") === "1";

    if (isVirtual && targetId !== null && cols && rows) {
      this.placements.set(targetId, { cols, rows });
      this.lastPlacementId = targetId;
    }

    if (action !== "t" && action !== "T") {
      return {};
    }
    if (targetId === null) return {};

    const more = fields.get("m") === "1";
    this.currentId = more ? targetId : null;

    let entry = this.pending.get(targetId);
    if (!entry) {
      entry = {
        b64Chunks: [],
        totalB64Len: 0,
        format: 32,
        zlib: false,
        isDirectDisplay: action === "T" && !isVirtual,
      };
      this.pending.set(targetId, entry);
    }

    const fmtRaw = fields.get("f");
    if (fmtRaw && /^\d+$/.test(fmtRaw)) {
      entry.format = Number.parseInt(fmtRaw, 10);
    }
    const widthRaw = fields.get("s");
    if (widthRaw && /^\d+$/.test(widthRaw)) {
      entry.width = Number.parseInt(widthRaw, 10);
    }
    const heightRaw = fields.get("v");
    if (heightRaw && /^\d+$/.test(heightRaw)) {
      entry.height = Number.parseInt(heightRaw, 10);
    }
    if (fields.get("o") === "z") {
      entry.zlib = true;
    }
    if (action === "T" && !isVirtual) {
      entry.isDirectDisplay = true;
      if (cols) entry.directCols = cols;
      if (rows) entry.directRows = rows;
    }

    if (semiIdx >= 0 && semiIdx + 1 < buf.length) {
      const payloadLen = buf.length - (semiIdx + 1);
      if (entry.totalB64Len + payloadLen <= MAX_KITTY_B64_BYTES) {
        let chunk = "";
        for (let i = semiIdx + 1; i < buf.length; i++) {
          chunk += String.fromCharCode(buf[i]);
        }
        entry.b64Chunks.push(chunk);
        entry.totalB64Len += payloadLen;
      } else {
        this.pending.delete(targetId);
        this.currentId = null;
        return {};
      }
    }

    if (more) {
      return {};
    }

    const completed = this.pending.get(targetId);
    this.pending.delete(targetId);
    if (!completed) return {};

    const rawBytes = decodeBase64ToUint8Array(completed.b64Chunks.join(""));
    if (!rawBytes || rawBytes.length === 0) return {};

    // LRU cap: evict oldest entry when over limit
    if (this.images.has(targetId)) {
      this.images.delete(targetId);
    } else if (this.images.size >= MAX_STORED_IMAGES) {
      const oldestKey = this.images.keys().next().value;
      if (oldestKey !== undefined) {
        this.images.delete(oldestKey);
        this.placements.delete(oldestKey);
      }
    }

    this.images.set(targetId, {
      id: targetId,
      format: completed.format,
      width: completed.width,
      height: completed.height,
      zlib: completed.zlib,
      rawBytes,
    });

    const reply =
      quiet === 0 && explicitId !== null ? `\x1b_Gi=${explicitId};OK\x1b\\` : undefined;

    return {
      reply,
      updatedId: targetId,
      directPlacement: completed.isDirectDisplay
        ? { id: targetId, cols: completed.directCols, rows: completed.directRows }
        : undefined,
    };
  }
}

export interface KittyDrawSlice {
  imageId: number;
  /** Viewport cell coordinates */
  screenCol: number;
  screenRow: number;
  spanCols: number;
  spanRows: number;
  /** Slice origin in the logical image grid */
  srcColStart: number;
  srcRowStart: number;
  totalCols: number;
  totalRows: number;
}

interface PlaceholderCell {
  imageId: number;
  imgRow: number;
  imgCol: number;
  screenRow: number;
  screenCol: number;
}

/**
 * Scan the visible viewport of an xterm.js Terminal for Kitty Unicode Placeholder (`U+10EEEE`)
 * cells and group them into contiguous image draw slices.
 */
export function scanKittyPlaceholderSlices(
  term: Terminal,
  scanner: KittyGraphicsScanner,
): KittyDrawSlice[] {
  if (!scanner.hasImages()) return [];

  const buf = term.buffer.active;
  const startRow = buf.viewportY;
  const rows = term.rows;
  const cols = term.cols;
  const fallbackId = scanner.getLatestPlacementId();
  const reusableCell = buf.getNullCell();

  const byImage = new Map<number, PlaceholderCell[]>();

  for (let r = 0; r < rows; r++) {
    const line = buf.getLine(startRow + r);
    if (!line) continue;

    let prev: {
      fgKey: number;
      imgRow: number;
      imgCol: number;
      msb: number;
      baseId: number;
    } | null = null as {
      fgKey: number;
      imgRow: number;
      imgCol: number;
      msb: number;
      baseId: number;
    } | null;

    for (let c = 0; c < cols; c++) {
      const cell = line.getCell(c, reusableCell);
      if (!cell) {
        prev = null;
        continue;
      }
      const chars = cell.getChars();
      if (!chars || chars.codePointAt(0) !== KITTY_PLACEHOLDER_CP) {
        prev = null;
        continue;
      }

      // Extract trailing diacritics after U+10EEEE (which is a surrogate pair -> 2 UTF-16 code units)
      const diacritics: number[] = [];
      let idx = 2; // U+10EEEE is always 2 UTF-16 code units
      while (idx < chars.length) {
        const cp = chars.codePointAt(idx);
        if (cp === undefined) break;
        const d = diacriticIndex(cp);
        if (d !== undefined) diacritics.push(d);
        idx += cp > 0xffff ? 2 : 1;
      }

      // Decode base image id from foreground color (24-bit RGB or 256-color palette)
      let baseId: number | null = null;
      let fgKey = -1;
      if (cell.isFgRGB()) {
        baseId = cell.getFgColor() & 0xffffff;
        fgKey = 0x1000000 | baseId;
      } else if (cell.isFgPalette()) {
        baseId = cell.getFgColor() & 0xff;
        fgKey = 0x2000000 | baseId;
      }

      if (baseId === null) {
        if (fallbackId !== null && diacritics.length >= 2) {
          baseId = fallbackId;
          fgKey = -2;
        } else {
          prev = null;
          continue;
        }
      }

      const prevCell: {
        fgKey: number;
        imgRow: number;
        imgCol: number;
        msb: number;
        baseId: number;
      } | null = prev;
      const sameImage: boolean = prevCell !== null && prevCell.fgKey === fgKey && fgKey !== -2;
      const rowD = diacritics[0];
      const colD = diacritics[1];
      const msbD = diacritics[2];

      const imgRow: number | undefined =
        rowD !== undefined ? rowD : sameImage && prevCell ? prevCell.imgRow : undefined;
      const imgCol: number | undefined =
        colD !== undefined ? colD : sameImage && prevCell ? prevCell.imgCol + 1 : undefined;
      if (imgRow === undefined || imgCol === undefined) {
        prev = null;
        continue;
      }
      const msb: number = msbD !== undefined ? msbD : sameImage && prevCell ? prevCell.msb : 0;

      prev = { fgKey, imgRow, imgCol, msb, baseId };
      const imageId = (baseId | (msb << 24)) >>> 0;

      let bucket = byImage.get(imageId);
      if (!bucket) {
        bucket = [];
        byImage.set(imageId, bucket);
      }
      bucket.push({
        imageId,
        imgRow,
        imgCol,
        screenRow: r,
        screenCol: c,
      });
    }
  }

  const slices: KittyDrawSlice[] = [];
  for (const [imageId, cells] of byImage) {
    if (!scanner.getImage(imageId) || cells.length === 0) continue;

    let minScreenRow = Infinity;
    let maxScreenRow = -1;
    let minScreenCol = Infinity;
    let maxScreenCol = -1;
    let minImgRow = Infinity;
    let maxImgRow = -1;
    let minImgCol = Infinity;
    let maxImgCol = -1;

    for (const cell of cells) {
      if (cell.screenRow < minScreenRow) minScreenRow = cell.screenRow;
      if (cell.screenRow > maxScreenRow) maxScreenRow = cell.screenRow;
      if (cell.screenCol < minScreenCol) minScreenCol = cell.screenCol;
      if (cell.screenCol > maxScreenCol) maxScreenCol = cell.screenCol;
      if (cell.imgRow < minImgRow) minImgRow = cell.imgRow;
      if (cell.imgRow > maxImgRow) maxImgRow = cell.imgRow;
      if (cell.imgCol < minImgCol) minImgCol = cell.imgCol;
      if (cell.imgCol > maxImgCol) maxImgCol = cell.imgCol;
    }

    const placement = scanner.getPlacementSize(imageId);
    const totalCols = Math.max(placement?.cols ?? 0, maxImgCol + 1, 1);
    const totalRows = Math.max(placement?.rows ?? 0, maxImgRow + 1, 1);

    slices.push({
      imageId,
      screenCol: minScreenCol,
      screenRow: minScreenRow,
      spanCols: maxScreenCol - minScreenCol + 1,
      spanRows: maxScreenRow - minScreenRow + 1,
      srcColStart: minImgCol,
      srcRowStart: minImgRow,
      totalCols,
      totalRows,
    });
  }

  return slices;
}

async function inflateZlib(bytes: Uint8Array): Promise<Uint8Array> {
  if (typeof DecompressionStream === "undefined") {
    throw new Error("DecompressionStream unavailable");
  }
  const stream = new DecompressionStream("deflate");
  const writer = stream.writable.getWriter();
  const copy = new Uint8Array(bytes.byteLength);
  copy.set(bytes);
  void writer.write(copy).then(() => writer.close());
  const reader = stream.readable.getReader();
  const chunks: Uint8Array[] = [];
  let total = 0;
  for (;;) {
    const { value, done } = await reader.read();
    if (done) break;
    if (value) {
      chunks.push(value);
      total += value.byteLength;
    }
  }
  const out = new Uint8Array(total);
  let offset = 0;
  for (const chunk of chunks) {
    out.set(chunk, offset);
    offset += chunk.byteLength;
  }
  return out;
}

/**
 * Decode a `StoredKittyImage` into a browser `ImageBitmap` ready for fast `<canvas>` drawing.
 */
export async function decodeKittyImageBitmap(stored: StoredKittyImage): Promise<ImageBitmap | null> {
  const bytes = stored.zlib ? await inflateZlib(stored.rawBytes) : stored.rawBytes;

  if (stored.format === 100) {
    const copy = new Uint8Array(bytes.byteLength);
    copy.set(bytes);
    const blob = new Blob([copy], { type: "image/png" });
    return await createImageBitmap(blob);
  }

  if (stored.format === 24 || stored.format === 32) {
    const w = stored.width ?? 0;
    const h = stored.height ?? 0;
    if (w <= 0 || h <= 0 || w > 8192 || h > 8192) return null;
    const bpp = stored.format === 24 ? 3 : 4;
    if (bytes.byteLength < w * h * bpp) return null;

    const rgba = new Uint8ClampedArray(w * h * 4);
    if (bpp === 4) {
      rgba.set(bytes.subarray(0, w * h * 4));
    } else {
      for (let src = 0, dst = 0; dst < rgba.length; src += 3, dst += 4) {
        rgba[dst] = bytes[src];
        rgba[dst + 1] = bytes[src + 1];
        rgba[dst + 2] = bytes[src + 2];
        rgba[dst + 3] = 255;
      }
    }
    const imgData = new ImageData(rgba, w, h);
    return await createImageBitmap(imgData);
  }

  return null;
}
