use gpui::Pixels;
use std::sync::atomic::{AtomicU32, Ordering};

pub(crate) const MIN_PERCENT: u32 = 75;
pub(crate) const MAX_PERCENT: u32 = 200;
pub(crate) const STEP_PERCENT: u32 = 25;

static PERCENT: AtomicU32 = AtomicU32::new(100);

pub(crate) fn normalize_percent(percent: u32) -> u32 {
    percent.clamp(MIN_PERCENT, MAX_PERCENT)
}

/// Accept a whole percentage, optionally pasted with its percent sign.
pub(crate) fn parse_percent(text: &str) -> Option<u32> {
    let text = text.trim().strip_suffix('%').unwrap_or(text.trim()).trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let percent = text.parse().ok()?;
    (MIN_PERCENT..=MAX_PERCENT)
        .contains(&percent)
        .then_some(percent)
}

pub(crate) fn set_percent(percent: u32) {
    PERCENT.store(normalize_percent(percent), Ordering::Relaxed);
}

pub(crate) fn percent() -> u32 {
    PERCENT.load(Ordering::Relaxed)
}

pub(crate) fn factor() -> f32 {
    percent() as f32 / 100.0
}

pub(crate) fn px(value: f32) -> Pixels {
    gpui::px(value * factor())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scale_is_clamped_without_rounding_custom_values() {
        assert_eq!(normalize_percent(0), MIN_PERCENT);
        assert_eq!(normalize_percent(123), 123);
        assert_eq!(normalize_percent(138), 138);
        assert_eq!(normalize_percent(999), MAX_PERCENT);
    }

    #[test]
    fn custom_percent_requires_an_integer_in_range() {
        for text in ["137", " 137% ", "137 %"] {
            assert_eq!(parse_percent(text), Some(137));
        }
        assert_eq!(parse_percent("75"), Some(75));
        assert_eq!(parse_percent("200"), Some(200));
        for text in [
            "",
            "abc",
            "74",
            "201",
            "137.5",
            "-100",
            "+100",
            "100%%",
            "999999999999",
        ] {
            assert_eq!(parse_percent(text), None, "{text}");
        }
    }
}
