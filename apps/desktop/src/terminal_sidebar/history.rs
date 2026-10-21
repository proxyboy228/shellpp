//! The History tab of the terminal sidebar (UI 4.2; stage 2b lead decision
//! L7; docs/security.md, sections 10.1 and 10.2).
//!
//! It shows the command history of the active pane's saved host. Commands
//! are captured by the terminal (`crate::terminal::history_capture`) and
//! written by the core; this view only reads, inserts and deletes.
//!
//! - **Search**: plain text, case-insensitive (the core's `history_list`
//!   filter); the list shows the newest commands first, at most
//!   [`LIST_LIMIT`].
//! - **Insert**: a click (or Enter on a focused row) puts the command at
//!   the prompt without running it, as injected input
//!   ([`TerminalView::insert_injected`]): a line that holds it is never
//!   recorded again. The focus moves to the terminal.
//! - **Delete** one entry with its button; **Clear history** removes every
//!   entry of the host after a confirmation (it cannot be undone).
//! - **Hint card** (when the host has commands and the hint was not
//!   closed): the three most frequent commands, each checked again by the
//!   secret detector in the core (`Core::history_hint`), in fields that can
//!   be edited. "Save" is disabled until snippets arrive (stage 5; the
//!   tooltip says so); "Close" hides the hint for the host for good
//!   (`history_hint_state`).
//! - History is kept for saved hosts only: quick connects and local
//!   terminals show an explanation instead. When recording is off for the
//!   host (its override or Settings > Terminal), a line says so; the stored
//!   entries stay usable.
//!
//! Core calls run on background threads; a result that arrives after the
//! target or the search changed is dropped. While the app is locked the
//! tab holds no commands. Nothing here logs commands.

use std::collections::HashMap;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, IntoElement, Render, SharedString,
    Subscription, Task, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariant};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, Sizable as _, WindowExt as _, h_flex, v_flex};
use shellpp_core::error::CoreError;
use shellpp_core::service::dto::{
    HistoryClearInput, HistoryEntryView, HistoryHint, HistoryListInput, RecordRef,
};
use uuid::Uuid;

use super::{SidebarTarget, heading};
use crate::panels::widgets;
use crate::state::{AppState, AppStateEvent};
use crate::terminal::TerminalView;
use crate::terminal::fonts::JETBRAINS_MONO;
use crate::theme::controls::ButtonColors as _;
use crate::theme::focus::FocusRingExt as _;
use crate::theme::{self, metrics};

/// Entries the list shows at most (the newest).
pub const LIST_LIMIT: u32 = 500;

/// Strings of the History tab (catalog `en`).
pub mod text {
    pub const TITLE: &str = "History";
    pub const SEARCH_PLACEHOLDER: &str = "Search history";
    pub const CLEAR: &str = "Clear history";
    pub const DELETE: &str = "Delete from history";
    pub const INSERT_HINT: &str = "Click a command to put it at the prompt without running it.";
    pub const NO_SESSION: &str = "Open a session to a saved host to see its command history.";
    pub const NOT_SAVED: &str = "Command history is kept for saved hosts only. Save this host to keep the commands you run.";
    pub const EMPTY: &str = "Commands you run on this host appear here.";
    pub const NO_MATCHES: &str = "No matching commands";
    pub const RECORDING_OFF: &str = "Recording is off for this host. Turn it on in Settings > Terminal or in the host's settings.";
    pub const LOAD_FAILED: &str = "The history could not be read. Try again.";
    pub const HINT_TITLE: &str = "Save frequent commands as snippets?";
    pub const HINT_BODY: &str =
        "Your most used commands on this host. You can edit them before saving.";
    pub const HINT_CLOSE: &str = "Close";
    pub const HINT_SAVE: &str = "Save";
    pub const HINT_SAVED: &str = "Saved as snippets of this host";
    pub const HINT_NOT_SAVED: &str = "The snippets could not be saved";
    pub const CLEAR_TITLE: &str = "Clear the history of this host?";
    pub const CLEAR_BODY: &str =
        "Every recorded command of this host is removed. This cannot be undone.";
    pub const CLEAR_OK: &str = "Clear";
    pub const CANCEL: &str = "Cancel";
}

/// What the core returned for a host.
struct Loaded {
    entries: Result<Vec<HistoryEntryView>, CoreError>,
    hint: Option<HistoryHint>,
    recording: bool,
}

/// The History tab.
pub struct HistoryTab {
    state: Entity<AppState>,
    target: Option<SidebarTarget>,
    search: Entity<InputState>,
    entries: Vec<HistoryEntryView>,
    /// The last load failed.
    failed: bool,
    /// The host records history now (its override, else the setting).
    recording: bool,
    /// The hint's commands as loaded; empty = no hint.
    hint: Vec<String>,
    /// The hint's editable fields (made at the next render: they need the
    /// window).
    hint_fields: Vec<Entity<InputState>>,
    hint_fields_stale: bool,
    /// Increases with every load; older results are dropped.
    generation: u64,
    row_focus: HashMap<Uuid, FocusHandle>,
    load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for HistoryTab {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HistoryTab")
            .field("entries", &self.entries.len())
            .field("hint", &self.hint.len())
            .field("recording", &self.recording)
            .finish_non_exhaustive()
    }
}

impl HistoryTab {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(text::SEARCH_PLACEHOLDER));
        let subscriptions = vec![
            cx.subscribe(&search, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.reload(cx);
                }
            }),
            // New, deleted and cleared entries arrive as record changes; a
            // lock drops what the tab holds.
            cx.subscribe(&state, |this, _, event: &AppStateEvent, cx| match event {
                AppStateEvent::RecordsChanged | AppStateEvent::LockChanged(_) => this.reload(cx),
                _ => {}
            }),
        ];
        Self {
            state,
            target: None,
            search,
            entries: Vec::new(),
            failed: false,
            recording: true,
            hint: Vec::new(),
            hint_fields: Vec::new(),
            hint_fields_stale: false,
            generation: 0,
            row_focus: HashMap::new(),
            load: None,
            _subscriptions: subscriptions,
        }
    }

    /// The active pane's session changed.
    pub fn set_target(
        &mut self,
        target: Option<SidebarTarget>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host_changed = self.target.as_ref().and_then(SidebarTarget::host_id)
            != target.as_ref().and_then(SidebarTarget::host_id);
        self.target = target;
        if host_changed {
            self.entries.clear();
            self.set_hint(Vec::new());
            self.failed = false;
            self.reload(cx);
        }
        cx.notify();
    }

    /// The saved host of the active pane, while the app is unlocked.
    pub fn host(&self, cx: &App) -> Option<RecordRef> {
        let record_id = self.target.as_ref()?.host_id()?;
        let state = self.state.read(cx);
        if !state.is_unlocked() {
            return None;
        }
        let vault_id = state
            .host(record_id)
            .map(|host| host.vault_id)
            .or_else(|| state.vault_id())?;
        Some(RecordRef {
            vault_id,
            record_id,
        })
    }

    /// The commands on display, newest first.
    pub fn commands(&self) -> Vec<String> {
        self.entries
            .iter()
            .map(|entry| entry.command.clone())
            .collect()
    }

    /// The editable field of the hint's `index`-th command (tests).
    #[cfg(test)]
    pub fn hint_field(&self, index: usize) -> Entity<InputState> {
        self.hint_fields[index].clone()
    }

    /// The hint's commands as loaded (empty: no hint).
    pub fn hint_commands(&self) -> &[String] {
        &self.hint
    }

    /// Whether recording is on for the host.
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    /// Replaces the search text (tests).
    pub fn set_search(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.search
            .update(cx, |input, cx| input.set_value(text.to_owned(), window, cx));
        self.reload(cx);
    }

    fn set_hint(&mut self, commands: Vec<String>) {
        if commands != self.hint {
            self.hint = commands;
            self.hint_fields_stale = true;
        }
        if self.hint.is_empty() {
            // No commands stay behind in the fields (a lock, another host).
            self.hint_fields.clear();
        }
    }

    /// Reads the list, the hint and the recording state of the host again.
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        self.generation += 1;
        let generation = self.generation;
        let (Some(host), Some(core)) = (self.host(cx), self.state.read(cx).core().cloned()) else {
            self.load = None;
            self.entries.clear();
            self.set_hint(Vec::new());
            self.failed = false;
            cx.notify();
            return;
        };
        let search = self.search.read(cx).value().trim().to_owned();
        let work = cx.background_spawn(async move {
            // One row per command, its newest use (run 3 polish, D64).
            let entries = core.history_list_distinct(HistoryListInput {
                host: Some(host),
                text: (!search.is_empty()).then_some(search),
                limit: Some(LIST_LIMIT),
            });
            let hint = core.history_hint(host).ok();
            let recording = core.record_history_now(Some(host)).unwrap_or(true);
            Loaded {
                entries,
                hint,
                recording,
            }
        });
        self.load = Some(cx.spawn(async move |this, cx| {
            let loaded = work.await;
            let _ = this.update(cx, |this, cx| this.apply(generation, loaded, cx));
        }));
    }

    fn apply(&mut self, generation: u64, loaded: Loaded, cx: &mut Context<Self>) {
        if generation != self.generation {
            return;
        }
        self.load = None;
        match loaded.entries {
            Ok(entries) => {
                self.entries = entries;
                self.failed = false;
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "history not read");
                self.entries.clear();
                self.failed = true;
            }
        }
        let ids: Vec<Uuid> = self.entries.iter().map(|entry| entry.record_id).collect();
        self.row_focus.retain(|id, _| ids.contains(id));
        self.recording = loaded.recording;
        let hint = loaded
            .hint
            .filter(|hint| !hint.dismissed)
            .map(|hint| hint.commands)
            .unwrap_or_default();
        self.set_hint(hint);
        cx.notify();
    }

    /// Puts `command` at the prompt of the active pane without running it
    /// (injected input, never recorded again) and focuses the terminal.
    pub fn insert(&mut self, command: &str, window: &mut Window, cx: &mut Context<Self>) {
        let Some(terminal) = self.target.as_ref().and_then(SidebarTarget::terminal) else {
            return;
        };
        terminal.update(cx, |terminal: &mut TerminalView, cx| {
            terminal.insert_injected(command, cx);
            terminal.focus(window, cx);
        });
    }

    /// Deletes a row: the command's entry and its older repeats.
    pub fn delete(&mut self, record_id: Uuid, cx: &mut Context<Self>) {
        let Some(entry) = self
            .entries
            .iter()
            .position(|entry| entry.record_id == record_id)
        else {
            return;
        };
        let entry = self.entries.remove(entry);
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let work =
            cx.background_spawn(
                async move { core.history_delete_command(entry.host, &entry.command) },
            );
        cx.spawn(async move |this, cx| {
            if let Err(error) = work.await {
                tracing::warn!(code = error.code(), "history entry not deleted");
            }
            let _ = this.update(cx, |this, cx| this.reload(cx));
        })
        .detach();
        cx.notify();
    }

    /// "Clear history": asks first, then removes every entry of the host.
    pub fn confirm_clear(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.host(cx).is_none() {
            return;
        }
        let tab = cx.entity().downgrade();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let theme = cx.theme();
            let tab = tab.clone();
            alert
                .title(
                    div()
                        .text_size(metrics::FONT_SIZE_TITLE)
                        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                        .child(text::CLEAR_TITLE),
                )
                .description(text::CLEAR_BODY)
                .show_cancel(true)
                .cancel_text(text::CANCEL)
                .ok_text(text::CLEAR_OK)
                .ok_variant(ButtonVariant::Danger)
                .bg(theme.popover)
                .rounded(metrics::RADIUS_OVERLAY)
                .shadow(theme::shadow_modal(cx))
                .on_ok(move |_, _, cx| {
                    let _ = tab.update(cx, |tab, cx| tab.clear(cx));
                    true
                })
        });
    }

    /// Removes every entry of the host (after the confirmation).
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        let (Some(host), Some(core)) = (self.host(cx), self.state.read(cx).core().cloned()) else {
            return;
        };
        self.entries.clear();
        self.set_hint(Vec::new());
        let work = cx.background_spawn(async move {
            core.history_clear(HistoryClearInput { host: Some(host) })
        });
        cx.spawn(async move |this, cx| {
            if let Err(error) = work.await {
                tracing::warn!(code = error.code(), "history not cleared");
            }
            let _ = this.update(cx, |this, cx| this.reload(cx));
        })
        .detach();
        cx.notify();
    }

    /// "Save" on the hint (UI 4.2): each (edited) command becomes a snippet
    /// of this host; the hint is not shown again for it.
    pub fn save_hint(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(host), Some(core)) = (self.host(cx), self.state.read(cx).core().cloned()) else {
            return;
        };
        let commands: Vec<String> = self
            .hint_fields
            .iter()
            .map(|field| field.read(cx).value().to_string())
            .collect();
        self.set_hint(Vec::new());
        let work = cx.background_spawn(async move { core.history_hint_save(host, &commands) });
        cx.spawn_in(window, async move |this, cx| {
            let saved = work.await;
            let _ = this.update_in(cx, |this, window, cx| {
                match saved {
                    Ok(_) => crate::shell::show_toast(text::HINT_SAVED, window, cx),
                    Err(error) => {
                        tracing::warn!(code = error.code(), "hint snippets not saved");
                        crate::shell::show_toast(text::HINT_NOT_SAVED, window, cx);
                    }
                }
                this.reload(cx);
            });
        })
        .detach();
        cx.notify();
    }

    /// "Close" on the hint: it is not shown again for this host.
    pub fn close_hint(&mut self, cx: &mut Context<Self>) {
        let (Some(host), Some(core)) = (self.host(cx), self.state.read(cx).core().cloned()) else {
            return;
        };
        self.set_hint(Vec::new());
        let work = cx.background_spawn(async move { core.history_hint_dismiss(host) });
        cx.spawn(async move |_, _| {
            if let Err(error) = work.await {
                tracing::warn!(code = error.code(), "history hint not dismissed");
            }
        })
        .detach();
        cx.notify();
    }

    // Rendering ---------------------------------------------------------------

    fn render_hint(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        if self.hint.is_empty() {
            self.hint_fields.clear();
            return None;
        }
        if self.hint_fields_stale || self.hint_fields.len() != self.hint.len() {
            self.hint_fields = self
                .hint
                .iter()
                .map(|command| {
                    let command = command.clone();
                    cx.new(|cx| {
                        let mut input = InputState::new(window, cx);
                        input.set_value(command, window, cx);
                        input
                    })
                })
                .collect();
            self.hint_fields_stale = false;
        }
        let fields: Vec<AnyElement> = self
            .hint_fields
            .iter()
            .enumerate()
            .map(|(index, field)| {
                widgets::text_input_with(
                    Input::new(field).font_family(JETBRAINS_MONO),
                    field,
                    SharedString::from(format!("history-hint-command-{index}")),
                    false,
                    window,
                    cx,
                )
            })
            .collect();
        let close = widgets::form_button("history-hint-close", text::HINT_CLOSE, true, cx)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.close_hint(cx)));
        let save = Button::new("history-hint-save")
            .label(text::HINT_SAVE)
            .accent_colors()
            // The size of "Close" next to it (`widgets::form_button`): 32 px
            // high, the label at body size (`large`).
            .large()
            .h(px(32.))
            .px(metrics::SPACE_3)
            .rounded(metrics::RADIUS_CONTROL)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.save_hint(window, cx)));
        Some(
            widgets::block("history-hint", Some(text::HINT_TITLE), cx)
                .child(widgets::caption(text::HINT_BODY, cx))
                .children(fields)
                .child(
                    h_flex()
                        .gap(metrics::SPACE_2)
                        .justify_end()
                        .child(close)
                        .child(save),
                )
                .into_any_element(),
        )
    }

    fn render_row(
        &mut self,
        index: usize,
        entry: &HistoryEntryView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let surface = theme.colors.list;
        let hover = theme.secondary_hover;
        let pressed = theme.secondary_active;
        let focus = self
            .row_focus
            .entry(entry.record_id)
            .or_insert_with(|| cx.focus_handle())
            .clone();
        let command = entry.command.clone();
        let record_id = entry.record_id;
        let delete_id = SharedString::from(format!("history-entry-{index}-delete"));
        let delete = trash_button(delete_id, text::DELETE, true, cx).on_click(cx.listener(
            move |this, _: &ClickEvent, _, cx| {
                cx.stop_propagation();
                this.delete(record_id, cx);
            },
        ));
        h_flex()
            .id(SharedString::from(format!("history-entry-{index}")))
            .test_support()
            .w_full()
            .h(metrics::HEIGHT_CONTROL)
            .pl(metrics::SPACE_2)
            .gap(metrics::SPACE_1)
            .items_center()
            .rounded(metrics::RADIUS_CONTROL)
            .cursor_pointer()
            .track_focus(&focus)
            .tab_index(0)
            .focus_ring_visible(surface, cx)
            .hover(move |style| style.bg(hover))
            .active(move |style| style.bg(pressed))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .font_family(JETBRAINS_MONO)
                    .text_size(metrics::FONT_SIZE_DENSE)
                    .text_color(theme.foreground)
                    .child(entry.command.clone()),
            )
            .child(delete)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.insert(&command, window, cx)
            }))
            .into_any_element()
    }

    fn render_header(&self, has_entries: bool, cx: &mut Context<Self>) -> impl IntoElement {
        let clear =
            trash_button("history-clear", text::CLEAR, has_entries, cx).when(has_entries, |this| {
                this.on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.confirm_clear(window, cx)),
                )
            });
        h_flex()
            .items_center()
            .justify_between()
            .child(heading(text::TITLE, cx))
            .child(clear)
    }
}

/// A 32 px icon button with the trash icon (the size of
/// `widgets::icon_button`). A dangerous action: hovered, its icon takes
/// `--danger-hover` (design-system 6). Disabled at `--opacity-disabled`
/// without hover.
fn trash_button(
    id: impl Into<gpui::ElementId>,
    tooltip: &'static str,
    enabled: bool,
    cx: &App,
) -> widgets::Hit {
    let theme = cx.theme();
    let hover = theme.secondary_hover;
    let pressed = theme.secondary_active;
    let danger_hover = theme::tokens(cx).danger_hover.hsla();
    widgets::hit(id)
        .flex_none()
        .size(px(32.))
        .flex()
        .items_center()
        .justify_center()
        .rounded(metrics::RADIUS_CONTROL)
        .text_color(theme.muted_foreground)
        .tooltip(move |window, cx| Tooltip::new(tooltip).build(window, cx))
        .child(Icon::new(IconName::Trash).small())
        .map(|this| {
            if enabled {
                this.keyboard_stop(cx.theme().colors.list, cx)
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover).text_color(danger_hover))
                    .active(move |style| style.bg(pressed).text_color(danger_hover))
            } else {
                this.opacity(metrics::OPACITY_DISABLED)
            }
        })
}

impl Render for HistoryTab {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let host = self.host(cx);
        let has_session = self
            .target
            .as_ref()
            .and_then(SidebarTarget::terminal)
            .is_some();
        let root = v_flex()
            .id("terminal-sidebar-history")
            .test_support()
            .p(metrics::SPACE_4)
            .gap(metrics::SPACE_3);
        if host.is_none() {
            let message = if has_session && self.target.is_some() {
                text::NOT_SAVED
            } else {
                text::NO_SESSION
            };
            return root
                .child(heading(text::TITLE, cx))
                .child(
                    div()
                        .id("history-unavailable")
                        .test_support()
                        .child(widgets::caption(message, cx)),
                )
                .into_any_element();
        }
        let searching = !self.search.read(cx).value().trim().is_empty();
        let hint = self.render_hint(window, cx);
        let icon = Icon::new(IconName::Search)
            .small()
            .text_color(cx.theme().muted_foreground);
        let field = widgets::text_input_with(
            Input::new(&self.search).prefix(icon),
            &self.search,
            "history-search",
            false,
            window,
            cx,
        );
        let entries = std::mem::take(&mut self.entries);
        let rows: Vec<AnyElement> = entries
            .iter()
            .enumerate()
            .map(|(index, entry)| self.render_row(index, entry, cx))
            .collect();
        self.entries = entries;
        let empty = if self.failed {
            Some(text::LOAD_FAILED)
        } else if self.entries.is_empty() && self.load.is_none() {
            Some(if searching {
                text::NO_MATCHES
            } else {
                text::EMPTY
            })
        } else {
            None
        };
        root.child(self.render_header(!self.entries.is_empty() && !searching, cx))
            .children(hint)
            .child(field)
            .when(!self.recording, |this| {
                this.child(
                    div()
                        .id("history-recording-off")
                        .test_support()
                        .child(widgets::caption(text::RECORDING_OFF, cx)),
                )
            })
            .when_some(empty, |this, message| {
                this.child(
                    div()
                        .id("history-empty")
                        .test_support()
                        .child(widgets::caption(message, cx)),
                )
            })
            .when(!rows.is_empty(), |this| {
                this.child(widgets::caption(text::INSERT_HINT, cx))
                    .child(v_flex().id("history-list").test_support().children(rows))
            })
            .into_any_element()
    }
}

#[cfg(test)]
mod tests;
