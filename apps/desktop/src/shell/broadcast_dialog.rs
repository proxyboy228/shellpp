//! Broadcast input over ticked tabs (UI 4.4, D86): the open session tabs,
//! each with a check box (the tab whose menu opened it, and those already
//! broadcasting, ticked); Start and Cancel. What is typed in a connected
//! pane of a ticked tab then goes to the connected panes of all of them.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{App, FontWeight, SharedString, Window, div, px};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};

use super::sessions::SessionId;
use super::strings;
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::theme::metrics;

const DIALOG_WIDTH: gpui::Pixels = px(440.);

/// Asks which tabs take part: `tabs` are (tab, title, ticked); `start` gets
/// the ticked tabs and returns whether the dialog may close.
pub fn open(
    tabs: Vec<(SessionId, SharedString, bool)>,
    window: &mut Window,
    cx: &mut App,
    start: impl Fn(Vec<SessionId>, &mut Window, &mut App) -> bool + 'static,
) {
    let ticked = Rc::new(RefCell::new(
        tabs.iter()
            .map(|(_, _, ticked)| *ticked)
            .collect::<Vec<bool>>(),
    ));
    let start = Rc::new(start);
    window.open_dialog(cx, move |dialog, _window, cx| {
        let submit = {
            let ticked = ticked.clone();
            let start = start.clone();
            let ids: Vec<SessionId> = tabs.iter().map(|(id, _, _)| *id).collect();
            move |window: &mut Window, cx: &mut App| {
                let chosen: Vec<SessionId> = ids
                    .iter()
                    .zip(ticked.borrow().iter())
                    .filter(|(_, ticked)| **ticked)
                    .map(|(id, _)| *id)
                    .collect();
                if start(chosen, window, cx) {
                    window.close_dialog(cx);
                }
            }
        };
        let submit = Rc::new(submit);
        let on_ok = submit.clone();
        let list = v_flex().gap(metrics::SPACE_1).children(
            tabs.iter()
                .enumerate()
                .map(|(index, (id, title, _))| {
                    let checked = ticked.borrow()[index];
                    let ticked = ticked.clone();
                    Checkbox::new(SharedString::from(format!("broadcast-tab-{id}")))
                        .label(title.clone())
                        .checked(checked)
                        .on_click(move |checked: &bool, window, _| {
                            ticked.borrow_mut()[index] = *checked;
                            window.refresh();
                        })
                })
                .collect::<Vec<_>>(),
        );
        dialog
            .w(DIALOG_WIDTH)
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(strings::BROADCAST_TITLE),
            )
            .child(
                div()
                    .id("broadcast-dialog")
                    .test_support()
                    .flex()
                    .flex_col()
                    .gap(metrics::SPACE_2)
                    .child(widgets::caption(strings::BROADCAST_HINT, cx))
                    .child(list),
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
                        ControlButton::new("broadcast-cancel", ControlKind::Neutral)
                            .label(strings::BROADCAST_CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("broadcast-start", ControlKind::Accent)
                            .label(strings::BROADCAST_START)
                            .on_click(move |_, window, cx| submit(window, cx)),
                    ),
            )
    });
}
