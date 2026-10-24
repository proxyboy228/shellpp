//! The certificate dialogs of the sync server (docs/sync-protocol.md,
//! section 2.1; UI 12.2, 13.2, 13.5 step 1; D187):
//!
//! - the trust choice of an unknown issuer: the certificate (server,
//!   subject, issuer, dates, the reason, the SHA-256 fingerprint as openssl
//!   prints it), "Trust this certificate", "Add a CA certificate…" (a PEM or
//!   DER file whose subject and fingerprint are shown for confirmation) and
//!   Cancel;
//! - the changed certificate of a pinned origin: the pinned and the new
//!   fingerprint side by side, whether the system trusts the new one, "Trust
//!   the new certificate", "Use the system's trust" (only when the system
//!   trusts it) and Cancel, which keeps sync stopped;
//! - a refusal (expired, another name, another TLS failure) with the reason
//!   and no way to trust it.
//!
//! Every choice is checked by the core against the server before it is kept
//! (`Core::sync_server_trust_verified`); a bound device goes on syncing with
//! it, without signing out. The same panel is a step of the connect wizard
//! and a dialog of its own (Account & Sync, the unlock screen's recovery).

use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, PathPromptOptions, Render, Styled as _, Task, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use shellpp_core::CoreError;
use shellpp_core::service::{ServerCheck, TrustChoice};
use shellpp_core::sync::{CertInfo, TlsProblem};

use super::{fingerprint_lines, label, strings, title, when};
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::state::AppState;
use crate::theme::metrics;

/// A CA certificate file is small; anything larger is not one.
const MAX_CA_FILE_BYTES: u64 = 256 * 1024;

/// What happens after the server answered under the confirmed trust.
pub type Trusted = Rc<dyn Fn(ServerCheck, &mut Window, &mut App)>;

/// A CA file read for confirmation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaCandidate {
    pub der: Vec<u8>,
    pub info: CertInfo,
}

pub struct TrustPanel {
    state: Entity<AppState>,
    pub origin: String,
    pub problem: TlsProblem,
    /// "Add a CA certificate…" read this file: its confirmation is shown.
    pub ca: Option<CaCandidate>,
    busy: bool,
    pub error: Option<&'static str>,
    on_trusted: Trusted,
    task: Option<Task<()>>,
}

impl std::fmt::Debug for TrustPanel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TrustPanel").finish_non_exhaustive()
    }
}

impl TrustPanel {
    pub fn new(
        state: Entity<AppState>,
        origin: String,
        problem: TlsProblem,
        on_trusted: Trusted,
    ) -> Self {
        Self {
            state,
            origin,
            problem,
            ca: None,
            busy: false,
            error: None,
            on_trusted,
            task: None,
        }
    }

    /// The certificate the server shows.
    fn presented(&self) -> Option<&CertInfo> {
        match &self.problem {
            TlsProblem::UnknownIssuer(info)
            | TlsProblem::Expired(info)
            | TlsProblem::WrongName(info) => Some(info),
            TlsProblem::PinChanged { new, .. } => Some(new),
            TlsProblem::Other(_) => None,
        }
    }

    /// A choice is possible (an unknown issuer or a changed pin).
    pub fn can_trust(&self) -> bool {
        matches!(
            self.problem,
            TlsProblem::UnknownIssuer(_) | TlsProblem::PinChanged { .. }
        )
    }

    /// Checks `choice` with the server and keeps it when the server answers
    /// (the core stores nothing otherwise).
    pub fn choose(&mut self, choice: TrustChoice, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        if self.busy {
            return;
        }
        let origin = self.origin.clone();
        let was_ca = matches!(choice, TrustChoice::Ca(_));
        self.busy = true;
        self.error = None;
        cx.notify();
        let checked =
            cx.background_spawn(async move { core.sync_server_trust_verified(&origin, choice) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = checked.await;
            let _ = this.update_in(cx, |panel, window, cx| {
                panel.busy = false;
                match result {
                    Ok(check @ ServerCheck::Ready { .. }) => {
                        let on_trusted = panel.on_trusted.clone();
                        cx.notify();
                        on_trusted(check, window, cx);
                        return;
                    }
                    Ok(ServerCheck::Certificate { problem, .. }) => {
                        // Nothing was kept.
                        panel.ca = None;
                        match (was_ca, problem) {
                            // A CA that did not issue the server's
                            // certificate (or one of the same name whose
                            // signature does not verify): back to the choice.
                            (true, TlsProblem::UnknownIssuer(_) | TlsProblem::Other(_)) => {
                                panel.error = Some(strings::TRUST_CA_NOT_ISSUER);
                            }
                            // Another name or out of its dates: refused with
                            // the reason, under the CA as under a pin.
                            (true, problem) => panel.problem = problem,
                            // The server shows something else now: that.
                            (false, problem) => {
                                panel.problem = problem;
                                panel.error = Some(strings::TRUST_NOT_KEPT);
                            }
                        }
                    }
                    Err(error) => panel.fail(&error),
                }
                cx.notify();
            });
        }));
    }

    fn fail(&mut self, error: &CoreError) {
        tracing::info!(code = error.code(), "certificate trust not kept");
        self.error = Some(match error {
            CoreError::Sync(shellpp_core::sync::HttpError::Refused("not_a_certificate")) => {
                strings::TRUST_CA_INVALID
            }
            other => super::error_text(other),
        });
    }

    /// "Trust this certificate" / "Trust the new certificate".
    pub fn pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sha256 = match &self.problem {
            TlsProblem::UnknownIssuer(info) => info.sha256,
            TlsProblem::PinChanged { new, .. } => new.sha256,
            _ => return,
        };
        self.choose(TrustChoice::Pin(sha256), window, cx);
    }

    /// "Use the system's trust" (only when the system trusts the new one).
    pub fn use_system(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if matches!(
            self.problem,
            TlsProblem::PinChanged {
                system_trusts: true,
                ..
            }
        ) {
            self.choose(TrustChoice::System, window, cx);
        }
    }

    /// The confirmation's "Add this CA".
    pub fn add_ca(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(ca) = self.ca.clone() {
            self.choose(TrustChoice::Ca(ca.der), window, cx);
        }
    }

    /// The bytes of the chosen CA file: shown for confirmation when they
    /// hold a certificate (PEM or DER).
    pub fn set_ca_file(&mut self, bytes: &[u8], cx: &mut Context<Self>) {
        self.ca = shellpp_core::sync::tls::ca_der_from_file(bytes).and_then(|der| {
            shellpp_core::sync::tls::cert_info(&der).map(|info| CaCandidate { der, info })
        });
        self.error = self.ca.is_none().then_some(strings::TRUST_CA_INVALID);
        cx.notify();
    }

    /// "Add a CA certificate…": a file picker, then the confirmation.
    pub fn browse_ca(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
            let bytes = std::fs::metadata(&path)
                .ok()
                .filter(|meta| meta.is_file() && meta.len() <= MAX_CA_FILE_BYTES)
                .and_then(|_| std::fs::read(&path).ok())
                .unwrap_or_default();
            let _ = this.update(cx, |panel, cx| panel.set_ca_file(&bytes, cx));
        })
        .detach();
    }

    /// The panel's buttons for a footer (the caller adds Cancel or Back).
    pub fn actions(panel: &Entity<TrustPanel>, cx: &App) -> Vec<AnyElement> {
        let this = panel.read(cx);
        let busy = this.busy;
        let handle = panel.downgrade();
        let button =
            |id: &'static str,
             text: &'static str,
             kind: ControlKind,
             run: fn(&mut TrustPanel, &mut Window, &mut Context<TrustPanel>)| {
                let handle = handle.clone();
                ControlButton::new(id, kind)
                    .label(text)
                    .disabled(busy)
                    .on_click(move |_, window, cx| {
                        let _ = handle.update(cx, |panel, cx| run(panel, window, cx));
                    })
                    .into_any_element()
            };
        if this.ca.is_some() {
            let back = handle.clone();
            return vec![
                ControlButton::new("sync-ca-back", ControlKind::Neutral)
                    .label(strings::BACK)
                    .disabled(busy)
                    .on_click(move |_, _, cx| {
                        let _ = back.update(cx, |panel, cx| {
                            panel.ca = None;
                            panel.error = None;
                            cx.notify();
                        });
                    })
                    .into_any_element(),
                button(
                    "sync-ca-add",
                    strings::TRUST_CA_ADD,
                    ControlKind::Accent,
                    TrustPanel::add_ca,
                ),
            ];
        }
        match &this.problem {
            TlsProblem::UnknownIssuer(_) => vec![
                button(
                    "sync-ca-browse",
                    strings::TRUST_CA_BROWSE,
                    ControlKind::Neutral,
                    TrustPanel::browse_ca,
                ),
                button(
                    "sync-pin",
                    strings::TRUST_PIN,
                    ControlKind::Accent,
                    TrustPanel::pin,
                ),
            ],
            TlsProblem::PinChanged { system_trusts, .. } => {
                let mut list = Vec::new();
                if *system_trusts {
                    list.push(button(
                        "sync-use-system",
                        strings::TRUST_USE_SYSTEM,
                        ControlKind::Neutral,
                        TrustPanel::use_system,
                    ));
                }
                list.push(button(
                    "sync-pin-new",
                    strings::TRUST_PIN_NEW,
                    ControlKind::Accent,
                    TrustPanel::pin,
                ));
                list
            }
            _ => Vec::new(),
        }
    }

    fn details(&self, info: &CertInfo, cx: &App) -> AnyElement {
        let row = |name: &'static str, value: String, cx: &App| {
            v_flex()
                .gap(metrics::SPACE_1)
                .child(widgets::caption(name, cx))
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(cx.theme().popover_foreground)
                        .child(value),
                )
        };
        v_flex()
            .gap(metrics::SPACE_3)
            .child(row(strings::TRUST_SERVER, self.origin.clone(), cx))
            .child(row(strings::TRUST_SUBJECT, info.subject.clone(), cx))
            .child(row(strings::TRUST_ISSUER, info.issuer.clone(), cx))
            .child(row(
                strings::TRUST_VALID,
                format!("{} – {}", when(&info.not_before), when(&info.not_after)),
                cx,
            ))
            .into_any_element()
    }
}

/// A fingerprint block: its caption over the two lines of 16 pairs.
fn fingerprint(id: &'static str, caption: &'static str, sha256: &[u8; 32], cx: &App) -> AnyElement {
    v_flex()
        .id(id)
        .test_support()
        .gap(metrics::SPACE_1)
        .child(widgets::caption(caption, cx))
        .child(
            div()
                .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                .text_size(metrics::FONT_SIZE_CAPTION)
                .text_color(cx.theme().popover_foreground)
                // Two lines of 16 pairs: a pair never breaks.
                .children(fingerprint_lines(sha256)),
        )
        .into_any_element()
}

impl Render for TrustPanel {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = v_flex()
            .id("sync-trust")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3);
        if let Some(ca) = &self.ca {
            // The CA's confirmation (SP 2.1).
            body = body.child(
                v_flex()
                    .id("sync-ca-confirm")
                    .test_support()
                    .gap(metrics::SPACE_3)
                    .child(label(strings::TRUST_CA_TITLE, cx))
                    .child(widgets::caption(strings::TRUST_CA_TEXT, cx))
                    .child(
                        v_flex()
                            .gap(metrics::SPACE_1)
                            .child(widgets::caption(strings::TRUST_SUBJECT, cx))
                            .child(
                                div()
                                    .id("sync-ca-subject")
                                    .test_support()
                                    .text_size(metrics::FONT_SIZE_BODY)
                                    .text_color(cx.theme().popover_foreground)
                                    .child(ca.info.subject.clone()),
                            ),
                    )
                    .child(fingerprint(
                        "sync-ca-fingerprint",
                        strings::TRUST_FINGERPRINT,
                        &ca.info.sha256,
                        cx,
                    )),
            );
            return body.children(super::error_line(self.error, cx));
        }
        let text = match &self.problem {
            TlsProblem::UnknownIssuer(_) => strings::TRUST_UNKNOWN,
            TlsProblem::PinChanged { .. } => strings::TRUST_CHANGED,
            TlsProblem::Expired(_) => strings::TRUST_REFUSED_EXPIRED,
            TlsProblem::WrongName(_) => strings::TRUST_REFUSED_NAME,
            TlsProblem::Other(_) => strings::TRUST_REFUSED_OTHER,
        };
        body = body.child(widgets::caption(text, cx));
        if let TlsProblem::PinChanged {
            pinned,
            new,
            system_trusts,
        } = &self.problem
        {
            body = body
                .child(
                    h_flex()
                        .id("sync-pin-changed")
                        .test_support()
                        .w_full()
                        .gap(metrics::SPACE_4)
                        .items_start()
                        .child(div().flex_1().child(fingerprint(
                            "sync-fingerprint-pinned",
                            strings::TRUST_PINNED,
                            pinned,
                            cx,
                        )))
                        .child(div().flex_1().child(fingerprint(
                            "sync-fingerprint",
                            strings::TRUST_NEW,
                            &new.sha256,
                            cx,
                        ))),
                )
                .child(
                    div()
                        .id(if *system_trusts {
                            "sync-system-trusts"
                        } else {
                            "sync-system-distrusts"
                        })
                        .test_support()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(cx.theme().popover_foreground)
                        .child(if *system_trusts {
                            strings::TRUST_SYSTEM_TRUSTS
                        } else {
                            strings::TRUST_SYSTEM_DISTRUSTS
                        }),
                )
                .child(self.details(new, cx));
        } else if let Some(info) = self.presented() {
            body = body.child(self.details(info, cx)).child(fingerprint(
                "sync-fingerprint",
                strings::TRUST_FINGERPRINT,
                &info.sha256,
                cx,
            ));
            if matches!(self.problem, TlsProblem::UnknownIssuer(_)) {
                body = body.child(widgets::caption(strings::TRUST_OPENSSL, cx));
            }
        }
        body.children(super::error_line(self.error, cx))
    }
}

/// Opens the panel as a dialog of its own (Account & Sync, the unlock
/// screen's recovery). Cancel closes it and keeps sync as it is; a kept
/// choice closes it and runs `on_trusted`.
pub fn open_trust(
    state: Entity<AppState>,
    origin: String,
    problem: TlsProblem,
    on_trusted: Trusted,
    window: &mut Window,
    cx: &mut App,
) -> Entity<TrustPanel> {
    let changed = matches!(problem, TlsProblem::PinChanged { .. });
    let close: Trusted = Rc::new(move |check, window, cx| {
        window.close_dialog(cx);
        on_trusted(check, window, cx);
    });
    let panel = cx.new(|_| TrustPanel::new(state, origin, problem, close));
    let shown = panel.clone();
    window.open_dialog(cx, move |modal, _, cx| {
        let can_trust = shown.read(cx).can_trust();
        let footer = h_flex()
            .w_full()
            .justify_end()
            .gap(metrics::SPACE_2)
            .child(
                ControlButton::new("sync-trust-cancel", ControlKind::Neutral)
                    .label(if can_trust {
                        strings::CANCEL
                    } else {
                        strings::CLOSE
                    })
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .children(TrustPanel::actions(&shown, cx));
        modal
            .w(metrics::scaled(super::DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(if changed {
                strings::TRUST_CHANGED_TITLE
            } else {
                strings::TRUST_TITLE
            }))
            .child(shown.clone())
            .footer(footer)
    });
    panel
}

/// Account & Sync's "Review" of a certificate problem: asks the server
/// again (the dialog shows what it shows now) and opens the dialog; when
/// the problem went away, sync is kicked.
pub fn review(state: Entity<AppState>, origin: String, window: &mut Window, cx: &mut App) {
    let Some(core) = state.read(cx).core().cloned() else {
        return;
    };
    let checked = cx.background_spawn(async move { core.sync_server_check(&origin) });
    window
        .spawn(cx, async move |cx| {
            let result = checked.await;
            let _ = cx.update(|window, cx| {
                let kick = state.clone();
                match result {
                    Ok(ServerCheck::Certificate { origin, problem }) => {
                        let on_trusted: Trusted = Rc::new(move |_, _, cx| {
                            kick.update(cx, |state, cx| state.sync_now(cx));
                        });
                        open_trust(state, origin, problem, on_trusted, window, cx);
                    }
                    Ok(ServerCheck::Ready { .. }) => {
                        kick.update(cx, |state, cx| state.sync_now(cx));
                    }
                    Err(error) => {
                        tracing::info!(code = error.code(), "certificate review failed");
                        kick.update(cx, |state, cx| state.refresh_sync_status(cx));
                    }
                }
            });
        })
        .detach();
}

/// The trust row under the server address (UI 12.2; D187).
pub fn trust_row(state: &Entity<AppState>, origin: &str, cx: &App) -> Option<AnyElement> {
    let core = state.read(cx).core()?;
    let trust = core.sync_server_trust_info(origin).ok()?;
    let (text, sha256) = match trust {
        shellpp_core::service::ServerTrust::System => (strings::TRUST_ROW_SYSTEM.to_owned(), None),
        shellpp_core::service::ServerTrust::Pinned { sha256 } => {
            (strings::TRUST_ROW_PINNED.to_owned(), Some(sha256))
        }
        shellpp_core::service::ServerTrust::Ca { name, sha256 } => {
            (format!("{}: {name}", strings::TRUST_ROW_CA), Some(sha256))
        }
    };
    Some(
        v_flex()
            .id("sync-trust-row")
            .test_support()
            .gap(metrics::SPACE_1)
            .child(widgets::caption(text, cx))
            .when_some(sha256, |this, sha256| {
                this.child(
                    div()
                        .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(cx.theme().muted_foreground)
                        .children(fingerprint_lines(&sha256)),
                )
            })
            .into_any_element(),
    )
}
