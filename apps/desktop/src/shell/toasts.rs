//! Toasts (UI 1.5, owner's audit item 13): short messages at the bottom of
//! the main area for 5 seconds. The shell places this view inside the main
//! area, so a toast never covers the right panel or its Connect button.
//!
//! TODO(shell): action buttons ("Save host" after a quick connect), the
//! notification center list, `--shadow-overlay`, enter/exit motion.

use gpui::{
    Context, IntoElement, ParentElement as _, Render, SharedString, Styled as _, Task, Window, div,
    prelude::*,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, v_flex};

use crate::theme::metrics;

/// One toast.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Toast {
    pub id: u64,
    pub message: SharedString,
}

/// The toast stack of one main area.
#[derive(Default)]
pub struct Toasts {
    items: Vec<Toast>,
    next_id: u64,
    timers: Vec<(u64, Task<()>)>,
}

impl std::fmt::Debug for Toasts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Toasts")
            .field("items", &self.items.len())
            .finish()
    }
}

impl Toasts {
    pub fn new() -> Self {
        Self::default()
    }

    /// The toasts on screen, oldest first.
    pub fn items(&self) -> &[Toast] {
        &self.items
    }

    /// Shows `message` for [`metrics::TOAST_DURATION`].
    pub fn push(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) -> u64 {
        self.next_id += 1;
        let id = self.next_id;
        self.items.push(Toast {
            id,
            message: message.into(),
        });
        let timer = cx.background_executor().timer(metrics::TOAST_DURATION);
        let task = cx.spawn(async move |this, cx| {
            timer.await;
            let _ = this.update(cx, |toasts, cx| toasts.dismiss(id, cx));
        });
        self.timers.push((id, task));
        cx.notify();
        id
    }

    /// Removes a toast now.
    pub fn dismiss(&mut self, id: u64, cx: &mut Context<Self>) {
        self.items.retain(|toast| toast.id != id);
        self.timers.retain(|(timer_id, _)| *timer_id != id);
        cx.notify();
    }
}

impl Render for Toasts {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .id("toasts")
            .test_support()
            .items_center()
            .gap(metrics::SPACE_2)
            .children(self.items.iter().map(|toast| {
                div()
                    .id(("toast", toast.id))
                    .test_support()
                    .px(metrics::SPACE_4)
                    .py(metrics::SPACE_2)
                    .rounded(metrics::RADIUS_CARD)
                    .bg(theme.popover)
                    .border_1()
                    .border_color(theme.border)
                    .text_color(theme.popover_foreground)
                    .child(toast.message.clone())
            }))
    }
}
