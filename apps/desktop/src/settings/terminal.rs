//! Settings > Terminal (UI 12.4; stage 2a lead decision 12): the blocks this
//! stage uses, top to bottom.
//!
//! - **Terminal**: Autoreconnect (on by default, UI 4.6), Copy on select,
//!   Bell sound (off by default), Bright colors for bold text, Terminal
//!   emulation type (the TERM value: `xterm`, `xterm-256color` (default),
//!   `vt100`, `linux`).
//! - **Font**: the family (the bundled JetBrains Mono first, then the
//!   installed monospace fonts, with a search field) and the size (-/+ and a
//!   number field, 14 by default).
//! - **Terminal theme**: the default theme for new hosts, Graphite or Paper.
//! - **Keepalive**: the interval in seconds, 30 by default, 0 = off.
//!
//! The other blocks of 12.4 (autocomplete, history, cursor, keyword
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

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, Global, IntoElement, SharedString, Subscription,
    Task, TextSystem, Window, div, px,
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
use crate::theme::{self, metrics};

// Setting keys (docs/data-model.md, section 16.1).
pub const KEY_AUTORECONNECT: &str = "terminal.autoreconnect";
pub const KEY_COPY_ON_SELECT: &str = "terminal.copy_on_select";
pub const KEY_BELL_SOUND: &str = "terminal.bell_sound";
pub const KEY_BOLD_IS_BRIGHT: &str = "terminal.bold_is_bright";
pub const KEY_TERM: &str = "terminal.term";
pub const KEY_FONT_FAMILY: &str = "terminal.font_family";
pub const KEY_FONT_SIZE: &str = "terminal.font_size";
pub const KEY_DEFAULT_THEME: &str = "terminal.default_theme";
pub const KEY_KEEPALIVE: &str = "terminal.keepalive_s";

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
/// Limits of the keepalive interval in seconds (0 = off).
pub const KEEPALIVE_MAX: u32 = 3600;
/// The default keepalive interval (UI 12.4).
pub const DEFAULT_KEEPALIVE: u32 = 30;

/// A built-in terminal theme (design-system 4.1, 4.2).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum BuiltinTheme {
    #[default]
    Graphite,
    Paper,
}

impl BuiltinTheme {
    pub const ALL: [BuiltinTheme; 2] = [BuiltinTheme::Graphite, BuiltinTheme::Paper];

    /// The slug of `{"builtin": "<slug>"}` theme references.
    pub fn slug(self) -> &'static str {
        match self {
            BuiltinTheme::Graphite => "graphite",
            BuiltinTheme::Paper => "paper",
        }
    }

    pub fn from_slug(slug: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|theme| theme.slug() == slug)
    }

    /// The theme of a `theme_ref` value (`{"builtin": "graphite"}`); `None`
    /// for anything else.
    pub fn from_ref(value: &Value) -> Option<Self> {
        value
            .get("builtin")
            .and_then(Value::as_str)
            .and_then(Self::from_slug)
    }

    pub fn label(self) -> &'static str {
        match self {
            BuiltinTheme::Graphite => strings::THEME_GRAPHITE,
            BuiltinTheme::Paper => strings::THEME_PAPER,
        }
    }

    /// The parsed theme file (`src/theme/terminal/<slug>.json`).
    pub fn theme(self) -> Option<theme::terminal::TerminalTheme> {
        let index = Self::ALL.iter().position(|theme| *theme == self)?;
        theme::terminal::builtin().into_iter().nth(index)
    }
}

/// The terminal settings in effect (UI 12.4). Also an app global: the
/// Settings window replaces it after every change, and the terminal views
/// observe it (`crate::terminal::settings`) and repaint at once.
#[derive(Clone, Debug, PartialEq)]
pub struct TerminalPrefs {
    pub autoreconnect: bool,
    pub copy_on_select: bool,
    pub bell_sound: bool,
    pub bold_is_bright: bool,
    /// The TERM value sent with the PTY request.
    pub term: String,
    pub font_family: String,
    pub font_size: u32,
    /// The default theme of hosts without their own.
    pub default_theme: BuiltinTheme,
    /// Seconds between keepalives; 0 = off.
    pub keepalive_s: u32,
}

impl Default for TerminalPrefs {
    fn default() -> Self {
        Self {
            autoreconnect: true,
            copy_on_select: false,
            bell_sound: false,
            bold_is_bright: true,
            term: DEFAULT_TERM.to_owned(),
            font_family: BUNDLED_FONT.to_owned(),
            font_size: DEFAULT_FONT_SIZE,
            default_theme: BuiltinTheme::Graphite,
            keepalive_s: DEFAULT_KEEPALIVE,
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
            copy_on_select: flag(KEY_COPY_ON_SELECT, defaults.copy_on_select),
            bell_sound: flag(KEY_BELL_SOUND, defaults.bell_sound),
            bold_is_bright: flag(KEY_BOLD_IS_BRIGHT, defaults.bold_is_bright),
            term: text(KEY_TERM, &defaults.term),
            font_family: text(KEY_FONT_FAMILY, &defaults.font_family),
            font_size: number(KEY_FONT_SIZE, defaults.font_size),
            default_theme: values
                .get(KEY_DEFAULT_THEME)
                .and_then(BuiltinTheme::from_ref)
                .unwrap_or_default(),
            keepalive_s: number(KEY_KEEPALIVE, defaults.keepalive_s),
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
fn installed_monospace(text_system: &TextSystem) -> Vec<SharedString> {
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
    Term,
    Font,
}

/// The Terminal section of the Settings window.
pub struct TerminalSection {
    state: Entity<AppState>,
    prefs: TerminalPrefs,
    open: Option<Open>,
    font_search: Entity<InputState>,
    font_size: Entity<InputState>,
    keepalive: Entity<InputState>,
    font_size_invalid: bool,
    keepalive_invalid: bool,
    /// Installed monospace families, once listed (the first open of the
    /// font list starts it on the background executor).
    fonts: Option<Arc<Vec<SharedString>>>,
    fonts_task: Option<Task<()>>,
    error: Option<SharedString>,
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
        let keepalive_text = prefs.keepalive_s.to_string();
        let keepalive = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(DEFAULT_KEEPALIVE.to_string())
                .default_value(keepalive_text)
        });
        let subscriptions = vec![
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
        Self {
            state,
            prefs,
            open: None,
            font_search,
            font_size,
            keepalive,
            font_size_invalid: false,
            keepalive_invalid: false,
            fonts: None,
            fonts_task: None,
            error: None,
            _subscriptions: subscriptions,
        }
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

    /// Writes one setting; on success the section and the global follow.
    fn write(&mut self, key: &'static str, value: Value, cx: &mut Context<Self>) -> bool {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return false;
        };
        let mut values = std::collections::BTreeMap::new();
        values.insert(key.to_owned(), value);
        match core.settings_set(SettingsSetInput {
            values,
            reset: Vec::new(),
        }) {
            Ok(values) => {
                self.prefs = TerminalPrefs::from_values(&values);
                self.error = None;
                cx.set_global(self.prefs.clone());
                tracing::info!(setting = key, "terminal setting changed");
                cx.notify();
                true
            }
            Err(error) => {
                tracing::warn!(
                    code = error.code(),
                    setting = key,
                    "terminal setting not saved"
                );
                self.error = Some(strings::FAILED.into());
                cx.notify();
                false
            }
        }
    }

    pub fn set_autoreconnect(&mut self, on: bool, cx: &mut Context<Self>) {
        self.write(KEY_AUTORECONNECT, json!(on), cx);
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
        let (input, value) = match field {
            Field::FontSize => {
                self.font_size_invalid = false;
                (self.font_size.clone(), self.prefs.font_size)
            }
            Field::Keepalive => {
                self.keepalive_invalid = false;
                (self.keepalive.clone(), self.prefs.keepalive_s)
            }
        };
        let text = value.to_string();
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
        block("settings-terminal-block", strings::TERMINAL_BLOCK, cx)
            .child(toggle_row(
                "settings-autoreconnect",
                strings::AUTORECONNECT,
                strings::AUTORECONNECT_HINT,
                prefs.autoreconnect,
                Self::set_autoreconnect,
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
            .child(div().w(px(64.)).child(widgets::text_input(
                &self.font_size,
                "settings-font-size",
                self.font_size_invalid,
                window,
                cx,
            )))
            .child(plus);
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
    }

    fn render_theme_block(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let current = self.prefs.default_theme;
        let cards: Vec<AnyElement> = BuiltinTheme::ALL
            .into_iter()
            .map(|choice| {
                theme_card(choice, choice == current, cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.set_default_theme(choice, cx)
                    }))
                    .into_any_element()
            })
            .collect();
        block("settings-theme-block", strings::THEME_BLOCK, cx)
            .child(widgets::caption(strings::THEME_HINT, cx))
            .child(h_flex().gap(metrics::SPACE_3).children(cards))
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
                        .child(div().w(px(96.)).child(widgets::text_input(
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
    Keepalive,
}

/// Width of the select-like fields.
const FIELD_WIDTH: gpui::Pixels = px(280.);
/// Size of a theme preview card.
const THEME_CARD_WIDTH: gpui::Pixels = px(148.);

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
fn step_button(id: &'static str, icon: IconName, enabled: bool, cx: &App) -> widgets::Hit {
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
        .child(gpui_kit::component::Icon::new(icon).size(px(16.)))
}

/// A theme choice: a preview in the theme's own colors (background, text
/// and six ANSI colors) and its name; the current one has the `--accent`
/// border and the `--accent-subtle` name row.
fn theme_card(choice: BuiltinTheme, selected: bool, cx: &App) -> widgets::Hit {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let hover = theme.secondary_hover;
    let preview = choice.theme().map(|terminal| {
        let swatches: Vec<AnyElement> = [1usize, 2, 3, 4, 5, 6]
            .into_iter()
            .map(|index| {
                div()
                    .size(px(10.))
                    .rounded(px(2.))
                    .bg(terminal.ansi.0[index].hsla())
                    .into_any_element()
            })
            .collect();
        v_flex()
            .h(px(56.))
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
            .child(h_flex().gap(px(3.)).children(swatches))
    });
    widgets::hit(SharedString::from(format!(
        "settings-theme-{}",
        choice.slug()
    )))
    .w(THEME_CARD_WIDTH)
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
            .justify_between()
            .items_center()
            .text_size(metrics::FONT_SIZE_DENSE)
            .text_color(theme.popover_foreground)
            .child(choice.label())
            .when(selected, |this| {
                this.child(
                    gpui_kit::component::Icon::new(IconName::Check)
                        .size(px(14.))
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
        assert_eq!(prefs.default_theme, BuiltinTheme::Graphite);
        assert_eq!(prefs.keepalive_s, 30);
        assert_eq!(
            BuiltinTheme::from_ref(&json!({"builtin": "paper"})),
            Some(BuiltinTheme::Paper)
        );
        assert_eq!(BuiltinTheme::from_ref(&json!({"builtin": "neon"})), None);
        assert!(BuiltinTheme::Paper.theme().is_some());
    }
}
