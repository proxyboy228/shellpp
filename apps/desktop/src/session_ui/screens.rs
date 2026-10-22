//! The connection screen of a session tab and its failure state (UI 4.8):
//! the host icon, name and address, the status dot ("connecting" in
//! `--warning`, a failure in `--danger`, design-system 3.3), the steps with
//! the current one marked, and Cancel; after a failed first attempt the
//! reason with Retry and Close. Centered on `--bg-surface`.

use gpui::prelude::*;
use gpui::{AnyElement, App, ClickEvent, Context, FocusHandle, IntoElement, Window, div, px};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, h_flex, v_flex};
use shellpp_core::service::dto::TagColor;

use super::dialog::{self, Kind};
use super::host_key;
use super::model::{ConnectStep, Failure, HostKeyInfo};
use super::strings;
use super::{SessionPane, TabInfo};
use crate::panels::widgets;
use crate::theme::palette::{self, Status};
use crate::theme::{self, metrics};

/// Width of the screen's column.
const COLUMN_WIDTH: gpui::Pixels = px(400.);

/// The status dot (8 px).
fn dot(status: Status, cx: &App) -> impl IntoElement {
    let color = palette::status_color(status, theme::tokens(cx)).hsla();
    div()
        .flex_none()
        .size(metrics::SIZE_STATUS_DOT)
        .rounded_full()
        .bg(color)
}

/// Icon tile, title and address.
fn header(target: &TabInfo, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    v_flex()
        .items_center()
        .gap(metrics::SPACE_2)
        .child(widgets::tile(target.icon(), TagColor::Graphite, px(44.)))
        .child(
            div()
                .id("connecting-title")
                .test_support()
                .max_w_full()
                .truncate()
                .text_size(metrics::FONT_SIZE_HEADING)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(theme.foreground)
                .child(target.title().clone()),
        )
        .children(
            target
                .address()
                .filter(|address| !address.is_empty())
                .map(|address| {
                    div()
                        .id("connecting-address")
                        .test_support()
                        .max_w_full()
                        .truncate()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(theme.muted_foreground)
                        .child(address.clone())
                }),
        )
}

/// The steps; those before `current` are done, `current` runs.
fn steps(target: &TabInfo, current: ConnectStep, cx: &App) -> impl IntoElement {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let steps: Vec<ConnectStep> = if target.is_local() {
        vec![ConnectStep::OpeningSession]
    } else {
        ConnectStep::ALL.to_vec()
    };
    let position = steps.iter().position(|step| *step == current).unwrap_or(0);
    let rows: Vec<AnyElement> = steps
        .iter()
        .enumerate()
        .map(|(index, step)| {
            let label = if target.is_local() {
                strings::STEP_LOCAL
            } else {
                step.label()
            };
            let (marker, color): (AnyElement, _) = if index < position {
                (
                    Icon::new(IconName::Check)
                        .size(px(14.))
                        .text_color(theme.success)
                        .into_any_element(),
                    theme.muted_foreground,
                )
            } else if index == position {
                (
                    dot(Status::Connecting, cx).into_any_element(),
                    theme.foreground,
                )
            } else {
                (
                    div()
                        .size(metrics::SIZE_STATUS_DOT)
                        .rounded_full()
                        .border_1()
                        .border_color(tokens.text_muted.hsla())
                        .into_any_element(),
                    tokens.text_muted.hsla(),
                )
            };
            h_flex()
                .id(gpui::SharedString::from(format!(
                    "connecting-step-{}",
                    step.slug()
                )))
                .test_support()
                .gap(metrics::SPACE_2)
                .items_center()
                .h(px(24.))
                .child(div().w(px(16.)).flex().justify_center().child(marker))
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(color)
                        .when(index == position, |this| {
                            this.font_weight(metrics::FONT_WEIGHT_MEDIUM)
                        })
                        .child(label),
                )
                .into_any_element()
        })
        .collect();
    v_flex()
        .id("connecting-steps")
        .test_support()
        .gap(metrics::SPACE_1)
        .children(rows)
}

/// The centered column on `--bg-surface`.
fn screen(
    id: &'static str,
    focus: FocusHandle,
    column: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .size_full()
        .track_focus(&focus)
        .overflow_y_scroll()
        .bg(cx.theme().colors.list)
        .child(
            div()
                .min_h_full()
                .w_full()
                .flex()
                .items_center()
                .justify_center()
                .p(metrics::SPACE_5)
                .child(
                    v_flex()
                        .w(COLUMN_WIDTH)
                        .max_w_full()
                        .items_center()
                        .gap(metrics::SPACE_5)
                        .child(column),
                ),
        )
}

/// The connection screen while the first attempt runs.
pub fn connecting(
    target: &TabInfo,
    step: ConnectStep,
    focus: FocusHandle,
    cx: &mut Context<SessionPane>,
) -> impl IntoElement {
    let theme = cx.theme();
    let status = h_flex()
        .id("connecting-status")
        .test_support()
        .gap(metrics::SPACE_2)
        .items_center()
        .child(dot(Status::Connecting, cx))
        .child(
            div()
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(theme.foreground)
                .child(strings::STATUS_CONNECTING),
        );
    let cancel = dialog::button(
        "connecting-cancel",
        strings::CANCEL,
        Kind::Neutral,
        true,
        cx,
    )
    .on_click(cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| this.cancel(cx)));
    screen(
        "connecting-screen",
        focus,
        v_flex()
            .w_full()
            .items_center()
            .gap(metrics::SPACE_4)
            .child(header(target, cx))
            .child(status)
            .child(steps(target, step, cx))
            .child(cancel),
        cx,
    )
}

/// The failure state of a first attempt: the reason, Retry and Close.
pub fn failed(
    target: &TabInfo,
    reason: Failure,
    place: Option<gpui::SharedString>,
    revoked: Option<&HostKeyInfo>,
    focus: FocusHandle,
    cx: &mut Context<SessionPane>,
) -> impl IntoElement {
    let theme = cx.theme();
    let status = h_flex()
        .id("connecting-status")
        .test_support()
        .gap(metrics::SPACE_2)
        .items_center()
        .child(dot(Status::Error, cx))
        .child(
            div()
                .text_size(metrics::FONT_SIZE_BODY)
                .font_weight(metrics::FONT_WEIGHT_MEDIUM)
                .text_color(theme.foreground)
                .child(strings::STATUS_FAILED),
        );
    let reason_text = div()
        .id(gpui::SharedString::from(format!(
            "failure-{}",
            reason.code()
        )))
        .test_support()
        .text_size(metrics::FONT_SIZE_BODY)
        .text_color(theme.muted_foreground)
        .text_center()
        .child(reason.text());
    // Where on the route (stage 5, D57): a jump host, the proxy, the chain.
    let place = place.map(|place| {
        div()
            .id("failure-place")
            .test_support()
            .text_size(metrics::FONT_SIZE_BODY)
            .font_weight(metrics::FONT_WEIGHT_MEDIUM)
            .text_color(theme.foreground)
            .text_center()
            .child(place)
    });
    // No common algorithm (D80): what the server offers, what we support,
    // and whether the host's legacy setting would help.
    let algorithms = match reason {
        Failure::NoCommonAlgorithm(mismatch) => {
            let line = |id: &'static str, text: String| {
                div()
                    .id(id)
                    .test_support()
                    .max_w(px(560.))
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(theme.muted_foreground)
                    .text_center()
                    .child(text)
            };
            Some(
                v_flex()
                    .items_center()
                    .gap(metrics::SPACE_1)
                    .child(line(
                        "failure-algorithms-offered",
                        strings::server_offers(&mismatch.offered(), mismatch.others()),
                    ))
                    .child(line(
                        "failure-algorithms-supported",
                        strings::we_support(&mismatch.supported()),
                    ))
                    .when(mismatch.legacy_would_help(), |this| {
                        this.child(
                            div()
                                .id("failure-algorithms-legacy")
                                .test_support()
                                .max_w(px(560.))
                                .text_size(metrics::FONT_SIZE_BODY)
                                .text_color(theme.foreground)
                                .text_center()
                                .child(if place.is_some() {
                                    strings::LEGACY_HINT_HOP
                                } else {
                                    strings::LEGACY_HINT
                                }),
                        )
                    }),
            )
        }
        _ => None,
    };
    // A revoked key cannot continue (UI 10.2), a missing host or an invalid
    // target fails again: no Retry.
    let blocked = !reason.can_retry();
    let buttons = h_flex()
        .gap(metrics::SPACE_2)
        .child(
            dialog::button("failed-close", strings::CLOSE, Kind::Neutral, true, cx)
                .on_click(cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| this.close(cx))),
        )
        .when(!blocked, |this| {
            this.child(
                dialog::button("failed-retry", strings::RETRY, Kind::Accent, true, cx).on_click(
                    cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| this.reconnect(cx)),
                ),
            )
        });
    screen(
        "failed-screen",
        focus,
        v_flex()
            .w_full()
            .items_center()
            .gap(metrics::SPACE_4)
            .child(header(target, cx))
            .child(
                v_flex()
                    .items_center()
                    .gap(metrics::SPACE_2)
                    .child(status)
                    .children(place)
                    .child(reason_text)
                    .children(algorithms),
            )
            .children(revoked.map(|key| host_key::revoked_block(key, cx)))
            .child(buttons),
        cx,
    )
}
