//! The Snippets tab of the terminal sidebar (UI 4.2; stage 5, D66): the
//! snippets of the vault with a search. A click puts the snippet at the
//! prompt of the active pane without running it; the run button sends
//! Enter after it. The values of `{{name}}` variables are asked first.
//!
//! Both go as injected input (never recorded in history, docs/security.md
//! 10.1). A script of several lines is pasted as one bracketed paste; a
//! shell without bracketed paste would run each line at once, so there
//! only Run takes it ([`TerminalView::insert_snippet`]): its insert is
//! unavailable, and the tooltip says why (D68).

use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, Entity, IntoElement, Render, SharedString, Subscription,
    Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use shellpp_core::service::snippets::SnippetView;
use uuid::Uuid;

use super::{SidebarTarget, heading, strings};
use crate::panels::widgets;
use crate::state::{AppState, AppStateEvent};
use crate::terminal::TerminalView;
use crate::terminal::fonts::JETBRAINS_MONO;
use crate::theme::focus::FocusRingExt as _;
use crate::theme::metrics;

/// The element id of a snippet's row and of its run button.
pub fn row_id(id: Uuid) -> SharedString {
    format!("sidebar-snippet-{id}").into()
}

pub fn run_id(id: Uuid) -> SharedString {
    format!("sidebar-snippet-run-{id}").into()
}

pub struct SnippetsTab {
    state: Entity<AppState>,
    search: Entity<InputState>,
    snippets: Vec<SnippetView>,
    shown: Vec<usize>,
    target: Option<SidebarTarget>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for SnippetsTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnippetsTab")
            .field("snippets", &self.snippets.len())
            .finish_non_exhaustive()
    }
}

impl SnippetsTab {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(strings::SNIPPETS_SEARCH));
        let subscriptions = vec![
            cx.subscribe(&search, |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.refilter(cx);
                }
            }),
            cx.subscribe(&state, |this: &mut Self, _, event, cx| {
                if matches!(
                    event,
                    AppStateEvent::RecordsChanged | AppStateEvent::LockChanged(_)
                ) {
                    this.reload(cx);
                }
            }),
        ];
        let mut tab = Self {
            state,
            search,
            snippets: Vec::new(),
            shown: Vec::new(),
            target: None,
            _subscriptions: subscriptions,
        };
        tab.reload(cx);
        tab
    }

    pub fn snippets(&self) -> &[SnippetView] {
        &self.snippets
    }

    /// The snippet ids on display.
    pub fn shown_ids(&self) -> Vec<Uuid> {
        self.shown
            .iter()
            .map(|index| self.snippets[*index].record.record_id)
            .collect()
    }

    pub fn set_target(&mut self, target: Option<SidebarTarget>, cx: &mut Context<Self>) {
        self.target = target;
        cx.notify();
    }

    /// Reads the snippets of the vault again (none while locked).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        self.snippets = state
            .core()
            .filter(|_| state.is_unlocked())
            .zip(state.vault_id())
            .and_then(|(core, vault)| core.snippets_list(vault).ok())
            .unwrap_or_default();
        self.refilter(cx);
    }

    fn refilter(&mut self, cx: &mut Context<Self>) {
        let text = self.search.read(cx).value().to_string();
        self.shown = (0..self.snippets.len())
            .filter(|&index| crate::snippets::matches_search(&self.snippets[index], &text))
            .collect();
        cx.notify();
    }

    fn terminal(&self) -> Option<Entity<TerminalView>> {
        self.target.as_ref().and_then(SidebarTarget::terminal)
    }

    /// Whether a click can put the snippet `id` at the prompt now (D68: a
    /// script of several lines only with bracketed paste).
    pub fn can_insert(&self, id: Uuid, cx: &gpui::App) -> bool {
        let Some(snippet) = self
            .snippets
            .iter()
            .find(|snippet| snippet.record.record_id == id)
        else {
            return false;
        };
        self.terminal()
            .is_some_and(|terminal| terminal.read(cx).can_insert(&snippet.script))
    }

    /// Insert (`run` false) or Run: the variables' values first.
    pub fn use_snippet(
        &mut self,
        id: Uuid,
        run: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(snippet) = self
            .snippets
            .iter()
            .find(|snippet| snippet.record.record_id == id)
            .cloned()
        else {
            return;
        };
        // The pane of the click, even if another becomes active while the
        // values are typed (its tab may close meanwhile: nothing is sent).
        let Some(terminal) = self.terminal() else {
            return;
        };
        if snippet.variables.is_empty() {
            Self::send(&terminal, &snippet.script, run, window, cx);
            return;
        }
        let terminal = terminal.downgrade();
        crate::snippets::ask_values(&snippet, window, cx, move |script, window, cx| {
            if let Some(terminal) = terminal.upgrade() {
                Self::send(&terminal, &script, run, window, cx);
            }
        });
    }

    fn send(
        terminal: &Entity<TerminalView>,
        script: &str,
        run: bool,
        window: &mut Window,
        cx: &mut gpui::App,
    ) {
        let sent = terminal.update(cx, |terminal, cx| {
            let sent = terminal.insert_snippet(script, run, cx);
            terminal.focus(window, cx);
            sent
        });
        if !sent {
            crate::shell::show_toast(strings::SNIPPET_NEEDS_RUN, window, cx);
        }
    }

    fn render_row(&self, snippet: &SnippetView, cx: &mut Context<Self>) -> AnyElement {
        let id = snippet.record.record_id;
        let insertable = self.can_insert(id, cx);
        let muted = crate::theme::tokens(cx).text_muted.hsla();
        let theme = cx.theme();
        let surface = theme.colors.list;
        let hover = theme.secondary_hover;
        let pressed = theme.secondary_active;
        let first_line = snippet
            .script
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or_default()
            .to_owned();
        // As `widgets::icon_button`, with the Lucide play icon.
        let run = widgets::hit(run_id(id))
            .flex_none()
            .size(px(32.))
            .flex()
            .items_center()
            .justify_center()
            .rounded(metrics::RADIUS_CONTROL)
            .text_color(theme.muted_foreground)
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
            .active(move |style| style.bg(pressed))
            .child(Icon::new(IconName::Play).small())
            .tooltip(|window, cx| Tooltip::new(strings::SNIPPET_RUN).build(window, cx))
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                cx.stop_propagation();
                this.use_snippet(id, true, window, cx);
            }));
        // Insert unavailable (several lines, no bracketed paste): the row
        // takes no click, and its tooltip says to use Run (D68).
        let texts = v_flex()
            .id(SharedString::from(format!("{}-insert", row_id(id))))
            .test_support()
            .flex_1()
            .min_w_0()
            .child(
                div()
                    .truncate()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .text_color(if insertable { theme.foreground } else { muted })
                    .child(crate::snippets::card_title(snippet)),
            )
            .child(
                div()
                    .truncate()
                    .font_family(JETBRAINS_MONO)
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(theme.muted_foreground)
                    .child(first_line),
            )
            .when(!insertable, |this| {
                this.tooltip(|window, cx| {
                    Tooltip::new(strings::SNIPPET_NEEDS_RUN).build(window, cx)
                })
            });
        h_flex()
            .id(row_id(id))
            .test_support()
            .w_full()
            .py(metrics::SPACE_1)
            .pl(metrics::SPACE_2)
            .gap(metrics::SPACE_1)
            .items_center()
            .rounded(metrics::RADIUS_CONTROL)
            .tab_index(0)
            .focus_ring_visible(surface, cx)
            .when(insertable, |this| {
                this.cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .active(move |style| style.bg(pressed))
            })
            .child(texts)
            .child(run)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                if this.can_insert(id, cx) {
                    this.use_snippet(id, false, window, cx)
                }
            }))
            .into_any_element()
    }
}

impl Render for SnippetsTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let root = v_flex()
            .id("terminal-sidebar-snippets")
            .test_support()
            .p(metrics::SPACE_4)
            .gap(metrics::SPACE_3)
            .child(heading(strings::SNIPPETS, cx));
        if self.terminal().is_none() {
            return root
                .child(
                    div()
                        .id("sidebar-snippets-no-session")
                        .test_support()
                        .child(widgets::caption(strings::SNIPPETS_NO_SESSION, cx)),
                )
                .into_any_element();
        }
        let searching = !self.search.read(cx).value().trim().is_empty();
        let icon = Icon::new(IconName::Search)
            .small()
            .text_color(cx.theme().muted_foreground);
        let field = widgets::text_input_with(
            Input::new(&self.search).prefix(icon),
            &self.search,
            "sidebar-snippets-search",
            false,
            window,
            cx,
        );
        let shown: Vec<SnippetView> = self
            .shown
            .iter()
            .map(|index| self.snippets[*index].clone())
            .collect();
        let rows: Vec<AnyElement> = shown
            .iter()
            .map(|snippet| self.render_row(snippet, cx))
            .collect();
        let empty = rows.is_empty().then_some(if searching {
            strings::SNIPPETS_NO_MATCHES
        } else {
            strings::SNIPPETS_EMPTY
        });
        root.child(field)
            .when_some(empty, |this, message| {
                this.child(
                    div()
                        .id("terminal-sidebar-snippets-empty")
                        .test_support()
                        .child(widgets::caption(message, cx)),
                )
            })
            .when(!rows.is_empty(), |this| {
                this.child(widgets::caption(strings::SNIPPETS_HINT, cx))
                    .child(
                        v_flex()
                            .id("sidebar-snippets-list")
                            .test_support()
                            .gap(px(2.))
                            .children(rows),
                    )
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
