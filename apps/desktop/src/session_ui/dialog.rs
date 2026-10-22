//! The frame of a dialog inside a session tab (prompts of UI 4.8, the host
//! key dialogs of 10.2, the paste confirmation of 4.3): `--scrim` over the
//! tab's area only, so other tabs and the top bar stay usable, and a card
//! of `--bg-overlay`, radius 12, `--shadow-modal`, 24 px padding.
//!
//! Keys: the card has the [`KEY_CONTEXT`] context; Enter dispatches
//! [`Confirm`] and Esc [`Dismiss`], which each dialog maps to its default
//! and cancel actions. A focused text field handles Enter itself (the
//! dialog listens to its `PressEnter`); Esc passes through it.

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Div, ElementId, FocusHandle, IntoElement, KeyBinding, SharedString, Stateful,
    div,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::button::Button;
use gpui_kit::component::{ActiveTheme as _, FocusableExt as _, h_flex, v_flex};

use crate::theme::controls::{ButtonColors as _, Filled};
use crate::theme::focus::FocusRingExt as _;
use crate::theme::{self, metrics};

/// Key context of the dialogs.
pub const KEY_CONTEXT: &str = "SessionDialog";

/// Width of a dialog card (narrowed by a small tab area).
pub const CARD_WIDTH: gpui::Rems = metrics::rpx(460.);

gpui_kit::actions!(
    session_dialog,
    [
        /// Enter: the dialog's default action.
        Confirm,
        /// Esc: the dialog's cancel action.
        Dismiss,
    ]
);

/// Enter and Esc in dialogs.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("enter", Confirm, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", Dismiss, Some(KEY_CONTEXT)),
    ]);
}

/// The scrim with the centered card; `id` names the card. The caller adds
/// `on_action` handlers for [`Confirm`] and [`Dismiss`] to the returned
/// element and its content to the card through `content`.
pub fn frame(
    id: impl Into<ElementId>,
    focus: &FocusHandle,
    content: impl IntoElement,
    cx: &App,
) -> Stateful<Div> {
    let theme = cx.theme();
    let scrim = theme::tokens(cx).scrim.hsla();
    div()
        .id("session-dialog-scrim")
        .absolute()
        .top_0()
        .left_0()
        .size_full()
        .bg(scrim)
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .p(metrics::SPACE_4)
        .key_context(KEY_CONTEXT)
        .track_focus(focus)
        .child(
            v_flex()
                .id(id)
                .test_support()
                .w(CARD_WIDTH)
                .max_w_full()
                .max_h_full()
                .overflow_y_scroll()
                .p(metrics::SPACE_5)
                .gap(metrics::SPACE_4)
                .rounded(metrics::RADIUS_OVERLAY)
                .bg(theme.popover)
                .border_1()
                .border_color(theme.border)
                .shadow(theme::shadow_modal(cx))
                .text_color(theme.popover_foreground)
                .child(content),
        )
}

/// A dialog title: 20 px, 600.
pub fn title(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(metrics::FONT_SIZE_TITLE)
        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
        .text_color(cx.theme().popover_foreground)
        .child(text.into())
}

/// Body text: 14 px, `--text-secondary`.
pub fn body(text: impl Into<SharedString>, cx: &App) -> Div {
    div()
        .text_size(metrics::FONT_SIZE_BODY)
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

/// One labelled value of a host key: caption, then the value (monospace
/// for fingerprints, so similar characters stay distinct).
pub fn key_row(
    id: impl Into<ElementId>,
    label: &'static str,
    value: impl Into<SharedString>,
    mono: bool,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .id(id)
        .test_support()
        .gap(metrics::rpx(2.))
        .child(
            div()
                .text_size(metrics::FONT_SIZE_CAPTION)
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(
            div()
                .text_size(if mono {
                    metrics::FONT_SIZE_DENSE
                } else {
                    metrics::FONT_SIZE_BODY
                })
                .when(mono, |this| {
                    this.font_family(crate::terminal::fonts::JETBRAINS_MONO)
                })
                .text_color(theme.popover_foreground)
                .child(value.into()),
        )
}

/// The button row at the bottom, right-aligned.
pub fn buttons(children: impl IntoIterator<Item = AnyElement>) -> Div {
    h_flex()
        .w_full()
        .flex_wrap()
        .justify_end()
        .gap(metrics::SPACE_2)
        .children(children)
}

/// The kind of a dialog button (design-system 2.4, 2.5, 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The one primary action (`--accent`).
    Accent,
    /// Neutral (`--bg-raised`).
    Neutral,
    /// A dangerous action (`--danger`).
    Danger,
}

/// A dialog button: 36 px, radius 6, the focus ring of design-system 6.
pub fn button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    kind: Kind,
    enabled: bool,
    cx: &App,
) -> Button {
    let surface = cx.theme().popover;
    let button = Button::new(id)
        .label(label.into())
        .h(metrics::HEIGHT_CONTROL)
        .px(metrics::SPACE_4)
        .rounded(metrics::to_px(metrics::RADIUS_CONTROL, cx))
        .focus_ring(false)
        .focus_ring_visible(surface, cx);
    let (button, filled) = match kind {
        Kind::Accent => (button.accent_colors(), Filled::Accent),
        Kind::Neutral => (button.neutral_colors(), Filled::Neutral),
        Kind::Danger => (
            gpui_kit::component::button::ButtonVariants::danger(button),
            Filled::Danger,
        ),
    };
    button.disabled_as_designed(!enabled, filled, cx)
}
