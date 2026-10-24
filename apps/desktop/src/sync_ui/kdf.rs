//! The confirmation of weaker key derivation parameters (docs/sync-protocol.md,
//! section 4.2; UI 13.5, step 2; D189): a bound, signed-out device whose
//! server answers pre-login with weaker parameters than the ones it holds
//! stops before anything is derived or sent, and this dialog of its own
//! says why. Cancel leaves the device signed out; "Sign in with weaker
//! settings" (a danger button; Enter does not press it) signs in with
//! exactly the parameters shown.

use gpui::{
    App, AppContext as _, Entity, InteractiveElement as _, ParentElement as _, Styled as _,
};
use gpui::{IntoElement, Render, Window, div};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _, h_flex, v_flex};
use shellpp_core::crypto::KdfParams;
use shellpp_core::service::WeakerKdf;

use super::{ConnectDialog, label, strings, title};
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::theme::metrics;

/// "64 MiB, 3 passes, 4 lanes".
pub fn describe(kdf: &KdfParams) -> String {
    let memory = if kdf.m_kib.is_multiple_of(1024) {
        format!("{} MiB", kdf.m_kib / 1024)
    } else {
        format!("{} KiB", kdf.m_kib)
    };
    format!("{memory}, {} passes, {} lanes", kdf.t, kdf.p)
}

pub struct WeakerKdfView {
    pub offer: WeakerKdf,
}

impl std::fmt::Debug for WeakerKdfView {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WeakerKdfView").finish_non_exhaustive()
    }
}

impl Render for WeakerKdfView {
    fn render(&mut self, _: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let row = |id: &'static str, name: &'static str, kdf: &KdfParams, cx: &App| {
            v_flex()
                .id(id)
                .test_support()
                .gap(metrics::SPACE_1)
                .child(widgets::caption(name, cx))
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(cx.theme().popover_foreground)
                        .child(describe(kdf)),
                )
        };
        v_flex()
            .id("sync-weaker-kdf")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(label(strings::KDF_WEAKER_HEADING, cx))
            .child(widgets::caption(strings::KDF_WEAKER_TEXT, cx))
            .child(
                h_flex()
                    .gap(metrics::SPACE_4)
                    .items_start()
                    .child(div().flex_1().child(row(
                        "sync-kdf-local",
                        strings::KDF_THIS_DEVICE,
                        &self.offer.local,
                        cx,
                    )))
                    .child(div().flex_1().child(row(
                        "sync-kdf-server",
                        strings::KDF_FROM_SERVER,
                        &self.offer.server,
                        cx,
                    ))),
            )
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(cx.theme().warning)
                    .child(strings::KDF_WEAKER_WARNING),
            )
    }
}

/// Opens the confirmation over the wizard; the confirm button signs in
/// through `wizard` with exactly the server's parameters.
pub fn open_weaker_kdf(
    wizard: Entity<ConnectDialog>,
    offer: WeakerKdf,
    window: &mut Window,
    cx: &mut App,
) -> Entity<WeakerKdfView> {
    let view = cx.new(|_| WeakerKdfView { offer });
    let shown = view.clone();
    window.open_dialog(cx, move |modal, _, cx| {
        let accepted = shown.read(cx).offer.server.clone();
        let wizard = wizard.downgrade();
        let footer = h_flex()
            .w_full()
            .justify_end()
            .gap(metrics::SPACE_2)
            .child(
                ControlButton::new("sync-kdf-cancel", ControlKind::Neutral)
                    .label(strings::CANCEL)
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .child(
                ControlButton::new("sync-kdf-continue", ControlKind::Danger)
                    .label(strings::KDF_CONTINUE)
                    .on_click(move |_, window, cx| {
                        window.close_dialog(cx);
                        let accepted = accepted.clone();
                        let _ = wizard.update(cx, |wizard, cx| {
                            wizard.run_accepting(Some(accepted), window, cx);
                        });
                    }),
            );
        modal
            .w(metrics::scaled(super::DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(strings::KDF_WEAKER_TITLE))
            .child(shown.clone())
            .footer(footer)
    });
    view
}
