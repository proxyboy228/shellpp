//! Messages over the kept terminal output (UI 4.6):
//!
//! - **ended** (non-zero exit status or a signal): "Session ended with code
//!   N", Reconnect and Close; no automatic attempts;
//! - **dropped** (a transport error, a keepalive timeout, a channel closed
//!   without an exit status): "Connection lost", the reason, the countdown
//!   to the next automatic attempt (or "after unlock") with Cancel, and
//!   Reconnect (now) and Close;
//! - **reconnecting**: the attempt and its current step, Cancel and Close.
//!
//! A card of `--bg-overlay` with `--shadow-overlay`, centered over the
//! terminal. Only the card takes the mouse: the output around it can still
//! be scrolled and selected.

use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, Context, IntoElement, SharedString, Window, div, px};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, h_flex, v_flex};

use super::SessionPane;
use super::dialog::{self, Kind};
use super::model::Phase;
use super::strings;
use crate::theme::palette::{self, Status};
use crate::theme::{self, metrics};

/// Width of the message card.
const CARD_WIDTH: gpui::Pixels = px(420.);

/// The overlay of the current phase, if it has one.
pub fn render(view: &SessionPane, cx: &mut Context<SessionPane>) -> Option<AnyElement> {
    let card = match view.phase() {
        Phase::Ended(status) => ended(status.text(), cx),
        Phase::Dropped(reason) => dropped(view, reason.text(), reason.code(), cx),
        Phase::Reconnecting(step) => reconnecting(view, step.label(), cx),
        _ => return None,
    };
    Some(
        div()
            .absolute()
            .top_0()
            .left_0()
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .p(metrics::SPACE_4)
            .child(card)
            .into_any_element(),
    )
}

/// The card frame with a status dot and a title.
fn card(
    id: &'static str,
    status: Status,
    title: impl Into<SharedString>,
    cx: &Context<SessionPane>,
) -> crate::panels::widgets::Hit {
    let theme = cx.theme();
    let dot = palette::status_color(status, theme::tokens(cx)).hsla();
    v_flex()
        .id(id)
        .test_support()
        .occlude()
        .w(CARD_WIDTH)
        .max_w_full()
        .p(metrics::SPACE_4)
        .gap(metrics::SPACE_3)
        .rounded(metrics::RADIUS_OVERLAY)
        .bg(theme.popover)
        .border_1()
        .border_color(theme.border)
        .shadow(theme::shadow_overlay(cx))
        .text_color(theme.popover_foreground)
        .child(
            h_flex()
                .gap(metrics::SPACE_2)
                .items_center()
                .child(
                    div()
                        .flex_none()
                        .size(metrics::SIZE_STATUS_DOT)
                        .rounded_full()
                        .bg(dot),
                )
                .child(
                    div()
                        .id(SharedString::from(format!("{id}-message")))
                        .test_support()
                        .text_size(metrics::FONT_SIZE_HEADING)
                        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                        .child(title.into()),
                ),
        )
}

fn line(
    id: &'static str,
    text: impl Into<SharedString>,
    cx: &Context<SessionPane>,
) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .text_size(metrics::FONT_SIZE_BODY)
        .text_color(cx.theme().muted_foreground)
        .child(text.into())
}

fn close_button(id: &'static str, cx: &mut Context<SessionPane>) -> AnyElement {
    dialog::button(id, strings::CLOSE, Kind::Neutral, true, cx)
        .on_click(cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| this.close(cx)))
        .into_any_element()
}

fn ended(message: String, cx: &mut Context<SessionPane>) -> AnyElement {
    card("session-ended", Status::Inactive, message, cx)
        .child(dialog::buttons([
            close_button("session-ended-close", cx),
            dialog::button(
                "session-ended-reconnect",
                strings::RECONNECT,
                Kind::Accent,
                true,
                cx,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| this.reconnect(cx)))
            .into_any_element(),
        ]))
        .into_any_element()
}

fn dropped(
    view: &SessionPane,
    reason: &'static str,
    code: &'static str,
    cx: &mut Context<SessionPane>,
) -> AnyElement {
    let countdown = view.reconnect_countdown();
    let waiting = view
        .reconnect_info()
        .is_some_and(|info| info.waiting_for_unlock);
    let scheduled = countdown.is_some() || waiting;
    let detail = if waiting {
        Some(strings::WAITING_FOR_UNLOCK.to_owned())
    } else {
        countdown.map(strings::reconnect_in)
    };
    let mut buttons = Vec::new();
    if scheduled {
        buttons.push(
            dialog::button(
                "session-dropped-cancel",
                strings::CANCEL,
                Kind::Neutral,
                true,
                cx,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| this.cancel(cx)))
            .into_any_element(),
        );
    }
    buttons.push(close_button("session-dropped-close", cx));
    buttons.push(
        dialog::button(
            "session-dropped-reconnect",
            if scheduled {
                strings::RECONNECT_NOW
            } else {
                strings::RECONNECT
            },
            Kind::Accent,
            true,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| this.reconnect(cx)))
        .into_any_element(),
    );
    card(
        "session-dropped",
        Status::Error,
        strings::CONNECTION_LOST,
        cx,
    )
    .child(
        div()
            .id(SharedString::from(format!("session-dropped-reason-{code}")))
            .test_support()
            .text_size(metrics::FONT_SIZE_BODY)
            .text_color(cx.theme().popover_foreground)
            .child(reason),
    )
    .children(detail.map(|text| line("session-dropped-countdown", text, cx)))
    .child(dialog::buttons(buttons))
    .into_any_element()
}

fn reconnecting(
    view: &SessionPane,
    step: &'static str,
    cx: &mut Context<SessionPane>,
) -> AnyElement {
    let attempt = view.reconnect_info().map_or(0, |info| info.attempt);
    let title = strings::reconnecting(attempt);
    card("session-reconnecting", Status::Connecting, title, cx)
        .child(line("session-reconnecting-step", step, cx))
        .child(dialog::buttons([
            dialog::button(
                "session-reconnecting-cancel",
                strings::CANCEL,
                Kind::Neutral,
                true,
                cx,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| this.cancel(cx)))
            .into_any_element(),
            close_button("session-reconnecting-close", cx),
        ]))
        .into_any_element()
}
