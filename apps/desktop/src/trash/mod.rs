//! The Recently deleted screen (UI 14; D93): every object deleted on this
//! device, or deleted elsewhere and received through sync, for 90 days, on
//! this device only. It needs no account: it works in local mode.
//!
//! - A line on top says so. The action bar holds Restore, Export… and
//!   Delete forever for the selection, the search by name, the filter by
//!   type and, on the right, Empty trash (danger).
//! - The table, newest first: name, type, vault, deleted (local time) and
//!   where. A click selects a row, Ctrl and Shift add rows, Ctrl+A selects
//!   every row shown; a row's context menu has the same three actions.
//! - One selected object opens read-only in the details panel, its fields
//!   as the core lists them, secrets masked by the core.
//! - Delete forever and Empty trash ask first and name the count.
//! - The list is read when the screen is built, on unlock, when records
//!   change and every 24 hours. Each read purges what is older than 90
//!   days (the core does it), so old entries leave in local mode too.

pub mod strings;

#[cfg(test)]
mod tests;

use std::rc::Rc;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, FocusHandle, Focusable as _, FontWeight,
    IntoElement, KeyDownEvent, Pixels, Render, SharedString, Subscription, Task, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, WindowExt as _, h_flex, v_flex,
};
use serde_json::{Map, Value};
use shellpp_core::service::DeletedItem;
use uuid::Uuid;

use crate::hosts::controls::{self, ControlButton, ControlKind, ICON_SIZE};
use crate::panels::widgets;
use crate::shell::details::DetailsHeader;
use crate::state::selection::{SelectMode, Selection};
use crate::state::{AppState, AppStateEvent};
use crate::theme::{self, metrics};

const CONTENT_PADDING: gpui::Rems = metrics::SPACE_5;
const TYPE_COLUMN: gpui::Rems = metrics::rpx(128.);
const VAULT_COLUMN: gpui::Rems = metrics::rpx(88.);
const DELETED_COLUMN: gpui::Rems = metrics::rpx(132.);
const WHERE_COLUMN: gpui::Rems = metrics::rpx(112.);
const SEARCH_MIN_WIDTH: gpui::Rems = metrics::rpx(160.);
const SEARCH_MAX_WIDTH: gpui::Rems = metrics::rpx(280.);
const DIALOG_WIDTH: gpui::Rems = metrics::rpx(420.);
/// The list is read again this often, so old entries leave (UI 14).
const RELOAD_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// Stable element id of a row.
pub fn row_id(id: Uuid) -> SharedString {
    format!("trash-row-{id}").into()
}

/// The fields of the details panel: `(path, value)` in name order, nested
/// objects flattened as `ssh.port`. Bookkeeping (the sort key) and
/// references to other records (`*_id`, `*_ids`: ids mean nothing here)
/// stay out.
pub fn fields(data: &Map<String, Value>) -> Vec<(String, String)> {
    fn walk(prefix: &str, map: &Map<String, Value>, out: &mut Vec<(String, String)>) {
        let mut names: Vec<&String> = map.keys().collect();
        names.sort();
        for name in names {
            if name == "order_key" || name.ends_with("_id") || name.ends_with("_ids") {
                continue;
            }
            let path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{prefix}.{name}")
            };
            match &map[name] {
                Value::Object(inner) => walk(&path, inner, out),
                Value::Null => {}
                Value::String(text) => out.push((path, text.clone())),
                other => out.push((path, other.to_string())),
            }
        }
    }
    let mut out = Vec::new();
    walk("", data, &mut out);
    out
}

pub struct TrashScreen {
    state: Entity<AppState>,
    search: Entity<InputState>,
    /// The type shown (`None`: every type).
    type_filter: Option<String>,
    items: Vec<DeletedItem>,
    selection: Selection,
    focus: FocusHandle,
    loading: Option<Task<()>>,
    /// The records changed while a load ran: one more after it.
    stale: bool,
    failed: bool,
    _daily: Task<()>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for TrashScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrashScreen")
            .field("items", &self.items.len())
            .finish_non_exhaustive()
    }
}

impl TrashScreen {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(strings::SEARCH_PLACEHOLDER));
        let search_subscription = cx.subscribe_in(
            &search,
            window,
            |this: &mut Self, _, event: &InputEvent, _, cx| match event {
                InputEvent::Change => this.refilter(cx),
                InputEvent::PressEnter { .. } | InputEvent::Focus | InputEvent::Blur => cx.notify(),
            },
        );
        let state_subscription =
            cx.subscribe(&state, |this: &mut Self, _, event, cx| match event {
                AppStateEvent::RecordsChanged => this.reload(cx),
                AppStateEvent::LockChanged(_) => {
                    // Locked: nothing stays on screen or in memory.
                    this.selection.clear();
                    this.items = Vec::new();
                    this.loading = None;
                    this.reload(cx);
                }
                AppStateEvent::SelectionChanged
                | AppStateEvent::LocationChanged(_)
                | AppStateEvent::AutoLockDue => {}
            });
        let daily = cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(RELOAD_EVERY).await;
                if this.update(cx, |screen, cx| screen.reload(cx)).is_err() {
                    break;
                }
            }
        });
        let mut screen = Self {
            state,
            search,
            type_filter: None,
            items: Vec::new(),
            selection: Selection::default(),
            focus: cx.focus_handle().tab_stop(true),
            loading: None,
            stale: false,
            failed: false,
            _daily: daily,
            _subscriptions: vec![search_subscription, state_subscription],
        };
        screen.reload(cx);
        screen
    }

    // -- Reading -----------------------------------------------------------------

    pub fn items(&self) -> &[DeletedItem] {
        &self.items
    }

    /// The entries on display, newest first: the search matches the name,
    /// ignoring case; the filter keeps one type.
    pub fn shown(&self, cx: &App) -> Vec<&DeletedItem> {
        let query = self.search.read(cx).value().trim().to_lowercase();
        self.items
            .iter()
            .filter(|item| {
                self.type_filter
                    .as_ref()
                    .is_none_or(|kind| *kind == item.type_name)
            })
            .filter(|item| query.is_empty() || item.title.to_lowercase().contains(&query))
            .collect()
    }

    fn shown_ids(&self, cx: &App) -> Vec<Uuid> {
        self.shown(cx).iter().map(|item| item.record_id).collect()
    }

    /// The types present, in the order of their labels (the filter's menu).
    pub fn types(&self) -> Vec<String> {
        let mut types: Vec<String> = self
            .items
            .iter()
            .map(|item| item.type_name.clone())
            .collect();
        types.sort_by_key(|kind| strings::type_label(kind));
        types.dedup();
        types
    }

    pub fn type_filter(&self) -> Option<&str> {
        self.type_filter.as_deref()
    }

    pub fn selected(&self) -> &[Uuid] {
        self.selection.ids()
    }

    pub fn is_loading(&self) -> bool {
        self.loading.is_some()
    }

    /// The panel's width while one object is open (the shell collapses its
    /// sidebar by it, UI 1.2).
    pub fn open_panel_width(&self) -> Option<Pixels> {
        self.single()
            .map(|_| metrics::at_100(metrics::WIDTH_DETAILS))
    }

    fn single(&self) -> Option<&DeletedItem> {
        let id = self.selection.single()?;
        self.items.iter().find(|item| item.record_id == id)
    }

    /// A row's cells: name, type, vault, deleted (local time), where.
    pub fn cells(&self, item: &DeletedItem, cx: &App) -> [String; 5] {
        [
            item.title.clone(),
            strings::type_label(&item.type_name),
            self.vault_label(item, cx).to_owned(),
            crate::dates::timestamp_text(&item.deleted_at),
            where_label(item).to_owned(),
        ]
    }

    /// The empty state's title while no row is shown.
    pub fn empty_title(&self) -> &'static str {
        if self.failed {
            strings::LOAD_FAILED
        } else if self.items.is_empty() {
            strings::EMPTY
        } else {
            strings::NO_MATCH
        }
    }

    /// The fields of the object open in the panel (secrets masked).
    pub fn panel_fields(&self) -> Vec<(String, String)> {
        self.single()
            .and_then(|item| item.data.as_ref())
            .map(fields)
            .unwrap_or_default()
    }

    fn vault_label(&self, item: &DeletedItem, cx: &App) -> &'static str {
        if self.state.read(cx).vault_id() == Some(item.vault_id) {
            crate::i18n::top_bar::PERSONAL_VAULT
        } else {
            strings::OTHER_VAULT
        }
    }

    // -- Loading -------------------------------------------------------------------

    /// Reads the list again on a background thread (empty while locked).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        if self.loading.is_some() {
            self.stale = true;
            return;
        }
        let state = self.state.read(cx);
        let Some(core) = state.core().filter(|_| state.is_unlocked()).cloned() else {
            self.items = Vec::new();
            self.selection.clear();
            cx.notify();
            return;
        };
        let listed = cx.background_spawn(async move { core.recently_deleted_list() });
        self.loading = Some(cx.spawn(async move |this, cx| {
            let listed = listed.await;
            let _ = this.update(cx, |screen, cx| {
                screen.loading = None;
                match listed {
                    Ok(items) => {
                        screen.items = items;
                        screen.failed = false;
                    }
                    Err(error) => {
                        tracing::warn!(code = error.code(), "recently deleted could not be read");
                        screen.failed = true;
                    }
                }
                screen.refilter(cx);
                if std::mem::take(&mut screen.stale) {
                    screen.reload(cx);
                }
            });
        }));
    }

    /// Drops selected rows that left the list or the view.
    fn refilter(&mut self, cx: &mut Context<Self>) {
        if self
            .type_filter
            .as_ref()
            .is_some_and(|kind| !self.items.iter().any(|item| item.type_name == *kind))
        {
            self.type_filter = None;
        }
        let shown = self.shown_ids(cx);
        self.selection.retain(|id| shown.contains(id));
        cx.notify();
    }

    // -- Commands --------------------------------------------------------------------

    pub fn set_type_filter(&mut self, kind: Option<String>, cx: &mut Context<Self>) {
        self.type_filter = kind;
        self.refilter(cx);
    }

    /// A click on a row (UI 14): only it, Ctrl adds or removes it, Shift
    /// takes the rows from the anchor.
    pub fn click(&mut self, id: Uuid, mode: SelectMode, cx: &mut Context<Self>) {
        let order = self.shown_ids(cx);
        self.selection.apply(id, mode, &order);
        cx.notify();
    }

    /// Ctrl+A: every row shown.
    pub fn select_all(&mut self, cx: &mut Context<Self>) {
        let order = self.shown_ids(cx);
        self.selection.clear();
        if let (Some(first), Some(last)) = (order.first(), order.last()) {
            self.selection.apply(*first, SelectMode::Replace, &order);
            self.selection.apply(*last, SelectMode::Range, &order);
        }
        cx.notify();
    }

    pub fn clear_selection(&mut self, cx: &mut Context<Self>) {
        self.selection.clear();
        cx.notify();
    }

    /// The ids an action applies to: the selection, or the row the context
    /// menu was opened on when it is not selected.
    fn targets(&self, row: Option<Uuid>) -> Vec<Uuid> {
        match row {
            Some(id) if !self.selection.contains(&id) => vec![id],
            _ => self.selection.ids().to_vec(),
        }
    }

    /// Restore: back where they were, under the same ids; they sync as
    /// ordinary changes. A toast confirms.
    pub fn restore(&mut self, ids: Vec<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        match core.recently_deleted_restore(&ids) {
            Ok(restored) => {
                crate::shell::show_toast(strings::restored(restored.len()), window, cx);
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "recently deleted not restored");
                crate::shell::show_toast(strings::ACTION_FAILED, window, cx);
            }
        }
        self.after_change(cx);
    }

    /// Export…: the Export Data dialog (1.4) for these objects only.
    pub fn export(&mut self, ids: Vec<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let dialog = crate::data_transfer::open_export(self.state.clone(), window, cx);
        dialog.update(cx, |dialog, _| dialog.recently_deleted = ids);
    }

    /// Delete forever: asks first, naming the count.
    pub fn confirm_delete(&mut self, ids: Vec<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        if ids.is_empty() {
            return;
        }
        let screen = cx.entity().downgrade();
        let count = ids.len();
        open_confirmation(
            "trash-delete",
            strings::DELETE_TITLE,
            strings::delete_body(count),
            strings::DELETE_FOREVER,
            window,
            cx,
            move |window, cx| {
                let ids = ids.clone();
                let _ = screen.update(cx, |screen, cx| screen.delete_forever(ids, window, cx));
            },
        );
    }

    /// Removes entries from this device now (after the confirmation).
    pub fn delete_forever(&mut self, ids: Vec<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        match core.recently_deleted_delete(&ids) {
            Ok(()) => crate::shell::show_toast(strings::deleted_forever(ids.len()), window, cx),
            Err(error) => {
                tracing::warn!(code = error.code(), "recently deleted entries not removed");
                crate::shell::show_toast(strings::ACTION_FAILED, window, cx);
            }
        }
        self.after_change(cx);
    }

    /// Empty trash: asks first, naming the count of every entry listed.
    /// It removes exactly those (UI 14): an entry that arrives through sync
    /// while the confirmation is open, or one this list does not show, stays.
    pub fn confirm_empty(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let ids: Vec<Uuid> = self.items.iter().map(|item| item.record_id).collect();
        if ids.is_empty() {
            return;
        }
        let screen = cx.entity().downgrade();
        open_confirmation(
            "trash-empty",
            strings::EMPTY_TITLE,
            strings::empty_body(ids.len()),
            strings::EMPTY_TRASH,
            window,
            cx,
            move |window, cx| {
                let ids = ids.clone();
                let _ = screen.update(cx, |screen, cx| screen.empty(ids, window, cx));
            },
        );
    }

    /// Removes the listed entries from this device now (after the
    /// confirmation).
    pub fn empty(&mut self, ids: Vec<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        match core.recently_deleted_delete(&ids) {
            Ok(()) => crate::shell::show_toast(strings::deleted_forever(ids.len()), window, cx),
            Err(error) => {
                tracing::warn!(code = error.code(), "recently deleted not emptied");
                crate::shell::show_toast(strings::ACTION_FAILED, window, cx);
            }
        }
        self.after_change(cx);
    }

    fn after_change(&mut self, cx: &mut Context<Self>) {
        self.selection.clear();
        // A load that started before the change would show the old rows
        // again: it is dropped and the list read anew.
        self.loading = None;
        self.stale = false;
        self.reload(cx);
        self.state.update(cx, |state, cx| {
            state.refresh_sync_status(cx);
            state.kick_sync(cx);
        });
    }

    // -- Rendering -------------------------------------------------------------------

    fn render_search(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let focused = self
            .search
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        h_flex()
            .id("trash-search-field")
            .test_support()
            .relative()
            .flex_1()
            .min_w(SEARCH_MIN_WIDTH)
            .max_w(SEARCH_MAX_WIDTH)
            .h(metrics::HEIGHT_CONTROL)
            .gap(metrics::SPACE_2)
            .px(metrics::SPACE_3)
            .rounded(metrics::RADIUS_CONTROL)
            .bg(tokens.bg_input.hsla())
            .border_1()
            .border_color(tokens.border_strong.hsla())
            .child(
                Icon::new(IconName::Search)
                    .size(ICON_SIZE)
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .child(theme::controls::muted_placeholder(&self.search, cx))
                    .child(
                        Input::new(&self.search)
                            .id("trash-search")
                            .appearance(false)
                            .cleanable(true)
                            .pl_0()
                            .text_size(metrics::FONT_SIZE_BODY),
                    ),
            )
            .when(focused, |this| {
                this.child(controls::focus_ring(
                    metrics::RADIUS_CONTROL,
                    metrics::rpx(1.),
                    cx,
                ))
            })
    }

    fn render_type_filter(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = cx.entity().downgrade();
        let current = self.type_filter.clone();
        let types = self.types();
        let label = current
            .as_deref()
            .map_or_else(|| strings::ALL_TYPES.to_owned(), strings::type_label);
        ControlButton::new("trash-type-filter", ControlKind::Neutral)
            .label(label)
            .trailing_icon(Lucide::ChevronDown)
            .tooltip(strings::TYPE_FILTER)
            .disabled(self.items.is_empty())
            .dropdown_menu(move |mut menu, _, cx| {
                menu = menu.min_w(metrics::scaled(200., cx));
                let every = screen.clone();
                menu = menu.item(
                    PopupMenuItem::element(|_, cx| {
                        menu_item(
                            "trash-type-all".into(),
                            strings::ALL_TYPES.into(),
                            false,
                            cx,
                        )
                    })
                    .checked(current.is_none())
                    .on_click(move |_, _, cx| {
                        let _ = every.update(cx, |screen, cx| screen.set_type_filter(None, cx));
                    }),
                );
                for kind in &types {
                    let screen = screen.clone();
                    let id: SharedString = format!("trash-type-{kind}").into();
                    let label: SharedString = strings::type_label(kind).into();
                    let chosen = kind.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, cx| {
                            menu_item(id.clone(), label.clone(), false, cx)
                        })
                        .checked(current.as_deref() == Some(kind.as_str()))
                        .on_click(move |_, _, cx| {
                            let chosen = chosen.clone();
                            let _ = screen
                                .update(cx, |screen, cx| screen.set_type_filter(Some(chosen), cx));
                        }),
                    );
                }
                menu
            })
    }

    /// Restore, Export… and Delete forever for the selection on the left;
    /// the search, the type filter and Empty trash on the right (UI 14).
    fn render_action_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let none = self.selection.is_empty();
        let restore = ControlButton::new("trash-restore", ControlKind::Neutral)
            .icon(Lucide::RotateCcw)
            .label(strings::RESTORE)
            .disabled(none)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                let ids = this.targets(None);
                this.restore(ids, window, cx)
            }));
        let export = ControlButton::new("trash-export", ControlKind::Neutral)
            .icon(Lucide::Upload)
            .label(strings::EXPORT)
            .disabled(none)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                let ids = this.targets(None);
                this.export(ids, window, cx)
            }));
        let delete = ControlButton::new("trash-delete", ControlKind::Neutral)
            .icon(Lucide::Trash)
            .label(strings::DELETE_FOREVER)
            .disabled(none)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                let ids = this.targets(None);
                this.confirm_delete(ids, window, cx)
            }));
        let empty = ControlButton::new("trash-empty", ControlKind::Danger)
            .icon(Lucide::BrushCleaning)
            .label(strings::EMPTY_TRASH)
            .disabled(self.items.is_empty())
            .on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.confirm_empty(window, cx)),
            );
        h_flex()
            .id("trash-action-bar")
            .test_support()
            .w_full()
            .flex_wrap()
            .items_center()
            .gap(metrics::SPACE_2)
            .child(
                h_flex()
                    .flex_none()
                    .gap(metrics::SPACE_2)
                    .child(restore)
                    .child(export)
                    .child(delete),
            )
            .child(
                h_flex()
                    .id("trash-action-bar-right")
                    .test_support()
                    // Never narrower than its controls: below that it
                    // wraps under the selection's buttons.
                    .flex_1()
                    .justify_end()
                    .items_center()
                    .gap(metrics::SPACE_2)
                    .child(self.render_search(window, cx))
                    .child(self.render_type_filter(cx))
                    .child(empty),
            )
    }

    fn render_header(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let cell = |text: &'static str| {
            div()
                .truncate()
                .text_size(metrics::FONT_SIZE_CAPTION)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.muted_foreground)
                .child(text)
        };
        h_flex()
            .id("trash-header")
            .test_support()
            .w_full()
            .h(metrics::HEIGHT_ROW)
            .px(metrics::SPACE_2)
            .gap(metrics::SPACE_3)
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .child(cell(strings::NAME).flex_1().min_w_0())
            .child(cell(strings::TYPE).flex_none().w(TYPE_COLUMN))
            .child(cell(strings::VAULT).flex_none().w(VAULT_COLUMN))
            .child(cell(strings::DELETED).flex_none().w(DELETED_COLUMN))
            .child(cell(strings::WHERE).flex_none().w(WHERE_COLUMN))
    }

    fn render_row(&self, item: &DeletedItem, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let id = item.record_id;
        let selected = self.selection.contains(&id);
        let hover = theme.secondary_hover;
        let cell = |text: String| {
            div()
                .truncate()
                .text_size(metrics::FONT_SIZE_BODY)
                .child(text)
        };
        let [name, kind, vault, deleted, place] = self.cells(item, cx);
        let screen = cx.entity().downgrade();
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
            .cursor_pointer()
            .map(|this| {
                if selected {
                    this.bg(tokens.accent_subtle.hsla())
                } else {
                    this.hover(move |style| style.bg(hover))
                }
            })
            .child(cell(name).flex_1().min_w_0())
            .child(cell(kind).flex_none().w(TYPE_COLUMN))
            .child(cell(vault).flex_none().w(VAULT_COLUMN))
            .child(
                div()
                    .id(SharedString::from(format!("trash-deleted-{id}")))
                    .flex_none()
                    .w(DELETED_COLUMN)
                    .child(cell(deleted))
                    .when_some(
                        crate::dates::timestamp_tooltip(&item.deleted_at),
                        |this, utc| this.tooltip(crate::dates::utc_tooltip(utc)),
                    ),
            )
            .child(cell(place).flex_none().w(WHERE_COLUMN))
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.focus.focus(window, cx);
                let modifiers = event.modifiers();
                let mode = if modifiers.shift {
                    SelectMode::Range
                } else if modifiers.secondary() {
                    SelectMode::Toggle
                } else {
                    SelectMode::Replace
                };
                this.click(id, mode, cx)
            }))
            .context_menu(move |menu, _, cx| {
                let ids = screen
                    .update(cx, |screen, _| screen.targets(Some(id)))
                    .unwrap_or_default();
                row_menu(menu, screen.clone(), ids)
            })
            .into_any_element()
    }

    fn render_empty(&self, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let title = self.empty_title();
        let icon = !self.failed && self.items.is_empty();
        v_flex()
            .id("trash-empty-state")
            .test_support()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(metrics::SPACE_3)
            .when(icon, |this| {
                this.child(
                    Icon::new(Lucide::Trash)
                        .size(metrics::rpx(32.))
                        .text_color(theme.muted_foreground),
                )
            })
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_HEADING)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .into_any_element()
    }

    fn render_panel(&self, window: &Window, cx: &mut Context<Self>) -> Option<AnyElement> {
        let item = self.single()?.clone();
        let theme = cx.theme();
        let screen = cx.entity().downgrade();
        let header = DetailsHeader::new(strings::PANEL_TITLE).on_collapse(move |_, _, cx| {
            let _ = screen.update(cx, |screen, cx| screen.clear_selection(cx));
        });
        let row = |id: SharedString, label: SharedString, value: String| {
            v_flex()
                .id(id)
                .test_support()
                .gap(metrics::SPACE_1)
                .child(widgets::caption(label, cx))
                .child(div().text_size(metrics::FONT_SIZE_BODY).child(value))
        };
        let summary = widgets::block("trash-panel-block", None, cx)
            .child(row(
                "trash-panel-name".into(),
                strings::NAME.into(),
                item.title.clone(),
            ))
            .child(row(
                "trash-panel-type".into(),
                strings::TYPE.into(),
                strings::type_label(&item.type_name),
            ))
            .child(row(
                "trash-panel-vault".into(),
                strings::VAULT.into(),
                self.vault_label(&item, cx).to_owned(),
            ))
            .child(row(
                "trash-panel-deleted".into(),
                strings::DELETED.into(),
                crate::dates::timestamp_text(&item.deleted_at),
            ))
            .child(row(
                "trash-panel-where".into(),
                strings::WHERE.into(),
                where_label(&item).to_owned(),
            ));
        let listed = self.panel_fields();
        let data = (!listed.is_empty()).then(|| {
            let mut block = widgets::block("trash-panel-fields", Some(strings::FIELDS), cx);
            for (path, value) in listed {
                block = block.child(row(
                    format!("trash-field-{path}").into(),
                    path.into(),
                    value,
                ));
            }
            block
        });
        let frame = if crate::shell::details::overlays_in(window) {
            let room = window.viewport_size().width / metrics::zoom_of(window)
                - crate::shell::sidebar::WIDTH_COLLAPSED;
            crate::shell::details::over_area(div().id("trash-panel"), cx).w(metrics::rems_of(
                crate::shell::details::overlay_width(metrics::at_100(metrics::WIDTH_DETAILS), room),
            ))
        } else {
            div().id("trash-panel").w(metrics::WIDTH_DETAILS)
        };
        Some(
            frame
                .test_support()
                .flex_none()
                .h_full()
                .border_l_1()
                .border_color(theme.border)
                .bg(theme.colors.list)
                .child(
                    v_flex().size_full().child(header).child(
                        v_flex()
                            .id("trash-panel-body")
                            .flex_1()
                            .min_h_0()
                            .overflow_y_scroll()
                            .px(metrics::SPACE_4)
                            .pb(metrics::SPACE_4)
                            .gap(metrics::SPACE_3)
                            .child(widgets::caption(strings::PANEL_READ_ONLY, cx))
                            .child(summary)
                            .children(data),
                    ),
                )
                .into_any_element(),
        )
    }
}

fn where_label(item: &DeletedItem) -> &'static str {
    if item.from_sync {
        strings::OTHER_DEVICE
    } else {
        strings::THIS_DEVICE
    }
}

/// A menu item; a danger one takes `--danger-hover` on hover, as on the
/// other screens (design-system 6).
fn menu_item(id: SharedString, label: SharedString, danger: bool, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let hover = theme.danger_hover;
    div()
        .id(id)
        .test_support()
        .flex_1()
        .text_color(if danger {
            theme.danger
        } else {
            theme.foreground
        })
        .when(danger, |this| {
            this.mx(-metrics::SPACE_2)
                .px(metrics::SPACE_2)
                .my(-metrics::SPACE_1)
                .py(metrics::SPACE_1)
                .hover(move |style| style.text_color(hover))
        })
        .child(label)
        .into_any_element()
}

/// A row's context menu (UI 14): Restore, Export…, Delete forever, for the
/// selection, or for the row when it is not selected.
fn row_menu(menu: PopupMenu, screen: gpui::WeakEntity<TrashScreen>, ids: Vec<Uuid>) -> PopupMenu {
    let (restore, export, delete) = (ids.clone(), ids.clone(), ids);
    let (to_restore, to_export) = (screen.clone(), screen.clone());
    menu.min_w(px(200.))
        .item(
            PopupMenuItem::element(|_, cx| {
                menu_item(
                    "trash-menu-restore".into(),
                    strings::RESTORE.into(),
                    false,
                    cx,
                )
            })
            .on_click(move |_, window, cx| {
                let ids = restore.clone();
                let _ = to_restore.update(cx, |screen, cx| screen.restore(ids, window, cx));
            }),
        )
        .item(
            PopupMenuItem::element(|_, cx| {
                menu_item(
                    "trash-menu-export".into(),
                    strings::EXPORT.into(),
                    false,
                    cx,
                )
            })
            .on_click(move |_, window, cx| {
                let ids = export.clone();
                let _ = to_export.update(cx, |screen, cx| screen.export(ids, window, cx));
            }),
        )
        .separator()
        .item(
            PopupMenuItem::element(|_, cx| {
                menu_item(
                    "trash-menu-delete".into(),
                    strings::DELETE_FOREVER.into(),
                    true,
                    cx,
                )
            })
            .on_click(move |_, window, cx| {
                let ids = delete.clone();
                let _ = screen.update(cx, |screen, cx| screen.confirm_delete(ids, window, cx));
            }),
        )
}

/// A confirmation with Cancel and a danger button; the button or Enter
/// confirms, Esc and Cancel close without a change.
fn open_confirmation(
    id: &'static str,
    heading: &'static str,
    body: String,
    action: &'static str,
    window: &mut Window,
    cx: &mut App,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
) {
    let on_confirm = Rc::new(on_confirm);
    let body: SharedString = body.into();
    window.open_dialog(cx, move |dialog, _, cx| {
        let confirm = on_confirm.clone();
        let on_ok = on_confirm.clone();
        dialog
            .w(metrics::to_px(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(heading),
            )
            .child(
                div()
                    .id(SharedString::from(format!("{id}-dialog")))
                    .test_support()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .text_color(cx.theme().muted_foreground)
                    .child(body.clone()),
            )
            .on_ok(move |_, window, cx| {
                on_ok(window, cx);
                true
            })
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new(
                            SharedString::from(format!("{id}-cancel")),
                            ControlKind::Neutral,
                        )
                        .label(strings::CANCEL)
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new(
                            SharedString::from(format!("{id}-confirm")),
                            ControlKind::Danger,
                        )
                        .label(action)
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            confirm(window, cx);
                        }),
                    ),
            )
    });
}

impl Render for TrashScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let shown: Vec<DeletedItem> = self.shown(cx).into_iter().cloned().collect();
        let rows: Vec<AnyElement> = shown.iter().map(|item| self.render_row(item, cx)).collect();
        let empty = (rows.is_empty() && self.loading.is_none()).then(|| self.render_empty(cx));
        let main = v_flex()
            .id("trash-main")
            .flex_1()
            .min_w_0()
            .h_full()
            .px(CONTENT_PADDING)
            .pt(metrics::SPACE_4)
            .gap(metrics::SPACE_3)
            .child(
                div()
                    .id("trash-hint")
                    .test_support()
                    .child(widgets::caption(strings::HINT, cx)),
            )
            .child(self.render_action_bar(window, cx))
            .child(
                v_flex()
                    .id("trash-table")
                    .test_support()
                    .flex_1()
                    .min_h_0()
                    .track_focus(&self.focus)
                    // Ctrl+A selects every row shown; Esc clears.
                    .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                        let keystroke = &event.keystroke;
                        if keystroke.key == "a" && keystroke.modifiers.secondary() {
                            cx.stop_propagation();
                            this.select_all(cx);
                        } else if keystroke.key == "escape" && !this.selection.is_empty() {
                            cx.stop_propagation();
                            this.clear_selection(cx);
                        }
                    }))
                    .child(self.render_header(cx))
                    .children(empty)
                    .when(!rows.is_empty(), |this| {
                        this.child(
                            v_flex()
                                .id("trash-rows")
                                .test_support()
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .children(rows),
                        )
                    }),
            );
        h_flex()
            .id("trash-screen")
            .test_support()
            .relative()
            .size_full()
            .bg(cx.theme().colors.list)
            .text_color(cx.theme().foreground)
            .child(main)
            .children(self.render_panel(window, cx))
    }
}
