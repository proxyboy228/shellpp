//! The Appearance tab of the terminal sidebar (UI 4.2; lead decision L6).
//!
//! - The Font row leads to the Font page: the family (the bundled
//!   JetBrains Mono first, then the installed monospace fonts, with a
//!   search field), the text size (6-72) and the line spacing (1.0-2.0,
//!   1.2 by default). These are the global settings of Settings > Terminal
//!   (`terminal.font_family`, `terminal.font_size`, `terminal.line_height`):
//!   every change is saved at once and every open terminal follows. Esc or
//!   the back button returns to the main page.
//! - The theme list: a preview in each theme's own colors and its name; the
//!   session's theme is highlighted; themes added in this release carry a
//!   "New" mark (no popularity counters). A click applies the theme to the
//!   session at once and, for a saved host, saves it in the host's
//!   `terminal_theme`; quick connect and local terminals keep it for the
//!   session only. When the host cannot be saved, the theme still applies
//!   to the session and the tab says that it was not saved.

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use gpui::Focusable as _;
use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, IntoElement, Render, SharedString,
    Subscription, Task, Window, div,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{self, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, FocusableExt as _, Icon, h_flex, v_flex};
use serde_json::{Value, json};
use shellpp_core::CoreError;

use super::{SidebarTarget, heading, strings};
use crate::panels::widgets;
use crate::settings::terminal::{
    self as terminal_settings, BUNDLED_FONT, BuiltinTheme, FONT_SIZE_MAX, FONT_SIZE_MIN,
    KEY_FONT_FAMILY, KEY_FONT_SIZE, KEY_LINE_HEIGHT, LINE_HEIGHT_MAX, LINE_HEIGHT_MIN,
    TerminalPrefs, line_height_text, line_height_value, parse_line_height, parse_number,
    stepped_line_height,
};
use crate::state::AppState;
use crate::theme::focus::FocusRingExt as _;
use crate::theme::terminal::TerminalTheme;
use crate::theme::{controls, metrics};

/// Width of a theme card: two columns in the sidebar, 8 px apart. The
/// sidebar's 1 px left border is taken off its width, and the result is
/// rounded down, so the two cards always fit one row.
fn theme_card_width() -> gpui::Rems {
    let width = metrics::design_px(super::WIDTH)
        - 1.
        - metrics::design_px(metrics::SPACE_4) * 2.
        - metrics::design_px(metrics::SPACE_2);
    metrics::rpx((width / 2.).floor())
}

/// One theme of the list.
#[derive(Clone, Debug, PartialEq)]
pub struct ThemeChoice {
    slug: SharedString,
    name: SharedString,
    theme: TerminalTheme,
    new: bool,
}

impl ThemeChoice {
    /// The built-in slug (`{"builtin": "<slug>"}`).
    pub fn slug(&self) -> &SharedString {
        &self.slug
    }

    pub fn name(&self) -> &SharedString {
        &self.name
    }

    pub fn theme(&self) -> &TerminalTheme {
        &self.theme
    }

    /// Added in this release ("New").
    pub fn is_new(&self) -> bool {
        self.new
    }
}

/// The themes of the list, in the registry's order.
pub fn theme_choices() -> Vec<ThemeChoice> {
    BuiltinTheme::all()
        .into_iter()
        .filter_map(|choice| {
            Some(ThemeChoice {
                slug: choice.slug().into(),
                name: choice.label().into(),
                theme: choice.theme()?,
                new: choice.is_new(),
            })
        })
        .collect()
}

/// The page of the tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Page {
    #[default]
    Main,
    Font,
}

/// A number field of the Font page.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Field {
    FontSize,
    LineHeight,
}

/// The Appearance tab.
pub struct AppearanceTab {
    state: Entity<AppState>,
    target: Option<SidebarTarget>,
    page: Page,
    themes: Rc<Vec<ThemeChoice>>,
    prefs: TerminalPrefs,
    font_search: Entity<InputState>,
    font_size: Entity<InputState>,
    line_height: Entity<InputState>,
    font_size_invalid: bool,
    line_height_invalid: bool,
    /// Installed monospace families, once listed (the Font page starts it).
    fonts: Option<Arc<Vec<SharedString>>>,
    fonts_task: Option<Task<()>>,
    /// A failed save: shown under the themes or on the Font page.
    notice: Option<&'static str>,
    font_row_focus: FocusHandle,
    card_focus: HashMap<SharedString, FocusHandle>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for AppearanceTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppearanceTab")
            .field("page", &self.page)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

impl AppearanceTab {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let prefs = TerminalPrefs::current(cx);
        let font_search =
            cx.new(|cx| InputState::new(window, cx).placeholder(strings::FONT_SEARCH_PLACEHOLDER));
        let size_text = prefs.font_size.to_string();
        let font_size = cx.new(|cx| InputState::new(window, cx).default_value(size_text));
        let spacing_text = line_height_text(prefs.line_height);
        let line_height = cx.new(|cx| InputState::new(window, cx).default_value(spacing_text));
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
        ];
        Self {
            state,
            target: None,
            page: Page::Main,
            themes: Rc::new(theme_choices()),
            prefs,
            font_search,
            font_size,
            line_height,
            font_size_invalid: false,
            line_height_invalid: false,
            fonts: None,
            fonts_task: None,
            notice: None,
            font_row_focus: cx.focus_handle(),
            card_focus: HashMap::new(),
            _subscriptions: subscriptions,
        }
    }

    pub fn page(&self) -> Page {
        self.page
    }

    /// The themes of the list.
    pub fn themes(&self) -> &[ThemeChoice] {
        &self.themes
    }

    /// The failed save, if any.
    pub fn notice(&self) -> Option<&'static str> {
        self.notice
    }

    /// The active pane changed.
    pub fn set_target(&mut self, target: Option<SidebarTarget>, cx: &mut Context<Self>) {
        if self.target != target {
            self.target = target;
            self.notice = None;
            cx.notify();
        }
    }

    /// The slug of the session's theme, if it is in the list.
    pub fn current_theme(&self, cx: &App) -> Option<SharedString> {
        let terminal = self.target.as_ref()?.terminal()?;
        let theme = terminal.read(cx).theme().clone();
        self.themes
            .iter()
            .find(|choice| choice.theme == theme)
            .map(|choice| choice.slug.clone())
    }

    /// Opens the Font page (the installed fonts are listed in the
    /// background).
    pub fn show_font_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.page = Page::Font;
        self.notice = None;
        self.load_fonts(cx);
        self.font_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        cx.notify();
    }

    /// Back to the theme list.
    pub fn show_main_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.page != Page::Main {
            self.page = Page::Main;
            self.notice = None;
            self.font_row_focus.focus(window, cx);
            cx.notify();
        }
    }

    // Themes ------------------------------------------------------------------

    /// A theme of the list was clicked: the session uses it at once; a saved
    /// host keeps it (`terminal_theme`).
    pub fn choose_theme(&mut self, slug: &str, cx: &mut Context<Self>) {
        let Some(choice) = self
            .themes
            .iter()
            .find(|choice| choice.slug == slug)
            .cloned()
        else {
            return;
        };
        let Some(target) = self.target.clone() else {
            return;
        };
        let Some(terminal) = target.terminal() else {
            return;
        };
        // The default theme follows later changes of the default, as when
        // the session started with it (UI 4.1).
        let follows_default = TerminalPrefs::current(cx).default_theme.slug() == slug;
        let theme = (!follows_default).then(|| choice.theme.clone());
        terminal.update(cx, |terminal, cx| terminal.set_theme(theme, cx));
        self.notice = match target.host_id() {
            Some(host_id) => match self.save_host_theme(host_id, slug, cx) {
                Ok(()) => None,
                Err(error) => {
                    tracing::warn!(code = error.code(), "the host theme was not saved");
                    Some(strings::THEME_NOT_SAVED)
                }
            },
            None => None,
        };
        cx.notify();
    }

    fn save_host_theme(
        &mut self,
        host_id: uuid::Uuid,
        slug: &str,
        cx: &mut Context<Self>,
    ) -> Result<(), CoreError> {
        self.state.update(cx, |state, cx| {
            let host = state.host(host_id).cloned().ok_or(CoreError::NotFound)?;
            let reference = json!({ "builtin": slug });
            if host.data.get("terminal_theme") == Some(&reference) {
                return Ok(());
            }
            let mut data = host.data_map();
            data.insert("terminal_theme".to_owned(), reference);
            state.save_host(Some(host_id), data, cx).map(|_| ())
        })
    }

    // Font --------------------------------------------------------------------

    /// Writes one font setting; the global prefs (and every terminal) follow.
    fn write(&mut self, key: &'static str, value: Value, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        match terminal_settings::save_setting(&core, key, value, cx) {
            Ok(prefs) => {
                self.prefs = prefs;
                self.notice = None;
            }
            Err(_) => self.notice = Some(strings::SETTING_NOT_SAVED),
        }
        cx.notify();
    }

    /// The settings changed (here, in Settings or in another window).
    fn prefs_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = TerminalPrefs::current(cx);
        if current == self.prefs {
            return;
        }
        self.prefs = current;
        for (field, input) in [
            (Field::FontSize, self.font_size.clone()),
            (Field::LineHeight, self.line_height.clone()),
        ] {
            if !input.read(cx).focus_handle(cx).is_focused(window) {
                self.reset_field(field, window, cx);
            }
        }
        cx.notify();
    }

    pub fn set_font_family(&mut self, family: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.font_search
            .update(cx, |input, cx| input.set_value("", window, cx));
        if family != self.prefs.font_family {
            self.write(KEY_FONT_FAMILY, json!(family), cx);
        }
    }

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

    pub fn step_line_height(&mut self, step: i32, window: &mut Window, cx: &mut Context<Self>) {
        let value = stepped_line_height(self.prefs.line_height, step);
        if (value - self.prefs.line_height).abs() > f32::EPSILON {
            self.write(KEY_LINE_HEIGHT, line_height_value(value), cx);
        }
        self.reset_field(Field::LineHeight, window, cx);
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
        };
        if input.read(cx).value() != text.as_str() {
            input.update(cx, |input, cx| input.set_value(text, window, cx));
        }
        cx.notify();
    }

    /// Lists the installed monospace fonts on the background executor (once).
    fn load_fonts(&mut self, cx: &mut Context<Self>) {
        if self.fonts.is_some() || self.fonts_task.is_some() {
            return;
        }
        let text_system = cx.text_system().clone();
        let work = cx
            .background_spawn(async move { terminal_settings::installed_monospace(&text_system) });
        self.fonts_task = Some(cx.spawn(async move |this, cx| {
            let fonts = work.await;
            let _ = this.update(cx, |this, cx| {
                this.fonts = Some(Arc::new(fonts));
                this.fonts_task = None;
                cx.notify();
            });
        }));
    }

    /// The families for the search text: the bundled font first.
    pub fn font_choices(&self, cx: &App) -> Vec<SharedString> {
        let query = self.font_search.read(cx).value().trim().to_lowercase();
        std::iter::once(SharedString::from(BUNDLED_FONT))
            .chain(self.fonts.iter().flat_map(|fonts| fonts.iter().cloned()))
            .filter(|name| query.is_empty() || name.to_lowercase().contains(&query))
            .collect()
    }

    // Rendering ---------------------------------------------------------------

    fn render_main(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let surface = theme.colors.list;
        let hover = theme.secondary_hover;
        let pressed = theme.secondary_active;
        let summary = strings::font_summary(
            &self.prefs.font_family,
            self.prefs.font_size,
            &line_height_text(self.prefs.line_height),
        );
        let font_row = h_flex()
            .id("terminal-appearance-font")
            .test_support()
            .w_full()
            .min_h(metrics::HEIGHT_ROW)
            .px(metrics::SPACE_3)
            .gap(metrics::SPACE_3)
            .items_center()
            .rounded(metrics::RADIUS_CARD)
            .bg(theme.secondary)
            .cursor_pointer()
            .track_focus(&self.font_row_focus)
            .tab_index(0)
            .focus_ring_visible(surface, cx)
            .hover(move |style| style.bg(hover))
            .active(move |style| style.bg(pressed))
            .child(
                Icon::new(IconName::Type)
                    .size(metrics::rpx(16.))
                    .text_color(theme.muted_foreground),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .py(metrics::SPACE_1)
                    .child(
                        div()
                            .text_size(metrics::FONT_SIZE_BODY)
                            .text_color(theme.foreground)
                            .child(strings::FONT),
                    )
                    .child(
                        div()
                            .truncate()
                            .text_size(metrics::FONT_SIZE_CAPTION)
                            .text_color(theme.muted_foreground)
                            .child(summary),
                    ),
            )
            .child(
                Icon::new(IconName::ChevronRight)
                    .size(metrics::rpx(16.))
                    .text_color(theme.muted_foreground),
            )
            .on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.show_font_page(window, cx)),
            );

        let has_terminal = self
            .target
            .as_ref()
            .and_then(SidebarTarget::terminal)
            .is_some();
        let scope = match &self.target {
            _ if !has_terminal => strings::THEME_NO_SESSION,
            Some(target) if target.host_id().is_some() => strings::THEME_SAVED_FOR_HOST,
            _ => strings::THEME_SESSION_ONLY,
        };
        let current = self.current_theme(cx);
        let themes = self.themes.clone();
        let cards: Vec<AnyElement> =
            themes
                .iter()
                .map(|choice| {
                    let focus = self
                        .card_focus
                        .entry(choice.slug.clone())
                        .or_insert_with(|| cx.focus_handle())
                        .clone();
                    let slug = choice.slug.clone();
                    let card = crate::settings::terminal::theme_card(
                        SharedString::from(format!("terminal-theme-{}", choice.slug)),
                        Some(&choice.theme),
                        choice.name.clone(),
                        current.as_ref() == Some(&choice.slug),
                        choice.new,
                        theme_card_width(),
                        cx,
                    )
                    .track_focus(&focus)
                    .tab_index(0)
                    .focus_ring_visible(surface, cx);
                    if has_terminal {
                        card.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                            this.choose_theme(&slug, cx)
                        }))
                        .into_any_element()
                    } else {
                        card.opacity(metrics::OPACITY_DISABLED).into_any_element()
                    }
                })
                .collect();
        let danger = theme.danger;
        v_flex()
            .id("terminal-sidebar-appearance")
            .test_support()
            .p(metrics::SPACE_4)
            .gap(metrics::SPACE_4)
            .child(heading(strings::APPEARANCE, cx))
            .child(font_row)
            .child(
                v_flex()
                    .gap(metrics::SPACE_2)
                    .child(
                        div()
                            .text_size(metrics::FONT_SIZE_BODY)
                            .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                            .text_color(theme.foreground)
                            .child(strings::THEMES),
                    )
                    .child(widgets::caption(scope, cx))
                    .children(self.notice.map(|notice| {
                        div()
                            .id("terminal-appearance-notice")
                            .test_support()
                            .text_size(metrics::FONT_SIZE_CAPTION)
                            .text_color(danger)
                            .child(notice)
                    }))
                    .child(
                        h_flex()
                            .id("terminal-theme-list")
                            .flex_wrap()
                            // Cards of one row share its height (a wrapped name).
                            .items_stretch()
                            .gap(metrics::SPACE_2)
                            .children(cards),
                    ),
            )
            .into_any_element()
    }

    fn render_font(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let surface = cx.theme().colors.list;
        let danger = cx.theme().danger;
        let back = Button::new("terminal-appearance-back")
            .custom(controls::ghost(cx))
            .icon(Icon::new(IconName::ChevronLeft).size(metrics::rpx(16.)))
            .size(metrics::rpx(32.))
            .rounded(metrics::to_px(metrics::RADIUS_CONTROL, cx))
            .tooltip(strings::BACK)
            .focus_ring(false)
            .focus_ring_visible(surface, cx)
            .on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.show_main_page(window, cx)),
            );
        let current = self.prefs.font_family.clone();
        let loading = self.fonts.is_none();
        let families: Vec<AnyElement> = self
            .font_choices(cx)
            .into_iter()
            .map(|family| {
                let bundled = family.as_ref() == BUNDLED_FONT;
                let selected = family == current.as_str();
                let chosen = family.clone();
                widgets::dropdown_item(
                    SharedString::from(format!("terminal-font-{family}")),
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
            })
            .collect();
        let family = v_flex()
            .gap(metrics::SPACE_1)
            .child(label(strings::FONT_FAMILY, cx))
            .child(widgets::text_input(
                &self.font_search,
                "terminal-font-search",
                false,
                window,
                cx,
            ))
            .child(
                widgets::dropdown_list("terminal-font-list", cx)
                    .max_h(widgets::PICKER_MAX_HEIGHT)
                    .overflow_y_scroll()
                    .children(families)
                    .when(loading, |list| {
                        list.child(
                            div()
                                .px(metrics::SPACE_2)
                                .py(metrics::SPACE_1)
                                .child(widgets::caption(strings::FONTS_LOADING, cx)),
                        )
                    }),
            );
        let size = self.number_row(
            "terminal-font-size",
            strings::FONT_SIZE,
            &self.font_size.clone(),
            self.font_size_invalid,
            self.prefs.font_size <= FONT_SIZE_MIN,
            self.prefs.font_size >= FONT_SIZE_MAX,
            Field::FontSize,
            crate::settings::strings::font_size_range(FONT_SIZE_MIN, FONT_SIZE_MAX),
            window,
            cx,
        );
        let spacing = self.number_row(
            "terminal-line-height",
            strings::LINE_HEIGHT,
            &self.line_height.clone(),
            self.line_height_invalid,
            self.prefs.line_height <= LINE_HEIGHT_MIN + 0.001,
            self.prefs.line_height >= LINE_HEIGHT_MAX - 0.001,
            Field::LineHeight,
            crate::settings::strings::line_height_range(LINE_HEIGHT_MIN, LINE_HEIGHT_MAX),
            window,
            cx,
        );
        v_flex()
            .id("terminal-sidebar-font")
            .test_support()
            .p(metrics::SPACE_4)
            .gap(metrics::SPACE_4)
            .on_action(
                cx.listener(|this, _: &input::Escape, window, cx| this.show_main_page(window, cx)),
            )
            .child(
                h_flex()
                    .gap(metrics::SPACE_2)
                    .items_center()
                    .child(back)
                    .child(heading(strings::FONT, cx)),
            )
            .child(widgets::caption(strings::FONT_SCOPE, cx))
            .children(self.notice.map(|notice| {
                div()
                    .id("terminal-appearance-notice")
                    .test_support()
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(danger)
                    .child(notice)
            }))
            .child(family)
            .child(size)
            .child(spacing)
            .into_any_element()
    }

    /// A number setting: the label, -/+ and the field, the reason under an
    /// invalid value.
    #[allow(clippy::too_many_arguments)]
    fn number_row(
        &self,
        id: &'static str,
        title: &'static str,
        input: &Entity<InputState>,
        invalid: bool,
        at_min: bool,
        at_max: bool,
        field: Field,
        range: String,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let (minus_id, plus_id, error_id) = match field {
            Field::FontSize => (
                "terminal-font-size-minus",
                "terminal-font-size-plus",
                "terminal-font-size-error",
            ),
            Field::LineHeight => (
                "terminal-line-height-minus",
                "terminal-line-height-plus",
                "terminal-line-height-error",
            ),
        };
        let step =
            move |this: &mut Self, step: i32, window: &mut Window, cx: &mut Context<Self>| {
                match field {
                    Field::FontSize => this.step_font_size(step, window, cx),
                    Field::LineHeight => this.step_line_height(step, window, cx),
                }
            };
        let minus = terminal_settings::step_button(
            minus_id,
            gpui_kit::component::IconName::Minus,
            !at_min,
            cx,
        )
        .when(!at_min, |this| {
            this.on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| step(this, -1, window, cx)),
            )
        });
        let plus = terminal_settings::step_button(
            plus_id,
            gpui_kit::component::IconName::Plus,
            !at_max,
            cx,
        )
        .when(!at_max, |this| {
            this.on_click(
                cx.listener(move |this, _: &ClickEvent, window, cx| step(this, 1, window, cx)),
            )
        });
        let danger = cx.theme().danger;
        v_flex()
            .gap(metrics::SPACE_1)
            .child(label(title, cx))
            .child(
                h_flex()
                    .gap(metrics::SPACE_2)
                    .items_center()
                    .child(minus)
                    .child(
                        div()
                            .w(metrics::rpx(64.))
                            .child(widgets::text_input(input, id, invalid, window, cx)),
                    )
                    .child(plus),
            )
            .when(invalid, |this| {
                this.child(
                    div()
                        .id(error_id)
                        .test_support()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(danger)
                        .child(range),
                )
            })
    }
}

/// A field label (14 px, 500).
fn label(text: &'static str, cx: &App) -> gpui::Div {
    div()
        .text_size(metrics::FONT_SIZE_BODY)
        .font_weight(metrics::FONT_WEIGHT_MEDIUM)
        .text_color(cx.theme().foreground)
        .child(text)
}

impl Render for AppearanceTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        match self.page {
            Page::Main => self.render_main(cx),
            Page::Font => self.render_font(window, cx),
        }
    }
}
