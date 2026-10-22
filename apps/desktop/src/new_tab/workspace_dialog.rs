//! The name of a new workspace (UI 4.5, 6): one field, Cancel and Create.
//! An empty name is not sent: the field says a name is needed.

use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{App, Entity, FontWeight, Window, div, px};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex};

use super::strings;
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::theme::metrics;

const DIALOG_WIDTH: gpui::Pixels = px(440.);

/// Asks the name; `save` returns whether the dialog may close.
pub fn open(
    name: String,
    window: &mut Window,
    cx: &mut App,
    save: impl Fn(String, &mut Window, &mut App) -> bool + 'static,
) {
    let input: Entity<InputState> = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(name)
            .placeholder(strings::WORKSPACE_NAME_PLACEHOLDER)
    });
    input.update(cx, |input, cx| input.focus(window, cx));
    let save = Rc::new(save);
    let missing = Rc::new(Cell::new(false));
    window.open_dialog(cx, move |dialog, window, cx| {
        let submit = {
            let input = input.clone();
            let save = save.clone();
            let missing = missing.clone();
            move |window: &mut Window, cx: &mut App| {
                let name = input.read(cx).value().to_string();
                if name.trim().is_empty() {
                    missing.set(true);
                    window.refresh();
                    return;
                }
                missing.set(false);
                if save(name, window, cx) {
                    window.close_dialog(cx);
                }
            }
        };
        let submit = Rc::new(submit);
        let on_ok = submit.clone();
        dialog
            .w(DIALOG_WIDTH)
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(strings::CREATE_WORKSPACE),
            )
            .child(
                div()
                    .id("workspace-dialog")
                    .test_support()
                    .flex()
                    .flex_col()
                    .gap(metrics::SPACE_2)
                    .child(widgets::caption(strings::WORKSPACE_HINT, cx))
                    .child(widgets::field(
                        strings::WORKSPACE_NAME,
                        widgets::text_input(&input, "workspace-name", missing.get(), window, cx),
                        None,
                        cx,
                    ))
                    .when(missing.get(), |this| {
                        this.child(
                            div()
                                .id("workspace-name-error")
                                .test_support()
                                .child(widgets::error_text(strings::WORKSPACE_NAME_NEEDED, cx)),
                        )
                    }),
            )
            .on_ok(move |_, window, cx| {
                on_ok(window, cx);
                false
            })
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("workspace-cancel", ControlKind::Neutral)
                            .label(strings::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("workspace-create", ControlKind::Accent)
                            .label(strings::CREATE)
                            .on_click(move |_, window, cx| submit(window, cx)),
                    ),
            )
    });
}
