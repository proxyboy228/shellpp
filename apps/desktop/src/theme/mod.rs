//! The interface theme on gpui-kit's theme system (docs/design-system.md,
//! section 7): `themes/graphite.json` holds two gpui-kit theme configs,
//! "Graphite Dark" and "Graphite Light", registered in `ThemeRegistry` and
//! installed as the theme's dark and light modes. Every gpui-kit component
//! then paints with our tokens.
//!
//! Token mapping (our token -> gpui-kit `ThemeColor` field):
//! [`THEME_COLOR_MAPPING`] is the full table, checked against the JSON by a
//! unit test. The trap: gpui-kit's `accent` is the hover background of menu
//! and list items (our `--bg-overlay-hover`); **our accent is gpui-kit's
//! `primary`**. Main pairs:
//!
//! | Our token | gpui-kit field(s) |
//! |---|---|
//! | `--bg-base` | `background`, `title_bar`, `tab_bar`, `tab`, `status_bar` |
//! | `--bg-sidebar` | `sidebar`, `tab_bar_segmented` |
//! | `--bg-surface` | `list`, `table` (and their even/head rows) |
//! | `--bg-raised` | `secondary`, `button`, `muted`, `group_box`, `accordion` |
//! | `--bg-raised-hover` | `secondary_hover`, `button_hover`, `list_hover`, `table_hover` |
//! | `--bg-pressed` | `secondary_active`, `button_active` |
//! | `--bg-overlay` | `popover` |
//! | `--bg-overlay-hover` | `accent` (menu and list item hover) |
//! | `--scrim` | `overlay` |
//! | `--border-subtle` | `border`, `title_bar_border`, `sidebar_border`, `window_border` |
//! | `--border-strong` | `input` (field borders), `switch` (off track) |
//! | `--text-primary` | `foreground` and the `*_foreground` of neutral surfaces |
//! | `--text-secondary` | `muted_foreground`, `tab_foreground`, table head text |
//! | `--accent` | `primary`, `button_primary`, `caret`, `progress_bar`, `slider_bar`, `drag_border` |
//! | `--accent-hover` / `--accent-pressed` | `primary_hover` / `primary_active` |
//! | `--accent-subtle` | `sidebar_accent` (active item), `list_active`, `table_active`, `tab_active`, `drop_target` |
//! | `--accent-text` | `link` |
//! | `--text-on-accent` | `primary_foreground`, `sidebar_primary_foreground` |
//! | `--focus-ring` | `ring` |
//! | `--danger*`, `--text-on-danger` | `danger*`, `button_danger*` |
//! | `--warning`, `--success`, `--info` | `warning`, `success`, `info` (and the `base.*` colors) |
//!
//! Not in `ThemeColor` (read them through [`tokens`] or [`palette`]):
//! `--bg-input`, `--text-muted`, `--text-disabled`, the `*-subtle` tints as
//! fills, `--tag-*`, `--text-on-tag`, badges, status dots, shadows. Sizes are
//! in [`metrics`].
//!
//! Known gaps for the shell work: gpui-kit's `muted_foreground` is also its
//! placeholder color, so placeholders use `--text-secondary` (above the 3:1
//! of `--text-muted`); the sidebar hover is `sidebar_accent` at 80 %
//! (an accent tint), not `--bg-raised-hover`.

pub mod metrics;
pub mod palette;

use gpui::{App, Window};
use gpui_kit::component::{Theme, ThemeMode, ThemeRegistry};

pub use palette::InterfaceTokens;

/// The theme file (gpui-kit theme-set schema).
pub const THEME_FILE: &str = include_str!("../../themes/graphite.json");
pub const DARK_THEME_NAME: &str = "Graphite Dark";
pub const LIGHT_THEME_NAME: &str = "Graphite Light";

/// Every gpui-kit theme-file color key and the design-system token it holds
/// (`--` prefix dropped). Derived entries: `selection` (`--accent` at 30 % /
/// 20 %, the terminal themes' selection),
/// `thumb` / `thumb-hover` (`--text-secondary` at 35 % / 55 %),
/// `plum-subtle` (`--tag-plum` at the `*-subtle` alpha), `transparent`.
pub const THEME_COLOR_MAPPING: &[(&str, &str)] = &[
    ("accent.background", "bg-overlay-hover"),
    ("accent.foreground", "text-primary"),
    ("accordion.background", "bg-raised"),
    ("background", "bg-base"),
    ("border", "border-subtle"),
    ("button.background", "bg-raised"),
    ("button.active.background", "bg-pressed"),
    ("button.foreground", "text-primary"),
    ("button.hover.background", "bg-raised-hover"),
    ("button.danger.background", "danger"),
    ("button.danger.active.background", "danger-pressed"),
    ("button.danger.foreground", "text-on-danger"),
    ("button.danger.hover.background", "danger-hover"),
    ("button.info.background", "info"),
    ("button.info.active.background", "info"),
    ("button.info.foreground", "text-on-accent"),
    ("button.info.hover.background", "info"),
    ("button.primary.background", "accent"),
    ("button.primary.active.background", "accent-pressed"),
    ("button.primary.foreground", "text-on-accent"),
    ("button.primary.hover.background", "accent-hover"),
    ("button.secondary.background", "bg-raised"),
    ("button.secondary.active.background", "bg-pressed"),
    ("button.secondary.foreground", "text-primary"),
    ("button.secondary.hover.background", "bg-raised-hover"),
    ("button.success.background", "success"),
    ("button.success.active.background", "success"),
    ("button.success.foreground", "text-on-accent"),
    ("button.success.hover.background", "success"),
    ("button.warning.background", "warning"),
    ("button.warning.active.background", "warning"),
    ("button.warning.foreground", "text-on-accent"),
    ("button.warning.hover.background", "warning"),
    ("group_box.background", "bg-raised"),
    ("group_box.foreground", "text-primary"),
    ("group_box.title.foreground", "text-secondary"),
    ("caret", "accent"),
    ("chart.1", "tag-jade"),
    ("chart.2", "tag-ocean"),
    ("chart.3", "tag-amber"),
    ("chart.4", "tag-plum"),
    ("chart.5", "tag-coral"),
    ("chart.bullish", "success"),
    ("chart.bearish", "danger"),
    ("chart.grid", "border-subtle"),
    ("danger.background", "danger"),
    ("danger.active.background", "danger-pressed"),
    ("danger.foreground", "text-on-danger"),
    ("danger.hover.background", "danger-hover"),
    ("description_list.label.background", "bg-raised"),
    ("description_list.label.foreground", "text-secondary"),
    ("drag.border", "accent"),
    ("drop_target.background", "accent-subtle"),
    ("foreground", "text-primary"),
    ("info.background", "info"),
    ("info.active.background", "info"),
    ("info.foreground", "text-on-accent"),
    ("info.hover.background", "info"),
    ("input.border", "border-strong"),
    ("link", "accent-text"),
    ("link.active", "accent-pressed"),
    ("link.hover", "accent-hover"),
    ("list.background", "bg-surface"),
    ("list.active.background", "accent-subtle"),
    ("list.active.border", "accent"),
    ("list.even.background", "bg-surface"),
    ("list.head.background", "bg-surface"),
    ("list.hover.background", "bg-raised-hover"),
    ("muted.background", "bg-raised"),
    ("muted.foreground", "text-secondary"),
    ("popover.background", "bg-overlay"),
    ("popover.foreground", "text-primary"),
    ("primary.background", "accent"),
    ("primary.active.background", "accent-pressed"),
    ("primary.foreground", "text-on-accent"),
    ("primary.hover.background", "accent-hover"),
    ("progress.bar.background", "accent"),
    ("ring", "focus-ring"),
    ("scrollbar.background", "transparent"),
    ("scrollbar.thumb.background", "thumb"),
    ("scrollbar.thumb.hover.background", "thumb-hover"),
    ("secondary.background", "bg-raised"),
    ("secondary.active.background", "bg-pressed"),
    ("secondary.foreground", "text-primary"),
    ("secondary.hover.background", "bg-raised-hover"),
    ("selection.background", "selection"),
    ("sidebar.background", "bg-sidebar"),
    ("sidebar.accent.background", "accent-subtle"),
    ("sidebar.accent.foreground", "text-primary"),
    ("sidebar.border", "border-subtle"),
    ("sidebar.foreground", "text-primary"),
    ("sidebar.primary.background", "accent"),
    ("sidebar.primary.foreground", "text-on-accent"),
    ("skeleton.background", "bg-raised"),
    ("slider.background", "accent"),
    ("slider.thumb.background", "text-on-tag"),
    ("success.background", "success"),
    ("success.foreground", "text-on-accent"),
    ("success.hover.background", "success"),
    ("success.active.background", "success"),
    ("switch.background", "border-strong"),
    ("switch.thumb.background", "text-on-tag"),
    ("tab.background", "bg-base"),
    ("tab.active.background", "accent-subtle"),
    ("tab.active.foreground", "text-primary"),
    ("tab_bar.background", "bg-base"),
    ("tab_bar.segmented.background", "bg-sidebar"),
    ("tab.foreground", "text-secondary"),
    ("table.background", "bg-surface"),
    ("table.active.background", "accent-subtle"),
    ("table.active.border", "accent"),
    ("table.even.background", "bg-surface"),
    ("table.head.background", "bg-surface"),
    ("table.head.foreground", "text-secondary"),
    ("table.foot.background", "bg-surface"),
    ("table.foot.foreground", "text-secondary"),
    ("table.hover.background", "bg-raised-hover"),
    ("table.row.border", "border-subtle"),
    ("title_bar.background", "bg-base"),
    ("title_bar.border", "border-subtle"),
    ("status_bar.background", "bg-base"),
    ("status_bar.border", "border-subtle"),
    ("warning.background", "warning"),
    ("warning.active.background", "warning"),
    ("warning.hover.background", "warning"),
    ("warning.foreground", "text-on-accent"),
    ("overlay", "scrim"),
    ("window.border", "border-subtle"),
    ("base.blue", "info"),
    ("base.blue.light", "info-subtle"),
    ("base.cyan", "accent"),
    ("base.cyan.light", "accent-subtle"),
    ("base.green", "success"),
    ("base.green.light", "success-subtle"),
    ("base.magenta", "tag-plum"),
    ("base.magenta.light", "plum-subtle"),
    ("base.red", "danger"),
    ("base.red.light", "danger-subtle"),
    ("base.yellow", "warning"),
    ("base.yellow.light", "warning-subtle"),
];

/// The interface theme setting (design-system 1: Auto, Light or Dark).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Appearance {
    /// Follows the system.
    #[default]
    Auto,
    Light,
    Dark,
}

impl Appearance {
    /// Parses `auto`, `light` or `dark`.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "auto" => Some(Self::Auto),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

/// Registers both Graphite themes and applies `appearance`. Call once after
/// `gpui_kit::init`.
pub fn install(appearance: Appearance, cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    if let Err(error) = registry.load_themes_from_str(THEME_FILE) {
        // A broken built-in theme is a build defect; the unit tests catch it.
        tracing::error!(error = %error, "theme file rejected");
        return;
    }
    let dark = registry.themes().get(DARK_THEME_NAME).cloned();
    let light = registry.themes().get(LIGHT_THEME_NAME).cloned();
    let theme = Theme::global_mut(cx);
    if let Some(dark) = dark {
        theme.dark_theme = dark;
    }
    if let Some(light) = light {
        theme.light_theme = light;
    }
    set_appearance(appearance, None, cx);
}

/// Switches the interface theme (every window repaints).
pub fn set_appearance(appearance: Appearance, window: Option<&mut Window>, cx: &mut App) {
    match appearance {
        Appearance::Auto => Theme::sync_system_appearance(window, cx),
        Appearance::Light => Theme::change(ThemeMode::Light, window, cx),
        Appearance::Dark => Theme::change(ThemeMode::Dark, window, cx),
    }
}

/// The tokens of the active interface theme.
pub fn tokens(cx: &App) -> &'static InterfaceTokens {
    if Theme::global(cx).mode.is_dark() {
        &palette::DARK
    } else {
        &palette::LIGHT
    }
}

#[cfg(test)]
mod tests;
