//! The host selection of a new pane (UI 4.7; lead decision L1): a compact
//! new tab page with the search field (focused) and one list. Its first row
//! is the split pane's target ([`PickerOrigin`]): the same host, the same
//! quick-connect address, or a new local terminal with the same shell, so
//! Enter at once opens another session like it. Below it Recent
//! connections; typing searches hosts like the page (no open tabs: the
//! pane needs a new session). Esc emits [`NewTabEvent::Dismissed`]: the
//! pane closes and the split is undone [decision].
//!
//! Layout: a column of at most [`COLUMN_WIDTH`] centered in the pane,
//! `--space-4` around it, the field and the list `--space-3` apart.

use gpui::prelude::*;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, Render, Window,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, v_flex};

use super::model::{Item, PickerKind, Row};
use super::picker::{self, COLUMN_WIDTH, Picker, PickerHost};
use super::{Dismiss, NewTabEvent, PickerOrigin};
use crate::state::AppState;
use crate::theme::{self, metrics};

/// The picker of a new pane.
pub struct CompactPicker {
    picker: Picker,
}

impl std::fmt::Debug for CompactPicker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompactPicker")
            .field("picker", &self.picker)
            .finish()
    }
}

impl EventEmitter<NewTabEvent> for CompactPicker {}

impl PickerHost for CompactPicker {
    fn picker(&self) -> &Picker {
        &self.picker
    }

    fn picker_mut(&mut self) -> &mut Picker {
        &mut self.picker
    }
}

impl Focusable for CompactPicker {
    /// The search field.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.focus_handle(cx)
    }
}

impl CompactPicker {
    /// The picker of a pane split from a pane with `origin` (`None`: no
    /// first row); the search field takes the focus.
    pub fn new(
        state: Entity<AppState>,
        origin: Option<PickerOrigin>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            picker: Picker::new(
                PickerKind::Compact,
                "pane-picker",
                state,
                origin,
                window,
                cx,
            ),
        }
    }

    /// Changes the first row's target (`None`: no first row).
    pub fn set_origin(&mut self, origin: Option<PickerOrigin>, cx: &mut Context<Self>) {
        if self.picker.set_origin(origin, cx) {
            cx.notify();
        }
    }

    /// Puts the focus in the search field (the pane became active).
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
}

impl Render for CompactPicker {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let field = self.picker.render_field(None, window, cx);
        let list = self.picker.render_list(None, cx);
        let column = picker::keys(v_flex().id("pane-picker-column"), cx)
            .w_full()
            .max_w(COLUMN_WIDTH)
            .flex_1()
            .min_h_0()
            .gap(metrics::SPACE_3)
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(NewTabEvent::Dismissed)))
            .child(field)
            .child(list);
        v_flex()
            .id("pane-picker")
            .test_support()
            .size_full()
            .items_center()
            .p(metrics::SPACE_4)
            .bg(theme::tokens(cx).bg_surface.hsla())
            .text_color(cx.theme().foreground)
            .child(column)
    }
}
