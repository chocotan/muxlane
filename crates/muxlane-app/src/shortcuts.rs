use crate::actions::{
    CloseTab, NewShellTab, NextTab, PreviousTab, SelectTab1, SelectTab2, SelectTab3, SelectTab4,
    SelectTab5, SelectTab6, SelectTab7, SelectTab8, SelectTab9, SplitDown, SplitRight,
    TogglePalette,
};
use gpui::{App, KeyBinding, Keystroke};
use muxlane_store::PersistedShortcutBindings;
use std::collections::HashMap;

#[cfg(target_os = "macos")]
pub(crate) const FIXED_CHORDS: &[&str] = &[
    "cmd-k",
    "ctrl-tab",
    "ctrl-shift-tab",
    "cmd-1",
    "cmd-2",
    "cmd-3",
    "cmd-4",
    "cmd-5",
    "cmd-6",
    "cmd-7",
    "cmd-8",
    "cmd-9",
];

#[cfg(not(target_os = "macos"))]
pub(crate) const FIXED_CHORDS: &[&str] = &[
    "super-k",
    "ctrl-tab",
    "ctrl-shift-tab",
    "super-1",
    "super-2",
    "super-3",
    "super-4",
    "super-5",
    "super-6",
    "super-7",
    "super-8",
    "super-9",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[allow(clippy::enum_variant_names)]
pub(crate) enum ShortcutAction {
    CloseTab,
    PreviousTab,
    NextTab,
    NewTab,
    SplitRight,
    SplitDown,
}

impl ShortcutAction {
    pub(crate) const ALL: [Self; 6] = [
        Self::CloseTab,
        Self::PreviousTab,
        Self::NextTab,
        Self::NewTab,
        Self::SplitRight,
        Self::SplitDown,
    ];

    pub(crate) fn build_binding(self, chord: &str) -> KeyBinding {
        match self {
            Self::CloseTab => KeyBinding::new(&expand_platform_chord(chord), CloseTab, None),
            Self::PreviousTab => KeyBinding::new(&expand_platform_chord(chord), PreviousTab, None),
            Self::NextTab => KeyBinding::new(&expand_platform_chord(chord), NextTab, None),
            Self::NewTab => KeyBinding::new(&expand_platform_chord(chord), NewShellTab, None),
            Self::SplitRight => KeyBinding::new(&expand_platform_chord(chord), SplitRight, None),
            Self::SplitDown => KeyBinding::new(&expand_platform_chord(chord), SplitDown, None),
        }
    }

    pub(crate) fn label_key(self) -> &'static str {
        match self {
            Self::CloseTab => "settings.shortcut.close_tab",
            Self::PreviousTab => "settings.shortcut.previous_tab",
            Self::NextTab => "settings.shortcut.next_tab",
            Self::NewTab => "settings.shortcut.new_tab",
            Self::SplitRight => "settings.shortcut.split_right",
            Self::SplitDown => "settings.shortcut.split_down",
        }
    }

    pub(crate) fn binding(self, bindings: &PersistedShortcutBindings) -> &Option<String> {
        match self {
            Self::CloseTab => &bindings.close_tab,
            Self::PreviousTab => &bindings.previous_tab,
            Self::NextTab => &bindings.next_tab,
            Self::NewTab => &bindings.new_tab,
            Self::SplitRight => &bindings.split_right,
            Self::SplitDown => &bindings.split_down,
        }
    }

    pub(crate) fn set_binding(
        self,
        bindings: &mut PersistedShortcutBindings,
        value: Option<String>,
    ) {
        match self {
            Self::CloseTab => bindings.close_tab = value,
            Self::PreviousTab => bindings.previous_tab = value,
            Self::NextTab => bindings.next_tab = value,
            Self::NewTab => bindings.new_tab = value,
            Self::SplitRight => bindings.split_right = value,
            Self::SplitDown => bindings.split_down = value,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ShortcutError {
    Invalid,
    MultipleChords,
    Conflict(String),
}

pub(crate) fn captured_chord(keystroke: &Keystroke) -> Result<String, ShortcutError> {
    canonical_chord(&keystroke.unparse())
}

fn canonical_chord(source: &str) -> Result<String, ShortcutError> {
    let mut chords = source.split_whitespace();
    let chord = chords.next().ok_or(ShortcutError::Invalid)?;
    if chords.next().is_some() {
        return Err(ShortcutError::MultipleChords);
    }
    let expanded = expand_platform_chord(chord);
    let parsed = Keystroke::parse(&expanded).map_err(|_| ShortcutError::Invalid)?;
    Ok(canonical_platform_chord(&parsed.unparse()))
}

fn expand_platform_chord(chord: &str) -> String {
    #[cfg(target_os = "macos")]
    let modifier = "cmd";
    #[cfg(not(target_os = "macos"))]
    let modifier = "super";
    replace_modifier(chord, "platform", modifier)
}

fn canonical_platform_chord(chord: &str) -> String {
    #[cfg(target_os = "macos")]
    let modifier = "cmd";
    #[cfg(not(target_os = "macos"))]
    let modifier = "super";
    replace_modifier(chord, modifier, "platform")
}

// GPUI emits alt-super-up, so the platform modifier need not be first.
// Retain empty tokens: the final key in ctrl-- is the literal minus key.
fn replace_modifier(chord: &str, from: &str, to: &str) -> String {
    let mut tokens: Vec<_> = chord.split("-").collect();
    let modifier_count = tokens.len().saturating_sub(1);
    for token in &mut tokens[..modifier_count] {
        if *token == from {
            *token = to;
        }
    }
    tokens.join("-")
}

pub(crate) fn normalize(
    bindings: &PersistedShortcutBindings,
) -> Result<PersistedShortcutBindings, ShortcutError> {
    let mut normalized = bindings.clone();
    let fixed: HashMap<_, _> = FIXED_CHORDS
        .iter()
        .map(|chord| {
            (
                canonical_chord(chord).expect("fixed shortcut must parse"),
                *chord,
            )
        })
        .collect();
    let mut configured = HashMap::<String, ShortcutAction>::new();

    for action in ShortcutAction::ALL {
        let Some(source) = action.binding(bindings) else {
            continue;
        };
        let chord = canonical_chord(source)?;
        if fixed.contains_key(&chord) || configured.insert(chord.clone(), action).is_some() {
            return Err(ShortcutError::Conflict(chord));
        }
        action.set_binding(&mut normalized, Some(chord));
    }

    Ok(normalized)
}

pub(crate) fn install_binding(
    cx: &mut App,
    current: &PersistedShortcutBindings,
    action: ShortcutAction,
    binding: Option<String>,
) -> Result<PersistedShortcutBindings, ShortcutError> {
    let mut candidate = current.clone();
    action.set_binding(&mut candidate, binding);
    install_keymap(cx, &candidate)
}

pub(crate) fn install_keymap(
    cx: &mut App,
    bindings: &PersistedShortcutBindings,
) -> Result<PersistedShortcutBindings, ShortcutError> {
    let normalized = normalize(bindings)?;
    let keymap = build_keymap(&normalized);
    cx.clear_key_bindings();
    cx.bind_keys(keymap);
    Ok(normalized)
}

pub(crate) fn install_keymap_or_defaults(
    cx: &mut App,
    bindings: &PersistedShortcutBindings,
) -> PersistedShortcutBindings {
    install_keymap(cx, &recover_defaults(bindings)).unwrap_or_else(|_| {
        install_keymap(cx, &PersistedShortcutBindings::default())
            .expect("default shortcuts must form a valid keymap")
    })
}

/// New defaults must not displace custom chords loaded from an older settings file.
pub(crate) fn recover_defaults(bindings: &PersistedShortcutBindings) -> PersistedShortcutBindings {
    let defaults = PersistedShortcutBindings::default();
    let mut candidate = bindings.clone();
    for action in ShortcutAction::ALL {
        if action == ShortcutAction::CloseTab {
            continue;
        }
        let Some(chord) = action
            .binding(bindings)
            .as_deref()
            .and_then(|s| canonical_chord(s).ok())
        else {
            continue;
        };
        let default = action
            .binding(&defaults)
            .as_deref()
            .and_then(|s| canonical_chord(s).ok());
        if Some(&chord) != default.as_ref() {
            continue;
        }
        let conflicts_with_custom = ShortcutAction::ALL.into_iter().any(|other| {
            other != action
                && other
                    .binding(bindings)
                    .as_deref()
                    .and_then(|s| canonical_chord(s).ok())
                    .as_ref()
                    == Some(&chord)
                && other
                    .binding(&defaults)
                    .as_deref()
                    .and_then(|s| canonical_chord(s).ok())
                    .as_ref()
                    != Some(&chord)
        });
        if conflicts_with_custom {
            action.set_binding(&mut candidate, None);
        }
    }
    normalize(&candidate).unwrap_or_default()
}

fn build_keymap(bindings: &PersistedShortcutBindings) -> Vec<KeyBinding> {
    let mut keymap = vec![
        KeyBinding::new(&expand_platform_chord("platform-k"), TogglePalette, None),
        KeyBinding::new("ctrl-tab", NextTab, None),
        KeyBinding::new("ctrl-shift-tab", PreviousTab, None),
        KeyBinding::new(&expand_platform_chord("platform-1"), SelectTab1, None),
        KeyBinding::new(&expand_platform_chord("platform-2"), SelectTab2, None),
        KeyBinding::new(&expand_platform_chord("platform-3"), SelectTab3, None),
        KeyBinding::new(&expand_platform_chord("platform-4"), SelectTab4, None),
        KeyBinding::new(&expand_platform_chord("platform-5"), SelectTab5, None),
        KeyBinding::new(&expand_platform_chord("platform-6"), SelectTab6, None),
        KeyBinding::new(&expand_platform_chord("platform-7"), SelectTab7, None),
        KeyBinding::new(&expand_platform_chord("platform-8"), SelectTab8, None),
        KeyBinding::new(&expand_platform_chord("platform-9"), SelectTab9, None),
    ];

    #[cfg(debug_assertions)]
    if std::env::var("MUXLANE_TEST_PALETTE_CTRL_K").as_deref() == Ok("1") {
        keymap.push(KeyBinding::new("ctrl-k", TogglePalette, None));
    }

    for action in ShortcutAction::ALL {
        if let Some(chord) = action.binding(bindings) {
            keymap.push(action.build_binding(chord));
        }
    }
    keymap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_cover_all_configurable_actions() {
        let defaults = normalize(&PersistedShortcutBindings::default()).unwrap();
        let chords: Vec<_> = ShortcutAction::ALL
            .into_iter()
            .map(|action| action.binding(&defaults).clone())
            .collect();
        assert_eq!(
            chords,
            [
                Some("ctrl-w".into()),
                Some("alt-platform-k".into()),
                Some("alt-platform-j".into()),
                Some("alt-platform-up".into()),
                Some("alt-platform-right".into()),
                Some("alt-platform-down".into())
            ]
        );
    }

    #[test]
    fn disabled_bindings_are_omitted_from_the_complete_keymap() {
        let bindings = PersistedShortcutBindings {
            close_tab: None,
            previous_tab: None,
            next_tab: None,
            ..Default::default()
        };
        let normalized = normalize(&bindings).unwrap();
        let remaining_configurable = ShortcutAction::ALL
            .into_iter()
            .filter(|action| action.binding(&normalized).is_some())
            .count();
        assert_eq!(
            build_keymap(&normalized).len(),
            FIXED_CHORDS.len() + remaining_configurable
        );
    }

    #[test]
    fn malformed_and_multiple_chords_are_rejected() {
        let bindings = PersistedShortcutBindings {
            close_tab: Some("ctrl--w".into()),
            ..Default::default()
        };
        assert_eq!(normalize(&bindings), Err(ShortcutError::Invalid));
        let bindings = PersistedShortcutBindings {
            close_tab: Some("ctrl-w ctrl-q".into()),
            ..Default::default()
        };
        assert_eq!(normalize(&bindings), Err(ShortcutError::MultipleChords));
    }

    #[test]
    fn configurable_duplicates_and_fixed_conflicts_are_rejected() {
        let bindings = PersistedShortcutBindings {
            next_tab: Some("platform-alt-k".into()),
            ..Default::default()
        };
        assert_eq!(
            normalize(&bindings),
            Err(ShortcutError::Conflict("alt-platform-k".into()))
        );
        let bindings = PersistedShortcutBindings {
            next_tab: Some("platform-k".into()),
            ..Default::default()
        };
        assert_eq!(
            normalize(&bindings),
            Err(ShortcutError::Conflict("platform-k".into()))
        );
    }

    #[test]
    fn candidate_update_is_validated_as_a_complete_set() {
        let current = PersistedShortcutBindings::default();
        let mut candidate = current.clone();
        ShortcutAction::CloseTab.set_binding(&mut candidate, Some("platform-k".into()));
        assert_eq!(
            normalize(&candidate),
            Err(ShortcutError::Conflict("platform-k".into()))
        );
        assert_eq!(current, PersistedShortcutBindings::default());
    }

    #[test]
    fn complete_keymap_always_contains_every_fixed_binding() {
        let bindings = PersistedShortcutBindings {
            close_tab: None,
            previous_workspace: None,
            next_workspace: None,
            previous_tab: None,
            next_tab: None,
            new_tab: None,
            split_right: None,
            split_down: None,
        };
        let keymap = build_keymap(&bindings);
        assert_eq!(keymap.len(), FIXED_CHORDS.len());
        for chord in FIXED_CHORDS {
            assert!(
                Keystroke::parse(chord).is_ok(),
                "invalid fixed chord: {chord}"
            );
        }
    }

    #[test]
    fn platform_alt_capture_roundtrips_in_any_modifier_order_and_preserves_minus() {
        for key in ["up", "down", "left", "right", "j", "k", "-"] {
            let source = format!("platform-alt-{key}");
            let expected = format!("alt-platform-{key}");
            let keystroke = Keystroke::parse(&expand_platform_chord(&source)).unwrap();
            assert_eq!(captured_chord(&keystroke).unwrap(), expected);
            assert_eq!(canonical_chord(&expected).unwrap(), expected);
            assert_eq!(canonical_chord(&source).unwrap(), expected);
        }
        assert_eq!(canonical_chord("ctrl--").unwrap(), "ctrl--");
        let bindings = PersistedShortcutBindings {
            close_tab: Some("alt-platform-up".into()),
            ..Default::default()
        };
        assert_eq!(
            normalize(&bindings),
            Err(ShortcutError::Conflict("alt-platform-up".into()))
        );
    }

    #[test]
    fn introduced_defaults_yield_to_custom_chords_without_resetting_other_preferences() {
        for action in ShortcutAction::ALL.into_iter().skip(1) {
            let mut bindings = PersistedShortcutBindings::default();
            bindings.close_tab = action.binding(&bindings).clone();
            let repaired = recover_defaults(&bindings);
            assert_eq!(
                repaired.close_tab,
                bindings
                    .close_tab
                    .as_deref()
                    .map(|chord| canonical_chord(chord).unwrap())
            );
            assert!(action.binding(&repaired).is_none());
            for other in ShortcutAction::ALL
                .into_iter()
                .skip(1)
                .filter(|other| *other != action)
            {
                assert_eq!(
                    other.binding(&repaired).as_ref().unwrap(),
                    &canonical_chord(other.binding(&bindings).as_ref().unwrap()).unwrap()
                );
            }
        }
    }

    #[test]
    fn new_actions_are_not_fixed_and_can_be_rebound_or_disabled() {
        let mut bindings = PersistedShortcutBindings::default();
        for (action, chord) in [
            (ShortcutAction::NewTab, "ctrl-shift-t"),
            (ShortcutAction::SplitRight, "ctrl-alt-r"),
            (ShortcutAction::SplitDown, "ctrl-alt-d"),
        ] {
            action.set_binding(&mut bindings, Some(chord.into()));
        }
        assert!(normalize(&bindings).is_ok());
        for action in ShortcutAction::ALL.into_iter().skip(3) {
            action.set_binding(&mut bindings, None);
        }
        let keymap = build_keymap(&normalize(&bindings).unwrap());
        for binding in keymap {
            assert!(!binding.action().as_any().is::<NewShellTab>());
            assert!(!binding.action().as_any().is::<SplitRight>());
            assert!(!binding.action().as_any().is::<SplitDown>());
            assert!(!binding
                .action()
                .as_any()
                .is::<crate::actions::FocusNextPart>());
            assert!(!binding
                .action()
                .as_any()
                .is::<crate::actions::FocusPreviousPart>());
        }
    }
}
