//! Toasts (UI 1.5, 2.1; owner's audit item 13): short messages at the bottom
//! center of the main area for 5 seconds, `--bg-overlay` with
//! `--shadow-overlay`, radius 12 (design-system 6), with an optional action
//! button ("Save" after a quick connect). The shell renders the toast host
//! inside the content area (between the sidebar and the details panel), so a
//! toast never covers the details panel or its Connect button.
//!
//! Other modules show toasts through [`crate::shell::show_toast`] (toast
//! only) or [`crate::shell::notify`] (toast + notification center).

use std::rc::Rc;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, ClickEvent, Context, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, Task, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};

use crate::theme::focus::FocusRingExt as _;
use crate::theme::{self, metrics, motion};

/// What a caller asks to show.
#[derive(Clone)]
pub struct Toast {
    message: SharedString,
    action: Option<ToastAction>,
}

/// What a toast action button runs.
type ToastHandler = Rc<dyn Fn(&mut Window, &mut App)>;

#[derive(Clone)]
struct ToastAction {
    id: Option<SharedString>,
    label: SharedString,
    handler: ToastHandler,
}

impl std::fmt::Debug for Toast {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Toast")
            .field("message", &self.message)
            .field("action", &self.action.as_ref().map(|action| &action.label))
            .finish()
    }
}

impl Toast {
    pub fn new(message: impl Into<SharedString>) -> Self {
        Self {
            message: message.into(),
            action: None,
        }
    }

    /// An action button on the toast; clicking it runs `handler` and closes
    /// the toast.
    pub fn action(
        mut self,
        label: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self.action = Some(ToastAction {
            id: None,
            label: label.into(),
            handler: Rc::new(handler),
        });
        self
    }

    /// [`Self::action`] with a stable element id for the button (for
    /// example `toast-save-host`); the default id is `toast-<n>-action`.
    pub fn action_with_id(
        mut self,
        id: impl Into<SharedString>,
        label: impl Into<SharedString>,
        handler: impl Fn(&mut Window, &mut App) + 'static,
    ) -> Self {
        self = self.action(label, handler);
        if let Some(action) = &mut self.action {
            action.id = Some(id.into());
        }
        self
    }

    pub fn message(&self) -> &SharedString {
        &self.message
    }
}

impl From<SharedString> for Toast {
    fn from(message: SharedString) -> Self {
        Self::new(message)
    }
}

impl From<&'static str> for Toast {
    fn from(message: &'static str) -> Self {
        Self::new(message)
    }
}

impl From<String> for Toast {
    fn from(message: String) -> Self {
        Self::new(message)
    }
}

/// A toast on screen.
#[derive(Clone, Debug)]
pub struct ShownToast {
    pub id: u64,
    toast: Toast,
}

impl ShownToast {
    pub fn message(&self) -> &SharedString {
        &self.toast.message
    }

    pub fn action_label(&self) -> Option<&SharedString> {
        self.toast.action.as_ref().map(|action| &action.label)
    }

    /// Stable element ids: the toast, its action button, its close button.
    pub fn element_id(&self) -> SharedString {
        format!("toast-{}", self.id).into()
    }

    pub fn action_element_id(&self) -> SharedString {
        match self
            .toast
            .action
            .as_ref()
            .and_then(|action| action.id.clone())
        {
            Some(id) => id,
            None => format!("toast-{}-action", self.id).into(),
        }
    }

    pub fn close_element_id(&self) -> SharedString {
        format!("toast-{}-close", self.id).into()
    }
}

/// The toast stack of one window.
#[derive(Default)]
pub struct Toasts {
    items: Vec<ShownToast>,
    last_id: u64,
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
    pub fn items(&self) -> &[ShownToast] {
        &self.items
    }

    /// Shows a toast for [`metrics::TOAST_DURATION`].
    pub fn push(&mut self, toast: impl Into<Toast>, cx: &mut Context<Self>) -> u64 {
        self.last_id += 1;
        let id = self.last_id;
        self.items.push(ShownToast {
            id,
            toast: toast.into(),
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

    fn render_toast(&self, shown: &ShownToast, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let surface = theme.popover;
        let id = shown.id;
        let action = shown.toast.action.clone().map(|action| {
            let accent_text = theme.link;
            let hover = theme.accent;
            div()
                .id(shown.action_element_id())
                .test_support()
                .flex_none()
                .px(metrics::SPACE_2)
                .py(metrics::SPACE_1)
                .rounded(metrics::RADIUS_CONTROL)
                .cursor_pointer()
                .focusable()
                .text_color(accent_text)
                .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                .hover(move |style| style.bg(hover))
                .focus_ring_visible(surface, cx)
                .child(action.label.clone())
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    (action.handler)(window, cx);
                    this.dismiss(id, cx);
                }))
        });
        let close = {
            let hover = theme.accent;
            div()
                .id(shown.close_element_id())
                .test_support()
                .flex_none()
                .size(px(24.))
                .flex()
                .items_center()
                .justify_center()
                .rounded(metrics::RADIUS_CONTROL)
                .cursor_pointer()
                .focusable()
                .text_color(theme.muted_foreground)
                .hover(move |style| style.bg(hover))
                .focus_ring_visible(surface, cx)
                .child(Icon::new(IconName::Close).small())
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.dismiss(id, cx)))
        };
        let toast = h_flex()
            .id(shown.element_id())
            .test_support()
            .min_w(px(240.))
            .max_w(px(480.))
            .pl(metrics::SPACE_4)
            .pr(metrics::SPACE_2)
            .py(metrics::SPACE_2)
            .gap(metrics::SPACE_3)
            .rounded(metrics::RADIUS_OVERLAY)
            .bg(surface)
            .shadow(theme::shadow_overlay(cx))
            .text_size(metrics::FONT_SIZE_BODY)
            .text_color(theme.popover_foreground)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .py(metrics::SPACE_1)
                    .child(shown.toast.message.clone()),
            )
            .children(action)
            .child(close);
        let duration = motion::normal(cx);
        if duration.is_zero() {
            toast.into_any_element()
        } else {
            toast
                .with_animation(
                    ("toast-enter", id),
                    Animation::new(duration).with_easing(motion::ease_out),
                    |toast, delta| toast.opacity(delta).mt(px(8. * (1. - delta))),
                )
                .into_any_element()
        }
    }
}

impl Render for Toasts {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let toasts: Vec<AnyElement> = self
            .items
            .clone()
            .iter()
            .map(|shown| self.render_toast(shown, cx))
            .collect();
        v_flex()
            .id("toasts")
            .test_support()
            .items_center()
            .gap(metrics::SPACE_2)
            .children(toasts)
    }
}
