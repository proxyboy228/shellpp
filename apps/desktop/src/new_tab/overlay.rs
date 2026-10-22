//! The search overlay (UI 6, 12.5): Ctrl+K outside the terminal and
//! Ctrl+Shift+K everywhere open the search of the new tab page over the
//! current window instead of a new tab.
//!
//! The shell puts the overlay over the whole window (it fills its parent
//! and occludes what is below). `--scrim` dims the window; the panel is
//! [`COLUMN_WIDTH`] wide at most, `--space-6` below the top bar,
//! `--bg-overlay`, radius 12, `--shadow-modal`, with the search field and
//! a list of at most [`LIST_ROWS`] rows' height before it scrolls
//! [decision]. Esc or a click outside the panel closes it: the focus goes
//! back where it was and [`NewTabEvent::Dismissed`] tells the shell to drop
//! the overlay. Choosing a row emits its event; the shell then closes the
//! overlay.

use gpui::prelude::*;
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, MouseButton,
    MouseDownEvent, Pixels, Render, Window, div,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, v_flex};

use super::model::{Item, PickerKind, Row};
use super::picker::{self, COLUMN_WIDTH, Picker, PickerHost};
use super::{Dismiss, NewTabEvent, OpenTab};
use crate::state::AppState;
use crate::theme::{self, metrics};

/// The list shows this many rows before it scrolls.
pub const LIST_ROWS: f32 = 10.;

/// The Ctrl+K overlay.
pub struct SearchOverlay {
    picker: Picker,
    /// Where the focus was when the overlay opened.
    previous_focus: Option<FocusHandle>,
}

impl std::fmt::Debug for SearchOverlay {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchOverlay")
            .field("picker", &self.picker)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<NewTabEvent> for SearchOverlay {}

impl PickerHost for SearchOverlay {
    fn picker(&self) -> &Picker {
        &self.picker
    }

    fn picker_mut(&mut self) -> &mut Picker {
        &mut self.picker
    }
}

impl Focusable for SearchOverlay {
    /// The search field.
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.picker.focus_handle(cx)
    }
}

impl SearchOverlay {
    /// The overlay with the open tabs of every window; it remembers the
    /// focused element and puts the focus in its search field.
    pub fn new(
        state: Entity<AppState>,
        tabs: Vec<OpenTab>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let previous_focus = window.focused(cx);
        let mut picker = Picker::new(
            PickerKind::Overlay,
            "search-overlay",
            state,
            None,
            window,
            cx,
        );
        picker.set_open_tabs(tabs, cx);
        Self {
            picker,
            previous_focus,
        }
    }

    /// The open tabs of every window the search offers.
    pub fn set_open_tabs(&mut self, tabs: Vec<OpenTab>, cx: &mut Context<Self>) {
        if self.picker.set_open_tabs(tabs, cx) {
            cx.notify();
        }
    }

    /// The saved workspaces the search finds (UI 4.5).
    pub fn set_workspaces(
        &mut self,
        workspaces: Vec<crate::state::WorkspaceItem>,
        cx: &mut Context<Self>,
    ) {
        if self.picker.set_workspaces(workspaces, cx) {
            cx.notify();
        }
    }

    /// Esc or a click outside: the focus goes back, then the shell is told.
    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(previous) = self.previous_focus.take() {
            window.focus(&previous, cx);
        }
        cx.emit(NewTabEvent::Dismissed);
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

impl Render for SearchOverlay {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let scrim = theme::tokens(cx).scrim.hsla();
        let surface = theme.popover;
        let border = theme.border;
        let max_list: Pixels = metrics::HEIGHT_ROW * LIST_ROWS;
        let content = self.picker.content_height();
        let height = if content < max_list {
            content
        } else {
            max_list
        };
        let field = self.picker.render_field(None, window, cx);
        let list = self.picker.render_list(Some(height), cx);
        let panel = picker::keys(v_flex().id("search-overlay-panel"), cx)
            .test_support()
            .w_full()
            .max_w(COLUMN_WIDTH)
            .max_h_full()
            .p(metrics::SPACE_2)
            .gap(metrics::SPACE_2)
            .rounded(metrics::RADIUS_OVERLAY)
            .bg(surface)
            .border_1()
            .border_color(border)
            .shadow(theme::shadow_modal(cx))
            .text_color(cx.theme().popover_foreground)
            // A click inside the panel is not a click outside.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_action(cx.listener(|this, _: &Dismiss, window, cx| this.dismiss(window, cx)))
            .child(field)
            .child(list);
        div()
            .id("search-overlay")
            .test_support()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .occlude()
            .bg(scrim)
            .flex()
            .flex_col()
            .items_center()
            .pt(metrics::HEIGHT_TOP_BAR + metrics::SPACE_6)
            .px(metrics::SPACE_4)
            .pb(metrics::SPACE_4)
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|this, _: &MouseDownEvent, window, cx| this.dismiss(window, cx)),
            )
            .child(panel)
    }
}
