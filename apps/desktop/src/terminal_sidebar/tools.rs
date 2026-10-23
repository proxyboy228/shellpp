//! The Tools tab of the terminal sidebar (UI 4.2): search in the terminal
//! output and the scrollback (`crate::terminal::search`).
//!
//! - Plain text, case-insensitive; every match is highlighted in the
//!   terminal (`searchMatch` of the terminal theme), the current one in
//!   `searchMatchActive`; the view scrolls to the current match when it is
//!   not visible.
//! - The search runs as the text changes. Enter goes to the next match
//!   (toward newer output), Shift+Enter to the previous one, both wrapping
//!   around; the arrow buttons do the same. The counter reads "N of M".
//! - Esc returns the focus to the terminal and keeps the highlights. The
//!   highlights go away when the field is emptied or the sidebar is hidden,
//!   and move with the search when the active pane changes [decision].
//! - Autocomplete (beta, D142): a menu of its modes, Off or this host's
//!   history; the same setting as Settings > Terminal (UI 12.4).

use gpui::prelude::*;
use gpui::{ClickEvent, Context, Entity, IntoElement, Render, SharedString, Subscription, Window};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{self, Input, InputEvent, InputState};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, FocusableExt as _, Icon, Sizable as _, h_flex, v_flex,
};

use super::{SidebarTarget, heading, strings};
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::settings::terminal::{KEY_AUTOCOMPLETE, TerminalPrefs, save_setting};
use crate::state::AppState;
use crate::terminal::{SearchSummary, SearchUpdated, TerminalView};
use crate::theme::focus::FocusRingExt as _;
use crate::theme::{controls, metrics};

/// Size of the previous / next buttons.
const STEP_BUTTON: gpui::Rems = metrics::rpx(32.);

/// The Tools tab.
pub struct ToolsTab {
    state: Entity<AppState>,
    query: Entity<InputState>,
    target: Option<SidebarTarget>,
    /// The sidebar is on screen: the search runs and highlights.
    active: bool,
    summary: Option<SearchSummary>,
    terminal_subscription: Option<Subscription>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for ToolsTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToolsTab")
            .field("active", &self.active)
            .field("summary", &self.summary)
            .finish_non_exhaustive()
    }
}

impl ToolsTab {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let query = cx.new(|cx| {
            let mut input = InputState::new(window, cx).placeholder(strings::SEARCH_PLACEHOLDER);
            input.set_disabled(true, cx);
            input
        });
        let subscriptions = vec![
            cx.subscribe(&query, |this, _, event: &InputEvent, cx| match event {
                InputEvent::Change => this.run(cx),
                InputEvent::PressEnter { shift, .. } => this.step(!*shift, cx),
                InputEvent::Focus | InputEvent::Blur => {}
            }),
            // The mode menu follows the setting wherever it changes.
            cx.observe_global::<TerminalPrefs>(|_, cx| cx.notify()),
        ];
        Self {
            state,
            query,
            target: None,
            active: false,
            summary: None,
            terminal_subscription: None,
            _subscriptions: subscriptions,
        }
    }

    /// Autocomplete's mode (D142): on suggests from the host's history.
    pub fn set_autocomplete(&mut self, on: bool, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let value = serde_json::json!(if on { "history" } else { "off" });
        let _ = save_setting(&core, KEY_AUTOCOMPLETE, value, cx);
        cx.notify();
    }

    fn render_autocomplete(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let on = TerminalPrefs::current(cx).autocomplete;
        let tab = cx.entity().downgrade();
        let menu = ControlButton::new("terminal-autocomplete", ControlKind::Neutral)
            .label(if on {
                strings::AUTOCOMPLETE_HISTORY
            } else {
                strings::AUTOCOMPLETE_OFF
            })
            .trailing_icon(IconName::ChevronDown)
            .dropdown_menu(move |mut menu, _, cx| {
                menu = menu.min_w(metrics::scaled(200., cx));
                for (label, mode) in [
                    (strings::AUTOCOMPLETE_OFF, false),
                    (strings::AUTOCOMPLETE_HISTORY, true),
                ] {
                    let tab = tab.clone();
                    menu = menu.item(PopupMenuItem::new(label).checked(mode == on).on_click(
                        move |_, _, cx| {
                            let _ = tab.update(cx, |tab, cx| tab.set_autocomplete(mode, cx));
                        },
                    ));
                }
                menu
            });
        let theme = cx.theme();
        v_flex()
            .id("terminal-autocomplete-block")
            .test_support()
            .gap(metrics::SPACE_2)
            .child(
                gpui::div()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                    .text_color(theme.foreground)
                    .child(strings::AUTOCOMPLETE),
            )
            .child(h_flex().child(menu))
            .child(widgets::caption(strings::AUTOCOMPLETE_HINT, cx))
    }

    /// The search text.
    pub fn query(&self, cx: &gpui::App) -> SharedString {
        self.query.read(cx).value()
    }

    /// The counter of the current search, if one runs.
    pub fn summary(&self) -> Option<SearchSummary> {
        self.summary
    }

    fn terminal(&self) -> Option<Entity<TerminalView>> {
        self.target.as_ref().and_then(SidebarTarget::terminal)
    }

    /// The active pane changed: the old terminal loses its highlights and
    /// the search runs in the new one.
    pub fn set_target(
        &mut self,
        target: Option<SidebarTarget>,
        active: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(old) = self.terminal() {
            old.update(cx, |terminal, cx| terminal.clear_search(cx));
        }
        self.terminal_subscription =
            target
                .as_ref()
                .and_then(SidebarTarget::terminal)
                .map(|terminal| {
                    cx.subscribe(&terminal, |this, terminal, _: &SearchUpdated, cx| {
                        this.summary = terminal.read(cx).search_summary();
                        cx.notify();
                    })
                });
        let enabled = target.as_ref().and_then(SidebarTarget::terminal).is_some();
        self.query
            .update(cx, |input, cx| input.set_disabled(!enabled, cx));
        self.target = target;
        self.active = active;
        self.run(cx);
    }

    /// The sidebar was shown (the search runs again) or hidden (the
    /// highlights go away).
    pub fn set_active(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.active != active {
            self.active = active;
            self.run(cx);
        }
    }

    /// Replaces the search text and searches (tests, Ctrl+Shift+F with a
    /// selection later).
    pub fn set_query(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.query
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        self.run(cx);
    }

    /// Focuses the search field with its text selected (Ctrl+Shift+F) and
    /// searches again.
    pub fn focus_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.active = true;
        self.query.update(cx, |input, cx| {
            input.focus(window, cx);
            input.select_all(window, cx);
        });
        self.run(cx);
    }

    /// Searches for the field's text (an empty text ends the search).
    fn run(&mut self, cx: &mut Context<Self>) {
        let query = self.query(cx).to_string();
        let Some(terminal) = self.terminal() else {
            self.summary = None;
            cx.notify();
            return;
        };
        if !self.active || query.is_empty() {
            terminal.update(cx, |terminal, cx| terminal.clear_search(cx));
            self.summary = None;
        } else {
            self.summary = Some(terminal.update(cx, |terminal, cx| terminal.search(&query, cx)));
        }
        cx.notify();
    }

    /// The next (`forward`) or the previous match.
    pub fn step(&mut self, forward: bool, cx: &mut Context<Self>) {
        let query = self.query(cx);
        let Some(terminal) = self.terminal().filter(|_| self.active && !query.is_empty()) else {
            return;
        };
        if terminal.read(cx).search_query() != Some(query.as_ref()) {
            self.run(cx);
        }
        self.summary = Some(terminal.update(cx, |terminal, cx| terminal.search_step(forward, cx)));
        cx.notify();
    }

    /// Esc in the field: back to the terminal, the highlights stay.
    fn focus_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(terminal) = self.terminal() {
            terminal.update(cx, |terminal, cx| terminal.focus(window, cx));
        }
    }

    fn step_button(
        &self,
        id: &'static str,
        icon: IconName,
        tooltip: &'static str,
        forward: bool,
        enabled: bool,
        cx: &mut Context<Self>,
    ) -> Button {
        let surface = cx.theme().colors.list;
        let button = Button::new(id)
            .custom(controls::ghost(cx))
            .icon(Icon::new(icon).size(metrics::rpx(16.)))
            .size(STEP_BUTTON)
            .rounded(metrics::to_px(metrics::RADIUS_CONTROL, cx))
            .tooltip(tooltip)
            .focus_ring(false)
            .focus_ring_visible(surface, cx);
        if enabled {
            button.on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.step(forward, cx)))
        } else {
            // Disabled per design-system 6: no hover, the whole button at 0.45.
            button
                .disabled(true)
                .text_color(cx.theme().muted_foreground)
                .opacity(metrics::OPACITY_DISABLED)
        }
    }
}

impl Render for ToolsTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let has_terminal = self.terminal().is_some();
        let query_empty = self.query(cx).is_empty();
        let count = self.summary.map_or(0, |summary| summary.count);
        let counter: Option<SharedString> = match self.summary {
            _ if query_empty || !has_terminal => None,
            Some(summary) if summary.count > 0 => Some(
                strings::match_counter(summary.current, summary.count, summary.truncated).into(),
            ),
            _ => Some(strings::NO_MATCHES.into()),
        };
        let caption = if !has_terminal {
            Some(strings::SEARCH_NO_SESSION)
        } else if query_empty {
            Some(strings::SEARCH_HINT)
        } else {
            None
        };
        let icon = Icon::new(IconName::Search)
            .small()
            .text_color(theme.muted_foreground);
        let field = widgets::text_input_with(
            Input::new(&self.query).prefix(icon),
            &self.query,
            "terminal-search",
            false,
            window,
            cx,
        );
        let previous = self.step_button(
            "terminal-search-previous",
            IconName::ArrowUp,
            strings::PREVIOUS_MATCH,
            false,
            count > 0,
            cx,
        );
        let next = self.step_button(
            "terminal-search-next",
            IconName::ArrowDown,
            strings::NEXT_MATCH,
            true,
            count > 0,
            cx,
        );
        let theme = cx.theme();
        v_flex()
            .id("terminal-sidebar-tools")
            .test_support()
            .p(metrics::SPACE_4)
            .gap(metrics::SPACE_3)
            .on_action(
                cx.listener(|this, _: &input::Escape, window, cx| this.focus_terminal(window, cx)),
            )
            .child(heading(strings::TOOLS, cx))
            .child(
                v_flex()
                    .gap(metrics::SPACE_2)
                    .child(
                        gpui::div()
                            .text_size(metrics::FONT_SIZE_BODY)
                            .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                            .text_color(theme.foreground)
                            .child(strings::SEARCH),
                    )
                    .child(field)
                    .child(
                        h_flex()
                            .gap(metrics::SPACE_1)
                            .items_center()
                            .child(
                                gpui::div()
                                    .id("terminal-search-count")
                                    .test_support()
                                    .flex_1()
                                    .min_w_0()
                                    .truncate()
                                    .text_size(metrics::FONT_SIZE_DENSE)
                                    .text_color(theme.muted_foreground)
                                    .children(counter),
                            )
                            .child(previous)
                            .child(next),
                    )
                    .children(caption.map(|caption| widgets::caption(caption, cx))),
            )
            .child(self.render_autocomplete(cx))
    }
}
