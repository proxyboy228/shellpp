//! The signature question of Agent Forwarding (D60, "Ask before every
//! signature"): the key's SHA256 fingerprint, "Deny" and "Allow". Enter and
//! Esc deny: only a click (or the focused Allow button) signs.

use gpui::prelude::*;
use gpui::{ClickEvent, Context, FocusHandle, IntoElement, SharedString, Window, div};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, v_flex};

use super::SessionPane;
use super::dialog::{self, Confirm, Dismiss, Kind};
use super::strings;
use crate::terminal::fonts::JETBRAINS_MONO;
use crate::theme::{self, metrics};

/// The dialog for a signature with the key of `fingerprint`.
pub fn render(
    fingerprint: String,
    focus: &FocusHandle,
    cx: &mut Context<SessionPane>,
) -> impl IntoElement {
    let theme = cx.theme();
    let key = div()
        .id("signature-fingerprint")
        .test_support()
        .w_full()
        .p(metrics::SPACE_3)
        .rounded(metrics::RADIUS_CONTROL)
        .bg(theme::tokens(cx).bg_input.hsla())
        .border_1()
        .border_color(theme.input)
        .font_family(JETBRAINS_MONO)
        .text_size(metrics::FONT_SIZE_DENSE)
        .text_color(theme.popover_foreground)
        .child(SharedString::from(fingerprint));
    let content = v_flex()
        .gap(metrics::SPACE_4)
        .child(
            v_flex()
                .gap(metrics::SPACE_2)
                .child(dialog::title(strings::SIGN_TITLE, cx))
                .child(dialog::body(strings::SIGN_BODY, cx)),
        )
        .child(key)
        .child(dialog::buttons([
            dialog::button(
                "signature-deny",
                strings::SIGN_DENY,
                Kind::Neutral,
                true,
                cx,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| {
                this.answer_signature(false, cx)
            }))
            .into_any_element(),
            dialog::button(
                "signature-allow",
                strings::SIGN_ALLOW,
                Kind::Accent,
                true,
                cx,
            )
            .on_click(cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| {
                this.answer_signature(true, cx)
            }))
            .into_any_element(),
        ]));
    dialog::frame("signature-dialog", focus, content, cx)
        .on_action(cx.listener(|this, _: &Confirm, _, cx| this.answer_signature(false, cx)))
        .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.answer_signature(false, cx)))
}
