//! The host key dialogs (UI 10.2), inside the tab:
//!
//! - **first connection** (TOFU): address, key type and SHA256 fingerprint;
//!   "Accept and save" (the default, Enter), "Connect once", "Cancel" (Esc);
//! - **changed key**: the connection is blocked; a warning on
//!   `--danger-subtle` with the danger icon explains a possible attack; the
//!   saved and the presented fingerprints; only the explicit "Replace saved
//!   key" (a danger button) continues. Enter and Esc cancel; there is no
//!   "don't ask again";
//! - **revoked key**: not a question: the attempt fails and the failure
//!   screen shows the danger warning with the key ([`revoked_block`]).

use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, Context, FocusHandle, IntoElement, SharedString, Window, div};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};

use super::SessionPane;
use super::dialog::{self, Confirm, Dismiss, Kind};
use super::model::{HostKeyAsk, HostKeyInfo, HostKeyReply};
use super::strings;
use crate::theme::{self, metrics};

/// The dialog for `ask`.
pub fn render(
    ask: &HostKeyAsk,
    focus: &FocusHandle,
    cx: &mut Context<SessionPane>,
) -> impl IntoElement {
    match ask {
        HostKeyAsk::Unknown(key) => unknown(key, focus, cx),
        HostKeyAsk::Changed {
            key,
            old_key_type,
            old_fingerprint,
        } => changed(key, old_key_type, old_fingerprint, focus, cx),
    }
}

fn reply(
    reply: HostKeyReply,
) -> impl Fn(&mut SessionPane, &ClickEvent, &mut Window, &mut Context<SessionPane>) + 'static {
    move |this, _, _, cx| this.answer_host_key(reply, cx)
}

fn key_rows(key: &HostKeyInfo, cx: &Context<SessionPane>) -> impl IntoElement {
    v_flex()
        .gap(metrics::SPACE_3)
        .child(dialog::key_row(
            "host-key-address",
            strings::HOST_KEY_ADDRESS,
            key.address.clone(),
            false,
            cx,
        ))
        .child(dialog::key_row(
            "host-key-type",
            strings::HOST_KEY_TYPE,
            key.key_type.clone(),
            false,
            cx,
        ))
        .child(dialog::key_row(
            "host-key-fingerprint",
            strings::HOST_KEY_FINGERPRINT,
            key.fingerprint.clone(),
            true,
            cx,
        ))
}

/// The blocking warning: danger icon and `--text-primary` text on
/// `--danger-subtle` (design-system 2.5).
fn warning(id: &'static str, text: String, cx: &Context<SessionPane>) -> impl IntoElement {
    let theme = cx.theme();
    h_flex()
        .id(id)
        .test_support()
        .w_full()
        .items_start()
        .gap(metrics::SPACE_2)
        .px(metrics::SPACE_3)
        .py(metrics::SPACE_3)
        .rounded(metrics::RADIUS_CONTROL)
        .bg(theme::tokens(cx).danger_subtle.hsla())
        .child(
            div().flex_none().pt(gpui::px(1.)).child(
                Icon::new(IconName::TriangleAlert)
                    .small()
                    .text_color(theme.danger),
            ),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(theme.foreground)
                .child(text),
        )
}

fn unknown(key: &HostKeyInfo, focus: &FocusHandle, cx: &mut Context<SessionPane>) -> AnyElement {
    let content = v_flex()
        .gap(metrics::SPACE_4)
        .child(
            v_flex()
                .gap(metrics::SPACE_2)
                .child(dialog::title(strings::UNKNOWN_TITLE, cx))
                .child(dialog::body(strings::unknown_text(&key.address), cx)),
        )
        .child(key_rows(key, cx))
        .child(dialog::buttons([
            dialog::button("host-key-cancel", strings::CANCEL, Kind::Neutral, true, cx)
                .on_click(cx.listener(reply(HostKeyReply::Cancel)))
                .into_any_element(),
            dialog::button(
                "host-key-once",
                strings::CONNECT_ONCE,
                Kind::Neutral,
                true,
                cx,
            )
            .on_click(cx.listener(reply(HostKeyReply::ConnectOnce)))
            .into_any_element(),
            dialog::button(
                "host-key-accept",
                strings::ACCEPT_AND_SAVE,
                Kind::Accent,
                true,
                cx,
            )
            .on_click(cx.listener(reply(HostKeyReply::AcceptAndSave)))
            .into_any_element(),
        ]));
    dialog::frame("host-key-unknown", focus, content, cx)
        .on_action(cx.listener(|this, _: &Confirm, _, cx| {
            this.answer_host_key(HostKeyReply::AcceptAndSave, cx)
        }))
        .on_action(
            cx.listener(|this, _: &Dismiss, _, cx| this.answer_host_key(HostKeyReply::Cancel, cx)),
        )
        .into_any_element()
}

fn changed(
    key: &HostKeyInfo,
    old_key_type: &str,
    old_fingerprint: &str,
    focus: &FocusHandle,
    cx: &mut Context<SessionPane>,
) -> AnyElement {
    let old = if old_key_type == key.key_type {
        SharedString::from(old_fingerprint.to_owned())
    } else {
        SharedString::from(format!("{old_fingerprint} ({old_key_type})"))
    };
    let new = if old_key_type == key.key_type {
        SharedString::from(key.fingerprint.clone())
    } else {
        SharedString::from(format!("{} ({})", key.fingerprint, key.key_type))
    };
    let content = v_flex()
        .gap(metrics::SPACE_4)
        .child(dialog::title(strings::CHANGED_TITLE, cx).text_color(cx.theme().danger))
        .child(warning(
            "host-key-changed-warning",
            strings::changed_text(&key.address),
            cx,
        ))
        .child(
            v_flex()
                .gap(metrics::SPACE_3)
                .child(dialog::key_row(
                    "host-key-address",
                    strings::HOST_KEY_ADDRESS,
                    key.address.clone(),
                    false,
                    cx,
                ))
                .child(dialog::key_row(
                    "host-key-old",
                    strings::CHANGED_OLD,
                    old,
                    true,
                    cx,
                ))
                .child(dialog::key_row(
                    "host-key-new",
                    strings::CHANGED_NEW,
                    new,
                    true,
                    cx,
                )),
        )
        .child(dialog::body(strings::CHANGED_HINT, cx))
        .child(dialog::buttons([
            dialog::button("host-key-cancel", strings::CANCEL, Kind::Neutral, true, cx)
                .on_click(cx.listener(reply(HostKeyReply::Cancel)))
                .into_any_element(),
            dialog::button(
                "host-key-replace",
                strings::REPLACE_KEY,
                Kind::Danger,
                true,
                cx,
            )
            .on_click(cx.listener(reply(HostKeyReply::Replace)))
            .into_any_element(),
        ]));
    // Enter never replaces the key: both keys cancel.
    dialog::frame("host-key-changed", focus, content, cx)
        .on_action(
            cx.listener(|this, _: &Confirm, _, cx| this.answer_host_key(HostKeyReply::Cancel, cx)),
        )
        .on_action(
            cx.listener(|this, _: &Dismiss, _, cx| this.answer_host_key(HostKeyReply::Cancel, cx)),
        )
        .into_any_element()
}

/// The revoked-key warning of the failure screen (UI 10.2): the danger
/// warning and the presented key; there is no way to continue.
pub fn revoked_block(key: &HostKeyInfo, cx: &Context<SessionPane>) -> impl IntoElement {
    v_flex()
        .id("host-key-revoked")
        .test_support()
        .w_full()
        .gap(metrics::SPACE_3)
        .child(warning(
            "host-key-revoked-warning",
            strings::revoked_text(&key.address),
            cx,
        ))
        .child(key_rows(key, cx))
        .child(dialog::body(strings::REVOKED_HINT, cx))
}
