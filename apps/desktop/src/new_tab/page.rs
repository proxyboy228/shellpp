//! The new tab page (UI 6): the content of a "New Tab" tab until a target is
//! chosen; the shell then opens the session in the same tab.
//!
//! Layout: a column of at most [`COLUMN_WIDTH`] centered at the top of the
//! tab's area (`--space-6` above it, `--space-5` at the sides): the search
//! field with the Ctrl+K hint, then the header "Recent connections" (16 px,
//! 600, design-system 5) with "Create a workspace" (disabled until stage 9,
//! tooltip) and Restore (disabled without a snapshot of the previous
//! launch), then the list. While the field holds text the header gives way
//! to the search results (open tabs, then hosts).

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render,
    SharedString, Window, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, h_flex, v_flex};
use uuid::Uuid;

use super::model::{Item, Row};
use super::picker::{self, COLUMN_WIDTH, Picker, PickerHost};
use super::{NewTabEvent, OpenTab, model::PickerKind, strings};
use crate::hosts::controls::{self, ControlButton, ControlKind};
use crate::state::AppState;
use crate::theme::{self, metrics};

/// The shortcut shown in the search field (UI 6, 12.5).
const SEARCH_SHORTCUT: &str = "secondary-k";

/// The new tab page.
pub struct NewTabPage {
    picker: Picker,
    restore_available: bool,
    /// The saved workspaces (record id and name), by name (UI 4.5).
    workspaces: Vec<(Uuid, SharedString)>,
    /// The window has tabs a workspace can keep.
    can_create_workspace: bool,
}

impl std::fmt::Debug for NewTabPage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NewTabPage")
            .field("picker", &self.picker)
            .field("restore_available", &self.restore_available)
            .finish()
    }
}

impl EventEmitter<NewTabEvent> for NewTabPage {}

impl PickerHost for NewTabPage {
    fn picker(&self) -> &Picker {
        &self.picker
    }

    fn picker_mut(&mut self) -> &mut Picker {
        &mut self.picker
    }
}

impl Focusable for NewTabPage {
    /// The search field.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.focus_handle(cx)
    }
}

impl NewTabPage {
    /// The page; the search field takes the focus.
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self {
            picker: Picker::new(PickerKind::Page, "new-tab", state, None, window, cx),
            restore_available: false,
            workspaces: Vec::new(),
            can_create_workspace: false,
        }
    }

    /// The open tabs of every window the search offers (the page's own
    /// tab left out by the caller).
    pub fn set_open_tabs(&mut self, tabs: Vec<OpenTab>, cx: &mut Context<Self>) {
        if self.picker.set_open_tabs(tabs, cx) {
            cx.notify();
        }
    }

    /// Whether a snapshot of the previous launch exists (Restore enabled).
    pub fn set_restore_available(&mut self, available: bool, cx: &mut Context<Self>) {
        if self.restore_available != available {
            self.restore_available = available;
            cx.notify();
        }
    }

    pub fn restore_available(&self) -> bool {
        self.restore_available
    }

    /// The saved workspaces and whether this window can make one.
    pub fn set_workspaces(
        &mut self,
        workspaces: Vec<(Uuid, SharedString)>,
        can_create: bool,
        cx: &mut Context<Self>,
    ) {
        if self.workspaces != workspaces || self.can_create_workspace != can_create {
            self.picker.set_workspaces(workspaces.clone(), cx);
            self.workspaces = workspaces;
            self.can_create_workspace = can_create;
            cx.notify();
        }
    }

    /// The block of saved workspaces (UI 4.5: a block of its own on the
    /// page): open one with a click, remove it with its ×.
    fn render_workspaces(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        if self.workspaces.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let items = self.workspaces.iter().map(|(id, name)| {
            let id = *id;
            h_flex()
                .gap(px(2.))
                .child(
                    ControlButton::new(
                        SharedString::from(format!("new-tab-workspace-{id}")),
                        ControlKind::Neutral,
                    )
                    .icon(IconName::LayoutDashboard)
                    .label(name.clone())
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.emit(NewTabEvent::OpenWorkspace(id))
                    })),
                )
                .child(
                    ControlButton::new(
                        SharedString::from(format!("new-tab-workspace-rename-{id}")),
                        ControlKind::Ghost,
                    )
                    .icon(IconName::Pencil)
                    .tooltip(SharedString::from(strings::rename_workspace(name)))
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.emit(NewTabEvent::RenameWorkspace(id))
                    })),
                )
                .child(
                    ControlButton::new(
                        SharedString::from(format!("new-tab-workspace-remove-{id}")),
                        ControlKind::Ghost,
                    )
                    .icon(IconName::Close)
                    .tooltip(SharedString::from(strings::remove_workspace(name)))
                    .on_click(cx.listener(move |_, _: &ClickEvent, _, cx| {
                        cx.emit(NewTabEvent::RemoveWorkspace(id))
                    })),
                )
        });
        Some(
            v_flex()
                .id("new-tab-workspaces")
                .test_support()
                .w_full()
                .flex_none()
                .gap(metrics::SPACE_2)
                .child(
                    gpui::div()
                        .text_size(metrics::FONT_SIZE_HEADING)
                        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                        .text_color(theme.foreground)
                        .child(strings::WORKSPACES),
                )
                .child(h_flex().flex_wrap().gap(metrics::SPACE_2).children(items)),
        )
    }

    /// Puts the focus in the search field.
    pub fn focus_search(&self, window: &mut Window, cx: &mut App) {
        self.picker
            .search()
            .update(cx, |input, cx| input.focus(window, cx));
    }

    /// The search text.
    pub fn query(&self) -> &str {
        self.picker.query()
    }

    /// The list entries (headings and rows), in order.
    pub fn items(&self) -> &[Item] {
        self.picker.items()
    }

    /// The rows, in order.
    pub fn rows(&self) -> Vec<&Row> {
        self.picker.rows()
    }

    /// The selected row.
    pub fn selected_row(&self) -> Option<&Row> {
        self.picker.selected_row()
    }

    fn render_header(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let available = self.restore_available;
        // One group: both as wide as the wider (design-system 6).
        let width = controls::group_width(window, &[strings::CREATE_WORKSPACE, strings::RESTORE]);
        let can_create = self.can_create_workspace;
        let workspace = ControlButton::new("new-tab-workspace", ControlKind::Ghost)
            .icon(IconName::Plus)
            .label(strings::CREATE_WORKSPACE)
            .min_w(width)
            .tooltip(if can_create {
                strings::CREATE_WORKSPACE_HINT
            } else {
                strings::CREATE_WORKSPACE_NOTHING
            })
            .disabled(!can_create)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if this.can_create_workspace {
                    cx.emit(NewTabEvent::CreateWorkspace);
                }
            }));
        let restore = ControlButton::new("new-tab-restore", ControlKind::Ghost)
            .icon(IconName::RotateCcwClock)
            .label(strings::RESTORE)
            .min_w(width)
            .tooltip(if available {
                strings::RESTORE_HINT
            } else {
                strings::RESTORE_NOTHING
            })
            .disabled(!available)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                if this.restore_available {
                    cx.emit(NewTabEvent::Restore);
                }
            }));
        h_flex()
            .id("new-tab-recent-header")
            .test_support()
            .w_full()
            .flex_none()
            .flex_wrap()
            .gap(metrics::SPACE_2)
            .items_center()
            .justify_between()
            .child(
                gpui::div()
                    .min_w_0()
                    .truncate()
                    .text_size(metrics::FONT_SIZE_HEADING)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(strings::RECENT_CONNECTIONS),
            )
            .child(
                h_flex()
                    .flex_none()
                    .gap(metrics::SPACE_1)
                    .child(workspace)
                    .child(restore),
            )
    }
}

impl Render for NewTabPage {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let field = self.picker.render_field(Some(SEARCH_SHORTCUT), window, cx);
        let header = (!self.picker.is_searching()).then(|| self.render_header(window, cx));
        let workspaces = (!self.picker.is_searching())
            .then(|| self.render_workspaces(cx))
            .flatten();
        let list = self.picker.render_list(None, cx);
        let column = picker::keys(v_flex().id("new-tab-column"), cx)
            .w_full()
            .max_w(COLUMN_WIDTH)
            .flex_1()
            .min_h_0()
            .gap(metrics::SPACE_4)
            .child(field)
            .children(workspaces)
            .children(header)
            .child(list);
        v_flex()
            .id("new-tab-page")
            .test_support()
            .size_full()
            .items_center()
            .pt(metrics::SPACE_6)
            .px(metrics::SPACE_5)
            .pb(metrics::SPACE_4)
            .bg(theme::tokens(cx).bg_surface.hsla())
            .text_color(cx.theme().foreground)
            .child(column)
    }
}
