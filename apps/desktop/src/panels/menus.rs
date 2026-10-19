//! The "…" menus of the panels and the removal confirmations.
//!
//! - Host panel (UI 3.3): Connect, Add Telnet, Duplicate, Remove, in this order.
//! - Group menu (UI 3.4, also the group card's right-click menu, UI 2.3):
//!   Open, Edit, Move to ▸, Remove, in this order.
//!
//! The order lives in [`HOST_PANEL_MENU`] and [`GROUP_MENU`]; the views build
//! their popup menus from them and tests check them. Remove is a dangerous
//! action: `--danger` text, `--danger-hover` under the pointer.
//! [`confirm_remove_group`] asks before removing a group and, for a non-empty
//! group, says that its subgroups and hosts move one level up (the core moves
//! them in the same transaction, data-model 7.1).

use gpui::prelude::*;
use gpui::{App, Entity, SharedString, Window, div};
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::menu::PopupMenuItem;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _};
use uuid::Uuid;

use super::strings;
use crate::state::AppState;
use crate::theme::metrics;

/// The actions of the host panel's "…" menu (UI 3.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostPanelAction {
    Connect,
    AddTelnet,
    Duplicate,
    Remove,
}

/// UI 3.3, in order.
pub const HOST_PANEL_MENU: [HostPanelAction; 4] = [
    HostPanelAction::Connect,
    HostPanelAction::AddTelnet,
    HostPanelAction::Duplicate,
    HostPanelAction::Remove,
];

impl HostPanelAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Connect => strings::MENU_CONNECT,
            Self::AddTelnet => strings::MENU_ADD_TELNET,
            Self::Duplicate => strings::MENU_DUPLICATE,
            Self::Remove => strings::MENU_REMOVE,
        }
    }

    pub fn is_danger(self) -> bool {
        self == Self::Remove
    }
}

/// The group menu (UI 3.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GroupMenuAction {
    Open,
    Edit,
    /// A submenu of target groups (never the group itself or its subtree).
    MoveTo,
    Remove,
}

/// UI 3.4, in order.
pub const GROUP_MENU: [GroupMenuAction; 4] = [
    GroupMenuAction::Open,
    GroupMenuAction::Edit,
    GroupMenuAction::MoveTo,
    GroupMenuAction::Remove,
];

impl GroupMenuAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::Open => strings::MENU_OPEN,
            Self::Edit => strings::MENU_EDIT,
            Self::MoveTo => strings::MENU_MOVE_TO,
            Self::Remove => strings::MENU_REMOVE,
        }
    }

    pub fn is_danger(self) -> bool {
        self == Self::Remove
    }
}

/// A popup menu item; a dangerous one is drawn in `--danger`.
pub fn menu_item(
    label: &'static str,
    danger: bool,
    disabled: bool,
    handler: impl Fn(&mut Window, &mut App) + 'static,
) -> PopupMenuItem {
    let item = if danger && !disabled {
        PopupMenuItem::element(move |_, cx: &mut App| {
            let theme = cx.theme();
            let danger = theme.danger;
            let danger_hover = theme.danger_hover;
            div()
                .id(SharedString::from(label))
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(danger)
                .hover(move |style| style.text_color(danger_hover))
                .child(label)
        })
    } else {
        PopupMenuItem::new(label)
    };
    item.disabled(disabled)
        .on_click(move |_, window, cx| handler(window, cx))
}

/// The groups a group can move to, as (target, title) with the top level
/// first: never the group itself, its subtree or its current parent.
pub fn group_move_choices(state: &AppState, group_id: Uuid) -> Vec<(Option<Uuid>, String)> {
    let current_parent = state.group(group_id).and_then(|group| group.parent_id);
    let mut choices = Vec::new();
    if current_parent.is_some() {
        choices.push((None, strings::TOP_LEVEL.to_owned()));
    }
    for group in state.group_move_targets(group_id) {
        if Some(group.record_id) != current_parent {
            choices.push((
                Some(group.record_id),
                state.group_path_label(group.record_id),
            ));
        }
    }
    choices
}

/// Asks before removing hosts; removes them on OK and then calls `done`.
pub fn confirm_remove_hosts(
    state: Entity<AppState>,
    host_ids: Vec<Uuid>,
    window: &mut Window,
    cx: &mut App,
    done: impl Fn(Vec<Uuid>, &mut Window, &mut App) + 'static,
) {
    let done = std::rc::Rc::new(done);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let state = state.clone();
        let host_ids = host_ids.clone();
        let done = done.clone();
        alert
            .title(strings::REMOVE_HOST_TITLE)
            .description(strings::REMOVE_HOST_BODY)
            .ok_text(strings::MENU_REMOVE)
            .ok_variant(ButtonVariant::Danger)
            .cancel_text(strings::CANCEL)
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                let ids = host_ids.clone();
                let result = state.update(cx, |state, cx| state.delete_records(ids, cx));
                match result {
                    Ok(removed) => done(removed.deleted, window, cx),
                    Err(error) => tracing::warn!(code = error.code(), "host not removed"),
                }
                true
            })
    });
}

/// Asks before removing a group (UI 3.4); a non-empty group's subgroups and
/// hosts move one level up. Calls `done` with the removed ids.
pub fn confirm_remove_group(
    state: Entity<AppState>,
    group_id: Uuid,
    window: &mut Window,
    cx: &mut App,
    done: impl Fn(Vec<Uuid>, &mut Window, &mut App) + 'static,
) {
    let non_empty = state
        .read(cx)
        .group(group_id)
        .is_some_and(|group| group.child_count > 0);
    let body = remove_group_body(non_empty);
    let done = std::rc::Rc::new(done);
    window.open_alert_dialog(cx, move |alert, _, _| {
        let state = state.clone();
        let done = done.clone();
        alert
            .title(strings::REMOVE_GROUP_TITLE)
            .description(body)
            .ok_text(strings::MENU_REMOVE)
            .ok_variant(ButtonVariant::Danger)
            .cancel_text(strings::CANCEL)
            .show_cancel(true)
            .on_ok(move |_, window, cx| {
                let result = state.update(cx, |state, cx| state.delete_records(vec![group_id], cx));
                match result {
                    Ok(removed) => done(removed.deleted, window, cx),
                    Err(error) => tracing::warn!(code = error.code(), "group not removed"),
                }
                true
            })
    });
}

/// The confirmation text for removing a group (UI 3.4).
pub fn remove_group_body(non_empty: bool) -> &'static str {
    if non_empty {
        strings::REMOVE_GROUP_NON_EMPTY_BODY
    } else {
        strings::REMOVE_GROUP_EMPTY_BODY
    }
}
