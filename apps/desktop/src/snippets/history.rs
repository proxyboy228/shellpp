//! Shell History (UI 9.1; stage 5, D66): the command history of every
//! host with a search and a host filter; "Save as snippet" opens a new
//! snippet with the command, targeting its host.
//!
//! The entries are what the history capture kept, so the rules of 4.2
//! hold: nothing from quick connects, local terminals, full-screen
//! programs or after a password prompt, and nothing that looks like a
//! secret. The list is read on a background thread; nothing is logged.

use gpui::prelude::*;
use gpui::{
    AnyElement, ClickEvent, Context, Entity, IntoElement, Render, SharedString, Subscription, Task,
    WeakEntity, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, h_flex, v_flex};
use shellpp_core::service::dto::{HistoryEntryView, HistorySearchInput, RecordRef};
use uuid::Uuid;

use super::{SnippetsScreen, strings};
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::state::{AppState, AppStateEvent};
use crate::terminal::fonts::JETBRAINS_MONO;
use crate::theme::metrics;

/// The element id of a history row and of its save button.
pub fn row_id(id: Uuid) -> SharedString {
    format!("shell-history-{id}").into()
}

pub fn save_id(id: Uuid) -> SharedString {
    format!("shell-history-save-{id}").into()
}

/// The entry's time in local time and the locale's format (D68).
fn when(timestamp: &str) -> String {
    crate::dates::timestamp_text(timestamp)
}

pub struct ShellHistory {
    state: Entity<AppState>,
    screen: WeakEntity<SnippetsScreen>,
    search: Entity<InputState>,
    /// The host filter (`None`: every entry).
    host: Option<Uuid>,
    filter_open: bool,
    entries: Vec<HistoryEntryView>,
    loading: Option<Task<()>>,
    failed: bool,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for ShellHistory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ShellHistory")
            .field("entries", &self.entries.len())
            .finish_non_exhaustive()
    }
}

impl ShellHistory {
    pub fn new(
        state: Entity<AppState>,
        screen: WeakEntity<SnippetsScreen>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(strings::HISTORY_SEARCH));
        let subscriptions = vec![
            cx.subscribe(&search, |this: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.reload(cx);
                }
            }),
            cx.subscribe(&state, |this: &mut Self, _, event, cx| {
                if matches!(
                    event,
                    AppStateEvent::LockChanged(_) | AppStateEvent::RecordsChanged
                ) {
                    // A host filtered by is gone: every entry again.
                    if this
                        .host
                        .is_some_and(|id| this.state.read(cx).host(id).is_none())
                    {
                        this.host = None;
                    }
                    this.reload(cx);
                }
            }),
        ];
        let mut history = Self {
            state,
            screen,
            search,
            host: None,
            filter_open: false,
            entries: Vec::new(),
            loading: None,
            failed: false,
            _subscriptions: subscriptions,
        };
        history.reload(cx);
        history
    }

    pub fn entries(&self) -> &[HistoryEntryView] {
        &self.entries
    }

    pub fn host_filter(&self) -> Option<Uuid> {
        self.host
    }

    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    /// Filters by a host (`None`: every entry).
    pub fn set_host(&mut self, host: Option<Uuid>, cx: &mut Context<Self>) {
        self.host = host;
        self.filter_open = false;
        self.reload(cx);
    }

    /// Reads the entries again on a background thread (none while locked).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        let Some((core, vault_id)) = state
            .core()
            .filter(|_| state.is_unlocked())
            .cloned()
            .zip(state.vault_id())
        else {
            self.entries = Vec::new();
            self.loading = None;
            cx.notify();
            return;
        };
        let text = self.search.read(cx).value().to_string();
        let hosts = self
            .host
            .map(|record_id| {
                vec![RecordRef {
                    vault_id,
                    record_id,
                }]
            })
            .unwrap_or_default();
        let input = HistorySearchInput {
            hosts,
            text: Some(text),
            limit: None,
        };
        let found = cx.background_spawn(async move { core.history_search(input) });
        self.loading = Some(cx.spawn(async move |this, cx| {
            let found = found.await;
            let _ = this.update(cx, |history, cx| {
                history.loading = None;
                match found {
                    Ok(entries) => {
                        history.entries = entries;
                        history.failed = false;
                    }
                    Err(error) => {
                        tracing::warn!(code = error.code(), "shell history could not be read");
                        history.entries = Vec::new();
                        history.failed = true;
                    }
                }
                cx.notify();
            });
        }));
        cx.notify();
    }

    /// Save as snippet: a new snippet with the command, on its host.
    pub fn save_as_snippet(&mut self, id: Uuid, window: &mut Window, cx: &mut Context<Self>) {
        let Some(entry) = self.entries.iter().find(|entry| entry.record_id == id) else {
            return;
        };
        let command = entry.command.clone();
        let host = entry.host.map(|host| host.record_id);
        let _ = self.screen.update(cx, |screen, cx| {
            screen.new_snippet_from(&command, host, window, cx)
        });
    }

    fn render_filter(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let hosts = self.state.read(cx).hosts();
        let chosen = self
            .host
            .and_then(|id| hosts.iter().find(|host| host.record_id == id))
            .map_or(strings::ALL_HOSTS.to_owned(), |host| {
                host.title().to_owned()
            });
        let list = self.filter_open.then(|| {
            let all = widgets::dropdown_item(
                "shell-history-host-all",
                None,
                strings::ALL_HOSTS,
                None,
                self.host.is_none(),
                cx,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_host(None, cx)));
            widgets::dropdown_list("shell-history-hosts", cx)
                .absolute()
                .top(metrics::HEIGHT_CONTROL + px(4.))
                .left_0()
                .right_0()
                .max_h(widgets::PICKER_MAX_HEIGHT)
                .overflow_y_scroll()
                .child(all)
                .children(hosts.iter().map(|host| {
                    let id = host.record_id;
                    widgets::dropdown_item(
                        SharedString::from(format!("shell-history-host-{id}")),
                        None,
                        host.title().to_owned(),
                        None,
                        self.host == Some(id),
                        cx,
                    )
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.set_host(Some(id), cx)),
                    )
                }))
        });
        div()
            .relative()
            .flex_none()
            .w(px(240.))
            .child(
                widgets::select_trigger("shell-history-host", None, chosen, self.filter_open, cx)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.filter_open = !this.filter_open;
                        cx.notify();
                    })),
            )
            .children(list)
    }

    fn render_row(&self, entry: &HistoryEntryView, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let hover = theme.secondary_hover;
        let id = entry.record_id;
        let host = entry
            .host
            .and_then(|host| self.state.read(cx).host(host.record_id))
            .map_or(strings::NO_HOST.to_owned(), |host| host.title().to_owned());
        h_flex()
            .id(row_id(id))
            .test_support()
            .w_full()
            .h(metrics::HEIGHT_ROW)
            .px(metrics::SPACE_2)
            .gap(metrics::SPACE_3)
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .hover(move |style| style.bg(hover))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(JETBRAINS_MONO)
                    .text_size(metrics::FONT_SIZE_DENSE)
                    .child(entry.command.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .w(px(180.))
                    .truncate()
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(theme.muted_foreground)
                    .child(host),
            )
            .child(
                div()
                    .id(SharedString::from(format!("shell-history-when-{id}")))
                    .flex_none()
                    .w(px(140.))
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(theme.muted_foreground)
                    .child(when(&entry.executed_at))
                    .when_some(
                        crate::dates::timestamp_tooltip(&entry.executed_at),
                        |this, utc| this.tooltip(crate::dates::utc_tooltip(utc)),
                    ),
            )
            .child(
                ControlButton::new(save_id(id), ControlKind::Ghost)
                    .icon(IconName::Braces)
                    .tooltip(strings::SAVE_AS_SNIPPET)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.save_as_snippet(id, window, cx)
                    })),
            )
            .into_any_element()
    }
}

impl Render for ShellHistory {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let icon = Icon::new(gpui_kit::component::IconName::Search)
            .small()
            .text_color(cx.theme().muted_foreground);
        let field = div()
            .flex_1()
            .min_w_0()
            .max_w(px(360.))
            .child(widgets::text_input_with(
                Input::new(&self.search).prefix(icon),
                &self.search,
                "shell-history-search",
                false,
                window,
                cx,
            ));
        let entries = std::mem::take(&mut self.entries);
        let rows: Vec<AnyElement> = entries
            .iter()
            .map(|entry| self.render_row(entry, cx))
            .collect();
        self.entries = entries;
        let empty = (rows.is_empty() && self.loading.is_none()).then_some(if self.failed {
            strings::HISTORY_FAILED
        } else {
            strings::HISTORY_EMPTY
        });
        v_flex()
            .id("shell-history")
            .test_support()
            .size_full()
            .px(metrics::SPACE_5)
            .gap(metrics::SPACE_3)
            .child(
                h_flex()
                    .w_full()
                    .gap(metrics::SPACE_2)
                    .items_center()
                    .child(field)
                    .child(self.render_filter(cx)),
            )
            .child(widgets::caption(strings::HISTORY_NOTE, cx))
            .when_some(empty, |this, message| {
                this.child(
                    div()
                        .id("shell-history-empty")
                        .test_support()
                        .child(widgets::caption(message, cx)),
                )
            })
            .when(!rows.is_empty(), |this| {
                this.child(
                    v_flex()
                        .id("shell-history-list")
                        .test_support()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .overflow_y_scroll()
                        .border_t_1()
                        .border_color(cx.theme().border)
                        .children(rows),
                )
            })
    }
}
