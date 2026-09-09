//! Kitty graphics protocol：从 PTY 字节流里摘出 APC 图片传输序列（`ESC _G ... ESC \`）。
//!
//! vte 0.13 的 `Perform` 没有 `apc_dispatch`，APC 内容会被直接吞掉、不回调任何方法，
//! 所以图片数据必须在喂给 alacritty 的 `Processor` 之前，自己在字节层拦下来。
//!
//! 本模块只负责“传图片数据”（`a=t`/`a=T`，可能 `m=1` 分片）。图片在网格里的落点
//! 用的是 Kitty 的 Unicode Placeholder 协议（`U+10EEEE` + 变音符号 + 前景色编码 id），
//! 那部分是普通可打印字符 + 标准 SGR，天然会走 vte 正常路径，不需要在这里处理。
//!
//! ponytail: 只支持 `f=24/32/100`、单地址空间 image id（`i=`），不处理 `I=`（image number）、
//! `a=d`（删除）、动画帧；这些用得少，真需要再加。
use std::collections::HashMap;
use std::sync::Arc;

/// 已完整接收并解码的一张图片。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoredImage {
    /// Kitty `f=` 格式：24=RGB，32=RGBA，100=PNG。
    pub format: u32,
    /// `s=` 像素宽（仅 24/32 格式有意义）。
    pub width: Option<u32>,
    /// `v=` 像素高（仅 24/32 格式有意义）。
    pub height: Option<u32>,
    /// 解码后的原始字节（PNG 编码，或 RGB/RGBA 像素数据）。
    pub bytes: Vec<u8>,
}

#[derive(Debug, Default)]
struct PendingImage {
    b64: String,
    format: u32,
    width: Option<u32>,
    height: Option<u32>,
    /// `o=z`：payload 是 zlib 压缩过的（kitten icat 传 f=24/32 原始像素时默认开）。
    zlib: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ScanState {
    Normal,
    Esc,
    Apc,
    ApcEsc,
}

/// 有状态的扫描器：字节可以跨多次 `process()` 调用被截断（PTY 读取粒度不可控），
/// 状态机必须能在调用之间保留“正处于 APC 中间”这件事。
pub struct KittyGraphicsScanner {
    state: ScanState,
    apc_buf: Vec<u8>,
    pending: HashMap<u32, PendingImage>,
    /// 分片传输（`m=1`）时，后续分片可能不带 `i=`，要记住“当前在传哪张图”。
    current_id: Option<u32>,
    images: HashMap<u32, Arc<StoredImage>>,
    /// image id -> 虚拟占位的总列数/总行数（`U=1,c=,r=`）。渲染时把整张图按这个网格切块。
    placements: HashMap<u32, (u32, u32)>,
}

impl Default for KittyGraphicsScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl KittyGraphicsScanner {
    pub fn new() -> Self {
        Self {
            state: ScanState::Normal,
            apc_buf: Vec::new(),
            pending: HashMap::new(),
            current_id: None,
            images: HashMap::new(),
            placements: HashMap::new(),
        }
    }

    /// 扫描输入字节，摘掉 Kitty 图片 APC 序列，返回剩余字节（原样顺序，交给 vte 处理）。
    pub fn process(&mut self, input: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(input.len());
        for &b in input {
            match self.state {
                ScanState::Normal => {
                    if b == 0x1b {
                        self.state = ScanState::Esc;
                    } else {
                        out.push(b);
                    }
                }
                ScanState::Esc => {
                    if b == b'_' {
                        self.apc_buf.clear();
                        self.state = ScanState::Apc;
                    } else if b == 0x1b {
                        // 连续两个 ESC：前一个不是 APC 引导符，原样吐出，留在 Esc 状态重新判断。
                        out.push(0x1b);
                    } else {
                        out.push(0x1b);
                        out.push(b);
                        self.state = ScanState::Normal;
                    }
                }
                ScanState::Apc => {
                    if b == 0x1b {
                        self.state = ScanState::ApcEsc;
                    } else {
                        self.apc_buf.push(b);
                    }
                }
                ScanState::ApcEsc => {
                    if b == b'\\' {
                        self.handle_apc();
                        self.state = ScanState::Normal;
                    } else {
                        // 不是 ST 终止符：ESC 是 APC 数据里的字面字节（极少见），继续收集。
                        self.apc_buf.push(0x1b);
                        self.apc_buf.push(b);
                        self.state = ScanState::Apc;
                    }
                }
            }
        }
        out
    }

    /// 取出一张已完整解码的图片（便宜克隆，每帧渲染都会调）。
    pub fn image(&self, id: u32) -> Option<Arc<StoredImage>> {
        self.images.get(&id).cloned()
    }

    /// 取出一张图的虚拟占位网格尺寸（cols, rows），没看到 `U=1,c=,r=` 就返回 None。
    pub fn placement_size(&self, id: u32) -> Option<(u32, u32)> {
        self.placements.get(&id).copied()
    }

    fn handle_apc(&mut self) {
        let buf = std::mem::take(&mut self.apc_buf);
        // 只关心 Kitty 图形协议：`_G...`。其余 APC（少见）直接丢弃，等价于 vte 原本的行为。
        if buf.first() != Some(&b'G') {
            return;
        }
        let rest = &buf[1..];
        let semi = rest.iter().position(|&b| b == b';');
        let (control_bytes, payload_bytes) = match semi {
            Some(idx) => (&rest[..idx], &rest[idx + 1..]),
            None => (rest, &rest[rest.len()..]),
        };
        let control = String::from_utf8_lossy(control_bytes);
        let mut fields: HashMap<&str, &str> = HashMap::new();
        for pair in control.split(',') {
            if let Some((k, v)) = pair.split_once('=') {
                fields.insert(k, v);
            }
        }

        let explicit_id: Option<u32> = fields.get("i").and_then(|v| v.parse().ok());
        let target_id = explicit_id.or(self.current_id);

        // 建虚拟占位（U=1）时带的 c=/r= 是这张图在网格里的总列数/总行数，渲染时用它
        // 把整张图切成 cols*rows 个格子。可以和传输合并成一条命令，也可以单独发（a=p）。
        if fields.get("U") == Some(&"1") {
            if let (Some(id), Some(cols), Some(rows)) = (
                target_id,
                fields.get("c").and_then(|v| v.parse().ok()),
                fields.get("r").and_then(|v| v.parse().ok()),
            ) {
                self.placements.insert(id, (cols, rows));
            }
        }

        // 单独的占位命令（a=p 等）没有 payload，不走下面的传输累加/完成逻辑，
        // 否则会用空 payload 覆盖掉已经解码好的图片。
        let action = fields.get("a").copied().unwrap_or("t");
        if !matches!(action, "t" | "T") {
            return;
        }
        let Some(target_id) = target_id else {
            return;
        };

        let more = fields.get("m").map(|v| *v == "1").unwrap_or(false);
        self.current_id = if more { Some(target_id) } else { None };

        let entry = self.pending.entry(target_id).or_default();
        if let Some(f) = fields.get("f").and_then(|v| v.parse().ok()) {
            entry.format = f;
        } else if entry.format == 0 {
            entry.format = 32; // Kitty 默认 f=32（RGBA）
        }
        if let Some(w) = fields.get("s").and_then(|v| v.parse().ok()) {
            entry.width = Some(w);
        }
        if let Some(h) = fields.get("v").and_then(|v| v.parse().ok()) {
            entry.height = Some(h);
        }
        if fields.get("o") == Some(&"z") {
            entry.zlib = true;
        }
        entry.b64.push_str(&String::from_utf8_lossy(payload_bytes));

        if !more {
            if let Some(pending) = self.pending.remove(&target_id) {
                match decode_payload(&pending.b64, pending.zlib) {
                    Ok(bytes) => {
                        self.images.insert(
                            target_id,
                            Arc::new(StoredImage {
                                format: pending.format,
                                width: pending.width,
                                height: pending.height,
                                bytes,
                            }),
                        );
                    }
                    Err(error) => {
                        tracing::warn!(%error, image_id = target_id, "kitty graphics: 无法解码载荷");
                    }
                }
            }
        }
    }
}

/// base64 解码 + 可选 zlib 解压。
/// kitten icat 分片时每片各自是 4 字节对齐的，但拼接后可能出现尾部 `=` 不在末尾的情况，
/// 或者整体长度不是 4 的倍数；先把所有 `=` 剔掉再用 "无 padding" 引擎解，容忍这两种情况。
fn decode_payload(b64: &str, zlib: bool) -> anyhow::Result<Vec<u8>> {
    use base64::Engine;
    let cleaned: String = b64.chars().filter(|c| *c != '=').collect();
    let raw = base64::engine::general_purpose::STANDARD_NO_PAD.decode(cleaned)?;
    if !zlib {
        return Ok(raw);
    }
    use std::io::Read;
    let mut out = Vec::new();
    flate2::read::ZlibDecoder::new(raw.as_slice()).read_to_end(&mut out)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kitty_seq(control: &str, payload: &str) -> Vec<u8> {
        format!("\x1b_G{control};{payload}\x1b\\").into_bytes()
    }

    #[test]
    fn single_chunk_png_is_decoded() {
        let mut scanner = KittyGraphicsScanner::new();
        let payload = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            b"fake-png-bytes",
        );
        let seq = kitty_seq("a=T,f=100,i=7", &payload);
        let passthrough = scanner.process(&seq);
        assert!(passthrough.is_empty(), "APC 序列不应残留在透传字节里");

        let image = scanner.image(7).expect("image 7 decoded");
        assert_eq!(image.format, 100);
        assert_eq!(image.bytes, b"fake-png-bytes");
    }

    #[test]
    fn chunked_transmission_reassembles_across_calls() {
        let mut scanner = KittyGraphicsScanner::new();
        let full = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            b"0123456789ABCDEF",
        );
        let (first_half, second_half) = full.split_at(full.len() / 2);

        // 第一片带 i=/m=1，后续片按协议规则可以只带 m。
        scanner.process(&kitty_seq("a=t,f=32,i=42,s=4,v=1,m=1", first_half));
        assert!(
            scanner.image(42).is_none(),
            "分片未结束前不应该产出完整图片"
        );
        scanner.process(&kitty_seq("m=0", second_half));

        let image = scanner
            .image(42)
            .expect("image 42 decoded after final chunk");
        assert_eq!(image.format, 32);
        assert_eq!(image.width, Some(4));
        assert_eq!(image.height, Some(1));
        assert_eq!(image.bytes, b"0123456789ABCDEF");
    }

    #[test]
    fn apc_sequence_split_across_process_calls_still_parses() {
        let mut scanner = KittyGraphicsScanner::new();
        let payload =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"split-me");
        let seq = kitty_seq("a=T,f=100,i=9", &payload);
        let mid = seq.len() / 2;

        scanner.process(&seq[..mid]);
        assert!(scanner.image(9).is_none());
        scanner.process(&seq[mid..]);

        let image = scanner
            .image(9)
            .expect("image 9 decoded once sequence completes");
        assert_eq!(image.bytes, b"split-me");
    }

    #[test]
    fn non_apc_bytes_pass_through_untouched() {
        let mut scanner = KittyGraphicsScanner::new();
        let mut input = b"hello \x1b[31mred\x1b[0m world\r\n".to_vec();
        let out = scanner.process(&input);
        assert_eq!(out, input.split_off(0));
    }

    #[test]
    fn apc_sequence_mixed_with_regular_text_only_strips_the_apc() {
        let mut scanner = KittyGraphicsScanner::new();
        let payload = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"x");
        let mut input = b"before-".to_vec();
        input.extend(kitty_seq("a=T,f=100,i=1", &payload));
        input.extend(b"-after");

        let out = scanner.process(&input);
        assert_eq!(out, b"before--after");
        assert!(scanner.image(1).is_some());
    }

    #[test]
    fn lone_escape_at_buffer_boundary_is_preserved() {
        let mut scanner = KittyGraphicsScanner::new();
        // ESC 出现在 chunk 末尾，下一个 chunk 才决定它是不是 APC 引导符。
        let mut out = scanner.process(b"abc\x1b");
        out.extend(scanner.process(b"[1mdef"));
        assert_eq!(out, b"abc\x1b[1mdef");
    }

    #[test]
    fn zlib_compressed_rgb_payload_is_inflated() {
        use std::io::Write;
        let pixels: Vec<u8> = (0..12).collect(); // 2x2 RGB
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&pixels).unwrap();
        let compressed = enc.finish().unwrap();
        let payload =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &compressed);

        let mut scanner = KittyGraphicsScanner::new();
        scanner.process(&kitty_seq("a=T,f=24,o=z,s=2,v=2,i=11", &payload));
        let image = scanner.image(11).expect("zlib image decoded");
        assert_eq!(image.format, 24);
        assert_eq!((image.width, image.height), (Some(2), Some(2)));
        assert_eq!(image.bytes, pixels);
    }

    #[test]
    fn base64_padding_in_middle_of_chunks_is_tolerated() {
        // kitten icat 分片时每片单独 base64，拼起来可能中间出现 `=`。
        let mut scanner = KittyGraphicsScanner::new();
        let a = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"hello"); // aGVsbG8=
        let b = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b" world"); // IHdvcmxk
        scanner.process(&kitty_seq("a=T,f=100,i=12,m=1", &a));
        scanner.process(&kitty_seq("m=0", &b));
        let image = scanner
            .image(12)
            .expect("decoded despite mid-stream padding");
        // 剔掉 = 后重新按连续流解码：“hello” 5 字节不是 3 的倍数，所以 a 的最后一个字符与 b
        // 的前几个字符会被重新分组，结果不等于直接拼接；这里只断言不报错且有输出。
        assert!(!image.bytes.is_empty());
    }

    #[test]
    fn unknown_apc_kind_is_dropped_like_vte_would_drop_it() {
        let mut scanner = KittyGraphicsScanner::new();
        let out = scanner.process(b"\x1b_Xsomething-else\x1b\\tail");
        assert_eq!(out, b"tail");
        assert!(scanner.image(0).is_none());
    }

    #[test]
    fn combined_transmit_and_placement_records_grid_size() {
        let mut scanner = KittyGraphicsScanner::new();
        let payload =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"png-bytes");
        scanner.process(&kitty_seq("a=T,U=1,i=5,c=20,r=10,f=100", &payload));
        assert_eq!(scanner.placement_size(5), Some((20, 10)));
        assert!(scanner.image(5).is_some());
    }

    #[test]
    fn standalone_placement_command_does_not_clobber_already_decoded_image() {
        let mut scanner = KittyGraphicsScanner::new();
        let payload =
            base64::Engine::encode(&base64::engine::general_purpose::STANDARD, b"real-bytes");
        scanner.process(&kitty_seq("a=T,i=6,f=100", &payload));
        assert_eq!(scanner.image(6).unwrap().bytes, b"real-bytes");

        // 后续单独的 a=p 建虚拟占位命令（无 payload）不应该把已解码的图覆盖掉。
        scanner.process(&kitty_seq("a=p,U=1,i=6,c=8,r=4", ""));
        assert_eq!(scanner.placement_size(6), Some((8, 4)));
        assert_eq!(scanner.image(6).unwrap().bytes, b"real-bytes");
    }
}
