//! The terminal settings a view uses (UI 12.4, docs/data-model.md 16.1).
//!
//! The source is the [`TerminalPrefs`] global of Settings → Terminal
//! (`crate::settings::terminal`), which the Settings window replaces after
//! every change; terminal views observe it and repaint. The values that
//! stage 2a does not show in Settings (cursor style, line height,
//! multi-line paste confirmation) keep their defaults.

use gpui::{App, SharedString};

use super::fonts::JETBRAINS_MONO;
use crate::settings::terminal::TerminalPrefs;
use crate::theme::terminal::{TerminalTheme, builtin};

/// Default scrollback of a new terminal (SPEC "Performance").
pub const DEFAULT_SCROLLBACK: usize = 10_000;
/// Default text size (`--font-size-mono`).
pub const DEFAULT_FONT_SIZE: f32 = 14.;
/// Text size limits of `terminal.font_size`.
pub const MIN_FONT_SIZE: f32 = 6.;
pub const MAX_FONT_SIZE: f32 = 72.;
/// Default line height (`--line-height-mono`).
pub const DEFAULT_LINE_HEIGHT: f32 = 1.2;

/// The cursor shape when the program does not choose one (`terminal.cursor_style`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CursorStyleSetting {
    #[default]
    Block,
    Bar,
    Underline,
}

/// The settings of all terminals.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalSettings {
    /// `terminal.font_family`.
    pub font_family: SharedString,
    /// `terminal.font_size`, 6-72.
    pub font_size: f32,
    /// `terminal.line_height`, 1.0-2.0.
    pub line_height: f32,
    /// `terminal.copy_on_select`: selecting with the mouse copies.
    pub copy_on_select: bool,
    /// `terminal.bold_is_bright`.
    pub bold_is_bright: bool,
    /// `terminal.confirm_multiline_paste`.
    pub confirm_multiline_paste: bool,
    /// `terminal.bell_sound`: the view reports the bell either way
    /// ([`super::TerminalEvent::Bell`]); the owner decides about a sound.
    pub bell_sound: bool,
    /// `terminal.cursor_style`.
    pub cursor_style: CursorStyleSetting,
    /// `terminal.default_theme`: the theme of terminals whose host has none.
    pub default_theme: TerminalTheme,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_family: JETBRAINS_MONO.into(),
            font_size: DEFAULT_FONT_SIZE,
            line_height: DEFAULT_LINE_HEIGHT,
            copy_on_select: false,
            bold_is_bright: true,
            confirm_multiline_paste: true,
            bell_sound: false,
            cursor_style: CursorStyleSetting::Block,
            default_theme: builtin()[0].clone(),
        }
    }
}

impl TerminalSettings {
    /// The view settings of the Settings window's values.
    pub fn from_prefs(prefs: &TerminalPrefs) -> Self {
        let defaults = Self::default();
        let font_size = prefs.font_size as f32;
        Self {
            font_family: if prefs.font_family.trim().is_empty() {
                defaults.font_family
            } else {
                SharedString::from(prefs.font_family.clone())
            },
            font_size: if (MIN_FONT_SIZE..=MAX_FONT_SIZE).contains(&font_size) {
                font_size
            } else {
                DEFAULT_FONT_SIZE
            },
            copy_on_select: prefs.copy_on_select,
            bold_is_bright: prefs.bold_is_bright,
            bell_sound: prefs.bell_sound,
            default_theme: prefs
                .default_theme
                .theme()
                .unwrap_or(defaults.default_theme),
            ..defaults
        }
    }
}

/// The settings in effect.
pub fn settings(cx: &App) -> TerminalSettings {
    TerminalSettings::from_prefs(&TerminalPrefs::current(cx))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::terminal::BuiltinTheme;

    #[test]
    fn prefs_map_to_view_settings() {
        let defaults = TerminalSettings::from_prefs(&TerminalPrefs::default());
        assert_eq!(defaults, TerminalSettings::default());
        assert_eq!(defaults.font_family, JETBRAINS_MONO);
        assert_eq!(defaults.font_size, 14.);
        assert_eq!(defaults.line_height, 1.2);
        assert!(defaults.confirm_multiline_paste);
        assert!(defaults.bold_is_bright);
        assert!(!defaults.copy_on_select);
        assert_eq!(defaults.default_theme.name, "Graphite");

        let prefs = TerminalPrefs {
            copy_on_select: true,
            font_size: 18,
            font_family: "  ".to_owned(),
            default_theme: BuiltinTheme::Paper,
            ..TerminalPrefs::default()
        };
        let settings = TerminalSettings::from_prefs(&prefs);
        assert!(settings.copy_on_select);
        assert_eq!(settings.font_size, 18.);
        assert_eq!(settings.font_family, JETBRAINS_MONO);
        assert_eq!(settings.default_theme.name, "Paper");
        let out_of_range = TerminalPrefs {
            font_size: 500,
            ..TerminalPrefs::default()
        };
        assert_eq!(TerminalSettings::from_prefs(&out_of_range).font_size, 14.);
    }
}
