use crate::ui_scale::px as ui_px;
use gpui::{prelude::*, rgba, svg, Svg};

pub(crate) const SPLIT_HORIZONTAL_ICON: &[u8] = br#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='#000' stroke-width='1.8' stroke-linecap='square' stroke-linejoin='miter'><rect x='3' y='4' width='18' height='16'/><path d='M12 4v16'/></svg>"#;
pub(crate) const SPLIT_VERTICAL_ICON: &[u8] = br#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='#000' stroke-width='1.8' stroke-linecap='square' stroke-linejoin='miter'><rect x='3' y='4' width='18' height='16'/><path d='M3 12h18'/></svg>"#;
pub(crate) const MAXIMIZE_ICON: &[u8] = br#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='#000' stroke-width='1.8' stroke-linecap='square' stroke-linejoin='miter'><path d='M8 3H3v5M16 3h5v5M21 16v5h-5M3 16v5h5'/></svg>"#;
pub(crate) const RESTORE_ICON: &[u8] = br#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='#000' stroke-width='1.8' stroke-linecap='square' stroke-linejoin='miter'><rect x='7' y='7' width='13' height='13'/><path d='M17 7V4a1 1 0 0 0-1-1H4a1 1 0 0 0-1 1v12a1 1 0 0 0 1 1h3'/></svg>"#;
pub(crate) const CLOSE_ICON: &[u8] = br#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='#000' stroke-width='1.8' stroke-linecap='square'><path d='M6 6l12 12M18 6L6 18'/></svg>"#;
pub(crate) const CONNECT_ICON: &[u8] = br#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='#000' stroke-width='1.8' stroke-linecap='square' stroke-linejoin='miter'><path d='M7 7h10v10H7z'/><path d='M4 4h10M4 4v10M20 20H10M20 20V10'/></svg>"#;
pub(crate) const THEME_ICON: &[u8] = br#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='#000' stroke-width='1.8' stroke-linecap='round' stroke-linejoin='round'><path d='M20 15.5A8.5 8.5 0 1 1 8.5 4 6.5 6.5 0 0 0 20 15.5z'/></svg>"#;
pub(crate) const NOTIFICATION_ICON: &[u8] = br#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='#000' stroke-width='1.8' stroke-linecap='round' stroke-linejoin='round'><path d='M18 8a6 6 0 0 0-12 0c0 7-3 7-3 9h18c0-2-3-2-3-9'/><path d='M10 21h4'/></svg>"#;
pub(crate) const PLUS_ICON: &[u8] = br#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='#000' stroke-width='1.8' stroke-linecap='square'><path d='M12 5v14M5 12h14'/></svg>"#;
pub(crate) const FOLDER_ICON: &[u8] = br#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='#000' stroke-width='1.8' stroke-linecap='square' stroke-linejoin='miter'><path d='M3 5h6l2 2h10v12H3z'/></svg>"#;
pub(crate) const SETTINGS_ICON: &[u8] = br#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='#000' stroke-width='1.7' stroke-linejoin='round'><path d='M9.38 5.67L10.72 3.04H13.28L14.62 5.67L17.43 4.76L19.24 6.57L18.33 9.38L20.96 10.72V13.28L18.33 14.62L19.24 17.43L17.43 19.24L14.62 18.33L13.28 20.96H10.72L9.38 18.33L6.57 19.24L4.76 17.43L5.67 14.62L3.04 13.28V10.72L5.67 9.38L4.76 6.57L6.57 4.76Z'/><circle cx='12' cy='12' r='3'/></svg>"#;
pub(crate) const SIDEBAR_COLLAPSE_ICON: &[u8] = br#"<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 24 24' fill='none' stroke='#000' stroke-width='1.8' stroke-linecap='square' stroke-linejoin='miter'><rect x='3' y='4' width='18' height='16'/><path d='M9 4v16M16 8l-4 4 4 4'/></svg>"#;

const SVG_ASSETS: &[(&str, &[u8])] = &[
    ("icons/split-horizontal.svg", SPLIT_HORIZONTAL_ICON),
    ("icons/split-vertical.svg", SPLIT_VERTICAL_ICON),
    ("icons/maximize.svg", MAXIMIZE_ICON),
    ("icons/restore.svg", RESTORE_ICON),
    ("icons/close.svg", CLOSE_ICON),
    ("icons/connect.svg", CONNECT_ICON),
    ("icons/theme.svg", THEME_ICON),
    ("icons/notification.svg", NOTIFICATION_ICON),
    ("icons/plus.svg", PLUS_ICON),
    ("icons/folder.svg", FOLDER_ICON),
    ("icons/settings.svg", SETTINGS_ICON),
    ("icons/sidebar-collapse.svg", SIDEBAR_COLLAPSE_ICON),
];

pub(crate) fn svg_asset(path: &str) -> Option<&'static [u8]> {
    SVG_ASSETS
        .iter()
        .find_map(|(asset_path, data)| (*asset_path == path).then_some(*data))
}

pub(crate) fn panel_icon(data: &[u8], color: u32) -> Svg {
    let path = SVG_ASSETS
        .iter()
        .find_map(|(path, bytes)| (*bytes == data).then_some(*path))
        .expect("panel icon must be registered");
    svg().path(path).size(ui_px(15.)).text_color(rgba(color))
}
