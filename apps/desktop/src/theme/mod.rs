//! The interface theme on gpui-kit's theme system (docs/design-system.md,
//! section 7): `themes/graphite.json` holds two gpui-kit theme configs,
//! "Graphite Dark" and "Graphite Light", registered in `ThemeRegistry` and
//! installed as the theme's dark and light modes. Every gpui-kit component
//! then paints with our tokens.
//!
//! Token mapping (our token -> gpui-kit `ThemeColor` field): design-system
//! 7.4; [`THEME_COLOR_MAPPING`] is the full table of theme-file keys, checked
//! against the JSON, the applied theme and the design-system tables by the
//! unit tests. The trap: gpui-kit's `accent` is the hover background of menu
//! and list items (our `--bg-overlay-hover`); **our accent is gpui-kit's
//! `primary`**.
//!
//! Colors without a `ThemeColor` field (`--bg-input`, `--text-muted`,
//! `--text-disabled`, the `*-subtle` tints, shadows, `--tag-*`,
//! `--text-on-tag`, badges, status dots, keyword highlighting) are in
//! [`palette`]; read the theme-dependent ones through [`tokens`]. Sizes,
//! spacing, radii, fonts: [`metrics`]. Motion: [`motion`]. The keyboard
//! focus ring: [`focus`]. Button colors for kit buttons: [`controls`].
//! Terminal themes: [`terminal`].
//!
//! Appearance (design-system 1, 7.1): Auto (default) follows the system and
//! keeps following it while the app runs ([`follow_system`] is called on
//! every window appearance change); Light and Dark set the mode explicitly.
//! A change applies to all windows at once.
//!
//! Known gaps of gpui-kit 0.7.3 (our own views do not have them): kit input
//! placeholders use `muted_foreground` (`--text-secondary`, not
//! `--text-muted`); kit popup menus draw their own fixed shadow and a
//! `border` (`--border-subtle`) separator; the kit dialog paints `background`.

pub mod controls;
pub mod focus;
pub mod highlight;
pub mod metrics;
pub mod motion;
pub mod palette;
pub mod terminal;
pub mod zoom;

use std::rc::Rc;

use gpui::{App, BoxShadow, Global, Window, px};
use gpui_kit::component::{Theme, ThemeConfig, ThemeMode, ThemeRegistry};

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
    ("link.active", "accent-text"),
    ("link.hover", "accent-text"),
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

/// The current appearance setting (one per app: a change applies to every
/// window).
#[derive(Clone, Copy, Debug, Default)]
struct AppearanceSetting(Appearance);

impl Global for AppearanceSetting {}

/// Registers both Graphite themes, the interface font, the motion durations,
/// and applies `appearance`. Call once after `gpui_kit::init`.
pub fn install(appearance: Appearance, cx: &mut App) {
    let registry = ThemeRegistry::global_mut(cx);
    if let Err(error) = registry.load_themes_from_str(THEME_FILE) {
        // A broken built-in theme is a build defect; the unit tests catch it.
        tracing::error!(error = %error, "theme file rejected");
        return;
    }
    let dark = registry.themes().get(DARK_THEME_NAME).cloned();
    let light = registry.themes().get(LIGHT_THEME_NAME).cloned();
    let font = ui_font(cx);
    let theme = Theme::global_mut(cx);
    if let Some(dark) = dark {
        theme.dark_theme = with_font(dark, font.as_deref());
    }
    if let Some(light) = light {
        theme.light_theme = with_font(light, font.as_deref());
    }
    motion::install(cx);
    set_appearance(appearance, None, cx);
}

/// The interface font family of this platform when it is installed
/// (design-system 5): Segoe UI Variable on Windows. Elsewhere the kit's
/// system font (`.SystemUIFont`: SF Pro on macOS, the desktop's sans on
/// Linux) is already right.
fn ui_font(cx: &App) -> Option<String> {
    if !cfg!(target_os = "windows") {
        return None;
    }
    let installed = cx.text_system().all_font_names();
    metrics::FONT_UI_WINDOWS
        .iter()
        .find(|family| installed.iter().any(|name| name == *family))
        .map(|family| (*family).to_owned())
}

fn with_font(config: Rc<ThemeConfig>, font: Option<&str>) -> Rc<ThemeConfig> {
    match font {
        Some(font) => {
            let mut config = (*config).clone();
            config.font_family = Some(font.to_owned().into());
            Rc::new(config)
        }
        None => config,
    }
}

/// The appearance setting in effect.
pub fn appearance(cx: &App) -> Appearance {
    cx.try_global::<AppearanceSetting>()
        .map(|setting| setting.0)
        .unwrap_or_default()
}

/// Changes the appearance setting and applies it (every window repaints).
pub fn set_appearance(appearance: Appearance, window: Option<&mut Window>, cx: &mut App) {
    cx.set_global(AppearanceSetting(appearance));
    match appearance {
        Appearance::Auto => Theme::sync_system_appearance(window, cx),
        Appearance::Light => Theme::change(ThemeMode::Light, window, cx),
        Appearance::Dark => Theme::change(ThemeMode::Dark, window, cx),
    }
}

/// Follows a system appearance change when the setting is Auto. Windows call
/// this from `observe_window_appearance`.
pub fn follow_system(window: &mut Window, cx: &mut App) {
    if appearance(cx) == Appearance::Auto {
        let mode = ThemeMode::from(window.appearance());
        if Theme::global(cx).mode != mode {
            Theme::change(mode, Some(window), cx);
        }
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

/// `--shadow-overlay` (menus, popovers, toasts) of the active theme.
pub fn shadow_overlay(cx: &App) -> Vec<BoxShadow> {
    vec![box_shadow(tokens(cx).shadow_overlay)]
}

/// `--shadow-modal` (modal windows) of the active theme.
pub fn shadow_modal(cx: &App) -> Vec<BoxShadow> {
    vec![box_shadow(tokens(cx).shadow_modal)]
}

/// A design-system shadow (`0 <y>px <blur>px <color>`, CSS terms) as a GPUI
/// shadow. GPUI's `blur_radius` is the gaussian's standard deviation, CSS's
/// blur radius is twice that, so the CSS value is halved.
pub fn box_shadow(shadow: palette::Shadow) -> BoxShadow {
    BoxShadow::new(px(0.), px(f32::from(shadow.y)), shadow.color.hsla())
        .blur_radius(px(f32::from(shadow.blur) / 2.))
}

#[cfg(test)]
mod contrast;
#[cfg(test)]
mod ds_tables;
#[cfg(test)]
mod literals;
#[cfg(test)]
mod tests;
