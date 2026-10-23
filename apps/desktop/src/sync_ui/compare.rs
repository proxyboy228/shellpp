//! Sync conflicts (UI 13.6): the comparison of the two versions, the list
//! "Conflicts: N" of Account & Sync opens, and the badge of a card.
//!
//! - The comparison shows the current version on the left and the other
//!   (losing) version on the right, each with the device and the time of
//!   its change; the fields that differ are highlighted, a deletion says
//!   "Deleted". "Keep this" keeps the current version, "Keep other"
//!   replaces it with the other one; closing leaves the conflict.
//! - The core masks passwords and hides private keys in a conflict
//!   (`ConflictItem`); nothing here unmasks them.
//! - Every kind of object uses the same comparison: both versions as lists
//!   of their fields (D93), so objects without a card of their own
//!   (workspaces, device names, a vault's name) need nothing else.

use std::collections::BTreeMap;
use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, FontWeight,
    InteractiveElement as _, IntoElement, MouseButton, ParentElement as _, Render, SharedString,
    StatefulInteractiveElement as _, Styled as _, Subscription, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::{ActiveTheme as _, Icon, WindowExt as _, h_flex, v_flex};
use serde_json::{Map, Value};
use shellpp_core::service::{ConflictChoice, ConflictItem};
use uuid::Uuid;

use super::{error_line, error_text, label, strings, title, when};
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::state::AppState;
use crate::theme::{self, metrics};

/// Wide enough for two columns of fields at the minimum window (960 px).
const COMPARE_WIDTH: f32 = 760.;
/// The field names' column inside a side.
const FIELD_NAME_WIDTH: f32 = 112.;
/// The field list scrolls past this height; the buttons stay in view.
const FIELDS_MAX_HEIGHT: f32 = 360.;

/// Called after a choice (the list under the comparison reloads).
pub type Resolved = Rc<dyn Fn(&mut Window, &mut App)>;

/// The object type as the list names it.
pub fn type_label(type_name: &str) -> &str {
    match type_name {
        "host" => "Host",
        "group" => "Group",
        "key" => "Key",
        "identity" => "Identity",
        "known_host" => "Known host",
        "snippet" => "Snippet",
        "snippet_package" => "Snippet package",
        "forwarding_rule" => "Forwarding rule",
        "highlight_rule" => "Highlighting rule",
        "workspace" => "Workspace",
        "history_entry" => "History entry",
        "history_hint_state" => "History hint",
        "connection_log_entry" => "Connection log entry",
        "last_session" => "Last session",
        "device_info" => "Device name",
        "vault_meta" => "Vault name",
        "recovery_key" => "Recovery key",
        "terminal_theme" => "Terminal theme",
        other => other,
    }
}

/// Which version a side shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Side {
    Current,
    Other,
}

impl Side {
    fn key(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Other => "other",
        }
    }
}

/// One row of the comparison: a field and its value on each side (`None`:
/// the version has no such field or is deleted).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FieldRow {
    /// Dotted path, for example `ssh.port`.
    pub path: String,
    pub current: Option<String>,
    pub other: Option<String>,
    pub differs: bool,
}

/// A value as one line of text (secrets arrive masked).
fn value_text(value: &Value) -> Option<String> {
    match value {
        Value::Null => None,
        Value::String(text) if text.is_empty() => None,
        Value::String(text) => Some(text.clone()),
        Value::Array(items) if items.is_empty() => None,
        Value::Array(items) => Some(
            items
                .iter()
                .map(|item| value_text(item).unwrap_or_else(|| item.to_string()))
                .collect::<Vec<_>>()
                .join(", "),
        ),
        other => Some(other.to_string()),
    }
}

/// The fields of a version, one level of nesting flattened (`ssh.port`) as
/// the core compares them.
fn flatten(data: Option<&Map<String, Value>>) -> BTreeMap<String, String> {
    let mut fields = BTreeMap::new();
    for (name, value) in data.into_iter().flatten() {
        match value {
            Value::Object(inner) => {
                for (key, value) in inner {
                    if let Some(text) = value_text(value) {
                        fields.insert(format!("{name}.{key}"), text);
                    }
                }
            }
            value => {
                if let Some(text) = value_text(value) {
                    fields.insert(name.clone(), text);
                }
            }
        }
    }
    fields
}

/// The rows of the comparison, by field name; a row differs when the core
/// lists its field (or the member it belongs to) among the differences.
pub fn field_rows(item: &ConflictItem) -> Vec<FieldRow> {
    let current = flatten(item.current_data.as_ref());
    let other = flatten(item.losing_data.as_ref());
    let mut paths: Vec<&String> = current.keys().chain(other.keys()).collect();
    paths.sort();
    paths.dedup();
    paths
        .into_iter()
        .map(|path| FieldRow {
            differs: item.differences.iter().any(|difference| {
                difference.path == *path
                    || path
                        .strip_prefix(difference.path.as_str())
                        .is_some_and(|rest| rest.starts_with('.'))
            }),
            current: current.get(path).cloned(),
            other: other.get(path).cloned(),
            path: path.clone(),
        })
        .collect()
}

/// The device of a version: this device, its name, or the start of its id.
fn device_text(item: &ConflictItem, side: Side) -> String {
    let (this, name, id) = match side {
        Side::Current => (
            item.current_this_device,
            &item.current_device_name,
            &item.current_device_id,
        ),
        Side::Other => (
            item.losing_this_device,
            &item.losing_device_name,
            &item.losing_device_id,
        ),
    };
    if this {
        return strings::VERSION_THIS_DEVICE.to_owned();
    }
    match (name, id) {
        (Some(name), _) => name.clone(),
        (None, Some(id)) => format!("{} {}", strings::DEVICE, &id[..id.len().min(8)]),
        (None, None) => strings::UNKNOWN_DEVICE.to_owned(),
    }
}

/// "Laptop · 23.10.2026 09:15".
fn origin_text(item: &ConflictItem, side: Side) -> String {
    let changed = match side {
        Side::Current => &item.current_changed_at,
        Side::Other => &item.losing_changed_at,
    };
    let device = device_text(item, side);
    match changed {
        Some(changed) => format!("{device} · {}", when(changed)),
        None => device,
    }
}

fn deleted(item: &ConflictItem, side: Side) -> bool {
    match side {
        Side::Current => item.current_deleted,
        Side::Other => item.losing_deleted,
    }
}

// ---------------------------------------------------------------------------
// The comparison
// ---------------------------------------------------------------------------

pub struct CompareDialog {
    state: Entity<AppState>,
    pub item: ConflictItem,
    on_resolved: Option<Resolved>,
    error: Option<&'static str>,
}

impl std::fmt::Debug for CompareDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompareDialog").finish_non_exhaustive()
    }
}

impl CompareDialog {
    fn resolve(&mut self, choice: ConflictChoice, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        match core.sync_conflict_resolve(self.item.conflict_id, choice) {
            Ok(()) => {
                self.state.update(cx, |state, cx| {
                    state.refresh_sync_status(cx);
                    state.kick_sync(cx);
                });
                if let Some(resolved) = self.on_resolved.clone() {
                    resolved(window, cx);
                }
                window.close_dialog(cx);
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "sync conflict not resolved");
                self.error = Some(error_text(&error));
                cx.notify();
            }
        }
    }

    /// The head of a side: its name, the device and the time, "Deleted".
    fn side_head(&self, side: Side, cx: &App) -> impl IntoElement {
        let heading = match side {
            Side::Current => strings::CURRENT_VERSION,
            Side::Other => strings::OTHER_VERSION,
        };
        v_flex()
            .id(SharedString::from(format!("sync-compare-{}", side.key())))
            .test_support()
            .flex_1()
            .min_w_0()
            .gap(metrics::SPACE_1)
            .child(label(heading, cx))
            .child(widgets::caption(origin_text(&self.item, side), cx).truncate())
            .when(deleted(&self.item, side), |this| {
                this.child(
                    div()
                        .id(SharedString::from(format!(
                            "sync-compare-{}-deleted",
                            side.key()
                        )))
                        .test_support()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                        .text_color(cx.theme().danger)
                        .child(strings::DELETED),
                )
            })
    }

    /// One side of a row: the field name and its value; a differing field
    /// on `--warning-subtle` with a `--warning` mark (text stays
    /// `--text-primary`, design-system 2.5), so the difference does not
    /// rest on the tint alone. A deleted side stays empty.
    fn cell(&self, row: &FieldRow, side: Side, cx: &App) -> AnyElement {
        let value = match side {
            Side::Current => &row.current,
            Side::Other => &row.other,
        };
        if deleted(&self.item, side) {
            return div().flex_1().min_w_0().into_any_element();
        }
        let theme = cx.theme();
        let mark = if row.differs { "diff" } else { "same" };
        h_flex()
            .id(SharedString::from(format!(
                "sync-compare-{mark}-{}-{}",
                side.key(),
                row.path
            )))
            .test_support()
            .flex_1()
            .min_w_0()
            .gap(metrics::SPACE_2)
            .pl(metrics::SPACE_1)
            .pr(metrics::SPACE_2)
            .py(metrics::SPACE_1)
            .rounded(metrics::RADIUS_CONTROL)
            .when(row.differs, |this| {
                this.bg(theme::tokens(cx).warning_subtle.hsla())
            })
            .child(
                div()
                    .flex_none()
                    .w(metrics::rpx(3.))
                    .h(metrics::rpx(16.))
                    .rounded(metrics::rpx(2.))
                    .when(row.differs, |this| this.bg(theme.warning)),
            )
            .child(
                div()
                    .flex_none()
                    .w(metrics::rpx(FIELD_NAME_WIDTH))
                    .truncate()
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(theme.muted_foreground)
                    .child(row.path.clone()),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .text_color(theme.popover_foreground)
                    .child(value.clone().unwrap_or_else(|| "—".to_owned())),
            )
            .into_any_element()
    }
}

impl Render for CompareDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let item = &self.item;
        let rows = field_rows(item);
        let mut fields = v_flex()
            .id("sync-compare-fields")
            .test_support()
            .w_full()
            .max_h(metrics::rpx(FIELDS_MAX_HEIGHT))
            .overflow_y_scroll()
            .gap(metrics::rpx(2.));
        for row in &rows {
            fields = fields.child(
                h_flex()
                    .w_full()
                    .gap(metrics::SPACE_3)
                    .child(self.cell(row, Side::Current, cx))
                    .child(self.cell(row, Side::Other, cx)),
            );
        }
        let keep = |id: &'static str, text: &'static str, choice: ConflictChoice| {
            // Outlined: `--bg-raised` alone is the dialog's own surface in
            // the light theme, and these two are the choice.
            ControlButton::new(id, ControlKind::Neutral)
                .label(text)
                .w_full()
                .border_1()
                .border_color(cx.theme().border)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.resolve(choice, window, cx)
                }))
        };
        v_flex()
            .id("sync-compare")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(
                v_flex()
                    .gap(metrics::SPACE_1)
                    .child(
                        div()
                            .id("sync-compare-object")
                            .test_support()
                            .text_size(metrics::FONT_SIZE_BODY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(cx.theme().popover_foreground)
                            .truncate()
                            .child(format!(
                                "{} · {}",
                                item.title,
                                type_label(&item.original_type)
                            )),
                    )
                    .child(widgets::caption(strings::COMPARE_HINT, cx)),
            )
            .child(
                h_flex()
                    .w_full()
                    .gap(metrics::SPACE_3)
                    .items_start()
                    .child(self.side_head(Side::Current, cx))
                    .child(self.side_head(Side::Other, cx)),
            )
            .child(widgets::divider(cx))
            .child(fields)
            .child(
                h_flex()
                    .w_full()
                    .gap(metrics::SPACE_3)
                    .child(div().flex_1().min_w_0().child(keep(
                        "sync-keep-this",
                        strings::KEEP_THIS,
                        ConflictChoice::Current,
                    )))
                    .child(div().flex_1().min_w_0().child(keep(
                        "sync-keep-other",
                        strings::KEEP_OTHER,
                        ConflictChoice::Copy,
                    ))),
            )
            .children(error_line(self.error, cx))
    }
}

/// Opens the comparison of `item` (a click on a badge or on a row of the
/// list). `on_resolved` runs after a choice, before the dialog closes.
pub fn open(
    state: Entity<AppState>,
    item: ConflictItem,
    on_resolved: Option<Resolved>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<CompareDialog> {
    let dialog = cx.new(|_| CompareDialog {
        state,
        item,
        on_resolved,
        error: None,
    });
    let view = dialog.clone();
    window.open_dialog(cx, move |modal, _, cx| {
        modal
            .w(metrics::scaled(COMPARE_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(strings::COMPARE_TITLE))
            .child(view.clone())
            .footer(super::close_footer("sync-compare-close"))
    });
    dialog
}

/// Opens the comparison of the first conflict of a record (its badge).
pub fn open_for_record(
    state: Entity<AppState>,
    record_id: Uuid,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<CompareDialog>> {
    let core = state.read(cx).core().cloned()?;
    let item = core
        .sync_conflicts()
        .unwrap_or_default()
        .into_iter()
        .find(|item| item.original_record_id == record_id);
    match item {
        Some(item) => Some(open(state, item, None, window, cx)),
        None => {
            // Resolved elsewhere since the badge was drawn.
            state.update(cx, |state, cx| state.refresh_sync_status(cx));
            None
        }
    }
}

// ---------------------------------------------------------------------------
// The list "Conflicts: N" opens
// ---------------------------------------------------------------------------

/// A conflict in a list: the object and its type, where the other version
/// comes from; a click (`on_click`) opens the comparison.
pub fn conflict_row(
    id: impl Into<gpui::ElementId>,
    item: &ConflictItem,
    on_click: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let hover = theme::tokens(cx).bg_overlay_hover.hsla();
    let detail = format!(
        "{} · {}: {}",
        type_label(&item.original_type),
        strings::OTHER_VERSION,
        if item.losing_deleted {
            format!("{}, {}", strings::DELETED, origin_text(item, Side::Other))
        } else {
            origin_text(item, Side::Other)
        }
    );
    h_flex()
        .id(id.into())
        .test_support()
        .w_full()
        .gap(metrics::SPACE_2)
        .px(metrics::SPACE_2)
        .py(metrics::SPACE_2)
        .rounded(metrics::RADIUS_CONTROL)
        .cursor_pointer()
        .hover(move |style| style.bg(hover))
        .on_click(on_click)
        .child(super::row_text(item.title.clone(), Some(detail), cx))
        .child(
            Icon::new(Lucide::ChevronRight)
                .size(metrics::rpx(16.))
                .text_color(cx.theme().muted_foreground),
        )
        .into_any_element()
}

pub struct ConflictsDialog {
    state: Entity<AppState>,
    pub items: Vec<ConflictItem>,
    _state: Subscription,
}

impl std::fmt::Debug for ConflictsDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConflictsDialog").finish_non_exhaustive()
    }
}

impl ConflictsDialog {
    fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        // A sync or a choice made elsewhere changes the status: the list
        // follows it.
        let subscription = cx.observe(&state, |this: &mut Self, _, cx| this.reload(cx));
        let mut dialog = Self {
            state,
            items: Vec::new(),
            _state: subscription,
        };
        dialog.reload(cx);
        dialog
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let items = core.sync_conflicts().unwrap_or_default();
        if items != self.items {
            self.items = items;
            cx.notify();
        }
    }
}

impl Render for ConflictsDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = v_flex()
            .id("sync-conflicts-list")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_1);
        if self.items.is_empty() {
            return body.child(widgets::caption(strings::NO_CONFLICTS, cx));
        }
        body = body.child(
            div()
                .pb(metrics::SPACE_2)
                .child(widgets::caption(strings::CONFLICTS_HINT, cx)),
        );
        for (index, item) in self.items.iter().enumerate() {
            let opened = item.clone();
            let click = cx.listener(move |this, _: &ClickEvent, window, cx| {
                open(this.state.clone(), opened.clone(), None, window, cx);
            });
            body = body.child(conflict_row(("sync-conflict", index), item, click, cx));
        }
        body
    }
}

/// Opens the list of conflicts ("Conflicts: N" in Account & Sync).
pub fn open_list(
    state: Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ConflictsDialog> {
    let dialog = cx.new(|cx| ConflictsDialog::new(state, cx));
    super::open_simple(
        dialog.clone(),
        strings::CONFLICTS_TITLE,
        "sync-conflicts-close",
        window,
        cx,
    );
    dialog
}

/// "Conflicts: N" of Account & Sync (only with conflicts).
pub fn conflicts_button(state: &Entity<AppState>, count: usize) -> ControlButton {
    let state = state.clone();
    ControlButton::new("sync-conflicts", ControlKind::Neutral)
        .label(format!("{}: {count}", strings::CONFLICTS))
        .on_click(move |_, window, cx| {
            window.close_dialog(cx);
            open_list(state.clone(), window, cx);
        })
}

// ---------------------------------------------------------------------------
// The badge of a card
// ---------------------------------------------------------------------------

/// The conflict badge of a card (UI 13.6): `--warning` outline and text, in
/// the shape of the known hosts' "Revoked" badge. A click opens the
/// comparison and does not reach the card.
pub fn conflict_badge(
    id: impl Into<gpui::ElementId>,
    state: &Entity<AppState>,
    record_id: Uuid,
    cx: &App,
) -> impl IntoElement {
    let warning = cx.theme().warning;
    let state = state.clone();
    div()
        .id(id.into())
        .test_support()
        .flex_none()
        .flex()
        .items_center()
        .h(metrics::rpx(20.))
        .px(metrics::SPACE_2)
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .border_color(warning)
        .text_color(warning)
        .text_size(metrics::FONT_SIZE_CAPTION)
        .whitespace_nowrap()
        .cursor_pointer()
        .tooltip(|window, cx| {
            gpui_kit::component::tooltip::Tooltip::new(strings::CONFLICT_TOOLTIP).build(window, cx)
        })
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .on_click(move |_, window, cx| {
            cx.stop_propagation();
            open_for_record(state.clone(), record_id, window, cx);
        })
        .child(strings::CONFLICT_BADGE)
}

#[cfg(test)]
mod tests;
