//! Lock screens (UI 13.1-13.3, decision D29): the first run (master
//! password, strength indicator, "Remember on this device"), the unlock
//! screen (attempt limit with a countdown, "Forgot password?") and the
//! "Start with a new vault" flow.
//!
//! Both screens are a centered card of [`CARD_WIDTH`] on `--bg-base` under
//! the bare title bar: `--bg-raised`, `--border-subtle`, radius 12, padding
//! 24 / 16 (sides / top and bottom), so the first-run card fits the
//! smallest window (960 x 600). The card scrolls when it is taller than the
//! window (an extra message under a field at the smallest size).
//!
//! Passwords travel as `SecretString` to the core. The text field itself
//! (gpui-kit `InputState`) keeps its text in a plain rope until it is cleared,
//! which these screens do right after reading it. They read that rope in
//! place (`InputState::text`: lengths, comparison) and copy it only into the
//! `SecretString` for the core ([`secret_text`]) and into a buffer that is
//! wiped on drop for the strength estimate ([`strength_of`]); never through
//! `InputState::value`, which makes a new string on every call that is freed
//! without being wiped. No password is logged, and the views' `Debug`
//! output shows no field contents.

mod first_run;
mod forgot;
pub mod strength;
pub mod strings;
mod unlock;

#[cfg(test)]
mod tests;

pub use first_run::FirstRunScreen;
pub use forgot::{ForgotDialog, ForgotEvent, ForgotStep};
pub use strength::Strength;
pub use unlock::UnlockScreen;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Entity, FontWeight, IntoElement, Pixels, SharedString, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use secrecy::SecretString;
use secrecy::zeroize::Zeroizing;

use crate::panels::widgets;
use crate::shell::title_bar::bare_title_bar;
use crate::theme::{self, metrics};

/// Width of the lock card (UI 13.1).
pub const CARD_WIDTH: Pixels = px(420.);

/// The text of a password field as a secret, copied straight from the
/// field's rope into the secret's own buffer (exact capacity, so turning it
/// into the boxed secret does not reallocate and leave a copy behind).
pub(crate) fn secret_text(input: &InputState) -> SecretString {
    let rope = input.text();
    let mut text = String::with_capacity(rope.len());
    for chunk in rope.chunks() {
        text.push_str(chunk);
    }
    SecretString::from(text)
}

/// The strength of a password field's text, estimated from a copy that is
/// wiped when dropped (`None` while empty).
pub(crate) fn strength_of(input: &InputState) -> Option<Strength> {
    let rope = input.text();
    let mut text = Zeroizing::new(String::with_capacity(rope.len()));
    for chunk in rope.chunks() {
        text.push_str(chunk);
    }
    Strength::of(&text)
}

/// The frame of a lock screen: the bare title bar (window buttons only) and
/// the centered card. `overlay` (the "Forgot password?" dialog) covers the
/// whole area under the title bar.
pub(crate) fn frame(
    id: &'static str,
    content: impl IntoElement,
    overlay: Option<AnyElement>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .id(id)
        .test_support()
        .size_full()
        .bg(theme.background)
        .child(bare_title_bar(cx))
        .child(
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(
                    div()
                        .id("lock-scroll")
                        .size_full()
                        .overflow_y_scroll()
                        .child(
                            div()
                                .min_h_full()
                                .w_full()
                                .flex()
                                .items_center()
                                .justify_center()
                                .p(metrics::SPACE_2)
                                .child(
                                    v_flex()
                                        .id("lock-card")
                                        .test_support()
                                        .w(CARD_WIDTH)
                                        .flex_none()
                                        .gap(metrics::SPACE_4)
                                        .px(metrics::SPACE_5)
                                        .py(metrics::SPACE_4)
                                        .rounded(metrics::RADIUS_OVERLAY)
                                        .bg(theme.secondary)
                                        .border_1()
                                        .border_color(theme.border)
                                        .child(content),
                                ),
                        ),
                )
                .children(overlay),
        )
}

/// The card heading: the app name (20 px, 600) and one line under it
/// (14 px, `--text-secondary`).
pub(crate) fn heading(subtitle: &'static str, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .gap(metrics::SPACE_1)
        .child(
            div()
                .text_size(metrics::FONT_SIZE_TITLE)
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.foreground)
                .child(strings::APP_NAME),
        )
        .child(
            div()
                .id("lock-subtitle")
                .test_support()
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(theme.muted_foreground)
                .child(subtitle),
        )
}

/// A labelled password field: caption, then the masked input with its
/// show/hide button (36 px, `--bg-input`, one focus ring). The caller adds
/// hints or the error under it (12 px).
pub(crate) fn password_field(
    label: &'static str,
    state: &Entity<InputState>,
    id: &'static str,
    invalid: bool,
    disabled: bool,
    window: &Window,
    cx: &App,
) -> gpui::Div {
    let input = Input::new(state).mask_toggle().disabled(disabled);
    v_flex()
        .w_full()
        .gap(metrics::SPACE_1)
        .child(widgets::caption(label, cx))
        .child(widgets::text_input_with(
            input, state, id, invalid, window, cx,
        ))
}

/// A message under a field: 12 px `--danger`.
pub(crate) fn error_line(id: &'static str, text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .text_size(metrics::FONT_SIZE_CAPTION)
        .text_color(cx.theme().danger)
        .child(text.into())
        .into_any_element()
}

/// A hint under a field: 12 px `--text-secondary`.
pub(crate) fn hint_line(id: &'static str, text: impl Into<SharedString>, cx: &App) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .text_size(metrics::FONT_SIZE_CAPTION)
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
        .into_any_element()
}

/// The kind of a notice block (design-system 2.5: a `*-subtle` background,
/// the semantic color on the icon only, `--text-primary` text).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NoticeKind {
    Warning,
    Info,
}

/// A notice block: icon and text on a `*-subtle` background, radius 6.
pub(crate) fn notice(
    id: &'static str,
    kind: NoticeKind,
    text: impl Into<SharedString>,
    cx: &App,
) -> impl IntoElement {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let (icon, icon_color, background) = match kind {
        NoticeKind::Warning => (
            IconName::TriangleAlert,
            theme.warning,
            tokens.warning_subtle.hsla(),
        ),
        NoticeKind::Info => (IconName::Info, theme.info, tokens.info_subtle.hsla()),
    };
    h_flex()
        .id(id)
        .test_support()
        .w_full()
        .items_center()
        .gap(metrics::SPACE_2)
        .px(metrics::SPACE_3)
        .py(metrics::SPACE_2)
        .rounded(metrics::RADIUS_CONTROL)
        .bg(background)
        .child(
            div()
                .flex_none()
                .child(Icon::new(icon).small().text_color(icon_color)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(metrics::FONT_SIZE_DENSE)
                .text_color(theme.foreground)
                .child(text.into()),
        )
}
