//! Settings page and appearance controls.
use crate::app::MuxlaneApp;
use crate::i18n::{self, Language};
use crate::shortcuts::{self, ShortcutAction, ShortcutError};
use crate::theme::{Theme, ThemeMode};
use crate::ui_scale::px as ui_px;
use crate::widgets::semantic_button;
use gpui::{
    deferred, div, prelude::*, relative, rgba, Context, Div, Focusable, MouseButton, ParentElement,
    Stateful, Styled, Window,
};

pub(crate) const FONT_FAMILIES: &[&str] = &[
    "Noto Sans Mono",
    "JetBrains Mono",
    "Iosevka",
    "DejaVu Sans Mono",
    "Liberation Mono",
];
pub(crate) const DEFAULT_FONT_FAMILY: &str = "Noto Sans Mono";

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum SettingsPage {
    General,
    Appearance,
    Shortcuts,
}

#[derive(Default)]
pub(crate) struct SettingsFocus {
    handles: std::collections::HashMap<String, gpui::FocusHandle>,
    order: Vec<gpui::FocusHandle>,
    previous: Option<gpui::WeakFocusHandle>,
}

impl SettingsFocus {
    #[cfg(test)]
    pub(crate) fn existing_control(&self, id: &str) -> gpui::FocusHandle {
        self.handles
            .get(id)
            .unwrap_or_else(|| panic!("missing settings control: {id}"))
            .clone()
    }

    pub(crate) fn control(&mut self, id: impl Into<String>, cx: &gpui::App) -> gpui::FocusHandle {
        let handle = self
            .handles
            .entry(id.into())
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone();
        if !self.order.contains(&handle) {
            self.order.push(handle.clone());
        }
        handle
    }

    fn cycle(&self, backwards: bool, window: &mut Window, cx: &mut gpui::App) {
        let len = self.order.len();
        if len == 0 {
            return;
        }
        let current = self
            .order
            .iter()
            .position(|handle| handle.is_focused(window));
        let index = match current {
            Some(index) if backwards => (index + len - 1) % len,
            Some(index) => (index + 1) % len,
            None if backwards => len - 1,
            None => 0,
        };
        self.order[index].focus(window, cx);
    }
}

fn setting_row(
    id: &'static str,
    title: &'static str,
    description: Option<&'static str>,
    control: impl IntoElement,
    theme: Theme,
) -> Stateful<Div> {
    div()
        .id(id)
        .w_full()
        .px_6()
        .py_3()
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .border_b_1()
        .border_color(rgba(theme.line))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .child(
                    div()
                        .text_size(ui_px(12.))
                        .text_color(rgba(theme.fg0))
                        .child(title),
                )
                .when_some(description, |labels, description| {
                    labels.child(
                        div()
                            .mt(ui_px(2.))
                            .text_size(ui_px(10.))
                            .text_color(rgba(theme.fg2))
                            .child(description),
                    )
                }),
        )
        .child(div().flex_none().child(control))
}

fn render_switch(id: &'static str, label: &'static str, on: bool, theme: Theme) -> Stateful<Div> {
    semantic_button(id, label, theme)
        .w(ui_px(40.))
        .h(ui_px(32.))
        .flex_none()
        .flex()
        .items_center()
        .justify_end()
        .cursor_pointer()
        .hover(|style| style.bg(rgba(theme.bg2)))
        .child(
            div()
                .relative()
                .w(ui_px(28.))
                .h(ui_px(16.))
                .border_1()
                .border_color(rgba(if on { theme.accent } else { theme.line }))
                .bg(rgba(if on { theme.accent } else { theme.bg0 }))
                .child(
                    div()
                        .absolute()
                        .top(ui_px(2.))
                        .when(on, |thumb| thumb.right(ui_px(2.)))
                        .when(!on, |thumb| thumb.left(ui_px(2.)))
                        .w(ui_px(10.))
                        .h(ui_px(10.))
                        .bg(rgba(if on { theme.on_accent } else { theme.fg2 })),
                ),
        )
}

impl MuxlaneApp {
    pub(crate) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.settings_open {
            return;
        }
        self.settings_focus.previous = window.focused(cx).map(|focus| focus.downgrade());
        self.settings_open = true;
        self.settings_scale_input.update(cx, |input, cx| {
            input.set_text(crate::ui_scale::percent().to_string(), cx);
        });
        self.settings_scale_error = false;
        self.palette_open = false;
        cx.notify();
    }

    pub(crate) fn cycle_settings_focus(
        &mut self,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.settings_focus.cycle(backwards, window, cx);
    }

    pub(crate) fn toggle_theme(&mut self, cx: &mut Context<Self>) {
        self.theme_mode = if self.theme_mode.is_dark() {
            ThemeMode::Light
        } else {
            ThemeMode::Dark
        };
        self.apply_theme_to_inputs(cx);
        self.persist();
        cx.notify();
    }

    fn apply_theme_to_inputs(&mut self, cx: &mut Context<Self>) {
        let mode = self.theme_mode;
        self.notifications.update(cx, |center, cx| {
            center.set_appearance(mode, self.language, cx)
        });
        self.settings_scale_input
            .update(cx, |input, cx| input.set_theme_mode(mode, cx));
        self.palette_input
            .update(cx, |input, cx| input.set_theme_mode(mode, cx));
        self.connect_input
            .update(cx, |input, cx| input.set_theme_mode(mode, cx));
        self.connect_username
            .update(cx, |input, cx| input.set_theme_mode(mode, cx));
        self.connect_password
            .update(cx, |input, cx| input.set_theme_mode(mode, cx));
        self.connect_key_path
            .update(cx, |input, cx| input.set_theme_mode(mode, cx));
        self.project_input
            .update(cx, |input, cx| input.set_theme_mode(mode, cx));
        self.remote_project_input
            .update(cx, |input, cx| input.set_theme_mode(mode, cx));
        let theme = Theme::for_mode(self.theme_mode);
        for term in self.terms.values() {
            term.update(cx, |term, cx| term.set_theme(theme, cx));
        }
    }

    fn dismiss_settings_menus(&mut self) {
        self.settings_theme_menu = false;
        self.settings_font_menu = false;
        self.settings_language_menu = false;
        self.settings_terminal_preset_menu = false;
        self.settings_scale_menu = false;
    }

    fn set_theme(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        self.theme_mode = mode;
        self.dismiss_settings_menus();
        self.apply_theme_to_inputs(cx);
        self.persist();
        cx.notify();
    }

    fn set_font_family(&mut self, font_family: &str, cx: &mut Context<Self>) {
        self.font_family = font_family.to_string();
        self.dismiss_settings_menus();
        let family = self.font_family.clone();
        let theme = Theme::for_mode(self.theme_mode);
        for term in self.terms.values() {
            let family = family.clone();
            term.update(cx, |term, cx| {
                term.set_font_family(family, cx);
                term.set_theme(theme, cx);
            });
        }
        self.persist();
        cx.notify();
    }

    pub(crate) fn apply_custom_ui_scale(&mut self, cx: &mut Context<Self>) {
        let text = self.settings_scale_input.read(cx).text();
        if let Some(percent) = crate::ui_scale::parse_percent(&text) {
            self.set_ui_scale(percent, cx);
        } else {
            self.settings_scale_error = true;
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) fn set_ui_scale_for_test(&mut self, percent: u32, cx: &mut Context<Self>) {
        self.set_ui_scale(percent, cx);
    }

    fn set_ui_scale(&mut self, percent: u32, cx: &mut Context<Self>) {
        crate::ui_scale::set_percent(percent);
        self.settings_scale_input.update(cx, |input, cx| {
            input.set_text(crate::ui_scale::percent().to_string(), cx);
        });
        self.settings_scale_error = false;
        self.dismiss_settings_menus();
        for term in self.terms.values() {
            term.update(cx, |term, cx| term.refresh_ui_scale(cx));
        }
        self.persist();
        cx.notify();
    }

    fn set_default_terminal_preset(&mut self, id: &str, cx: &mut Context<Self>) {
        if !self.presets.iter().any(|preset| preset.id == id) {
            return;
        }
        self.default_terminal_preset = id.to_string();
        self.dismiss_settings_menus();
        self.persist();
        cx.notify();
    }

    fn set_language(&mut self, language: Language, cx: &mut Context<Self>) {
        self.language = language;
        self.notifications.update(cx, |center, cx| {
            center.set_appearance(self.theme_mode, language, cx)
        });
        for (input, key) in [
            (&self.palette_input, "palette.placeholder"),
            (&self.connect_input, "placeholder.remote_target"),
            (&self.connect_username, "placeholder.username"),
            (&self.connect_password, "placeholder.password"),
            (&self.connect_key_path, "placeholder.private_key"),
        ] {
            input.update(cx, |input, cx| {
                input.set_placeholder(i18n::text(language, key), cx)
            });
        }
        self.dismiss_settings_menus();
        self.persist();
        cx.notify();
    }

    pub(crate) fn close_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_shortcut_capture();
        self.settings_open = false;
        self.dismiss_settings_menus();
        if let Some(previous) = self
            .settings_focus
            .previous
            .take()
            .and_then(|focus| focus.upgrade())
            .filter(|focus| self.focus.contains(focus, window))
        {
            previous.focus(window, cx);
        } else if let Some(active) = self
            .active
            .clone()
            .filter(|agent| self.terms.contains_key(agent))
        {
            self.focus_agent(&active, window, cx);
        } else {
            self.focus.focus(window, cx);
        }
        cx.notify();
    }

    fn cancel_shortcut_capture(&mut self) {
        self.shortcut_capture_subscription.take();
        self.shortcut_capture = None;
    }

    fn start_shortcut_capture(&mut self, action: ShortcutAction, cx: &mut Context<Self>) {
        self.cancel_shortcut_capture();
        self.shortcut_capture = Some(action);
        self.shortcut_error = None;
        let listener = cx.listener(|this, event: &gpui::KeystrokeEvent, window, cx| {
            cx.stop_propagation();
            if matches!(event.keystroke.key.as_str(), "tab" | "f6") {
                this.cycle_settings_focus(event.keystroke.modifiers.shift, window, cx);
                return;
            }
            if event.keystroke.key.as_str() == "escape" {
                this.cancel_shortcut_capture();
                cx.notify();
                return;
            }
            let Some(action) = this.shortcut_capture else {
                return;
            };
            match shortcuts::captured_chord(&event.keystroke) {
                Ok(chord) => this.apply_shortcut_binding(action, Some(chord), cx),
                Err(error) => {
                    this.shortcut_error = Some(error);
                    cx.notify();
                }
            }
        });
        self.shortcut_capture_subscription = Some(cx.intercept_keystrokes(listener));
        cx.notify();
    }

    fn apply_shortcut_binding(
        &mut self,
        action: ShortcutAction,
        binding: Option<String>,
        cx: &mut Context<Self>,
    ) {
        match shortcuts::install_binding(cx, &self.shortcut_bindings, action, binding) {
            Ok(normalized) => {
                self.shortcut_bindings = normalized;
                self.shortcut_error = None;
                self.cancel_shortcut_capture();
                self.persist();
            }
            Err(error) => self.shortcut_error = Some(error),
        }
        cx.notify();
    }

    fn restore_default_shortcuts(&mut self, cx: &mut Context<Self>) {
        self.cancel_shortcut_capture();
        let defaults = muxlane_store::PersistedShortcutBindings::default();
        match shortcuts::install_keymap(cx, &defaults) {
            Ok(normalized) => {
                self.shortcut_bindings = normalized;
                self.shortcut_error = None;
                self.persist();
            }
            Err(error) => self.shortcut_error = Some(error),
        }
        cx.notify();
    }

    fn shortcut_error_text(&self) -> Option<String> {
        self.shortcut_error.as_ref().map(|error| match error {
            ShortcutError::Invalid => {
                i18n::text(self.language, "settings.shortcut_error_invalid").into()
            }
            ShortcutError::MultipleChords => {
                i18n::text(self.language, "settings.shortcut_error_multiple").into()
            }
            ShortcutError::Conflict(chord) => {
                i18n::text(self.language, "settings.shortcut_error_conflict")
                    .replace("{shortcut}", chord)
            }
        })
    }

    fn render_general_settings(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let project_workspaces_enabled = self.workspace.enabled();
        let preset_select = self.render_terminal_preset_select(cx);

        div()
            .flex()
            .flex_col()
            .w_full()
            .child(
                div()
                    .px_6()
                    .pt_4()
                    .pb_3()
                    .text_size(ui_px(15.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgba(theme.fg0))
                    .child(i18n::text(self.language, "settings.general")),
            )
            .child(setting_row(
                "settings-row-terminal-preset",
                i18n::text(self.language, "settings.default_terminal_preset"),
                None,
                preset_select,
                theme,
            ))
            .child(setting_row(
                "settings-row-project-workspaces",
                i18n::text(self.language, "settings.project_workspaces"),
                Some(i18n::text(
                    self.language,
                    "settings.project_workspaces_help",
                )),
                render_switch(
                    "settings-project-workspaces-toggle",
                    i18n::text(self.language, "settings.project_workspaces"),
                    project_workspaces_enabled,
                    theme,
                )
                .track_focus(
                    &self
                        .settings_focus
                        .control("settings-project-workspaces-toggle", cx),
                )
                .on_click(cx.listener(|this, _event, window, cx| {
                    this.set_project_workspaces_enabled(!this.workspace.enabled(), window, cx);
                })),
                theme,
            ))
            .child(setting_row(
                "settings-row-notification-sound",
                i18n::text(self.language, "settings.notification_sound"),
                Some(i18n::text(
                    self.language,
                    "settings.notification_sound_help",
                )),
                render_switch(
                    "settings-sound-toggle",
                    i18n::text(self.language, "settings.notification_sound"),
                    self.sound_enabled,
                    theme,
                )
                .track_focus(&self.settings_focus.control("settings-sound-toggle", cx))
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.sound_enabled = !this.sound_enabled;
                    this.persist();
                    cx.notify();
                })),
                theme,
            ))
            .child(setting_row(
                "settings-row-osc52",
                i18n::text(self.language, "settings.osc52"),
                Some(i18n::text(self.language, "settings.osc52_help")),
                render_switch(
                    "settings-osc52-toggle",
                    i18n::text(self.language, "settings.osc52"),
                    self.osc52_clipboard_enabled,
                    theme,
                )
                .track_focus(&self.settings_focus.control("settings-osc52-toggle", cx))
                .on_click(cx.listener(|this, _event, _window, cx| {
                    this.toggle_osc52_clipboard(cx);
                })),
                theme,
            ))
            .into_any_element()
    }

    fn render_terminal_preset_select(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let selected = self.selected_terminal_preset();
        let presets = self.presets.clone();
        div()
            .relative()
            .child(
                semantic_button(
                    "settings-terminal-preset-select",
                    selected.label.clone(),
                    theme,
                )
                .track_focus(
                    &self
                        .settings_focus
                        .control("settings-terminal-preset-select", cx),
                )
                .w(ui_px(180.))
                .h(ui_px(28.))
                .px_2()
                .flex()
                .items_center()
                .border_1()
                .border_color(rgba(theme.line))
                .bg(rgba(theme.bg0))
                .hover(|style| style.bg(rgba(theme.bg2)))
                .on_click(cx.listener(|this, _event, _window, cx| {
                    let open = !this.settings_terminal_preset_menu;
                    this.dismiss_settings_menus();
                    this.settings_terminal_preset_menu = open;
                    cx.notify();
                }))
                .child(
                    div()
                        .flex_1()
                        .text_size(ui_px(11.))
                        .text_color(rgba(theme.fg0))
                        .child(selected.label),
                )
                .child(
                    div()
                        .text_size(ui_px(12.))
                        .text_color(rgba(theme.fg1))
                        .child(if self.settings_terminal_preset_menu {
                            "⌃"
                        } else {
                            "⌄"
                        }),
                ),
            )
            .when(self.settings_terminal_preset_menu, |anchor| {
                anchor.child(
                    deferred(
                        div()
                            .id("settings-terminal-preset-menu")
                            .absolute()
                            .top_full()
                            .left_0()
                            .w(ui_px(180.))
                            .max_h(ui_px(280.))
                            .overflow_y_scroll()
                            .bg(rgba(theme.bg1))
                            .border_1()
                            .border_color(rgba(theme.line))
                            .shadow_lg()
                            .occlude()
                            .on_mouse_down_out(cx.listener(|this, _event, _window, cx| {
                                this.dismiss_settings_menus();
                                cx.notify();
                            }))
                            .on_any_mouse_down(cx.listener(
                                |_this, _event: &gpui::MouseDownEvent, _window, cx| {
                                    cx.stop_propagation();
                                },
                            ))
                            .children(presets.into_iter().map(|preset| {
                                let is_selected = preset.id == selected.id;
                                let id = format!("settings-terminal-preset-option-{}", preset.id);
                                semantic_button(
                                    gpui::ElementId::Name(id.clone().into()),
                                    preset.label.clone(),
                                    theme,
                                )
                                .track_focus(&self.settings_focus.control(id, cx))
                                .h(ui_px(28.))
                                .px_2()
                                .flex()
                                .items_center()
                                .when(is_selected, |item| item.bg(rgba(theme.selection())))
                                .when(!is_selected, |item| {
                                    item.hover(|style| style.bg(rgba(theme.bg2)))
                                })
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.set_default_terminal_preset(&preset.id, cx);
                                }))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(ui_px(11.))
                                        .text_color(rgba(theme.fg0))
                                        .child(preset.label),
                                )
                                .child(
                                    div()
                                        .text_size(ui_px(11.))
                                        .text_color(rgba(theme.accent))
                                        .child(if is_selected { "✓" } else { "" }),
                                )
                            })),
                    )
                    .with_priority(1),
                )
            })
            .into_any_element()
    }

    fn render_appearance_settings(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let current_mode = self.theme_mode;
        let current_font = self.font_family.clone();
        let current_language = self.language;

        let theme_select =
            div()
                .relative()
                .child(
                    semantic_button(
                        "settings-theme-select",
                        self.theme_mode.label(self.language),
                        theme,
                    )
                    .track_focus(&self.settings_focus.control("settings-theme-select", cx))
                    .w(ui_px(210.))
                    .h(ui_px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .gap_2()
                    .border_1()
                    .border_color(rgba(theme.line))
                    .bg(rgba(theme.bg0))
                    .hover(|style| style.bg(rgba(theme.bg2)))
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        let open = !this.settings_theme_menu;
                        this.dismiss_settings_menus();
                        this.settings_theme_menu = open;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .w(ui_px(24.))
                            .h(ui_px(16.))
                            .bg(rgba(theme.bg0))
                            .border_1()
                            .border_color(rgba(theme.accent)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .text_size(ui_px(11.))
                            .text_color(rgba(theme.fg0))
                            .child(self.theme_mode.label(self.language)),
                    )
                    .child(
                        div()
                            .text_size(ui_px(12.))
                            .text_color(rgba(theme.fg1))
                            .child(if self.settings_theme_menu {
                                "⌃"
                            } else {
                                "⌄"
                            }),
                    ),
                )
                .when(self.settings_theme_menu, |anchor| {
                    anchor.child(
                        deferred(
                            div()
                                .id("settings-theme-menu")
                                .absolute()
                                .top_full()
                                .left_0()
                                .w(ui_px(210.))
                                .max_h(ui_px(280.))
                                .overflow_y_scroll()
                                .bg(rgba(theme.bg1))
                                .border_1()
                                .border_color(rgba(theme.line))
                                .shadow_lg()
                                .occlude()
                                .on_mouse_down_out(cx.listener(|this, _event, _window, cx| {
                                    this.dismiss_settings_menus();
                                    cx.notify();
                                }))
                                .on_any_mouse_down(cx.listener(
                                    |_this, _event: &gpui::MouseDownEvent, _window, cx| {
                                        cx.stop_propagation();
                                    },
                                ))
                                .children(ThemeMode::ALL.into_iter().map(|mode| {
                                    let selected = mode == current_mode;
                                    let swatch = Theme::for_mode(mode);
                                    semantic_button(
                                        gpui::ElementId::Name(
                                            format!("settings-theme-option-{}", mode.id()).into(),
                                        ),
                                        mode.label(current_language),
                                        theme,
                                    )
                                    .track_focus(&self.settings_focus.control(
                                        format!("settings-theme-option-{}", mode.id()),
                                        cx,
                                    ))
                                    .h(ui_px(28.))
                                    .px_2()
                                    .flex()
                                    .items_center()
                                    .gap_2()
                                    .when(selected, |item| item.bg(rgba(theme.selection())))
                                    .when(!selected, |item| {
                                        item.hover(|style| style.bg(rgba(theme.bg2)))
                                    })
                                    .on_click(cx.listener(move |this, _event, _window, cx| {
                                        this.set_theme(mode, cx);
                                    }))
                                    .child(
                                        div()
                                            .w(ui_px(22.))
                                            .h(ui_px(14.))
                                            .bg(rgba(swatch.bg0))
                                            .border_1()
                                            .border_color(rgba(swatch.accent)),
                                    )
                                    .child(
                                        div()
                                            .flex_1()
                                            .text_size(ui_px(11.))
                                            .text_color(rgba(theme.fg0))
                                            .child(mode.label(current_language)),
                                    )
                                    .child(
                                        div()
                                            .text_size(ui_px(11.))
                                            .text_color(rgba(theme.accent))
                                            .child(if selected { "✓" } else { "" }),
                                    )
                                })),
                        )
                        .with_priority(1),
                    )
                });

        let font_select = div()
            .relative()
            .child(
                semantic_button("settings-font-select", self.font_family.clone(), theme)
                    .track_focus(&self.settings_focus.control("settings-font-select", cx))
                    .w(ui_px(260.))
                    .h(ui_px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .border_1()
                    .border_color(rgba(theme.line))
                    .bg(rgba(theme.bg0))
                    .hover(|style| style.bg(rgba(theme.bg2)))
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        let open = !this.settings_font_menu;
                        this.dismiss_settings_menus();
                        this.settings_font_menu = open;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex_1()
                            .text_size(ui_px(11.))
                            .text_color(rgba(theme.fg0))
                            .font_family(self.font_family.clone())
                            .child(self.font_family.clone()),
                    )
                    .child(
                        div()
                            .text_size(ui_px(12.))
                            .text_color(rgba(theme.fg1))
                            .child(if self.settings_font_menu {
                                "⌃"
                            } else {
                                "⌄"
                            }),
                    ),
            )
            .when(self.settings_font_menu, |anchor| {
                anchor.child(
                    deferred(
                        div()
                            .id("settings-font-menu")
                            .absolute()
                            .top_full()
                            .left_0()
                            .w(ui_px(260.))
                            .max_h(ui_px(280.))
                            .overflow_y_scroll()
                            .bg(rgba(theme.bg1))
                            .border_1()
                            .border_color(rgba(theme.line))
                            .shadow_lg()
                            .occlude()
                            .on_mouse_down_out(cx.listener(|this, _event, _window, cx| {
                                this.dismiss_settings_menus();
                                cx.notify();
                            }))
                            .on_any_mouse_down(cx.listener(
                                |_this, _event: &gpui::MouseDownEvent, _window, cx| {
                                    cx.stop_propagation();
                                },
                            ))
                            .children(FONT_FAMILIES.iter().map(|family| {
                                let selected = current_font == *family;
                                let family = (*family).to_string();
                                semantic_button(
                                    gpui::ElementId::Name(
                                        format!(
                                            "settings-font-option-{}",
                                            family.replace(' ', "-")
                                        )
                                        .into(),
                                    ),
                                    family.clone(),
                                    theme,
                                )
                                .track_focus(
                                    &self
                                        .settings_focus
                                        .control(format!("settings-font-option-{family}"), cx),
                                )
                                .h(ui_px(28.))
                                .px_2()
                                .flex()
                                .items_center()
                                .when(selected, |item| item.bg(rgba(theme.selection())))
                                .when(!selected, |item| {
                                    item.hover(|style| style.bg(rgba(theme.bg2)))
                                })
                                .on_click(cx.listener({
                                    let family = family.clone();
                                    move |this, _event, _window, cx| {
                                        this.set_font_family(&family, cx);
                                    }
                                }))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(ui_px(11.))
                                        .text_color(rgba(theme.fg0))
                                        .font_family(family.clone())
                                        .child(family),
                                )
                                .child(
                                    div()
                                        .text_size(ui_px(11.))
                                        .text_color(rgba(theme.accent))
                                        .child(if selected { "✓" } else { "" }),
                                )
                            })),
                    )
                    .with_priority(1),
                )
            });

        let scale_select = {
            let current_scale = crate::ui_scale::percent();
            div()
                .relative()
                .child(
                    semantic_button("settings-scale-select", format!("{current_scale}%"), theme)
                        .when(cfg!(test), |button| {
                            button.debug_selector(|| "ux-scale-select".into())
                        })
                        .track_focus(&self.settings_focus.control("settings-scale-select", cx))
                        .w(ui_px(180.))
                        .h(ui_px(28.))
                        .px_2()
                        .flex()
                        .items_center()
                        .border_1()
                        .border_color(rgba(theme.line))
                        .bg(rgba(theme.bg0))
                        .hover(|style| style.bg(rgba(theme.bg2)))
                        .on_click(cx.listener(|this, _event, _window, cx| {
                            let open = !this.settings_scale_menu;
                            this.dismiss_settings_menus();
                            this.settings_scale_menu = open;
                            cx.notify();
                        }))
                        .child(
                            div()
                                .flex_1()
                                .text_size(ui_px(11.))
                                .text_color(rgba(theme.fg0))
                                .child(format!("{current_scale}%")),
                        )
                        .child(
                            div()
                                .text_size(ui_px(12.))
                                .text_color(rgba(theme.fg1))
                                .child(if self.settings_scale_menu {
                                    "⌃"
                                } else {
                                    "⌄"
                                }),
                        ),
                )
                .when(self.settings_scale_menu, |anchor| {
                    anchor.child(
                        deferred(
                            div()
                                .id("settings-scale-menu")
                                .absolute()
                                .top_full()
                                .left_0()
                                .w(ui_px(180.))
                                .max_h(ui_px(280.))
                                .overflow_y_scroll()
                                .bg(rgba(theme.bg1))
                                .border_1()
                                .border_color(rgba(theme.line))
                                .shadow_lg()
                                .occlude()
                                .on_mouse_down_out(cx.listener(|this, _event, _window, cx| {
                                    this.dismiss_settings_menus();
                                    cx.notify();
                                }))
                                .on_any_mouse_down(cx.listener(
                                    |_this, _event: &gpui::MouseDownEvent, _window, cx| {
                                        cx.stop_propagation();
                                    },
                                ))
                                .children(
                                    (crate::ui_scale::MIN_PERCENT..=crate::ui_scale::MAX_PERCENT)
                                        .step_by(crate::ui_scale::STEP_PERCENT as usize)
                                        .map(|percent| {
                                            let selected = percent == current_scale;
                                            semantic_button(
                                                gpui::ElementId::Name(
                                                    format!("settings-scale-option-{percent}")
                                                        .into(),
                                                ),
                                                format!("{percent}%"),
                                                theme,
                                            )
                                            .when(cfg!(test), |button| {
                                                button.debug_selector(move || {
                                                    format!("ux-scale-option-{percent}").into()
                                                })
                                            })
                                            .track_focus(&self.settings_focus.control(
                                                format!("settings-scale-option-{percent}"),
                                                cx,
                                            ))
                                            .h(ui_px(28.))
                                            .px_2()
                                            .flex()
                                            .items_center()
                                            .when(selected, |item| item.bg(rgba(theme.selection())))
                                            .when(!selected, |item| {
                                                item.hover(|style| style.bg(rgba(theme.bg2)))
                                            })
                                            .on_click(cx.listener(
                                                move |this, _event, _window, cx| {
                                                    this.set_ui_scale(percent, cx);
                                                },
                                            ))
                                            .child(
                                                div()
                                                    .flex_1()
                                                    .text_size(ui_px(11.))
                                                    .text_color(rgba(theme.fg0))
                                                    .child(format!("{percent}%")),
                                            )
                                        }),
                                ),
                        )
                        .with_priority(1),
                    )
                })
        };

        let input_focus = self.settings_scale_input.focus_handle(cx);
        self.settings_focus.order.push(input_focus);
        let apply_label = i18n::text(self.language, "settings.ui_scale_apply");
        let scale_select = div()
            .w(ui_px(180.))
            .flex()
            .flex_col()
            .gap_1()
            .child(scale_select)
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_1()
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        if event.keystroke.key == "enter"
                            && this
                                .settings_scale_input
                                .focus_handle(cx)
                                .is_focused(window)
                        {
                            this.apply_custom_ui_scale(cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        div()
                            .id("settings-scale-input")
                            .when(cfg!(test), |input| {
                                input.debug_selector(|| "ux-scale-input".into())
                            })
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .child(self.settings_scale_input.clone()),
                    )
                    .child(div().text_size(ui_px(11.)).child("%"))
                    .child(
                        semantic_button("settings-scale-apply", apply_label, theme)
                            .when(cfg!(test), |button| {
                                button.debug_selector(|| "ux-scale-apply".into())
                            })
                            .track_focus(&self.settings_focus.control("settings-scale-apply", cx))
                            .h(ui_px(34.))
                            .px_2()
                            .flex_none()
                            .flex()
                            .items_center()
                            .text_size(ui_px(11.))
                            .border_1()
                            .border_color(rgba(theme.line))
                            .bg(rgba(theme.bg0))
                            .hover(|style| style.bg(rgba(theme.bg2)))
                            .on_click(cx.listener(|this, _, _, cx| this.apply_custom_ui_scale(cx)))
                            .child(apply_label),
                    ),
            )
            .when(self.settings_scale_error, |control| {
                control.child(
                    div()
                        .text_size(ui_px(10.))
                        .text_color(rgba(theme.red))
                        .child(i18n::text(self.language, "settings.ui_scale_invalid")),
                )
            });

        let language_select = div()
            .relative()
            .child(
                semantic_button("settings-language-select", self.language.label(), theme)
                    .track_focus(&self.settings_focus.control("settings-language-select", cx))
                    .w(ui_px(180.))
                    .h(ui_px(28.))
                    .px_2()
                    .flex()
                    .items_center()
                    .border_1()
                    .border_color(rgba(theme.line))
                    .bg(rgba(theme.bg0))
                    .hover(|style| style.bg(rgba(theme.bg2)))
                    .on_click(cx.listener(|this, _event, _window, cx| {
                        let open = !this.settings_language_menu;
                        this.dismiss_settings_menus();
                        this.settings_language_menu = open;
                        cx.notify();
                    }))
                    .child(
                        div()
                            .flex_1()
                            .text_size(ui_px(11.))
                            .text_color(rgba(theme.fg0))
                            .child(self.language.label()),
                    )
                    .child(
                        div()
                            .text_size(ui_px(12.))
                            .text_color(rgba(theme.fg1))
                            .child(if self.settings_language_menu {
                                "⌃"
                            } else {
                                "⌄"
                            }),
                    ),
            )
            .when(self.settings_language_menu, |anchor| {
                anchor.child(
                    deferred(
                        div()
                            .id("settings-language-menu")
                            .absolute()
                            .top_full()
                            .left_0()
                            .w(ui_px(180.))
                            .bg(rgba(theme.bg1))
                            .border_1()
                            .border_color(rgba(theme.line))
                            .shadow_lg()
                            .occlude()
                            .on_mouse_down_out(cx.listener(|this, _event, _window, cx| {
                                this.dismiss_settings_menus();
                                cx.notify();
                            }))
                            .on_any_mouse_down(cx.listener(
                                |_this, _event: &gpui::MouseDownEvent, _window, cx| {
                                    cx.stop_propagation();
                                },
                            ))
                            .children(Language::ALL.into_iter().map(|language| {
                                let selected = language == current_language;
                                semantic_button(
                                    gpui::ElementId::Name(
                                        format!("settings-language-option-{}", language.id())
                                            .into(),
                                    ),
                                    language.label(),
                                    theme,
                                )
                                .track_focus(&self.settings_focus.control(
                                    format!("settings-language-option-{}", language.id()),
                                    cx,
                                ))
                                .h(ui_px(28.))
                                .px_2()
                                .flex()
                                .items_center()
                                .when(selected, |item| item.bg(rgba(theme.selection())))
                                .when(!selected, |item| {
                                    item.hover(|style| style.bg(rgba(theme.bg2)))
                                })
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.set_language(language, cx);
                                }))
                                .child(
                                    div()
                                        .flex_1()
                                        .text_size(ui_px(11.))
                                        .text_color(rgba(theme.fg0))
                                        .child(language.label()),
                                )
                                .child(
                                    div()
                                        .text_size(ui_px(11.))
                                        .text_color(rgba(theme.accent))
                                        .child(if selected { "✓" } else { "" }),
                                )
                            })),
                    )
                    .with_priority(1),
                )
            });

        div()
            .flex()
            .flex_col()
            .w_full()
            .child(
                div()
                    .px_6()
                    .pt_4()
                    .pb_3()
                    .text_size(ui_px(15.))
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .text_color(rgba(theme.fg0))
                    .child(i18n::text(self.language, "settings.appearance")),
            )
            .child(setting_row(
                "settings-row-interface-theme",
                i18n::text(self.language, "settings.interface_theme"),
                Some(i18n::text(self.language, "settings.interface_theme_help")),
                theme_select,
                theme,
            ))
            .child(setting_row(
                "settings-row-terminal-font",
                i18n::text(self.language, "settings.terminal_font"),
                Some(i18n::text(self.language, "settings.terminal_font_help")),
                font_select,
                theme,
            ))
            .child(setting_row(
                "settings-row-terminal-scale",
                i18n::text(self.language, "settings.ui_scale"),
                Some(i18n::text(self.language, "settings.ui_scale_help")),
                scale_select,
                theme,
            ))
            .child(setting_row(
                "settings-row-language",
                i18n::text(self.language, "settings.language"),
                Some(i18n::text(self.language, "settings.language_help")),
                language_select,
                theme,
            ))
            .into_any_element()
    }

    fn render_shortcuts_settings(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let theme = Theme::for_mode(self.theme_mode);
        let shortcut_bindings = self.shortcut_bindings.clone();
        let shortcut_capture = self.shortcut_capture;
        let shortcut_error = self.shortcut_error_text();

        div()
            .flex()
            .flex_col()
            .w_full()
            .child(
                div()
                    .px_6()
                    .pt_4()
                    .pb_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(ui_px(15.))
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .text_color(rgba(theme.fg0))
                            .child(i18n::text(self.language, "settings.shortcuts")),
                    )
                    .child(
                        semantic_button(
                            "settings-shortcuts-restore",
                            i18n::text(self.language, "settings.shortcuts_restore"),
                            theme,
                        )
                        .track_focus(
                            &self
                                .settings_focus
                                .control("settings-shortcuts-restore", cx),
                        )
                        .h(ui_px(26.))
                        .px_2()
                        .flex()
                        .items_center()
                        .border_1()
                        .border_color(rgba(theme.line))
                        .text_size(ui_px(10.))
                        .text_color(rgba(theme.fg1))
                        .hover(|style| style.bg(rgba(theme.bg2)))
                        .on_click(cx.listener(|this, _event, _window, cx| {
                            this.restore_default_shortcuts(cx);
                        }))
                        .child(i18n::text(self.language, "settings.shortcuts_restore")),
                    ),
            )
            .child(
                div()
                    .px_6()
                    .pb_2()
                    .text_size(ui_px(10.))
                    .text_color(rgba(theme.fg2))
                    .child(i18n::text(self.language, "settings.shortcuts_help")),
            )
            .children(ShortcutAction::ALL.into_iter().map(|action| {
                let recording = shortcut_capture == Some(action);
                let binding = action.binding(&shortcut_bindings).clone();
                let record_id = format!("settings-shortcut-record-{action:?}");
                let clear_id = format!("settings-shortcut-clear-{action:?}");
                div()
                    .w_full()
                    .px_6()
                    .py_3()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_4()
                    .border_b_1()
                    .border_color(rgba(theme.line))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_size(ui_px(12.))
                            .text_color(rgba(theme.fg0))
                            .child(i18n::text(self.language, action.label_key())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(
                                semantic_button(
                                    gpui::ElementId::Name(record_id.clone().into()),
                                    if recording {
                                        i18n::text(self.language, "settings.shortcut_recording")
                                            .to_string()
                                    } else {
                                        binding.clone().unwrap_or_else(|| {
                                            i18n::text(self.language, "settings.shortcut_disabled")
                                                .into()
                                        })
                                    },
                                    theme,
                                )
                                .track_focus(&self.settings_focus.control(record_id, cx))
                                .w(ui_px(170.))
                                .h(ui_px(28.))
                                .px_2()
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .border_1()
                                .border_color(rgba(if recording {
                                    theme.accent
                                } else {
                                    theme.line
                                }))
                                .bg(rgba(theme.bg0))
                                .text_size(ui_px(10.))
                                .text_color(rgba(if recording { theme.accent } else { theme.fg1 }))
                                .hover(|style| style.bg(rgba(theme.bg2)))
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.start_shortcut_capture(action, cx);
                                }))
                                .child(if recording {
                                    i18n::text(self.language, "settings.shortcut_recording")
                                        .to_string()
                                } else {
                                    binding.unwrap_or_else(|| {
                                        i18n::text(self.language, "settings.shortcut_disabled")
                                            .into()
                                    })
                                }),
                            )
                            .child(
                                semantic_button(
                                    gpui::ElementId::Name(clear_id.clone().into()),
                                    i18n::text(self.language, "common.clear"),
                                    theme,
                                )
                                .track_focus(&self.settings_focus.control(clear_id, cx))
                                .w(ui_px(52.))
                                .h(ui_px(28.))
                                .flex_none()
                                .flex()
                                .items_center()
                                .justify_center()
                                .border_1()
                                .border_color(rgba(theme.line))
                                .text_size(ui_px(10.))
                                .text_color(rgba(theme.fg2))
                                .hover(|style| style.bg(rgba(theme.bg2)))
                                .on_click(cx.listener(move |this, _event, _window, cx| {
                                    this.cancel_shortcut_capture();
                                    this.apply_shortcut_binding(action, None, cx);
                                }))
                                .child(i18n::text(self.language, "common.clear")),
                            ),
                    )
            }))
            .when_some(shortcut_error, |page, error| {
                page.child(
                    div()
                        .px_6()
                        .py_2()
                        .text_size(ui_px(10.))
                        .text_color(rgba(theme.red))
                        .child(error),
                )
            })
            .into_any_element()
    }

    pub(crate) fn render_settings(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.settings_focus.order.clear();
        for id in [
            "settings-close",
            "settings-nav-general",
            "settings-nav-appearance",
            "settings-nav-shortcuts",
        ] {
            self.settings_focus.control(id, cx);
        }
        // Reconcile after painting: newly opened pages and dismissed menus may remove focus.
        let view = cx.entity().downgrade();
        window.on_next_frame(move |window, cx| {
            view.update(cx, |this, cx| {
                if this.settings_open
                    && !this
                        .settings_focus
                        .order
                        .iter()
                        .any(|focus| focus.is_focused(window))
                {
                    this.cycle_settings_focus(false, window, cx);
                }
            })
            .ok();
        });
        let theme = Theme::for_mode(self.theme_mode);
        let current_page = self.settings_page;
        let content_id = match current_page {
            SettingsPage::General => "settings-content-general",
            SettingsPage::Appearance => "settings-content-appearance",
            SettingsPage::Shortcuts => "settings-content-shortcuts",
        };
        let content = match current_page {
            SettingsPage::General => self.render_general_settings(cx),
            SettingsPage::Appearance => self.render_appearance_settings(cx),
            SettingsPage::Shortcuts => self.render_shortcuts_settings(cx),
        };

        div()
            .id("settings-backdrop")
            .absolute()
            .inset_0()
            .occlude()
            .flex()
            .items_start()
            .justify_center()
            .pt(ui_px(48.))
            .bg(rgba(theme.overlay()))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _event, window, cx| {
                    this.close_settings(window, cx);
                }),
            )
            .child(
                div()
                    .id("settings-page")
                    .relative()
                    .occlude()
                    .w(ui_px(880.))
                    .h(ui_px(600.))
                    .max_w(relative(0.92))
                    .max_h(relative(0.82))
                    .flex()
                    .flex_col()
                    .bg(rgba(theme.bg0))
                    .border_1()
                    .border_color(rgba(theme.line))
                    .shadow_lg()
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(|_this, _event, _window, cx| {
                            cx.stop_propagation();
                        }),
                    )
                    .on_key_down(cx.listener(|this, event: &gpui::KeyDownEvent, window, cx| {
                        if event.keystroke.key.as_str() == "escape" {
                            this.close_settings(window, cx);
                            cx.stop_propagation();
                        }
                    }))
                    .child(
                        div()
                            .h(ui_px(40.))
                            .px_4()
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(rgba(theme.line))
                            .child(
                                div()
                                    .text_size(ui_px(13.))
                                    .font_weight(gpui::FontWeight::SEMIBOLD)
                                    .text_color(rgba(theme.fg0))
                                    .child(i18n::text(self.language, "common.settings")),
                            )
                            .child(
                                semantic_button("settings-close", "Close settings", theme)
                                    .track_focus(&self.settings_focus.control("settings-close", cx))
                                    .w(ui_px(24.))
                                    .h(ui_px(24.))
                                    .flex()
                                    .items_center()
                                    .justify_center()
                                    .text_size(ui_px(16.))
                                    .text_color(rgba(theme.fg1))
                                    .hover(|style| {
                                        style.bg(rgba(theme.bg2)).text_color(rgba(theme.fg0))
                                    })
                                    .on_click(cx.listener(|this, _event, window, cx| {
                                        this.close_settings(window, cx);
                                    }))
                                    .child("×"),
                            ),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_1()
                            .min_h_0()
                            .child(
                                div()
                                    .w(ui_px(180.))
                                    .h_full()
                                    .flex_none()
                                    .flex()
                                    .flex_col()
                                    .py_2()
                                    .bg(rgba(theme.bg1))
                                    .border_r_1()
                                    .border_color(rgba(theme.line))
                                    .children(
                                        [
                                            (
                                                SettingsPage::General,
                                                "settings-nav-general",
                                                "settings.general",
                                            ),
                                            (
                                                SettingsPage::Appearance,
                                                "settings-nav-appearance",
                                                "settings.appearance",
                                            ),
                                            (
                                                SettingsPage::Shortcuts,
                                                "settings-nav-shortcuts",
                                                "settings.shortcuts",
                                            ),
                                        ]
                                        .into_iter()
                                        .map(
                                            |(page, id, label_key)| {
                                                let selected = page == current_page;
                                                semantic_button(
                                                    id,
                                                    i18n::text(self.language, label_key),
                                                    theme,
                                                )
                                                .track_focus(&self.settings_focus.control(id, cx))
                                                .h(ui_px(28.))
                                                .px_3()
                                                .flex()
                                                .items_center()
                                                .text_size(ui_px(12.))
                                                .text_color(rgba(if selected {
                                                    theme.fg0
                                                } else {
                                                    theme.fg1
                                                }))
                                                .when(selected, |item| {
                                                    item.bg(rgba(theme.selection()))
                                                        .font_weight(gpui::FontWeight::SEMIBOLD)
                                                })
                                                .when(!selected, |item| {
                                                    item.hover(|style| style.bg(rgba(theme.bg2)))
                                                })
                                                .on_click(cx.listener(
                                                    move |this, _event, _window, cx| {
                                                        this.settings_page = page;
                                                        this.cancel_shortcut_capture();
                                                        this.dismiss_settings_menus();
                                                        cx.notify();
                                                    },
                                                ))
                                                .child(i18n::text(self.language, label_key))
                                            },
                                        ),
                                    ),
                            )
                            .child(
                                div()
                                    .id(content_id)
                                    .flex_1()
                                    .min_w_0()
                                    .h_full()
                                    .overflow_y_scroll()
                                    .child(content),
                            ),
                    ),
            )
            .into_any_element()
    }
}

impl MuxlaneApp {
    pub(crate) fn toggle_osc52_clipboard(&mut self, cx: &mut Context<Self>) {
        self.osc52_clipboard_enabled = !self.osc52_clipboard_enabled;
        for term in self.terms.values() {
            term.update(cx, |term, _cx| {
                term.set_osc52_clipboard_enabled(self.osc52_clipboard_enabled)
            });
        }
        self.persist();
        cx.notify();
    }
}
