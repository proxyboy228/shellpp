//! Modal dialogs of the Hosts screen: the removal confirmation (UI 2.4) and
//! the "In development" placeholders for dialogs of later stages (Import,
//! 2.5; Move or copy to a vault, 2.6). They use gpui-kit's dialog layer
//! (scrim = `--scrim`) with our surface: `--bg-overlay`, radius 12, a 20 px
//! title and buttons per design-system 6.

use std::rc::Rc;

use gpui::{
    App, FontWeight, InteractiveElement as _, IntoElement, ParentElement as _, SharedString,
    Styled as _, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};

use super::controls::{ControlButton, ControlKind};
use super::strings;
use crate::theme::metrics;

/// Width of the small dialogs.
const DIALOG_WIDTH: f32 = 420.;

fn title(text: SharedString) -> impl IntoElement {
    div()
        .text_size(metrics::FONT_SIZE_TITLE)
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

fn body(id: &'static str, text: SharedString, cx: &App) -> impl IntoElement {
    v_flex()
        .id(id)
        .test_support()
        .text_size(metrics::FONT_SIZE_BODY)
        .text_color(cx.theme().muted_foreground)
        .child(text)
}

/// "Remove …?" with Cancel and Remove (`--danger`). `on_confirm` runs on
/// Remove or Enter; Esc and Cancel close without removing.
pub(super) fn open_remove_confirmation(
    heading: String,
    text: String,
    window: &mut Window,
    cx: &mut App,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
) {
    let on_confirm = Rc::new(on_confirm);
    let heading: SharedString = heading.into();
    let text: SharedString = text.into();
    window.open_dialog(cx, move |dialog, _, cx| {
        let confirm = on_confirm.clone();
        let on_ok = on_confirm.clone();
        dialog
            .w(px(DIALOG_WIDTH))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(heading.clone()))
            .child(body("remove-dialog", text.clone(), cx))
            .on_ok(move |_, window, cx| {
                on_ok(window, cx);
                true
            })
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("remove-cancel", ControlKind::Neutral)
                            .label(strings::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("remove-confirm", ControlKind::Danger)
                            .label(strings::MENU_REMOVE)
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                confirm(window, cx);
                            }),
                    ),
            )
    });
}

/// A placeholder dialog: the title of the real dialog and "In development".
pub(super) fn open_placeholder(
    id: &'static str,
    heading: &'static str,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_dialog(cx, move |dialog, _, cx| {
        dialog
            .w(px(DIALOG_WIDTH))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(heading.into()))
            .child(body(id, strings::IN_DEVELOPMENT.into(), cx))
            .footer(
                h_flex().w_full().justify_end().child(
                    ControlButton::new("placeholder-close", ControlKind::Neutral)
                        .label(strings::CLOSE)
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                ),
            )
    });
}
