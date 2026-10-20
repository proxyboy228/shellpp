//! The multi-line paste confirmation (UI 4.3; SPEC.md): the number of lines,
//! a preview of the first lines in the terminal font (the text is already
//! cleaned by the terminal: no control characters or bracketed paste
//! markers), "Paste" (Enter, the default) and "Cancel" (Esc).

use gpui::prelude::*;
use gpui::{AnyElement, ClickEvent, Context, FocusHandle, IntoElement, SharedString, Window, div};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, v_flex};

use super::SessionPane;
use super::dialog::{self, Confirm, Dismiss, Kind};
use super::strings;
use crate::terminal::PendingPaste;
use crate::terminal::fonts::JETBRAINS_MONO;
use crate::theme::{self, metrics};

/// The dialog for `paste`.
pub fn render(
    paste: &PendingPaste,
    focus: &FocusHandle,
    cx: &mut Context<SessionPane>,
) -> impl IntoElement {
    let theme = cx.theme();
    let preview = paste.preview();
    let lines: Vec<AnyElement> = preview
        .lines
        .iter()
        .map(|line| {
            div()
                .truncate()
                .child(SharedString::from(if line.is_empty() {
                    " ".to_owned()
                } else {
                    line.clone()
                }))
                .into_any_element()
        })
        .collect();
    let more = preview.more;
    let block = v_flex()
        .id("paste-preview")
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
        .children(lines)
        .when(more > 0, |this| {
            this.child(
                div()
                    .id("paste-more")
                    .test_support()
                    .text_color(theme.muted_foreground)
                    .child(strings::paste_more(more)),
            )
        });
    let content = v_flex()
        .gap(metrics::SPACE_4)
        .child(
            v_flex()
                .gap(metrics::SPACE_2)
                .child(dialog::title(strings::paste_title(preview.line_count), cx))
                .child(
                    dialog::body(strings::paste_lines(preview.line_count), cx)
                        .id("paste-line-count")
                        .test_support(),
                ),
        )
        .child(block)
        .child(dialog::buttons([
            dialog::button("paste-cancel", strings::CANCEL, Kind::Neutral, true, cx)
                .on_click(
                    cx.listener(|this, _: &ClickEvent, _: &mut Window, cx| this.cancel_paste(cx)),
                )
                .into_any_element(),
            dialog::button("paste-confirm", strings::PASTE, Kind::Accent, true, cx)
                .on_click(
                    cx.listener(|this, _: &ClickEvent, window, cx| this.accept_paste(window, cx)),
                )
                .into_any_element(),
        ]));
    dialog::frame("paste-dialog", focus, content, cx)
        .on_action(cx.listener(|this, _: &Confirm, window, cx| this.accept_paste(window, cx)))
        .on_action(cx.listener(|this, _: &Dismiss, _, cx| this.cancel_paste(cx)))
}
