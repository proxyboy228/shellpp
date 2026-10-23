//! Settings > Terminal (UI 12.4; stage 2a lead decision 12): the blocks this
//! stage uses, top to bottom.
//!
//! - **Terminal**: Autoreconnect (on by default, UI 4.6), Record command
//!   history (on by default, stage 2b; `terminal.record_history`), Copy on select,
//!   Bell sound (off by default), Bright colors for bold text, Adjust
//!   indistinguishable colors (Off / Palette only (default) / Always;
//!   owner's stage 2b step 0.4, `crate::terminal::contrast`), Terminal
//!   emulation type (the TERM value: `xterm`, `xterm-256color` (default),
//!   `vt100`, `linux`).
//! - **Font**: the family (the bundled JetBrains Mono first, then the
//!   installed monospace fonts, with a search field), the size (-/+ and a
//!   number field, 14 by default) and the line spacing (-/+ in steps of
//!   0.1 and a number field, 1.0-2.0, 1.2 by default; UI 4.2).
//! - **Terminal theme**: the default theme for new hosts, Graphite or Paper.
//! - **Keepalive**: the interval in seconds, 30 by default, 0 = off.
//!
//! - **Autocomplete (beta)** first in the Terminal block (D142): Off
//!   (default) or suggestions from this host's history.
//!
//! The other blocks of 12.4 (shell history import, cursor, keyword
//! highlighting, OS detection, local shell) are hidden until their stage.
//!
//! Every change is written to the core at once (UI 12.1). A number field
//! that holds an invalid value shows the reason under it and is not saved;
//! the last valid value stays in effect (UI 1.3). After every write the
//! [`TerminalPrefs`] global is
//! replaced, so open terminals (which observe it) repaint with the new font,
//! theme and behavior at once; the session layer reads TERM, keepalive
//! and Autoreconnect from the core when a session connects.

use std::sync::Arc;

use gpui::Focusable as _;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, ElementId, Entity, Global, IntoElement, SharedString,
    Subscription, Task, TextSystem, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, IconName, h_flex, v_flex};
use serde_json::{Value, json};
use shellpp_core::Core;
use shellpp_core::service::dto::SettingsSetInput;

use super::strings;
use crate::panels::widgets;
use crate::state::AppState;
pub use crate::terminal::contrast::MinContrast;
use crate::theme::focus::FocusRingExt as _;
use crate::theme::{self, metrics};

// Setting keys (docs/data-model.md, section 16.1).
pub const KEY_AUTORECONNECT: &str = "terminal.autoreconnect";
pub const KEY_RECORD_HISTORY: &str = "terminal.record_history";
pub const KEY_COPY_ON_SELECT: &str = "terminal.copy_on_select";
pub const KEY_BELL_SOUND: &str = "terminal.bell_sound";
pub const KEY_BOLD_IS_BRIGHT: &str = "terminal.bold_is_bright";
pub const KEY_HIGHLIGHTING: &str = "terminal.highlighting_enabled";
pub const KEY_TERM: &str = "terminal.term";
pub const KEY_FONT_FAMILY: &str = "terminal.font_family";
pub const KEY_FONT_SIZE: &str = "terminal.font_size";
pub const KEY_LINE_HEIGHT: &str = "terminal.line_height";
pub const KEY_MIN_CONTRAST: &str = "terminal.min_contrast";
pub const KEY_DEFAULT_THEME: &str = "terminal.default_theme";
pub const KEY_KEEPALIVE: &str = "terminal.keepalive_s";
pub const KEY_AUTOCOMPLETE: &str = "terminal.autocomplete";

/// The TERM choices of UI 12.4, in menu order.
pub const TERM_CHOICES: [&str; 4] = ["xterm", "xterm-256color", "vt100", "linux"];
/// The default TERM value.
pub const DEFAULT_TERM: &str = "xterm-256color";
/// The bundled terminal font (design-system 5; OFL-1.1).
pub const BUNDLED_FONT: &str = crate::terminal::fonts::JETBRAINS_MONO;
/// Limits of the font size (the core's `terminal.font_size` range).
pub const FONT_SIZE_MIN: u32 = 6;
pub const FONT_SIZE_MAX: u32 = 72;
/// The default font size (`--font-size-mono`).
pub const DEFAULT_FONT_SIZE: u32 = 14;
/// Limits of the line spacing (the core's `terminal.line_height` range).
pub const LINE_HEIGHT_MIN: f32 = 1.0;
pub const LINE_HEIGHT_MAX: f32 = 2.0;
/// The default line spacing (`--line-height-mono`).
pub const DEFAULT_LINE_HEIGHT: f32 = metrics::LINE_HEIGHT_MONO;
/// One step of the line spacing -/+ buttons.
pub const LINE_HEIGHT_STEP: f32 = 0.1;
/// Limits of the keepalive interval in seconds (0 = off).
pub const KEEPALIVE_MAX: u32 = 3600;
/// The default keepalive interval (UI 12.4).
pub const DEFAULT_KEEPALIVE: u32 = 30;

/// One theme of the built-in theme registry.
#[derive(Debug)]
struct RegistryEntry {
    slug: &'static str,
    theme: theme::terminal::TerminalTheme,
    new: bool,
}

/// The built-in themes in list order, the default (Graphite) first: the
/// slug of `{"builtin": "<slug>"}` references, the parsed theme file
/// (`src/theme/terminal/<slug>.json`) and the "New" mark (UI 4.2).
fn registry() -> &'static [RegistryEntry] {
    static REGISTRY: std::sync::OnceLock<Vec<RegistryEntry>> = std::sync::OnceLock::new();
    REGISTRY.get_or_init(|| {
        theme::terminal::registry()
            .iter()
            .map(|entry| RegistryEntry {
                slug: entry.id(),
                theme: entry.theme.clone(),
                new: entry.file.new,
            })
            .collect()
    })
}

/// A built-in terminal theme (design-system 4; the registry of
/// `crate::theme::terminal`), named by its slug.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct BuiltinTheme(usize);

impl BuiltinTheme {
    /// Graphite (dark), the default.
    pub const GRAPHITE: BuiltinTheme = BuiltinTheme(0);
    /// Paper (light).
    pub const PAPER: BuiltinTheme = BuiltinTheme(1);

    /// Every built-in theme, in list order.
    pub fn all() -> Vec<BuiltinTheme> {
        (0..registry().len()).map(BuiltinTheme).collect()
    }

    fn entry(self) -> Option<&'static RegistryEntry> {
        registry().get(self.0)
    }

    /// The slug of `{"builtin": "<slug>"}` theme references.
    pub fn slug(self) -> &'static str {
        self.entry().map_or("graphite", |entry| entry.slug)
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        registry()
            .iter()
            .position(|entry| entry.slug == slug)
            .map(BuiltinTheme)
    }

    /// The theme of a `theme_ref` value (`{"builtin": "graphite"}`); `None`
    /// for anything else.
    pub fn from_ref(value: &Value) -> Option<Self> {
        value
            .get("builtin")
            .and_then(Value::as_str)
            .and_then(Self::from_slug)
    }

    /// The name in theme lists (the theme file's `name`).
    pub fn label(self) -> &'static str {
        self.entry().map_or("", |entry| entry.theme.name.as_str())
    }

    /// Added in this release: the lists mark it "New" (UI 4.2).
    pub fn is_new(self) -> bool {
        self.entry().is_some_and(|entry| entry.new)
    }

    /// The parsed theme file (`src/theme/terminal/<slug>.json`).
    pub fn theme(self) -> Option<theme::terminal::TerminalTheme> {
        self.entry().map(|entry| entry.theme.clone())
    }
}

/// The terminal settings in effect (UI 12.4). Also an app global: the
/// Settings window replaces it after every change, and the terminal views
/// observe it (`crate::terminal::settings`) and repaint at once.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalPrefs {
    pub autoreconnect: bool,
    /// "Record command history" (UI 12.4; a host can override it). The core
    /// reads the stored value when it records; this copy is for the UI.
    pub record_history: bool,
    pub copy_on_select: bool,
    pub bell_sound: bool,
    pub bold_is_bright: bool,
    /// The TERM value sent with the PTY request.
    pub term: String,
    pub font_family: String,
    pub font_size: u32,
    /// The line height as a multiple of the text size, 1.0-2.0.
    pub line_height: f32,
    /// Which text is moved to a contrast of 4.5:1 with its background.
    pub min_contrast: MinContrast,
    /// The default theme of hosts without their own.
    pub default_theme: BuiltinTheme,
    /// Seconds between keepalives; 0 = off.
    pub keepalive_s: u32,
    /// Keyword highlighting (UI 12.4).
    pub highlighting: bool,
    /// Autocomplete from this host's history (beta, D142): the setting is
    /// `history`; `off` by default.
    pub autocomplete: bool,
}

impl Default for TerminalPrefs {
    fn default() -> Self {
        Self {
            autoreconnect: true,
            record_history: true,
            copy_on_select: false,
            bell_sound: false,
            bold_is_bright: true,
            term: DEFAULT_TERM.to_owned(),
            font_family: BUNDLED_FONT.to_owned(),
            font_size: DEFAULT_FONT_SIZE,
            line_height: DEFAULT_LINE_HEIGHT,
            min_contrast: MinContrast::default(),
            default_theme: BuiltinTheme::GRAPHITE,
            keepalive_s: DEFAULT_KEEPALIVE,
            highlighting: true,
            autocomplete: false,
        }
    }
}

impl Global for TerminalPrefs {}

impl TerminalPrefs {
    /// Reads the settings from the core (defaults for anything missing).
    pub fn load(core: &Core) -> Self {
        match core.settings_get() {
            Ok(values) => Self::from_values(&values),
            Err(error) => {
                tracing::warn!(code = error.code(), "terminal settings not read");
                Self::default()
            }
        }
    }

    /// The settings of a `settings_get` result.
    pub fn from_values(values: &std::collections::BTreeMap<String, Value>) -> Self {
        let defaults = Self::default();
        let flag =
            |key: &str, default: bool| values.get(key).and_then(Value::as_bool).unwrap_or(default);
        let number = |key: &str, default: u32| {
            values
                .get(key)
                .and_then(Value::as_u64)
                .and_then(|value| u32::try_from(value).ok())
                .unwrap_or(default)
        };
        let text = |key: &str, default: &str| {
            values
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.trim().is_empty())
                .unwrap_or(default)
                .to_owned()
        };
        Self {
            autoreconnect: flag(KEY_AUTORECONNECT, defaults.autoreconnect),
            record_history: flag(KEY_RECORD_HISTORY, defaults.record_history),
            copy_on_select: flag(KEY_COPY_ON_SELECT, defaults.copy_on_select),
            bell_sound: flag(KEY_BELL_SOUND, defaults.bell_sound),
            bold_is_bright: flag(KEY_BOLD_IS_BRIGHT, defaults.bold_is_bright),
            term: text(KEY_TERM, &defaults.term),
            font_family: text(KEY_FONT_FAMILY, &defaults.font_family),
            font_size: number(KEY_FONT_SIZE, defaults.font_size),
            line_height: values
                .get(KEY_LINE_HEIGHT)
                .and_then(Value::as_f64)
                .map(|value| value as f32)
                .filter(|value| (LINE_HEIGHT_MIN..=LINE_HEIGHT_MAX).contains(value))
                .unwrap_or(defaults.line_height),
            min_contrast: values
                .get(KEY_MIN_CONTRAST)
                .and_then(Value::as_str)
                .and_then(MinContrast::from_key)
                .unwrap_or(defaults.min_contrast),
            default_theme: values
                .get(KEY_DEFAULT_THEME)
                .and_then(BuiltinTheme::from_ref)
                .unwrap_or_default(),
            keepalive_s: number(KEY_KEEPALIVE, defaults.keepalive_s),
            highlighting: flag(KEY_HIGHLIGHTING, defaults.highlighting),
            autocomplete: values.get(KEY_AUTOCOMPLETE).and_then(Value::as_str) == Some("history"),
        }
    }

    /// The global value, or the defaults before the first read.
    pub fn current(cx: &App) -> Self {
        cx.try_global::<Self>().cloned().unwrap_or_default()
    }

    /// Reads the settings from the core and publishes them as the global
    /// (a window opens, the app unlocks).
    pub fn refresh(core: &Core, cx: &mut App) {
        cx.set_global(Self::load(core));
    }
}

/// Parses a whole number field within `min..=max`.
pub fn parse_number(text: &str, min: u32, max: u32) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    text.parse::<u32>()
        .ok()
        .filter(|value| (min..=max).contains(value))
}

/// Parses a line spacing field: a number from 1.0 to 2.0 (a dot or a comma
/// as the decimal mark), rounded to two decimals.
pub fn parse_line_height(text: &str) -> Option<f32> {
    let text = text.trim().replace(',', ".");
    if text.is_empty()
        || !text
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return None;
    }
    let value = text.parse::<f32>().ok()?;
    let value = (value * 100.).round() / 100.;
    (LINE_HEIGHT_MIN..=LINE_HEIGHT_MAX)
        .contains(&value)
        .then_some(value)
}

/// One -/+ step of 0.1 from `value`, on the tenths grid, within 1.0-2.0.
pub fn stepped_line_height(value: f32, step: i32) -> f32 {
    let tenths = if step > 0 {
        (value * 10. + 0.001).floor() as i32 + step
    } else {
        (value * 10. - 0.001).ceil() as i32 + step
    };
    (tenths as f32 / 10.).clamp(LINE_HEIGHT_MIN, LINE_HEIGHT_MAX)
}

/// A line spacing as the field shows it: one decimal, or two when needed.
pub fn line_height_text(value: f32) -> String {
    let tenths = (value * 10.).round() / 10.;
    if (tenths - value).abs() < 0.001 {
        format!("{tenths:.1}")
    } else {
        format!("{value:.2}")
    }
}

/// The JSON number of a line spacing (two decimals, so 1.3 is stored as
/// 1.3 and not as the nearest binary fraction of an `f32`).
pub fn line_height_value(value: f32) -> Value {
    let rounded = (f64::from(value) * 100.).round() / 100.;
    json!(rounded)
}

/// Writes one terminal setting to the core and publishes the result as the
/// [`TerminalPrefs`] global, so open terminals, the Settings window and the
/// terminal sidebars follow at once (UI 12.1: every change is saved).
pub fn save_setting(
    core: &Core,
    key: &'static str,
    value: Value,
    cx: &mut App,
) -> Result<TerminalPrefs, shellpp_core::CoreError> {
    let mut values = std::collections::BTreeMap::new();
    values.insert(key.to_owned(), value);
    match core.settings_set(SettingsSetInput {
        values,
        reset: Vec::new(),
    }) {
        Ok(values) => {
            let prefs = TerminalPrefs::from_values(&values);
            cx.set_global(prefs.clone());
            tracing::info!(setting = key, "terminal setting changed");
            Ok(prefs)
        }
        Err(error) => {
            tracing::warn!(
                code = error.code(),
                setting = key,
                "terminal setting not saved"
            );
            Err(error)
        }
    }
}

/// Whether a family is monospace: the same advance for a narrow and a wide
/// letter, a digit and a dot.
fn is_monospace(text_system: &TextSystem, family: &str) -> bool {
    let font_id = text_system.resolve_font(&gpui::font(family.to_owned()));
    let size = px(14.);
    let mut widths = ['i', 'W', 'M', '0', '.'].into_iter().map(|ch| {
        text_system
            .advance(font_id, size, ch)
            .map(|size| size.width)
    });
    let Some(Ok(first)) = widths.next() else {
        return false;
    };
    widths.all(|width| width.is_ok_and(|width| (width - first).abs() < px(0.01)))
}

/// The installed monospace families (vertical `@` variants left out),
/// sorted by name; the bundled font is not repeated.
pub(crate) fn installed_monospace(text_system: &TextSystem) -> Vec<SharedString> {
    text_system
        .all_font_names()
        .into_iter()
        .filter(|name| !name.starts_with('@') && name != BUNDLED_FONT)
        .filter(|name| is_monospace(text_system, name))
        .map(SharedString::from)
        .collect()
}

/// Which dropdown of the section is open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Open {
    Autocomplete,
    Term,
    Font,
    MinContrast,
}

/// The Terminal section of the Settings window.
pub struct TerminalSection {
    state: Entity<AppState>,
    prefs: TerminalPrefs,
    open: Option<Open>,
    font_search: Entity<InputState>,
    font_size: Entity<InputState>,
    line_height: Entity<InputState>,
    keepalive: Entity<InputState>,
    font_size_invalid: bool,
    line_height_invalid: bool,
    keepalive_invalid: bool,
    /// Installed monospace families, once listed (the first open of the
    /// font list starts it on the background executor).
    fonts: Option<Arc<Vec<SharedString>>>,
    fonts_task: Option<Task<()>>,
    error: Option<SharedString>,
    /// The keyword highlighting rules (D77).
    highlight_rules: Entity<super::highlight_rules::HighlightRulesBlock>,
    /// Script generation (beta, UI 9.5, D144).
    generation: Entity<super::generation::GenerationBlock>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for TerminalSection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalSection")
            .field("prefs", &self.prefs)
            .field("open", &self.open)
            .finish_non_exhaustive()
    }
}

impl TerminalSection {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let prefs = state
            .read(cx)
            .core()
            .map(|core| TerminalPrefs::load(core))
            .unwrap_or_default();
        let font_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(strings::FONT_SEARCH_PLACEHOLDER));
        let font_size_text = prefs.font_size.to_string();
        let font_size = cx.new(|cx| InputState::new(window, cx).default_value(font_size_text));
        let line_height_text = line_height_text(prefs.line_height);
        let line_height = cx.new(|cx| InputState::new(window, cx).default_value(line_height_text));
        let keepalive_text = prefs.keepalive_s.to_string();
        let keepalive = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(DEFAULT_KEEPALIVE.to_string())
                .default_value(keepalive_text)
        });
        let subscriptions = vec![
            cx.observe_global_in::<TerminalPrefs>(window, |this, window, cx| {
                this.prefs_changed(window, cx)
            }),
            cx.subscribe_in(&font_search, window, |_, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    cx.notify();
                }
            }),
            cx.subscribe_in(
                &font_size,
                window,
                |this, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        this.font_size_typed(&text, cx);
                    }
                    InputEvent::Blur | InputEvent::PressEnter { .. } => {
                        this.reset_field(Field::FontSize, window, cx)
                    }
                    InputEvent::Focus => {}
                },
            ),
            cx.subscribe_in(
                &line_height,
                window,
                |this, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        this.line_height_typed(&text, cx);
                    }
                    InputEvent::Blur | InputEvent::PressEnter { .. } => {
                        this.reset_field(Field::LineHeight, window, cx)
                    }
                    InputEvent::Focus => {}
                },
            ),
            cx.subscribe_in(
                &keepalive,
                window,
                |this, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        let text = input.read(cx).value().to_string();
                        this.keepalive_typed(&text, cx);
                    }
                    InputEvent::Blur | InputEvent::PressEnter { .. } => {
                        this.reset_field(Field::Keepalive, window, cx)
                    }
                    InputEvent::Focus => {}
                },
            ),
        ];
        let rules_state = state.clone();
        let highlight_rules =
            cx.new(|cx| super::highlight_rules::HighlightRulesBlock::new(rules_state, window, cx));
        let generation_state = state.clone();
        let generation =
            cx.new(|cx| super::generation::GenerationBlock::new(generation_state, window, cx));
        Self {
            state,
            prefs,
            open: None,
            font_search,
            font_size,
            line_height,
            keepalive,
            font_size_invalid: false,
            line_height_invalid: false,
            keepalive_invalid: false,
            fonts: None,
            fonts_task: None,
            error: None,
            highlight_rules,
            generation,
            _subscriptions: subscriptions,
        }
    }

    /// The script generation block (tests).
    pub fn generation(&self) -> &Entity<super::generation::GenerationBlock> {
        &self.generation
    }

    /// The keyword highlighting rules block (tests).
    pub fn highlight_rules(&self) -> &Entity<super::highlight_rules::HighlightRulesBlock> {
        &self.highlight_rules
    }

    /// The settings as this section shows them.
    pub fn prefs(&self) -> &TerminalPrefs {
        &self.prefs
    }

    pub fn is_font_size_invalid(&self) -> bool {
        self.font_size_invalid
    }

    pub fn is_keepalive_invalid(&self) -> bool {
        self.keepalive_invalid
    }

    pub fn is_line_height_invalid(&self) -> bool {
        self.line_height_invalid
    }

    /// Writes one setting; on success the section and the global follow.
    fn write(&mut self, key: &'static str, value: Value, cx: &mut Context<Self>) -> bool {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return false;
        };
        match save_setting(&core, key, value, cx) {
            Ok(prefs) => {
                self.prefs = prefs;
                self.error = None;
                cx.notify();
                true
            }
            Err(_) => {
                self.error = Some(strings::FAILED.into());
                cx.notify();
                false
            }
        }
    }

    /// Another window (the terminal sidebar's Appearance tab) changed the
    /// settings: show them, and put the values in effect into the number
    /// fields the user is not editing.
    fn prefs_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = TerminalPrefs::current(cx);
        if current == self.prefs {
            return;
        }
        self.prefs = current;
        for (field, input) in [
            (Field::FontSize, self.font_size.clone()),
            (Field::LineHeight, self.line_height.clone()),
            (Field::Keepalive, self.keepalive.clone()),
        ] {
            if !input.read(cx).focus_handle(cx).is_focused(window) {
                self.reset_field(field, window, cx);
            }
        }
        cx.notify();
    }

    pub fn set_autoreconnect(&mut self, on: bool, cx: &mut Context<Self>) {
        self.write(KEY_AUTORECONNECT, json!(on), cx);
    }

    /// "Record command history" (UI 12.4): running sessions follow at their
    /// next command (the core checks the setting when it records).
    pub fn set_record_history(&mut self, on: bool, cx: &mut Context<Self>) {
        self.write(KEY_RECORD_HISTORY, json!(on), cx);
    }

    pub fn set_copy_on_select(&mut self, on: bool, cx: &mut Context<Self>) {
        self.write(KEY_COPY_ON_SELECT, json!(on), cx);
    }

    pub fn set_bell_sound(&mut self, on: bool, cx: &mut Context<Self>) {
        self.write(KEY_BELL_SOUND, json!(on), cx);
    }

    pub fn set_bold_is_bright(&mut self, on: bool, cx: &mut Context<Self>) {
        self.write(KEY_BOLD_IS_BRIGHT, json!(on), cx);
    }

    /// Keyword highlighting (UI 12.4).
    pub fn set_highlighting(&mut self, on: bool, cx: &mut Context<Self>) {
        self.write(KEY_HIGHLIGHTING, json!(on), cx);
    }

    /// "Adjust indistinguishable colors" (UI 12.4).
    pub fn set_min_contrast(&mut self, mode: MinContrast, cx: &mut Context<Self>) {
        self.open = None;
        self.write(KEY_MIN_CONTRAST, json!(mode.key()), cx);
        cx.notify();
    }

    /// Autocomplete (beta, D142): on takes suggestions from this host's
    /// history.
    pub fn set_autocomplete(&mut self, on: bool, cx: &mut Context<Self>) {
        self.open = None;
        self.write(
            KEY_AUTOCOMPLETE,
            json!(if on { "history" } else { "off" }),
            cx,
        );
        cx.notify();
    }

    pub fn set_term(&mut self, term: &str, cx: &mut Context<Self>) {
        self.open = None;
        if TERM_CHOICES.contains(&term) {
            self.write(KEY_TERM, json!(term), cx);
        }
        cx.notify();
    }

    pub fn set_font_family(&mut self, family: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.open = None;
        self.font_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.write(KEY_FONT_FAMILY, json!(family), cx);
        cx.notify();
    }

    pub fn set_default_theme(&mut self, theme: BuiltinTheme, cx: &mut Context<Self>) {
        self.write(KEY_DEFAULT_THEME, json!({ "builtin": theme.slug() }), cx);
    }

    /// The -/+ buttons of the font size: one step within the limits.
    pub fn step_font_size(&mut self, step: i32, window: &mut Window, cx: &mut Context<Self>) {
        let size = self
            .prefs
            .font_size
            .saturating_add_signed(step)
            .clamp(FONT_SIZE_MIN, FONT_SIZE_MAX);
        if size != self.prefs.font_size {
            self.write(KEY_FONT_SIZE, json!(size), cx);
        }
        self.reset_field(Field::FontSize, window, cx);
    }

    /// The -/+ buttons of the line spacing: one step of 0.1 within 1.0-2.0.
    pub fn step_line_height(&mut self, step: i32, window: &mut Window, cx: &mut Context<Self>) {
        let value = stepped_line_height(self.prefs.line_height, step);
        if (value - self.prefs.line_height).abs() > f32::EPSILON {
            self.write(KEY_LINE_HEIGHT, line_height_value(value), cx);
        }
        self.reset_field(Field::LineHeight, window, cx);
    }

    fn line_height_typed(&mut self, text: &str, cx: &mut Context<Self>) {
        match parse_line_height(text) {
            Some(value) => {
                self.line_height_invalid = false;
                if (value - self.prefs.line_height).abs() > f32::EPSILON {
                    self.write(KEY_LINE_HEIGHT, line_height_value(value), cx);
                }
            }
            None => self.line_height_invalid = true,
        }
        cx.notify();
    }

    fn font_size_typed(&mut self, text: &str, cx: &mut Context<Self>) {
        match parse_number(text, FONT_SIZE_MIN, FONT_SIZE_MAX) {
            Some(size) => {
                self.font_size_invalid = false;
                if size != self.prefs.font_size {
                    self.write(KEY_FONT_SIZE, json!(size), cx);
                }
            }
            None => self.font_size_invalid = true,
        }
        cx.notify();
    }

    fn keepalive_typed(&mut self, text: &str, cx: &mut Context<Self>) {
        match parse_number(text, 0, KEEPALIVE_MAX) {
            Some(seconds) => {
                self.keepalive_invalid = false;
                if seconds != self.prefs.keepalive_s {
                    self.write(KEY_KEEPALIVE, json!(seconds), cx);
                }
            }
            None => self.keepalive_invalid = true,
        }
        cx.notify();
    }

    /// After leaving a number field (or -/+): it shows the value in effect.
    fn reset_field(&mut self, field: Field, window: &mut Window, cx: &mut Context<Self>) {
        let (input, text) = match field {
            Field::FontSize => {
                self.font_size_invalid = false;
                (self.font_size.clone(), self.prefs.font_size.to_string())
            }
            Field::LineHeight => {
                self.line_height_invalid = false;
                (
                    self.line_height.clone(),
                    line_height_text(self.prefs.line_height),
                )
            }
            Field::Keepalive => {
                self.keepalive_invalid = false;
                (self.keepalive.clone(), self.prefs.keepalive_s.to_string())
            }
        };
        if input.read(cx).value() != text.as_str() {
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
        cx.notify();
    }

    fn toggle(&mut self, which: Open, window: &mut Window, cx: &mut Context<Self>) {
        self.open = if self.open == Some(which) {
            None
        } else {
            Some(which)
        };
        if self.open == Some(Open::Font) {
            self.load_fonts(cx);
            self.font_search.update(cx, |input, cx| {
                input.set_value("", window, cx);
                input.focus(window, cx);
            });
        }
        cx.notify();
    }

    /// Lists the installed monospace fonts on the background executor (once).
    fn load_fonts(&mut self, cx: &mut Context<Self>) {
        if self.fonts.is_some() || self.fonts_task.is_some() {
            return;
        }
        let text_system = cx.text_system().clone();
        let work = cx.background_spawn(async move { installed_monospace(&text_system) });
        self.fonts_task = Some(cx.spawn(async move |this, cx| {
            let fonts = work.await;
            let _ = this.update(cx, |this, cx| {
                this.fonts = Some(Arc::new(fonts));
                this.fonts_task = None;
                cx.notify();
            });
        }));
    }

    /// The families the font list offers for the search text: the bundled
    /// font first, then the installed monospace fonts.
    pub fn font_choices(&self, cx: &App) -> Vec<SharedString> {
        let query = self.font_search.read(cx).value().trim().to_lowercase();
        std::iter::once(SharedString::from(BUNDLED_FONT))
            .chain(self.fonts.iter().flat_map(|fonts| fonts.iter().cloned()))
            .filter(|name| query.is_empty() || name.to_lowercase().contains(&query))
            .collect()
    }

    fn render_terminal_block(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let prefs = &self.prefs;
        let toggle_row = |id: &'static str,
                          label: &'static str,
                          hint: &'static str,
                          checked: bool,
                          on_change: fn(&mut Self, bool, &mut Context<Self>),
                          cx: &mut Context<Self>| {
            v_flex()
                .id(SharedString::from(format!("{id}-row")))
                .test_support()
                .gap(metrics::SPACE_1)
                .child(
                    h_flex()
                        .justify_between()
                        .gap(metrics::SPACE_3)
                        .child(super::label(label, cx))
                        .child(Switch::new(id).checked(checked).on_click(cx.listener(
                            move |this, checked: &bool, _, cx| on_change(this, *checked, cx),
                        ))),
                )
                .child(widgets::caption(hint, cx))
        };
        let term_trigger = widgets::select_trigger(
            "settings-term",
            None,
            prefs.term.clone(),
            self.open == Some(Open::Term),
            cx,
        )
        .on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| this.toggle(Open::Term, window, cx)),
        );
        let term_list = (self.open == Some(Open::Term)).then(|| {
            widgets::dropdown_list("settings-term-list", cx).children(TERM_CHOICES.map(|term| {
                widgets::dropdown_item(
                    SharedString::from(format!("settings-term-{term}")),
                    None,
                    term,
                    (term == DEFAULT_TERM).then(|| SharedString::from(strings::DEFAULT_CHOICE)),
                    term == prefs.term,
                    cx,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_term(term, cx)))
            }))
        });
        let autocomplete_open = self.open == Some(Open::Autocomplete);
        let autocomplete_trigger = widgets::select_trigger(
            "settings-autocomplete",
            None,
            if prefs.autocomplete {
                strings::AUTOCOMPLETE_HISTORY
            } else {
                strings::AUTOCOMPLETE_OFF
            },
            autocomplete_open,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.toggle(Open::Autocomplete, window, cx)
        }));
        let autocomplete_list =
            autocomplete_open.then(|| {
                widgets::dropdown_list("settings-autocomplete-list", cx).children(
                    [
                        (
                            "settings-autocomplete-off",
                            strings::AUTOCOMPLETE_OFF,
                            false,
                        ),
                        (
                            "settings-autocomplete-history",
                            strings::AUTOCOMPLETE_HISTORY,
                            true,
                        ),
                    ]
                    .map(|(id, label, on)| {
                        widgets::dropdown_item(
                            SharedString::from(id),
                            None,
                            label,
                            (!on).then(|| SharedString::from(strings::DEFAULT_CHOICE)),
                            on == prefs.autocomplete,
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, _, cx| this.set_autocomplete(on, cx),
                        ))
                    }),
                )
            });
        let autocomplete_row = v_flex()
            .id("settings-autocomplete-row")
            .test_support()
            .gap(metrics::SPACE_1)
            .child(super::label(strings::AUTOCOMPLETE, cx))
            .child(widgets::caption(strings::AUTOCOMPLETE_HINT, cx))
            .child(div().w(FIELD_WIDTH).child(autocomplete_trigger))
            .children(autocomplete_list.map(|list| div().w(FIELD_WIDTH).child(list)));
        let contrast_open = self.open == Some(Open::MinContrast);
        let contrast_trigger = widgets::select_trigger(
            "settings-min-contrast",
            None,
            min_contrast_label(prefs.min_contrast),
            contrast_open,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.toggle(Open::MinContrast, window, cx)
        }));
        let contrast_list = contrast_open.then(|| {
            widgets::dropdown_list("settings-min-contrast-list", cx).children(MinContrast::ALL.map(
                |mode| {
                    widgets::dropdown_item(
                        SharedString::from(format!("settings-min-contrast-{}", mode.key())),
                        None,
                        min_contrast_label(mode),
                        min_contrast_hint(mode).map(SharedString::from),
                        mode == prefs.min_contrast,
                        cx,
                    )
                    .on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| this.set_min_contrast(mode, cx),
                    ))
                },
            ))
        });
        let contrast_row = v_flex()
            .id("settings-min-contrast-row")
            .test_support()
            .gap(metrics::SPACE_1)
            .child(super::label(strings::MIN_CONTRAST, cx))
            .child(widgets::caption(strings::MIN_CONTRAST_HINT, cx))
            .child(div().w(FIELD_WIDTH).child(contrast_trigger))
            .children(contrast_list.map(|list| div().w(FIELD_WIDTH).child(list)));
        block("settings-terminal-block", strings::TERMINAL_BLOCK, cx)
            .child(autocomplete_row)
            .child(toggle_row(
                "settings-autoreconnect",
                strings::AUTORECONNECT,
                strings::AUTORECONNECT_HINT,
                prefs.autoreconnect,
                Self::set_autoreconnect,
                cx,
            ))
            .child(toggle_row(
                "settings-record-history",
                strings::RECORD_HISTORY,
                strings::RECORD_HISTORY_HINT,
                prefs.record_history,
                Self::set_record_history,
                cx,
            ))
            .child(toggle_row(
                "settings-copy-on-select",
                strings::COPY_ON_SELECT,
                strings::COPY_ON_SELECT_HINT,
                prefs.copy_on_select,
                Self::set_copy_on_select,
                cx,
            ))
            .child(toggle_row(
                "settings-bell-sound",
                strings::BELL_SOUND,
                strings::BELL_SOUND_HINT,
                prefs.bell_sound,
                Self::set_bell_sound,
                cx,
            ))
            .child(toggle_row(
                "settings-bold-bright",
                strings::BOLD_IS_BRIGHT,
                strings::BOLD_IS_BRIGHT_HINT,
                prefs.bold_is_bright,
                Self::set_bold_is_bright,
                cx,
            ))
            .child(toggle_row(
                "settings-highlighting",
                strings::HIGHLIGHTING,
                strings::HIGHLIGHTING_HINT,
                prefs.highlighting,
                Self::set_highlighting,
                cx,
            ))
            .when(prefs.highlighting, |this| {
                this.child(self.highlight_rules.clone())
            })
            .child(contrast_row)
            .child(
                v_flex()
                    .id("settings-term-row")
                    .test_support()
                    .gap(metrics::SPACE_1)
                    .child(super::label(strings::TERM, cx))
                    .child(widgets::caption(strings::TERM_HINT, cx))
                    .child(div().w(FIELD_WIDTH).child(term_trigger))
                    .children(term_list.map(|list| div().w(FIELD_WIDTH).child(list))),
            )
    }

    fn render_font_block(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let prefs = &self.prefs;
        let open = self.open == Some(Open::Font);
        let trigger =
            widgets::select_trigger("settings-font", None, prefs.font_family.clone(), open, cx)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.toggle(Open::Font, window, cx)
                }));
        let list = open.then(|| {
            let choices = self.font_choices(cx);
            let loading = self.fonts.is_none();
            let current = prefs.font_family.clone();
            v_flex()
                .gap(metrics::SPACE_1)
                .child(widgets::text_input(
                    &self.font_search,
                    "settings-font-search",
                    false,
                    window,
                    cx,
                ))
                .child(
                    widgets::dropdown_list("settings-font-list", cx)
                        .max_h(widgets::PICKER_MAX_HEIGHT)
                        .overflow_y_scroll()
                        .children(choices.into_iter().map(|family| {
                            let bundled = family.as_ref() == BUNDLED_FONT;
                            let selected = family == current.as_str();
                            let id = SharedString::from(format!("settings-font-{family}"));
                            let chosen = family.clone();
                            widgets::dropdown_item(
                                id,
                                None,
                                family,
                                bundled.then(|| SharedString::from(strings::FONT_BUNDLED)),
                                selected,
                                cx,
                            )
                            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                this.set_font_family(&chosen, window, cx)
                            }))
                            .into_any_element()
                        }))
                        .when(loading, |list| {
                            list.child(
                                div()
                                    .px(metrics::SPACE_2)
                                    .py(metrics::SPACE_1)
                                    .child(widgets::caption(strings::FONTS_LOADING, cx)),
                            )
                        }),
                )
        });
        let at_min = prefs.font_size <= FONT_SIZE_MIN;
        let at_max = prefs.font_size >= FONT_SIZE_MAX;
        let minus = step_button("settings-font-size-minus", IconName::Minus, !at_min, cx).when(
            !at_min,
            |this| {
                this.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.step_font_size(-1, window, cx)
                }))
            },
        );
        let plus = step_button("settings-font-size-plus", IconName::Plus, !at_max, cx).when(
            !at_max,
            |this| {
                this.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.step_font_size(1, window, cx)
                }))
            },
        );
        let size = h_flex()
            .gap(metrics::SPACE_2)
            .items_center()
            .child(minus)
            .child(div().w(metrics::rpx(64.)).child(widgets::text_input(
                &self.font_size,
                "settings-font-size",
                self.font_size_invalid,
                window,
                cx,
            )))
            .child(plus);
        let line_at_min = self.prefs.line_height <= LINE_HEIGHT_MIN + 0.001;
        let line_at_max = self.prefs.line_height >= LINE_HEIGHT_MAX - 0.001;
        let line_minus =
            step_button(
                "settings-line-height-minus",
                IconName::Minus,
                !line_at_min,
                cx,
            )
            .when(!line_at_min, |this| {
                this.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.step_line_height(-1, window, cx)
                }))
            });
        let line_plus =
            step_button(
                "settings-line-height-plus",
                IconName::Plus,
                !line_at_max,
                cx,
            )
            .when(!line_at_max, |this| {
                this.on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.step_line_height(1, window, cx)
                }))
            });
        let line_height = h_flex()
            .gap(metrics::SPACE_2)
            .items_center()
            .child(line_minus)
            .child(div().w(metrics::rpx(64.)).child(widgets::text_input(
                &self.line_height,
                "settings-line-height",
                self.line_height_invalid,
                window,
                cx,
            )))
            .child(line_plus);
        block("settings-font-block", strings::FONT_BLOCK, cx)
            .child(
                v_flex()
                    .id("settings-font-row")
                    .test_support()
                    .gap(metrics::SPACE_1)
                    .child(super::label(strings::FONT, cx))
                    .child(div().w(FIELD_WIDTH).child(trigger))
                    .children(list.map(|list| div().w(FIELD_WIDTH).child(list))),
            )
            .child(
                v_flex()
                    .id("settings-font-size-row")
                    .test_support()
                    .gap(metrics::SPACE_1)
                    .child(super::label(strings::FONT_SIZE, cx))
                    .child(size)
                    .when(self.font_size_invalid, |this| {
                        this.child(error_text(
                            "settings-font-size-error",
                            strings::font_size_range(FONT_SIZE_MIN, FONT_SIZE_MAX),
                            cx,
                        ))
                    }),
            )
            .child(
                v_flex()
                    .id("settings-line-height-row")
                    .test_support()
                    .gap(metrics::SPACE_1)
                    .child(super::label(strings::LINE_HEIGHT, cx))
                    .child(line_height)
                    .when(self.line_height_invalid, |this| {
                        this.child(error_text(
                            "settings-line-height-error",
                            strings::line_height_range(LINE_HEIGHT_MIN, LINE_HEIGHT_MAX),
                            cx,
                        ))
                    }),
            )
    }

    fn render_theme_block(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.prefs.default_theme;
        let cards: Vec<AnyElement> = BuiltinTheme::all()
            .into_iter()
            .map(|choice| {
                theme_card(
                    SharedString::from(format!("settings-theme-{}", choice.slug())),
                    choice.theme().as_ref(),
                    choice.label(),
                    choice == current,
                    // The same list and "new" marks as the sidebar (UI 12.4, 4.2).
                    choice.is_new(),
                    THEME_CARD_WIDTH,
                    cx,
                )
                .tab_index(0)
                .focus_ring_visible(cx.theme().secondary, cx)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_default_theme(choice, cx)
                }))
                .into_any_element()
            })
            .collect();
        block("settings-theme-block", strings::THEME_BLOCK, cx)
            .child(widgets::caption(strings::THEME_HINT, cx))
            // Cards of one row share its height (a wrapped name).
            .child(
                h_flex()
                    .flex_wrap()
                    .items_stretch()
                    .gap(metrics::SPACE_3)
                    .children(cards),
            )
    }

    fn render_keepalive_block(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        block("settings-keepalive-block", strings::KEEPALIVE_BLOCK, cx).child(
            v_flex()
                .id("settings-keepalive-row")
                .test_support()
                .gap(metrics::SPACE_1)
                .child(widgets::caption(strings::KEEPALIVE_HINT, cx))
                .child(super::label(strings::KEEPALIVE_INTERVAL, cx))
                .child(
                    h_flex()
                        .gap(metrics::SPACE_2)
                        .items_center()
                        .child(div().w(metrics::rpx(96.)).child(widgets::text_input(
                            &self.keepalive,
                            "settings-keepalive",
                            self.keepalive_invalid,
                            window,
                            cx,
                        )))
                        .child(widgets::caption(strings::SECONDS, cx)),
                )
                .when(self.keepalive_invalid, |this| {
                    this.child(error_text(
                        "settings-keepalive-error",
                        strings::keepalive_range(KEEPALIVE_MAX),
                        cx,
                    ))
                }),
        )
    }
}

/// Which number field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    FontSize,
    LineHeight,
    Keepalive,
}

/// The name of a minimum contrast choice.
pub fn min_contrast_label(mode: MinContrast) -> &'static str {
    match mode {
        MinContrast::Off => strings::MIN_CONTRAST_OFF,
        MinContrast::Palette => strings::MIN_CONTRAST_PALETTE,
        MinContrast::Always => strings::MIN_CONTRAST_ALWAYS,
    }
}

/// The explanation under a minimum contrast choice in the list.
fn min_contrast_hint(mode: MinContrast) -> Option<&'static str> {
    match mode {
        MinContrast::Off => None,
        MinContrast::Palette => Some(strings::MIN_CONTRAST_PALETTE_HINT),
        MinContrast::Always => Some(strings::MIN_CONTRAST_ALWAYS_HINT),
    }
}

/// Width of the select-like fields.
const FIELD_WIDTH: gpui::Rems = metrics::rpx(280.);
/// Size of a theme preview card.
const THEME_CARD_WIDTH: gpui::Rems = metrics::rpx(148.);

/// A settings block: the heading (14 px, 600) and its rows, 12 px apart.
fn block(id: &'static str, title: &'static str, cx: &App) -> gpui::Stateful<gpui::Div> {
    v_flex().id(id).gap(metrics::SPACE_3).child(
        div()
            .text_size(metrics::FONT_SIZE_BODY)
            .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
            .text_color(cx.theme().popover_foreground)
            .child(title),
    )
}

/// The reason under an invalid number field (12 px, `--danger`).
fn error_text(id: &'static str, text: String, cx: &App) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .text_size(metrics::FONT_SIZE_CAPTION)
        .text_color(cx.theme().danger)
        .child(text)
}

/// The -/+ button of the font size (36 px square, outlined; disabled at
/// the limit: the whole control at 0.45 without hover).
pub(crate) fn step_button(
    id: &'static str,
    icon: IconName,
    enabled: bool,
    cx: &App,
) -> widgets::Hit {
    let theme = cx.theme();
    let hover = theme.secondary_hover;
    let pressed = theme.secondary_active;
    widgets::hit(id)
        .flex_none()
        .size(metrics::HEIGHT_CONTROL)
        .flex()
        .items_center()
        .justify_center()
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .border_color(theme.input)
        .text_color(theme.foreground)
        .map(|this| {
            if enabled {
                this.cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .active(move |style| style.bg(pressed))
            } else {
                this.opacity(metrics::OPACITY_DISABLED)
            }
        })
        .child(gpui_kit::component::Icon::new(icon).size(metrics::rpx(16.)))
}

/// A theme choice: a preview in the theme's own colors (background, text
/// and six ANSI colors) and its name; the current one has the `--accent`
/// border and the `--accent-subtle` fill. `new` adds the neutral "New"
/// mark of themes added in this release (UI 4.2). Shared by Settings >
/// Terminal and the terminal sidebar's Appearance tab.
pub(crate) fn theme_card(
    id: impl Into<ElementId>,
    terminal: Option<&theme::terminal::TerminalTheme>,
    name: impl Into<SharedString>,
    selected: bool,
    new: bool,
    width: gpui::Rems,
    cx: &App,
) -> widgets::Hit {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let hover = theme.secondary_hover;
    let preview = terminal.map(|terminal| {
        let swatches: Vec<AnyElement> = [1usize, 2, 3, 4, 5, 6]
            .into_iter()
            .map(|index| {
                div()
                    .size(metrics::rpx(10.))
                    .rounded(metrics::rpx(2.))
                    .bg(terminal.ansi.0[index].hsla())
                    .into_any_element()
            })
            .collect();
        v_flex()
            .h(metrics::rpx(56.))
            .p(metrics::SPACE_2)
            .gap(metrics::SPACE_1)
            .rounded(metrics::RADIUS_CONTROL)
            .bg(terminal.background.hsla())
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(terminal.foreground.hsla())
                    .child(strings::THEME_PREVIEW_SAMPLE),
            )
            .child(h_flex().gap(metrics::rpx(3.)).children(swatches))
    });
    let mark = new.then(|| {
        div()
            .flex_none()
            .px(metrics::SPACE_1)
            .rounded(metrics::RADIUS_CONTROL)
            .border_1()
            .border_color(theme.input)
            .text_size(metrics::FONT_SIZE_CAPTION)
            .text_color(theme.muted_foreground)
            .child(strings::THEME_NEW)
    });
    widgets::hit(id)
        .w(width)
        .flex_none()
        .p(metrics::SPACE_1)
        .flex()
        .flex_col()
        .gap(metrics::SPACE_1)
        .rounded(metrics::RADIUS_CARD)
        .border_1()
        .cursor_pointer()
        .map(|this| {
            if selected {
                this.border_color(theme.primary)
                    .bg(tokens.accent_subtle.hsla())
            } else {
                this.border_color(theme.input)
                    .hover(move |style| style.bg(hover))
            }
        })
        .children(preview)
        .child(
            h_flex()
                .px(metrics::SPACE_1)
                .gap(metrics::SPACE_1)
                .items_center()
                .text_size(metrics::FONT_SIZE_DENSE)
                .text_color(theme.popover_foreground)
                // A long name wraps rather than being cut next to the "new"
                // mark (Catppuccin Mocha in the 320 px terminal sidebar).
                .child(div().flex_1().min_w_0().child(name.into()))
                .children(mark)
                .when(selected, |this| {
                    this.child(
                        gpui_kit::component::Icon::new(IconName::Check)
                            .size(metrics::rpx(14.))
                            .text_color(theme.primary),
                    )
                }),
        )
}

impl Render for TerminalSection {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let heading = div()
            .text_size(metrics::FONT_SIZE_HEADING)
            .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
            .text_color(theme.popover_foreground)
            .child(strings::TERMINAL);
        let error = self.error.clone().map(|error| {
            div()
                .id("settings-error")
                .test_support()
                .text_size(metrics::FONT_SIZE_CAPTION)
                .text_color(cx.theme().danger)
                .child(error)
        });
        v_flex()
            .id("settings-terminal")
            .test_support()
            .gap(metrics::SPACE_4)
            .pb(metrics::SPACE_4)
            .child(heading)
            .children(error)
            .child(self.render_terminal_block(cx))
            .child(widgets::divider(cx))
            .child(self.render_font_block(window, cx))
            .child(widgets::divider(cx))
            .child(self.render_theme_block(cx))
            .child(widgets::divider(cx))
            .child(self.render_keepalive_block(window, cx))
            .child(widgets::divider(cx))
            .child(self.generation.clone())
    }
}

#[cfg(test)]
mod unit_tests {
    use super::*;

    #[test]
    fn numbers_parse_within_their_limits() {
        assert_eq!(parse_number("14", 6, 72), Some(14));
        assert_eq!(parse_number(" 0 ", 0, 3600), Some(0));
        assert_eq!(parse_number("5", 6, 72), None);
        assert_eq!(parse_number("73", 6, 72), None);
        assert_eq!(parse_number("-1", 0, 3600), None);
        assert_eq!(parse_number("1.5", 0, 3600), None);
        assert_eq!(parse_number("", 0, 3600), None);
        assert_eq!(parse_number("99999999999", 0, 3600), None);
        assert_eq!(parse_line_height("1.2"), Some(1.2));
        assert_eq!(parse_line_height(" 1,5 "), Some(1.5));
        assert_eq!(parse_line_height("2"), Some(2.0));
        assert_eq!(parse_line_height("0.9"), None);
        assert_eq!(parse_line_height("2.1"), None);
        assert_eq!(parse_line_height("-1"), None);
        assert_eq!(parse_line_height(""), None);
        assert_eq!(line_height_text(1.2), "1.2");
        assert_eq!(line_height_text(1.25), "1.25");
        assert_eq!(line_height_value(1.3), json!(1.3));
        assert_eq!(stepped_line_height(1.2, 1), 1.3);
        assert_eq!(stepped_line_height(1.25, -1), 1.2);
        assert_eq!(stepped_line_height(2.0, 1), 2.0);
        assert_eq!(stepped_line_height(1.0, -1), 1.0);
    }

    #[test]
    fn prefs_default_to_the_spec_values() {
        let prefs = TerminalPrefs::from_values(&Default::default());
        assert!(prefs.autoreconnect);
        assert!(!prefs.copy_on_select);
        assert!(!prefs.bell_sound);
        assert_eq!(prefs.term, "xterm-256color");
        assert_eq!(prefs.font_family, BUNDLED_FONT);
        assert_eq!(prefs.font_size, 14);
        assert_eq!(prefs.default_theme, BuiltinTheme::GRAPHITE);
        assert_eq!(prefs.keepalive_s, 30);
        assert_eq!(prefs.line_height, 1.2);
        assert_eq!(prefs.min_contrast, MinContrast::Palette);
        // Autocomplete (beta, D142) is off unless the setting is `history`.
        assert!(!prefs.autocomplete);
        let on =
            TerminalPrefs::from_values(&[(KEY_AUTOCOMPLETE.to_owned(), json!("history"))].into());
        assert!(on.autocomplete);
        assert_eq!(
            BuiltinTheme::from_ref(&json!({"builtin": "paper"})),
            Some(BuiltinTheme::PAPER)
        );
        assert_eq!(BuiltinTheme::from_ref(&json!({"builtin": "neon"})), None);
        assert!(BuiltinTheme::PAPER.theme().is_some());
        assert_eq!(BuiltinTheme::GRAPHITE.slug(), "graphite");
        assert_eq!(BuiltinTheme::PAPER.label(), "Paper");
        assert_eq!(BuiltinTheme::default(), BuiltinTheme::GRAPHITE);
        // Every built-in slug is one the core accepts in `theme_ref`.
        for theme in BuiltinTheme::all() {
            assert!(
                shellpp_core::model::fields::BUILTIN_THEMES.contains(&theme.slug()),
                "{}",
                theme.slug()
            );
            assert_eq!(BuiltinTheme::from_slug(theme.slug()), Some(theme));
        }
    }
}
