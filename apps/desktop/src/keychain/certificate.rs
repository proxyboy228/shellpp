//! Certificate (UI 7.1; data-model 9.2; stage 4, item 9): an OpenSSH user
//! certificate (`*-cert.pub`), pasted or chosen, goes onto the key of the
//! Keychain whose public key it certifies; the key's panel opens and shows
//! it (principals, validity, the CA, a warning when it is expired).

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Entity, FontWeight, IntoElement, PathPromptOptions, Window, div,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::input::{InputEvent, Textarea, TextareaState};
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use shellpp_core::{CoreError, ValidationReason};

use super::strings;
use super::{KeychainEvent, KeychainScreen};
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::state::AppState;
use crate::theme::metrics;

/// The largest certificate file read (a certificate is a few kilobytes).
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// The Add a certificate dialog.
pub struct CertificateForm {
    state: Entity<AppState>,
    screen: gpui::WeakEntity<KeychainScreen>,
    text: Entity<TextareaState>,
    error: Option<&'static str>,
    _subscription: gpui::Subscription,
}

impl std::fmt::Debug for CertificateForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CertificateForm")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

/// The message of a refused certificate.
pub fn error_text(error: &CoreError) -> &'static str {
    match error {
        CoreError::Validation(fields)
            if fields
                .iter()
                .any(|field| field.reason == ValidationReason::MissingRef) =>
        {
            strings::CERTIFICATE_NO_KEY
        }
        _ => strings::CERTIFICATE_INVALID,
    }
}

impl CertificateForm {
    pub fn new(
        state: Entity<AppState>,
        screen: gpui::WeakEntity<KeychainScreen>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let text = cx.new(|cx| {
            TextareaState::new(window, cx).placeholder("ssh-ed25519-cert-v01@openssh.com AAAA…")
        });
        let subscription = cx.subscribe(&text, |this: &mut Self, _, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.error = None;
                cx.notify();
            }
        });
        text.update(cx, |input, cx| input.focus(window, cx));
        Self {
            state,
            screen,
            text,
            error: None,
            _subscription: subscription,
        }
    }

    pub fn set_text(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        let text = text.to_owned();
        self.text
            .update(cx, |input, cx| input.set_value(text, window, cx));
        cx.notify();
    }

    /// Add: stores the certificate and shows its key.
    pub fn add(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let text = self.text.read(cx).value().to_string();
        match self
            .state
            .update(cx, |state, cx| state.import_certificate(&text, cx))
        {
            Ok(view) => {
                let key_id = view.record_id;
                let _ = self.screen.update(cx, |screen, cx| {
                    screen.on_panel_event(
                        &KeychainEvent::Saved {
                            key_id,
                            created: true,
                        },
                        window,
                        cx,
                    )
                });
                true
            }
            Err(error) => {
                tracing::info!(code = error.code(), "certificate not added");
                self.error = Some(error_text(&error));
                cx.notify();
                false
            }
        }
    }

    /// Choose file…: the certificate file's text.
    fn browse(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: None,
        });
        cx.spawn_in(window, async move |this, cx| {
            let Ok(Ok(Some(paths))) = paths.await else {
                return;
            };
            let Some(path) = paths.into_iter().next() else {
                return;
            };
            let text = std::fs::metadata(&path)
                .ok()
                .filter(|meta| meta.len() <= MAX_FILE_BYTES)
                .and_then(|_| std::fs::read_to_string(&path).ok());
            let _ = this.update_in(cx, |this, window, cx| match text {
                Some(text) => this.set_text(&text, window, cx),
                None => {
                    this.error = Some(strings::CERTIFICATE_INVALID);
                    cx.notify();
                }
            });
        })
        .detach();
    }
}

impl Render for CertificateForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.text.read(cx).value().trim().is_empty();
        v_flex()
            .id("kc-certificate-form")
            .test_support()
            .gap(metrics::SPACE_3)
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(strings::CERTIFICATE_DIALOG_TITLE),
            )
            .child(widgets::caption(strings::CERTIFICATE_HINT, cx))
            .child(
                div().h(metrics::rpx(140.)).child(
                    Textarea::new(&self.text)
                        .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                        .h_full(),
                ),
            )
            .when_some(self.error, |this, error| {
                this.child(
                    div()
                        .id("kc-certificate-error")
                        .test_support()
                        .child(widgets::error_text(error, cx)),
                )
            })
            .child(
                h_flex()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("kc-certificate-file", ControlKind::Neutral)
                            .label(strings::CHOOSE_FILE)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.browse(window, cx)
                            })),
                    )
                    .child(div().flex_1())
                    .child(
                        ControlButton::new("kc-certificate-cancel", ControlKind::Neutral)
                            .label(strings::EXPORT_CANCEL)
                            .on_click(|_: &ClickEvent, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("kc-certificate-add", ControlKind::Accent)
                            .label(strings::CERTIFICATE_ADD)
                            .disabled(empty)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                if this.add(window, cx) {
                                    window.close_dialog(cx);
                                }
                            })),
                    ),
            )
            .text_color(cx.theme().foreground)
            .when(window.is_window_active(), |this| this)
    }
}

impl KeychainScreen {
    /// Certificate (UI 7.1): the Add a certificate dialog.
    pub fn open_certificate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let screen = cx.entity().downgrade();
        let state = self.state.clone();
        let form = cx.new(|cx| CertificateForm::new(state, screen, window, cx));
        window.open_dialog(cx, move |modal, _, cx: &mut App| {
            modal
                .w(metrics::scaled(520., cx))
                .bg(cx.theme().popover)
                .rounded(metrics::RADIUS_OVERLAY)
                .close_button(false)
                .child(form.clone())
        });
    }
}
