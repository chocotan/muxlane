//! muxlane 主题系统：内置浅色、暖色与深色主题。

use crate::i18n::{self, Language};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    Light,
    Paper,
    Sky,
    Jade,
    Sakura,
    Dark,
    Synthwave,
    OneDark,
}

impl ThemeMode {
    pub const ALL: [Self; 8] = [
        Self::Light,
        Self::Paper,
        Self::Sky,
        Self::Jade,
        Self::Sakura,
        Self::Dark,
        Self::Synthwave,
        Self::OneDark,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Light => "light",
            Self::Paper => "paper",
            Self::Sky => "sky",
            Self::Jade => "jade",
            Self::Sakura => "sakura",
            Self::Dark => "dark",
            Self::Synthwave => "synthwave",
            Self::OneDark => "onedark",
        }
    }

    pub fn from_id(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|mode| mode.id() == value)
    }

    pub fn label(self, language: Language) -> &'static str {
        i18n::text(
            language,
            match self {
                Self::Light => "theme.light",
                Self::Paper => "theme.paper",
                Self::Sky => "theme.sky",
                Self::Jade => "theme.jade",
                Self::Sakura => "theme.sakura",
                Self::Dark => "theme.dark",
                Self::Synthwave => "theme.synthwave",
                Self::OneDark => "theme.one_dark",
            },
        )
    }

    pub fn is_dark(self) -> bool {
        matches!(self, Self::Dark | Self::Synthwave | Self::OneDark)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub bg0: u32,
    pub bg1: u32,
    pub bg2: u32,
    pub bg3: u32,
    pub line: u32,
    pub fg0: u32,
    pub fg1: u32,
    pub fg2: u32,
    pub accent: u32,
    pub on_accent: u32,
    pub on_warning: u32,
    pub green: u32,
    pub yellow: u32,
    pub red: u32,
}

impl Theme {
    pub fn with_alpha(color: u32, alpha: u8) -> u32 {
        (color & 0xffffff00) | u32::from(alpha)
    }

    pub fn overlay(self) -> u32 {
        Self::with_alpha(0x00000000, 0x44)
    }

    pub fn selection(self) -> u32 {
        Self::with_alpha(self.accent, 0x55)
    }

    pub fn cursor(self) -> u32 {
        Self::with_alpha(self.accent, 0xc0)
    }

    pub fn scrollbar_track(self) -> u32 {
        Self::with_alpha(self.bg3, 0x4d)
    }

    pub fn scrollbar_thumb(self) -> u32 {
        Self::with_alpha(self.fg1, 0xaa)
    }

    pub fn for_mode(mode: ThemeMode) -> Self {
        match mode {
            ThemeMode::Light => Self {
                bg0: 0xfaf9f6ff,
                bg1: 0xece8e1ff,
                bg2: 0xdbd7cfff,
                bg3: 0xd0ccc3ff,
                line: 0xd0ccc3ff,
                fg0: 0x242831ff,
                fg1: 0x4b5563ff,
                fg2: 0x59636fff,
                accent: 0x3d6cd8ff,
                on_accent: 0xffffffff,
                on_warning: 0x101010ff,
                green: 0x529633ff,
                yellow: 0xb88226ff,
                red: 0xd13e50ff,
            },
            ThemeMode::Paper => Self {
                bg0: 0xf4ecdfff,
                bg1: 0xeadfceff,
                bg2: 0xdfd0bcff,
                bg3: 0xd2c0a8ff,
                line: 0xd2c0a8ff,
                fg0: 0x3d3027ff,
                fg1: 0x574a41ff,
                fg2: 0x66584dff,
                accent: 0xb35f2aff,
                on_accent: 0xffffffff,
                on_warning: 0x101010ff,
                green: 0x4e8c62ff,
                yellow: 0xb8872eff,
                red: 0xc94b4bff,
            },
            ThemeMode::Sky => Self {
                bg0: 0xf2f7ffff,
                bg1: 0xe3edfcff,
                bg2: 0xd2e0f4ff,
                bg3: 0xc1d3ecff,
                line: 0xc1d3ecff,
                fg0: 0x1e293bff,
                fg1: 0x4b5c73ff,
                fg2: 0x52647cff,
                accent: 0x2563ebff,
                on_accent: 0xffffffff,
                on_warning: 0x101010ff,
                green: 0x16805cff,
                yellow: 0xb7791fff,
                red: 0xdc3f51ff,
            },
            ThemeMode::Jade => Self {
                bg0: 0xeff8f2ff,
                bg1: 0xe0f0e6ff,
                bg2: 0xcfe4d8ff,
                bg3: 0xbfd8caff,
                line: 0xbfd8caff,
                fg0: 0x1f3529ff,
                fg1: 0x465d50ff,
                fg2: 0x526b5eff,
                accent: 0x059669ff,
                on_accent: 0xffffffff,
                on_warning: 0x101010ff,
                green: 0x2f855aff,
                yellow: 0xb7791fff,
                red: 0xc94b4bff,
            },
            ThemeMode::Sakura => Self {
                bg0: 0xfff2f5ff,
                bg1: 0xf9e3eaff,
                bg2: 0xf1d0dcff,
                bg3: 0xe8bdcdff,
                line: 0xe8bdcdff,
                fg0: 0x452530ff,
                fg1: 0x674350ff,
                fg2: 0x765060ff,
                accent: 0xe11d48ff,
                on_accent: 0xffffffff,
                on_warning: 0x101010ff,
                green: 0x378557ff,
                yellow: 0xb7791fff,
                red: 0xc2415aff,
            },
            ThemeMode::Dark => Self {
                bg0: 0x181a1fff,
                bg1: 0x21252bff,
                bg2: 0x282c34ff,
                bg3: 0x3b4048ff,
                line: 0x333842ff,
                fg0: 0xd7dae0ff,
                fg1: 0xa4aab5ff,
                fg2: 0x9aa1adff,
                accent: 0x528bffff,
                on_accent: 0x0f1419ff,
                on_warning: 0x101010ff,
                green: 0x98c379ff,
                yellow: 0xe5c07bff,
                red: 0xe06c75ff,
            },
            ThemeMode::Synthwave => Self {
                bg0: 0x1b1029ff,
                bg1: 0x28163bff,
                bg2: 0x38204dff,
                bg3: 0x4b2861ff,
                line: 0x4b2861ff,
                fg0: 0xf9eaffff,
                fg1: 0xc6a9d8ff,
                fg2: 0xb39bc8ff,
                accent: 0xe879f9ff,
                on_accent: 0x1b1029ff,
                on_warning: 0x101010ff,
                green: 0x7ee2b8ff,
                yellow: 0xf5ca7aff,
                red: 0xfb7185ff,
            },
            ThemeMode::OneDark => Self {
                bg0: 0x282c34ff,
                bg1: 0x21252bff,
                bg2: 0x313640ff,
                bg3: 0x3e4451ff,
                line: 0x3e4451ff,
                fg0: 0xabb2bfff,
                fg1: 0x9299a7ff,
                fg2: 0x9da4b2ff,
                accent: 0x61afefff,
                on_accent: 0x0f1419ff,
                on_warning: 0x101010ff,
                green: 0x98c379ff,
                yellow: 0xe5c07bff,
                red: 0xe06c75ff,
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn theme_ids_round_trip() {
        for mode in ThemeMode::ALL {
            assert_eq!(ThemeMode::from_id(mode.id()), Some(mode));
            assert_ne!(Theme::for_mode(mode).bg0, 0);
        }
    }

    #[test]
    fn dark_theme_ids_are_marked_dark() {
        assert!(ThemeMode::Dark.is_dark());
        assert!(ThemeMode::Synthwave.is_dark());
        assert!(!ThemeMode::Paper.is_dark());
    }

    fn srgb_luminance(color: u32) -> f64 {
        let channel = |shift: u32| {
            let value = f64::from((color >> shift) & 0xff) / 255.0;
            if value <= 0.04045 {
                value / 12.92
            } else {
                ((value + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * channel(24) + 0.7152 * channel(16) + 0.0722 * channel(8)
    }

    fn contrast_ratio(foreground: u32, background: u32) -> f64 {
        let foreground = srgb_luminance(foreground);
        let background = srgb_luminance(background);
        (foreground.max(background) + 0.05) / (foreground.min(background) + 0.05)
    }

    #[test]
    fn small_text_contrasts_on_all_theme_surfaces() {
        for mode in ThemeMode::ALL {
            let theme = Theme::for_mode(mode);
            for background in [theme.bg0, theme.bg1] {
                assert!(
                    contrast_ratio(theme.fg1, background) >= 4.5,
                    "{} fg1 on {background:#x}",
                    mode.id()
                );
                assert!(
                    contrast_ratio(theme.fg2, background) >= 4.5,
                    "{} fg2 on {background:#x}",
                    mode.id()
                );
            }
        }
    }

    #[test]
    fn light_secondary_text_is_lighter_than_primary_muted() {
        let theme = Theme::for_mode(ThemeMode::Light);
        assert!(srgb_luminance(theme.fg2) > srgb_luminance(theme.fg1));
        assert!(srgb_luminance(theme.fg1) > srgb_luminance(theme.fg0));
    }

    #[test]
    fn synthwave_on_accent_is_not_white() {
        let theme = Theme::for_mode(ThemeMode::Synthwave);
        assert_ne!(theme.on_accent, 0xffffffff);
        assert_eq!(Theme::with_alpha(theme.accent, 0x44) & 0xff, 0x44);
    }

    #[test]
    fn warning_badge_text_meets_aa_in_every_theme() {
        for mode in ThemeMode::ALL {
            let theme = Theme::for_mode(mode);
            assert!(
                contrast_ratio(theme.on_warning, theme.yellow) >= 4.5,
                "{}",
                mode.id()
            );
        }
    }
}
