//! Pairing QR. Encoder is the vendored Nayuki generator (`qrcodegen`).

use crate::qrcodegen::{QrCode, QrCodeEcc};
use gpui::{ImageSource, RenderImage};
use smallvec::SmallVec;
use std::sync::Arc;

/// Modules, true = dark, quiet zone of 4 included.
pub fn encode(payload: &str) -> Option<Vec<bool>> {
    let qr = QrCode::encode_text(payload, QrCodeEcc::Medium).ok()?;
    let size = qr.size();
    let n = size + 8;
    let mut out = vec![false; (n * n) as usize];
    for y in 0..size {
        for x in 0..size {
            out[((y + 4) * n + x + 4) as usize] = qr.get_module(x, y);
        }
    }
    Some(out)
}

/// Black modules on white, 4px per module, for GPUI `img()`.
pub fn image(payload: &str) -> Option<ImageSource> {
    let modules = encode(payload)?;
    let n = (modules.len() as f64).sqrt() as u32;
    const SCALE: u32 = 4;
    let px = n * SCALE;
    let mut bgra = vec![255u8; (px * px * 4) as usize];
    for y in 0..n {
        for x in 0..n {
            if !modules[(y * n + x) as usize] {
                continue;
            }
            for dy in 0..SCALE {
                for dx in 0..SCALE {
                    let i = (((y * SCALE + dy) * px + x * SCALE + dx) * 4) as usize;
                    bgra[i] = 0;
                    bgra[i + 1] = 0;
                    bgra[i + 2] = 0;
                    bgra[i + 3] = 255;
                }
            }
        }
    }
    let buffer = image::RgbaImage::from_raw(px, px, bgra)?;
    Some(ImageSource::Render(Arc::new(RenderImage::new(
        SmallVec::from_elem(image::Frame::new(buffer), 1),
    ))))
}

/// `muxlane://pair?relay=<urlencoded>&id=<urlencoded>`
pub fn pair_link(relay: &str, machine_id: &str) -> String {
    format!(
        "muxlane://pair?relay={}&id={}",
        url_encode(relay),
        url_encode(machine_id)
    )
}

fn url_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pair_link_encodes_relay_and_id() {
        assert_eq!(
            pair_link("ws://192.168.1.8:9843", "machine_01ABC"),
            "muxlane://pair?relay=ws%3A%2F%2F192.168.1.8%3A9843&id=machine_01ABC"
        );
    }

    /// Nayuki encode_text("HI", Medium) with ECC boost (becomes High), mask 3.
    #[test]
    fn matches_known_qr() {
        let modules = encode("HI").expect("fits");
        let n = (modules.len() as f64).sqrt() as usize;
        let size = n - 8;
        let mut bits = String::new();
        for y in 0..size {
            for x in 0..size {
                bits.push(if modules[(y + 4) * n + x + 4] {
                    '1'
                } else {
                    '0'
                });
            }
        }
        assert_eq!(
            bits,
            "111111100110001111111100000100000101000001101110100010001011101101110100010001011101101110101010101011101100000100000001000001111111101010101111111000000001110000000000001100111110011010000000110010100011100111001101101100101111010011010011001110000010010100101000011010100000000001100110100110111111101110111010110100000100110111001010101110100111111001001101110101010000010010101110101000000011100100000100100010011000111111100101101101010"
        );
    }

    #[test]
    fn pair_payload_fits_and_has_quiet_zone() {
        let modules = encode(&pair_link("ws://192.168.1.8:9843", "machine_01ABC")).expect("fits");
        let n = (modules.len() as f64).sqrt() as usize;
        assert!(modules[..n].iter().all(|d| !d));
        assert!(modules[4 * n + 4]);
    }
}
