//! The sync interface of stage 7 (UI 12.2, 13.2, 13.5, 13.6, 13.7, 14):
//!
//! - the connect wizard: the server address, the certificate trust dialog
//!   when the system does not trust the server, sign-in or registration
//!   (with the invitation code of an invite-only server) and the TOTP step;
//!   at first run the same wizard is "Restore from sync" (sign-in only);
//! - the Account & Sync block of Settings: status, Sync now, the toggles,
//!   Devices, Review, Recently deleted, Sign out, Delete account and the
//!   re-post of "server state rolled back";
//! - Review (conflicts with "Keep current" / "Use other version", possible
//!   duplicates with Merge / Keep both, the records a resync brought back
//!   with Keep / Delete again) and Recently deleted (Restore, Delete
//!   forever, Empty).
//!
//! Every network call runs off the UI thread; nothing here logs addresses,
//! emails, passwords, codes or tokens.

pub mod strings;

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, Entity, FontWeight,
    InteractiveElement as _, IntoElement, ParentElement as _, Render, Styled as _, Task, Window,
    div, prelude::FluentBuilder as _,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::InputState;
use gpui_kit::component::switch::Switch;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use secrecy::SecretString;
use serde_json::{Map, Value};
use shellpp_core::CoreError;
use shellpp_core::service::{
    BroughtBackItem, ConflictChoice, ConflictItem, DeletedItem, DeviceItem, DuplicateItem,
    ServerCheck, SignInInput, SignInOutcome, SyncStatus, TrustChoice,
};
use shellpp_core::sync::{HttpError, OriginError, TlsProblem, fingerprint_text};

use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::state::AppState;
use crate::theme::metrics;

const DIALOG_WIDTH: f32 = 480.;

/// The text for an error of a sync operation.
pub fn error_text(error: &CoreError) -> &'static str {
    match error {
        CoreError::InvalidPassword => strings::ERROR_PASSWORD,
        CoreError::Sync(HttpError::Origin(OriginError::HttpNotLocal)) => strings::ERROR_HTTP,
        CoreError::Sync(error) => match error.code().as_str() {
            "invalid_origin" => strings::ERROR_ADDRESS,
            "network" | "timeout" => strings::ERROR_NETWORK,
            "invalid_credentials" => strings::ERROR_CREDENTIALS,
            "invalid_totp" => strings::ERROR_TOTP,
            "ticket_expired" => strings::ERROR_TICKET,
            "registration_closed" => strings::ERROR_CLOSED,
            "email_taken" => strings::ERROR_TAKEN,
            "unsafe_kdf" | "weak_kdf_params" => strings::ERROR_KDF,
            "previous_account" => strings::ERROR_PREVIOUS,
            "rolled_back" => strings::ERROR_ROLLED_BACK,
            "integrity" => strings::ERROR_INTEGRITY,
            _ => strings::ERROR_OTHER,
        },
        _ => strings::ERROR_OTHER,
    }
}

/// The fingerprint in two lines of 16 pairs each.
fn fingerprint_lines(sha256: &[u8; 32]) -> Vec<gpui::Div> {
    let text = fingerprint_text(sha256);
    let pairs: Vec<&str> = text.split(':').collect();
    pairs
        .chunks(16)
        .map(|line| div().child(line.join(":")))
        .collect()
}

/// A stored RFC 3339 time as the app shows dates.
fn when(text: &str) -> String {
    crate::dates::parse(text).map_or_else(|| text.to_owned(), crate::dates::date_time)
}

fn title(text: &'static str) -> impl IntoElement {
    div()
        .text_size(metrics::FONT_SIZE_TITLE)
        .font_weight(FontWeight::SEMIBOLD)
        .child(text)
}

fn label(text: impl Into<gpui::SharedString>, cx: &App) -> impl IntoElement {
    div()
        .text_size(metrics::FONT_SIZE_BODY)
        .font_weight(metrics::FONT_WEIGHT_MEDIUM)
        .text_color(cx.theme().popover_foreground)
        .child(text.into())
}

fn error_line(error: Option<&'static str>, cx: &App) -> Option<impl IntoElement> {
    error.map(|error| {
        div()
            .id("sync-error")
            .test_support()
            .text_size(metrics::FONT_SIZE_CAPTION)
            .text_color(cx.theme().danger)
            .child(error)
    })
}

fn input_text(input: &Entity<InputState>, cx: &App) -> String {
    input.read(cx).value().to_string()
}

fn secret(input: &Entity<InputState>, cx: &App) -> SecretString {
    SecretString::from(input.read(cx).value().to_string())
}

// ---------------------------------------------------------------------------
// The connect wizard (UI 13.5; "Restore from sync", UI 13.2)
// ---------------------------------------------------------------------------

/// Where the wizard is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectStep {
    Server,
    Trust(TlsProblem),
    Account,
    Totp,
    Done,
}

pub struct ConnectDialog {
    state: Entity<AppState>,
    server: Entity<InputState>,
    email: Entity<InputState>,
    password: Entity<InputState>,
    invite: Entity<InputState>,
    totp: Entity<InputState>,
    pub step: ConnectStep,
    origin: Option<String>,
    invite_only: bool,
    /// Create an account (else sign in); never at first run.
    pub register: bool,
    first_run: bool,
    busy: bool,
    error: Option<&'static str>,
    task: Option<Task<()>>,
}

impl ConnectDialog {
    pub fn new(
        state: Entity<AppState>,
        first_run: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self {
            state,
            server: cx.new(|cx| InputState::new(window, cx).placeholder("sync.example.com")),
            email: cx.new(|cx| InputState::new(window, cx).placeholder("you@example.com")),
            password: cx.new(|cx| InputState::new(window, cx).masked(true)),
            invite: cx.new(|cx| InputState::new(window, cx)),
            totp: cx.new(|cx| InputState::new(window, cx).placeholder("123456")),
            step: ConnectStep::Server,
            origin: None,
            invite_only: false,
            register: false,
            first_run,
            busy: false,
            error: None,
            task: None,
        }
    }

    fn fail(&mut self, error: &CoreError, cx: &mut Context<Self>) {
        self.busy = false;
        self.error = Some(error_text(error));
        tracing::info!(code = error.code(), "sync connect step failed");
        cx.notify();
    }

    /// Step 1: `GET /v1/server` with the stored trust.
    pub fn check(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let address = input_text(&self.server, cx);
        if address.trim().is_empty() {
            self.error = Some(strings::ERROR_ADDRESS);
            cx.notify();
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let checked = cx.background_spawn(async move { core.sync_server_check(address.trim()) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = checked.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.busy = false;
                match result {
                    Ok(ServerCheck::Ready {
                        origin,
                        invite_only,
                        ..
                    }) => {
                        dialog.origin = Some(origin);
                        dialog.invite_only = invite_only;
                        dialog.step = ConnectStep::Account;
                    }
                    Ok(ServerCheck::Certificate { origin, problem }) => {
                        dialog.origin = Some(origin);
                        dialog.step = ConnectStep::Trust(problem);
                    }
                    Err(error) => return dialog.fail(&error, cx),
                }
                cx.notify();
            });
        }));
    }

    /// The trust dialog's "Trust this certificate": pin it, check again.
    pub fn pin(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(core), Some(origin)) = (self.state.read(cx).core().cloned(), self.origin.clone())
        else {
            return;
        };
        let sha256 = match &self.step {
            ConnectStep::Trust(TlsProblem::UnknownIssuer(info)) => info.sha256,
            ConnectStep::Trust(TlsProblem::PinChanged { new, .. }) => new.sha256,
            _ => return,
        };
        if let Err(error) = core.sync_server_trust(&origin, TrustChoice::Pin(sha256)) {
            return self.fail(&error, cx);
        }
        self.server
            .update(cx, |input, cx| input.set_value(origin, window, cx));
        self.step = ConnectStep::Server;
        self.check(window, cx);
    }

    /// Step 2: sign in or register.
    pub fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(core), Some(origin)) = (self.state.read(cx).core().cloned(), self.origin.clone())
        else {
            return;
        };
        let code = input_text(&self.invite, cx);
        let input = SignInInput {
            origin,
            email: input_text(&self.email, cx),
            password: secret(&self.password, cx),
            registration_code: (!code.trim().is_empty()).then(|| SecretString::from(code)),
        };
        let register = self.register && !self.first_run;
        self.busy = true;
        self.error = None;
        cx.notify();
        let ran = cx.background_spawn(async move {
            if register {
                core.sync_register(input).map(SignInOutcome::Done)
            } else {
                core.sync_sign_in(input)
            }
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = ran.await;
            let _ = this.update(cx, |dialog, cx| dialog.finish(result, cx));
        }));
    }

    /// The TOTP step.
    pub fn verify(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let code = input_text(&self.totp, cx);
        self.busy = true;
        self.error = None;
        cx.notify();
        let ran = cx.background_spawn(async move { core.sync_sign_in_totp(&code) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = ran.await;
            let _ = this.update(cx, |dialog, cx| dialog.finish(result, cx));
        }));
    }

    fn finish(&mut self, result: Result<SignInOutcome, CoreError>, cx: &mut Context<Self>) {
        self.busy = false;
        match result {
            Ok(SignInOutcome::TotpRequired) => self.step = ConnectStep::Totp,
            Ok(SignInOutcome::Done(status)) => {
                self.step = ConnectStep::Done;
                self.state
                    .update(cx, |state, cx| state.set_sync_status(status, cx));
            }
            Err(error) => return self.fail(&error, cx),
        }
        cx.notify();
    }

    pub fn back(&mut self, cx: &mut Context<Self>) {
        self.error = None;
        self.step = match self.step {
            ConnectStep::Totp => ConnectStep::Account,
            _ => ConnectStep::Server,
        };
        cx.notify();
    }

    fn render_trust(&self, problem: &TlsProblem, cx: &mut Context<Self>) -> AnyElement {
        let (text, info) = match problem {
            TlsProblem::UnknownIssuer(info) => (strings::TRUST_UNKNOWN, Some(info)),
            TlsProblem::PinChanged { new, .. } => (strings::TRUST_CHANGED, Some(new)),
            TlsProblem::Expired(info) => (strings::TRUST_REFUSED_EXPIRED, Some(info)),
            TlsProblem::WrongName(info) => (strings::TRUST_REFUSED_NAME, Some(info)),
            TlsProblem::Other(_) => (strings::ERROR_NETWORK, None),
        };
        let mut body = v_flex()
            .id("sync-trust")
            .test_support()
            .gap(metrics::SPACE_3)
            .child(label(strings::TRUST_TITLE, cx))
            .child(widgets::caption(text, cx));
        if let Some(info) = info {
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
            body = body
                .child(row(strings::TRUST_SUBJECT, info.subject.clone(), cx))
                .child(row(strings::TRUST_ISSUER, info.issuer.clone(), cx))
                .child(row(
                    strings::TRUST_VALID,
                    format!("{} – {}", when(&info.not_before), when(&info.not_after)),
                    cx,
                ))
                .child(
                    v_flex()
                        .id("sync-fingerprint")
                        .test_support()
                        .gap(metrics::SPACE_1)
                        .child(widgets::caption(strings::TRUST_FINGERPRINT, cx))
                        .child(
                            div()
                                .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                                .text_size(metrics::FONT_SIZE_CAPTION)
                                .text_color(cx.theme().popover_foreground)
                                // Two lines of 16 pairs: a pair never breaks.
                                .children(fingerprint_lines(&info.sha256)),
                        ),
                );
        }
        body.into_any_element()
    }

    fn render_account(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let register = self.register && !self.first_run;
        let mut body = v_flex()
            .id("sync-account-form")
            .test_support()
            .gap(metrics::SPACE_3);
        if !self.first_run {
            body = body.child(
                h_flex()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new(
                            "sync-mode-sign-in",
                            if register {
                                ControlKind::Neutral
                            } else {
                                ControlKind::Accent
                            },
                        )
                        .label(strings::MODE_SIGN_IN)
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.register = false;
                                cx.notify();
                            },
                        )),
                    )
                    .child(
                        ControlButton::new(
                            "sync-mode-register",
                            if register {
                                ControlKind::Accent
                            } else {
                                ControlKind::Neutral
                            },
                        )
                        .label(strings::MODE_REGISTER)
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, _, cx| {
                                this.register = true;
                                cx.notify();
                            },
                        )),
                    ),
            );
        }
        body = body
            .child(widgets::field(
                strings::EMAIL,
                widgets::text_input(&self.email, "sync-email", false, window, cx),
                None,
                cx,
            ))
            .child(widgets::field(
                strings::PASSWORD,
                widgets::text_input(&self.password, "sync-password", false, window, cx),
                None,
                cx,
            ))
            .child(widgets::caption(
                if register {
                    strings::PASSWORD_HINT_REGISTER
                } else {
                    strings::PASSWORD_HINT_SIGN_IN
                },
                cx,
            ));
        if register && self.invite_only {
            body = body
                .child(widgets::field(
                    strings::INVITE_CODE,
                    widgets::text_input(&self.invite, "sync-invite", false, window, cx),
                    None,
                    cx,
                ))
                .child(widgets::caption(strings::INVITE_HINT, cx));
        }
        if !register && !self.first_run {
            body = body.child(widgets::caption(strings::MERGE_NOTE, cx));
        }
        body.into_any_element()
    }
}

impl Render for ConnectDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let step = match self.step.clone() {
            ConnectStep::Server => v_flex()
                .id("sync-server")
                .test_support()
                .gap(metrics::SPACE_2)
                .child(widgets::field(
                    strings::SERVER_ADDRESS,
                    widgets::text_input(
                        &self.server,
                        "sync-server-address",
                        self.error.is_some(),
                        window,
                        cx,
                    ),
                    None,
                    cx,
                ))
                .child(widgets::caption(strings::SERVER_HINT, cx))
                .into_any_element(),
            ConnectStep::Trust(problem) => self.render_trust(&problem, cx),
            ConnectStep::Account => self.render_account(window, cx),
            ConnectStep::Totp => v_flex()
                .id("sync-totp")
                .test_support()
                .gap(metrics::SPACE_2)
                .child(label(strings::TOTP_TITLE, cx))
                .child(widgets::field(
                    strings::TOTP_CODE,
                    widgets::text_input(
                        &self.totp,
                        "sync-totp-code",
                        self.error.is_some(),
                        window,
                        cx,
                    ),
                    None,
                    cx,
                ))
                .into_any_element(),
            ConnectStep::Done => v_flex()
                .id("sync-done")
                .test_support()
                .gap(metrics::SPACE_2)
                .child(label(strings::DONE_TITLE, cx))
                .child(widgets::caption(strings::DONE_TEXT, cx))
                .into_any_element(),
        };
        v_flex()
            .id("sync-connect")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(step)
            .children(error_line(self.error, cx))
    }
}

fn connect_footer(dialog: &Entity<ConnectDialog>, cx: &App) -> AnyElement {
    let this = dialog.read(cx);
    let busy = this.busy;
    let step = this.step.clone();
    let register = this.register && !this.first_run;
    let handle = dialog.downgrade();
    let row = h_flex().w_full().justify_end().gap(metrics::SPACE_2);
    if step == ConnectStep::Done {
        return row
            .child(
                ControlButton::new("sync-close", ControlKind::Accent)
                    .label(strings::CLOSE)
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .into_any_element();
    }
    let row = row.child(
        ControlButton::new("sync-cancel", ControlKind::Neutral)
            .label(strings::CANCEL)
            .on_click({
                let handle = handle.clone();
                move |_, window, cx| {
                    if let Some(dialog) = handle.upgrade() {
                        let core = dialog.read(cx).state.read(cx).core().cloned();
                        if let Some(core) = core {
                            core.sync_sign_in_cancel();
                        }
                    }
                    window.close_dialog(cx);
                }
            }),
    );
    let row = if matches!(step, ConnectStep::Server) {
        row
    } else {
        let back = handle.clone();
        row.child(
            ControlButton::new("sync-back", ControlKind::Neutral)
                .label(strings::BACK)
                .disabled(busy)
                .on_click(move |_, _, cx| {
                    let _ = back.update(cx, |dialog, cx| dialog.back(cx));
                }),
        )
    };
    let (id, text): (&'static str, &'static str) = match &step {
        ConnectStep::Server => (
            "sync-check",
            if busy {
                strings::CHECKING
            } else {
                strings::CHECK
            },
        ),
        ConnectStep::Trust(TlsProblem::UnknownIssuer(_) | TlsProblem::PinChanged { .. }) => {
            ("sync-pin", strings::TRUST_PIN)
        }
        ConnectStep::Trust(_) => return row.into_any_element(),
        ConnectStep::Account => (
            "sync-run",
            if busy {
                strings::RUNNING
            } else if register {
                strings::RUN_REGISTER
            } else {
                strings::RUN_SIGN_IN
            },
        ),
        ConnectStep::Totp => ("sync-verify", strings::TOTP_RUN),
        ConnectStep::Done => unreachable!("handled above"),
    };
    row.child(
        ControlButton::new(id, ControlKind::Accent)
            .label(text)
            .disabled(busy)
            .on_click(move |_, window, cx| {
                let _ = handle.update(cx, |dialog, cx| match dialog.step {
                    ConnectStep::Server => dialog.check(window, cx),
                    ConnectStep::Trust(_) => dialog.pin(window, cx),
                    ConnectStep::Account => dialog.run(window, cx),
                    ConnectStep::Totp => dialog.verify(window, cx),
                    ConnectStep::Done => {}
                });
            }),
    )
    .into_any_element()
}

/// Opens the connect wizard (`first_run`: "Restore from sync", sign-in only).
pub fn open_connect(
    state: Entity<AppState>,
    first_run: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ConnectDialog> {
    let dialog = cx.new(|cx| ConnectDialog::new(state, first_run, window, cx));
    let shown = dialog.clone();
    window.open_dialog(cx, move |modal, _, cx| {
        modal
            .w(metrics::scaled(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(if first_run {
                strings::RESTORE_FROM_SYNC
            } else {
                strings::CONNECT_TITLE
            }))
            .child(shown.clone())
            .footer(connect_footer(&shown, cx))
    });
    dialog
}

// ---------------------------------------------------------------------------
// The Account & Sync block of Settings (UI 12.2)
// ---------------------------------------------------------------------------

/// The toggles of data-model 17 with their labels and defaults.
pub const TOGGLES: [(&str, &str, bool); 10] = [
    ("hosts", "Hosts and groups", true),
    ("keys", "Keys and identities", true),
    ("snippets", "Snippets", true),
    ("forwarding", "Forwarding rules", true),
    ("known_hosts", "Known hosts", true),
    ("workspaces", "Workspaces", true),
    ("terminal_themes", "Custom terminal themes", true),
    ("highlight_rules", "Keyword highlighting rules", true),
    ("history", "Command history", false),
    ("connection_log", "Connection log", false),
];

fn status_line(status: &SyncStatus) -> String {
    let mut parts = Vec::new();
    match &status.last_sync_at {
        Some(at) => parts.push(format!("{} {}", strings::STATUS_SYNCED, when(at))),
        None => parts.push(strings::STATUS_NEVER.to_owned()),
    }
    if status.pending > 0 {
        parts.push(format!("{} {}", status.pending, strings::STATUS_PENDING));
    }
    if status.integrity_warnings > 0 {
        parts.push(format!(
            "{} {}",
            status.integrity_warnings,
            strings::INTEGRITY
        ));
    }
    parts.join(" · ")
}

/// The Account & Sync block: the connect button, or the bound account.
pub fn account_block(state: &Entity<AppState>, cx: &App) -> AnyElement {
    let status = state.read(cx).sync_status().cloned().unwrap_or_default();
    if !status.bound {
        let connect = state.clone();
        return v_flex()
            .id("sync-block")
            .test_support()
            .gap(metrics::SPACE_2)
            .child(label(crate::settings::strings::SYNC_SERVER, cx))
            .child(widgets::caption(
                crate::settings::strings::SYNC_SERVER_HINT,
                cx,
            ))
            .child(
                h_flex().child(
                    ControlButton::new("settings-connect-sync", ControlKind::Neutral)
                        .label(crate::settings::strings::CONNECT_SYNC)
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            open_connect(connect.clone(), false, window, cx);
                        }),
                ),
            )
            .into_any_element();
    }
    let theme = cx.theme();
    let mut block = v_flex()
        .id("sync-block")
        .test_support()
        .gap(metrics::SPACE_3)
        .child(label(strings::ACCOUNT, cx))
        .child(
            v_flex()
                .gap(metrics::SPACE_1)
                .child(
                    div()
                        .id("sync-account-email")
                        .test_support()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(theme.popover_foreground)
                        .child(status.email.clone().unwrap_or_default()),
                )
                .child(widgets::caption(
                    status.server_url.clone().unwrap_or_default(),
                    cx,
                )),
        )
        .child(
            div()
                .id("sync-status")
                .test_support()
                .text_size(metrics::FONT_SIZE_CAPTION)
                .text_color(theme.muted_foreground)
                .child(if status.signed_in {
                    status_line(&status)
                } else {
                    strings::STATUS_SIGNED_OUT.to_owned()
                }),
        );
    let warning = |id: &'static str, text: &'static str, cx: &App| {
        div()
            .id(id)
            .test_support()
            .text_size(metrics::FONT_SIZE_CAPTION)
            .text_color(cx.theme().warning)
            .child(text)
    };
    let rolled_back = status.last_error.as_deref() == Some("rolled_back");
    if rolled_back {
        block = block.child(warning("sync-rolled-back", strings::ROLLED_BACK, cx));
    } else if let Some(error) = &status.last_error
        && status.signed_in
    {
        block = block.child(widgets::caption(
            format!("{}: {error}", strings::STATUS_ERROR),
            cx,
        ));
    }
    if status.newer_epoch {
        block = block.child(warning("sync-newer-epoch", strings::NEWER_EPOCH, cx));
    }
    if status.recovery_mismatch {
        block = block.child(warning(
            "sync-recovery-mismatch",
            strings::RECOVERY_MISMATCH,
            cx,
        ));
    }
    let totp = state.clone();
    let (now, again, devices, review, trash, out, delete, repost) = (
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
        state.clone(),
    );
    let mut buttons = h_flex().gap(metrics::SPACE_2).flex_wrap();
    buttons = if status.signed_in {
        buttons.child(
            ControlButton::new("sync-now", ControlKind::Accent)
                .label(strings::SYNC_NOW)
                .on_click(move |_, _, cx| now.update(cx, |state, cx| state.sync_now(cx))),
        )
    } else {
        buttons.child(
            ControlButton::new("sync-sign-in-again", ControlKind::Accent)
                .label(strings::SIGN_IN_AGAIN)
                .on_click(move |_, window, cx| {
                    window.close_dialog(cx);
                    let dialog = open_connect(again.clone(), false, window, cx);
                    let origin = again
                        .read(cx)
                        .sync_status()
                        .and_then(|s| s.server_url.clone());
                    dialog.update(cx, |dialog, cx| {
                        if let Some(origin) = origin {
                            dialog
                                .server
                                .update(cx, |input, cx| input.set_value(origin, window, cx));
                        }
                    });
                }),
        )
    };
    if rolled_back {
        buttons = buttons.child(
            ControlButton::new("sync-repost", ControlKind::Neutral)
                .label(strings::REPOST)
                .on_click(move |_, window, cx| {
                    window.close_dialog(cx);
                    open_password_action(repost.clone(), PasswordAction::Repost, window, cx);
                }),
        );
    }
    let review_count = status.conflicts + status.duplicates + status.brought_back;
    buttons = buttons
        .child(
            ControlButton::new("sync-devices", ControlKind::Neutral)
                .label(strings::DEVICES)
                .on_click(move |_, window, cx| {
                    window.close_dialog(cx);
                    open_devices(devices.clone(), window, cx);
                }),
        )
        .child(
            ControlButton::new("sync-review", ControlKind::Neutral)
                .label(if review_count > 0 {
                    format!("{} ({review_count})", strings::REVIEW)
                } else {
                    strings::REVIEW.to_owned()
                })
                .on_click(move |_, window, cx| {
                    window.close_dialog(cx);
                    open_review(review.clone(), window, cx);
                }),
        )
        .child(
            ControlButton::new("sync-trash", ControlKind::Neutral)
                .label(strings::RECENTLY_DELETED)
                .on_click(move |_, window, cx| {
                    window.close_dialog(cx);
                    open_trash(trash.clone(), window, cx);
                }),
        );
    block = block.child(buttons).child(widgets::divider(cx));
    // The toggles (data-model 17).
    let toggles: Map<String, Value> = state
        .read(cx)
        .core()
        .and_then(|core| core.settings_get().ok())
        .and_then(|values| {
            values
                .get("sync.toggles")
                .and_then(Value::as_object)
                .cloned()
        })
        .unwrap_or_default();
    let mut list = v_flex()
        .id("sync-toggles")
        .test_support()
        .gap(metrics::SPACE_2)
        .child(label(strings::TOGGLES, cx))
        .child(widgets::caption(strings::TOGGLES_HINT, cx));
    for (key, text, default) in TOGGLES {
        let on = toggles.get(key).and_then(Value::as_bool).unwrap_or(default);
        let writer = state.clone();
        list = list.child(
            h_flex()
                .justify_between()
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(cx.theme().popover_foreground)
                        .child(text),
                )
                .child(
                    Switch::new(gpui::SharedString::from(format!("sync-toggle-{key}")))
                        .checked(on)
                        .on_click(move |checked: &bool, _, cx| {
                            set_toggle(&writer, key, *checked, cx);
                        }),
                ),
        );
    }
    block
        .child(list)
        .child(widgets::divider(cx))
        .child(
            h_flex()
                .gap(metrics::SPACE_2)
                .child(
                    ControlButton::new("sync-totp-setup", ControlKind::Neutral)
                        .label(strings::TOTP_SETUP)
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            open_totp(totp.clone(), window, cx);
                        }),
                )
                .child(
                    ControlButton::new("sync-sign-out", ControlKind::Neutral)
                        .label(strings::SIGN_OUT)
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            open_sign_out(out.clone(), window, cx);
                        }),
                )
                .child(
                    ControlButton::new("sync-delete-account", ControlKind::Danger)
                        .label(strings::DELETE_ACCOUNT)
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            open_password_action(
                                delete.clone(),
                                PasswordAction::DeleteAccount,
                                window,
                                cx,
                            );
                        }),
                ),
        )
        .into_any_element()
}

fn set_toggle(state: &Entity<AppState>, key: &str, on: bool, cx: &mut App) {
    let Some(core) = state.read(cx).core().cloned() else {
        return;
    };
    let mut toggles: Map<String, Value> = core
        .settings_get()
        .ok()
        .and_then(|values| {
            values
                .get("sync.toggles")
                .and_then(Value::as_object)
                .cloned()
        })
        .unwrap_or_default();
    toggles.insert(key.to_owned(), Value::Bool(on));
    let input = shellpp_core::service::dto::SettingsSetInput {
        values: std::collections::BTreeMap::from([(
            "sync.toggles".to_owned(),
            Value::Object(toggles),
        )]),
        reset: Vec::new(),
    };
    if let Err(error) = core.settings_set(input) {
        tracing::warn!(code = error.code(), "sync toggle not saved");
    }
    state.update(cx, |_, cx| cx.notify());
}

// ---------------------------------------------------------------------------
// Review: conflicts, duplicates, brought back (UI 13.6, 13.7; SP 6.9)
// ---------------------------------------------------------------------------

pub struct ReviewDialog {
    state: Entity<AppState>,
    pub conflicts: Vec<ConflictItem>,
    pub duplicates: Vec<DuplicateItem>,
    pub brought_back: Vec<BroughtBackItem>,
    error: Option<&'static str>,
}

impl ReviewDialog {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let mut dialog = Self {
            state,
            conflicts: Vec::new(),
            duplicates: Vec::new(),
            brought_back: Vec::new(),
            error: None,
        };
        dialog.reload(cx);
        dialog
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        self.conflicts = core.sync_conflicts().unwrap_or_default();
        self.duplicates = core.sync_duplicates().unwrap_or_default();
        self.brought_back = core.sync_brought_back().unwrap_or_default();
        self.state
            .update(cx, |state, cx| state.refresh_sync_status(cx));
        cx.notify();
    }

    fn act(&mut self, result: Result<(), CoreError>, cx: &mut Context<Self>) {
        if let Err(error) = result {
            self.error = Some(error_text(&error));
        } else {
            self.error = None;
        }
        self.reload(cx);
        self.state.update(cx, |state, cx| state.kick_sync(cx));
    }
}

fn row_text(main: String, detail: Option<String>, cx: &App) -> impl IntoElement {
    v_flex()
        .flex_1()
        .min_w_0()
        .child(
            div()
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(cx.theme().popover_foreground)
                .truncate()
                .child(main),
        )
        .children(detail.map(|detail| widgets::caption(detail, cx)))
}

impl Render for ReviewDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = v_flex()
            .id("sync-review")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3);
        if self.conflicts.is_empty() && self.duplicates.is_empty() && self.brought_back.is_empty() {
            body = body.child(widgets::caption(strings::NOTHING_TO_REVIEW, cx));
        }
        if !self.conflicts.is_empty() {
            body = body
                .child(label(strings::CONFLICTS, cx))
                .child(widgets::caption(strings::CONFLICTS_HINT, cx));
            for (index, item) in self.conflicts.clone().into_iter().enumerate() {
                let other = if item.losing_deleted {
                    strings::OTHER_DELETED.to_owned()
                } else {
                    item.losing_data
                        .as_ref()
                        .and_then(|data| {
                            data.get("label")
                                .or_else(|| data.get("name"))
                                .or_else(|| data.get("address"))
                        })
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                        .unwrap_or_default()
                };
                let (keep, use_other) = (item.conflict_id, item.conflict_id);
                body = body.child(
                    h_flex()
                        .id(("sync-conflict", index))
                        .test_support()
                        .gap(metrics::SPACE_2)
                        .child(row_text(item.title.clone(), Some(other), cx))
                        .child(
                            ControlButton::new(("sync-keep-current", index), ControlKind::Neutral)
                                .label(strings::KEEP_CURRENT)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let core = this.state.read(cx).core().cloned();
                                    if let Some(core) = core {
                                        let result = core
                                            .sync_conflict_resolve(keep, ConflictChoice::Current);
                                        this.act(result, cx);
                                    }
                                })),
                        )
                        .child(
                            ControlButton::new(("sync-use-other", index), ControlKind::Neutral)
                                .label(strings::KEEP_OTHER)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let core = this.state.read(cx).core().cloned();
                                    if let Some(core) = core {
                                        let result = core
                                            .sync_conflict_resolve(use_other, ConflictChoice::Copy);
                                        this.act(result, cx);
                                    }
                                })),
                        ),
                );
                // The comparison (UI 13.6): the fields that differ, secrets
                // masked by the core.
                for (line, difference) in item.differences.iter().take(8).enumerate() {
                    body = body.child(
                        div()
                            .id(("sync-difference", index * 100 + line))
                            .test_support()
                            .child(widgets::caption(
                                format!(
                                    "{}: {} → {}",
                                    difference.path, difference.current, difference.other
                                ),
                                cx,
                            )),
                    );
                }
            }
        }
        if !self.duplicates.is_empty() {
            body = body
                .child(label(strings::DUPLICATES, cx))
                .child(widgets::caption(strings::DUPLICATES_HINT, cx));
            for (index, item) in self.duplicates.clone().into_iter().enumerate() {
                let (a, b) = (item.record_id, item.other_record_id);
                body = body.child(
                    h_flex()
                        .id(("sync-duplicate", index))
                        .test_support()
                        .gap(metrics::SPACE_2)
                        .child(row_text(item.title.clone(), None, cx))
                        .child(
                            ControlButton::new(("sync-merge", index), ControlKind::Neutral)
                                .label(strings::MERGE)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let core = this.state.read(cx).core().cloned();
                                    if let Some(core) = core {
                                        let result = core.sync_duplicates_merge(a, b);
                                        this.act(result, cx);
                                    }
                                })),
                        )
                        .child(
                            ControlButton::new(("sync-keep-both", index), ControlKind::Neutral)
                                .label(strings::KEEP_BOTH)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let core = this.state.read(cx).core().cloned();
                                    if let Some(core) = core {
                                        let result = core.sync_duplicates_keep(a, b);
                                        this.act(result, cx);
                                    }
                                })),
                        ),
                );
            }
        }
        if !self.brought_back.is_empty() {
            body = body
                .child(label(strings::BROUGHT_BACK, cx))
                .child(widgets::caption(strings::BROUGHT_BACK_HINT, cx));
            for (index, item) in self.brought_back.clone().into_iter().enumerate() {
                let id = item.record_id;
                body = body.child(
                    h_flex()
                        .id(("sync-brought-back", index))
                        .test_support()
                        .gap(metrics::SPACE_2)
                        .child(row_text(item.title.clone(), None, cx))
                        .child(
                            ControlButton::new(("sync-keep", index), ControlKind::Neutral)
                                .label(strings::KEEP)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let core = this.state.read(cx).core().cloned();
                                    if let Some(core) = core {
                                        let result = core.sync_brought_back_keep(&[id]);
                                        this.act(result, cx);
                                    }
                                })),
                        )
                        .child(
                            ControlButton::new(("sync-delete-again", index), ControlKind::Danger)
                                .label(strings::DELETE_AGAIN)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    let core = this.state.read(cx).core().cloned();
                                    if let Some(core) = core {
                                        let result = core.sync_brought_back_delete(&[id]);
                                        this.act(result, cx);
                                    }
                                })),
                        ),
                );
            }
        }
        body.children(error_line(self.error, cx))
    }
}

fn close_footer(id: &'static str) -> AnyElement {
    h_flex()
        .w_full()
        .justify_end()
        .child(
            ControlButton::new(id, ControlKind::Neutral)
                .label(strings::CLOSE)
                .on_click(|_, window, cx| window.close_dialog(cx)),
        )
        .into_any_element()
}

fn open_simple<V: Render + 'static>(
    view: Entity<V>,
    heading: &'static str,
    footer_id: &'static str,
    window: &mut Window,
    cx: &mut App,
) {
    window.open_dialog(cx, move |modal, _, cx| {
        modal
            .w(metrics::scaled(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(heading))
            .child(view.clone())
            .footer(close_footer(footer_id))
    });
}

pub fn open_review(
    state: Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<ReviewDialog> {
    let dialog = cx.new(|cx| ReviewDialog::new(state, cx));
    open_simple(
        dialog.clone(),
        strings::REVIEW_TITLE,
        "sync-review-close",
        window,
        cx,
    );
    dialog
}

// ---------------------------------------------------------------------------
// Recently deleted (UI 14)
// ---------------------------------------------------------------------------

pub struct TrashDialog {
    state: Entity<AppState>,
    pub items: Vec<DeletedItem>,
    error: Option<&'static str>,
}

impl TrashDialog {
    pub fn new(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        let mut dialog = Self {
            state,
            items: Vec::new(),
            error: None,
        };
        dialog.reload(cx);
        dialog
    }

    fn reload(&mut self, cx: &mut Context<Self>) {
        self.items = self
            .state
            .read(cx)
            .core()
            .and_then(|core| core.recently_deleted_list().ok())
            .unwrap_or_default();
        cx.notify();
    }

    fn act(&mut self, result: Result<(), CoreError>, cx: &mut Context<Self>) {
        self.error = result.err().map(|error| error_text(&error));
        self.reload(cx);
        self.state.update(cx, |state, cx| {
            state.refresh_sync_status(cx);
            state.kick_sync(cx);
        });
    }
}

impl Render for TrashDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = v_flex()
            .id("sync-trash")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(widgets::caption(strings::TRASH_HINT, cx));
        if self.items.is_empty() {
            body = body.child(widgets::caption(strings::TRASH_EMPTY, cx));
        }
        for (index, item) in self.items.clone().into_iter().enumerate() {
            let id = item.record_id;
            let mut detail = when(&item.deleted_at);
            if item.from_sync {
                detail = format!("{detail} · {}", strings::FROM_SYNC);
            }
            body = body.child(
                h_flex()
                    .id(("sync-trash-item", index))
                    .test_support()
                    .gap(metrics::SPACE_2)
                    .child(row_text(item.title.clone(), Some(detail), cx))
                    .child(
                        ControlButton::new(("sync-restore", index), ControlKind::Neutral)
                            .label(strings::RESTORE)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                let core = this.state.read(cx).core().cloned();
                                if let Some(core) = core {
                                    let result = core.recently_deleted_restore(&[id]).map(|_| ());
                                    this.act(result, cx);
                                }
                            })),
                    )
                    .child(
                        ControlButton::new(("sync-delete-forever", index), ControlKind::Danger)
                            .label(strings::DELETE_FOREVER)
                            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                let core = this.state.read(cx).core().cloned();
                                if let Some(core) = core {
                                    let result = core.recently_deleted_delete(&[id]);
                                    this.act(result, cx);
                                }
                            })),
                    ),
            );
        }
        if !self.items.is_empty() {
            let ids: Vec<uuid::Uuid> = self.items.iter().map(|item| item.record_id).collect();
            let state = self.state.clone();
            body = body.child(
                h_flex()
                    .gap(metrics::SPACE_2)
                    .child(
                        // UI 14 "Export…": the Export Data dialog for these
                        // entries only.
                        ControlButton::new("sync-export-trash", ControlKind::Neutral)
                            .label(strings::EXPORT_TRASH)
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                let dialog =
                                    crate::data_transfer::open_export(state.clone(), window, cx);
                                let ids = ids.clone();
                                dialog.update(cx, |dialog, _| dialog.recently_deleted = ids);
                            }),
                    )
                    .child(
                        ControlButton::new("sync-empty-trash", ControlKind::Danger)
                            .label(strings::EMPTY_TRASH)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                                let core = this.state.read(cx).core().cloned();
                                if let Some(core) = core {
                                    let result = core.recently_deleted_empty().map(|_| ());
                                    this.act(result, cx);
                                }
                            })),
                    ),
            );
        }
        body.children(error_line(self.error, cx))
    }
}

pub fn open_trash(
    state: Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<TrashDialog> {
    let dialog = cx.new(|cx| TrashDialog::new(state, cx));
    open_simple(
        dialog.clone(),
        strings::TRASH_TITLE,
        "sync-trash-close",
        window,
        cx,
    );
    dialog
}

// ---------------------------------------------------------------------------
// Devices (UI 12.2)
// ---------------------------------------------------------------------------

pub struct DevicesDialog {
    state: Entity<AppState>,
    pub devices: Vec<DeviceItem>,
    /// The device whose revoke asks "Was this device lost or stolen?".
    pub asking: Option<uuid::Uuid>,
    loading: bool,
    error: Option<&'static str>,
    task: Option<Task<()>>,
}

impl DevicesDialog {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let mut dialog = Self {
            state,
            devices: Vec::new(),
            asking: None,
            loading: true,
            error: None,
            task: None,
        };
        dialog.reload(window, cx);
        dialog
    }

    fn reload(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        self.loading = true;
        let listed = cx.background_spawn(async move { core.sync_devices() });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = listed.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.loading = false;
                match result {
                    Ok(devices) => dialog.devices = devices,
                    Err(error) => dialog.error = Some(error_text(&error)),
                }
                cx.notify();
            });
        }));
    }

    /// Revokes; with `lost`, then opens the password change with the
    /// rotation fixed on (UI 12.2, "Lost or stolen").
    fn revoke(
        &mut self,
        device_id: uuid::Uuid,
        lost: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        self.asking = None;
        let revoked = cx.background_spawn(async move { core.sync_device_revoke(device_id) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = revoked.await;
            let _ = this.update_in(cx, |dialog, window, cx| {
                match result {
                    Ok(()) if lost => {
                        window.close_dialog(cx);
                        open_password_change(dialog.state.clone(), true, window, cx);
                        return;
                    }
                    Ok(()) => {}
                    Err(error) => dialog.error = Some(error_text(&error)),
                }
                dialog.reload(window, cx);
            });
        }));
    }
}

impl Render for DevicesDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut body = v_flex()
            .id("sync-devices")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3);
        if self.loading {
            body = body.child(widgets::caption(strings::CHECKING, cx));
        }
        for (index, device) in self.devices.clone().into_iter().enumerate() {
            let id = device.device_id;
            let mut detail = format!("{} {}", strings::LAST_SEEN, when(&device.last_seen_at));
            if device.current {
                detail = format!("{} · {detail}", strings::THIS_DEVICE);
            }
            if device.revoked {
                detail = format!("{detail} · {}", strings::REVOKED);
            }
            let asking = self.asking == Some(id);
            body = body.child(
                h_flex()
                    .id(("sync-device", index))
                    .test_support()
                    .gap(metrics::SPACE_2)
                    .child(row_text(device.device_id.to_string(), Some(detail), cx))
                    .when(!device.current && !device.revoked && !asking, |row| {
                        row.child(
                            ControlButton::new(("sync-revoke", index), ControlKind::Danger)
                                .label(strings::REVOKE)
                                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                                    this.asking = Some(id);
                                    cx.notify();
                                })),
                        )
                    }),
            );
            if asking {
                body = body.child(
                    v_flex()
                        .id("sync-lost-question")
                        .test_support()
                        .gap(metrics::SPACE_2)
                        .child(label(strings::LOST_QUESTION, cx))
                        .child(
                            h_flex()
                                .gap(metrics::SPACE_2)
                                .child(
                                    ControlButton::new("sync-revoke-only", ControlKind::Neutral)
                                        .label(strings::REVOKE_ONLY)
                                        .on_click(cx.listener(
                                            move |this, _: &ClickEvent, window, cx| {
                                                this.revoke(id, false, window, cx)
                                            },
                                        )),
                                )
                                .child(
                                    ControlButton::new("sync-lost", ControlKind::Danger)
                                        .label(strings::LOST_OR_STOLEN)
                                        .on_click(cx.listener(
                                            move |this, _: &ClickEvent, window, cx| {
                                                this.revoke(id, true, window, cx)
                                            },
                                        )),
                                ),
                        ),
                );
            }
        }
        body.children(error_line(self.error, cx))
    }
}

pub fn open_devices(
    state: Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DevicesDialog> {
    let dialog = cx.new(|cx| DevicesDialog::new(state, window, cx));
    open_simple(
        dialog.clone(),
        strings::DEVICES_TITLE,
        "sync-devices-close",
        window,
        cx,
    );
    dialog
}

// ---------------------------------------------------------------------------
// Sign out, delete account, re-post (password and confirmation dialogs)
// ---------------------------------------------------------------------------

pub struct SignOutDialog {
    state: Entity<AppState>,
    pub delete_local: bool,
    busy: bool,
    error: Option<&'static str>,
    task: Option<Task<()>>,
}

impl SignOutDialog {
    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let delete_local = self.delete_local;
        self.busy = true;
        cx.notify();
        let done = cx.background_spawn(async move { core.sync_sign_out(delete_local) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = done.await;
            let _ = this.update_in(cx, |dialog, window, cx| {
                dialog.busy = false;
                match result {
                    Ok(()) => {
                        dialog
                            .state
                            .update(cx, |state, cx| state.refresh_sync_status(cx));
                        window.close_dialog(cx);
                    }
                    Err(error) => dialog.error = Some(error_text(&error)),
                }
                cx.notify();
            });
        }));
    }
}

impl Render for SignOutDialog {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("sync-sign-out-dialog")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(widgets::caption(strings::SIGN_OUT_TEXT, cx))
            .child(
                Checkbox::new("sync-delete-local")
                    .label(strings::DELETE_LOCAL)
                    .checked(self.delete_local)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.delete_local = *checked;
                        cx.notify();
                    })),
            )
            .children(error_line(self.error, cx))
    }
}

pub fn open_sign_out(
    state: Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SignOutDialog> {
    let dialog = cx.new(|_| SignOutDialog {
        state,
        delete_local: false,
        busy: false,
        error: None,
        task: None,
    });
    let shown = dialog.clone();
    window.open_dialog(cx, move |modal, _, cx| {
        let run = shown.downgrade();
        let busy = shown.read(cx).busy;
        let delete_local = shown.read(cx).delete_local;
        modal
            .w(metrics::scaled(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(strings::SIGN_OUT_TITLE))
            .child(shown.clone())
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("sync-sign-out-cancel", ControlKind::Neutral)
                            .label(strings::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new(
                            "sync-sign-out-run",
                            if delete_local {
                                ControlKind::Danger
                            } else {
                                ControlKind::Accent
                            },
                        )
                        .label(strings::SIGN_OUT_RUN)
                        .disabled(busy)
                        .on_click(move |_, window, cx| {
                            let _ = run.update(cx, |dialog, cx| dialog.run(window, cx));
                        }),
                    ),
            )
    });
    dialog
}

/// What a password dialog does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordAction {
    DeleteAccount,
    Repost,
}

pub struct PasswordDialog {
    state: Entity<AppState>,
    action: PasswordAction,
    password: Entity<InputState>,
    code: Entity<InputState>,
    busy: bool,
    error: Option<&'static str>,
    task: Option<Task<()>>,
}

impl PasswordDialog {
    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let password = secret(&self.password, cx);
        let code = input_text(&self.code, cx);
        let action = self.action;
        self.busy = true;
        self.error = None;
        cx.notify();
        let done = cx.background_spawn(async move {
            match action {
                PasswordAction::DeleteAccount => {
                    core.sync_account_delete(password, (!code.trim().is_empty()).then_some(code))
                }
                PasswordAction::Repost => core.sync_repost_epochs(password).map(|_| ()),
            }
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = done.await;
            let _ = this.update_in(cx, |dialog, window, cx| {
                dialog.busy = false;
                match result {
                    Ok(()) => {
                        dialog
                            .state
                            .update(cx, |state, cx| state.refresh_sync_status(cx));
                        window.close_dialog(cx);
                    }
                    Err(error) => dialog.error = Some(error_text(&error)),
                }
                cx.notify();
            });
        }));
    }
}

impl Render for PasswordDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let text = match self.action {
            PasswordAction::DeleteAccount => strings::DELETE_TEXT,
            PasswordAction::Repost => strings::REPOST_TEXT,
        };
        let export = self.state.clone();
        v_flex()
            .id("sync-password-dialog")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(widgets::caption(text, cx))
            .when(self.action == PasswordAction::DeleteAccount, |this| {
                this.child(
                    h_flex().child(
                        ControlButton::new("sync-export-first", ControlKind::Neutral)
                            .label(strings::EXPORT_FIRST)
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                crate::data_transfer::open_export(export.clone(), window, cx);
                            }),
                    ),
                )
            })
            .child(widgets::field(
                strings::PASSWORD,
                widgets::text_input(
                    &self.password,
                    "sync-action-password",
                    self.error.is_some(),
                    window,
                    cx,
                ),
                None,
                cx,
            ))
            .when(self.action == PasswordAction::DeleteAccount, |this| {
                this.child(widgets::field(
                    strings::TOTP_CODE,
                    widgets::text_input(&self.code, "sync-action-code", false, window, cx),
                    None,
                    cx,
                ))
            })
            .children(error_line(self.error, cx))
    }
}

pub fn open_password_action(
    state: Entity<AppState>,
    action: PasswordAction,
    window: &mut Window,
    cx: &mut App,
) -> Entity<PasswordDialog> {
    let dialog = cx.new(|cx| PasswordDialog {
        state,
        action,
        password: cx.new(|cx| InputState::new(window, cx).masked(true)),
        code: cx.new(|cx| InputState::new(window, cx)),
        busy: false,
        error: None,
        task: None,
    });
    let shown = dialog.clone();
    let (heading, run_text, kind) = match action {
        PasswordAction::DeleteAccount => (
            strings::DELETE_TITLE,
            strings::DELETE_RUN,
            ControlKind::Danger,
        ),
        PasswordAction::Repost => (
            strings::REPOST_TITLE,
            strings::REPOST_RUN,
            ControlKind::Accent,
        ),
    };
    window.open_dialog(cx, move |modal, _, cx| {
        let run = shown.downgrade();
        let busy = shown.read(cx).busy;
        modal
            .w(metrics::scaled(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(heading))
            .child(shown.clone())
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("sync-action-cancel", ControlKind::Neutral)
                            .label(strings::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("sync-action-run", kind)
                            .label(run_text)
                            .disabled(busy)
                            .on_click(move |_, window, cx| {
                                let _ = run.update(cx, |dialog, cx| dialog.run(window, cx));
                            }),
                    ),
            )
    });
    dialog
}

macro_rules! debug_opaque {
    ($($name:ident),*) => {
        $(impl std::fmt::Debug for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.debug_struct(stringify!($name)).finish_non_exhaustive()
            }
        })*
    };
}

debug_opaque!(
    ConnectDialog,
    ReviewDialog,
    TrashDialog,
    DevicesDialog,
    SignOutDialog,
    PasswordDialog
);

// ---------------------------------------------------------------------------
// Two-step sign-in (TOTP, UI 13.5 step 5; docs/sync-protocol.md, 5.5)
// ---------------------------------------------------------------------------

/// Where the TOTP setup is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TotpStep {
    Password,
    /// The secret for the authenticator app and the code to confirm it.
    Code {
        secret: String,
        uri: String,
    },
    Done,
}

pub struct TotpDialog {
    state: Entity<AppState>,
    password: Entity<InputState>,
    code: Entity<InputState>,
    pub step: TotpStep,
    busy: bool,
    error: Option<&'static str>,
    task: Option<Task<()>>,
}

impl TotpDialog {
    fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        self.busy = true;
        self.error = None;
        cx.notify();
        match self.step.clone() {
            TotpStep::Password => {
                let password = secret(&self.password, cx);
                let setup = cx.background_spawn(async move { core.sync_totp_setup(password) });
                self.task = Some(cx.spawn_in(window, async move |this, cx| {
                    let result = setup.await;
                    let _ = this.update(cx, |dialog, cx| {
                        dialog.busy = false;
                        match result {
                            Ok(setup) => {
                                use secrecy::ExposeSecret as _;
                                dialog.step = TotpStep::Code {
                                    secret: setup.secret.expose_secret().to_owned(),
                                    uri: setup.otpauth_uri.expose_secret().to_owned(),
                                };
                            }
                            Err(error) => dialog.error = Some(error_text(&error)),
                        }
                        cx.notify();
                    });
                }));
            }
            TotpStep::Code { .. } => {
                let code = input_text(&self.code, cx);
                let enabled = cx.background_spawn(async move { core.sync_totp_enable(&code) });
                self.task = Some(cx.spawn_in(window, async move |this, cx| {
                    let result = enabled.await;
                    let _ = this.update(cx, |dialog, cx| {
                        dialog.busy = false;
                        match result {
                            Ok(()) => dialog.step = TotpStep::Done,
                            Err(error) => dialog.error = Some(error_text(&error)),
                        }
                        cx.notify();
                    });
                }));
            }
            TotpStep::Done => {}
        }
    }
}

impl Render for TotpDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.step.clone() {
            TotpStep::Password => v_flex()
                .gap(metrics::SPACE_2)
                .child(widgets::caption(strings::TOTP_INTRO, cx))
                .child(widgets::field(
                    strings::PASSWORD,
                    widgets::text_input(
                        &self.password,
                        "sync-totp-password",
                        self.error.is_some(),
                        window,
                        cx,
                    ),
                    None,
                    cx,
                ))
                .into_any_element(),
            TotpStep::Code { secret, uri } => v_flex()
                .gap(metrics::SPACE_2)
                .child(widgets::caption(strings::TOTP_SECRET_HINT, cx))
                .children(qr_code(&uri))
                .child(
                    div()
                        .id("sync-totp-secret")
                        .test_support()
                        .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(cx.theme().popover_foreground)
                        .child(secret),
                )
                .child(widgets::caption(uri, cx))
                .child(widgets::field(
                    strings::TOTP_CODE,
                    widgets::text_input(
                        &self.code,
                        "sync-totp-confirm",
                        self.error.is_some(),
                        window,
                        cx,
                    ),
                    None,
                    cx,
                ))
                .into_any_element(),
            TotpStep::Done => widgets::caption(strings::TOTP_ON, cx).into_any_element(),
        };
        v_flex()
            .id("sync-totp-dialog")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(body)
            .children(error_line(self.error, cx))
    }
}

debug_opaque!(TotpDialog);

pub fn open_totp(state: Entity<AppState>, window: &mut Window, cx: &mut App) -> Entity<TotpDialog> {
    let dialog = cx.new(|cx| TotpDialog {
        state,
        password: cx.new(|cx| InputState::new(window, cx).masked(true)),
        code: cx.new(|cx| InputState::new(window, cx).placeholder("123456")),
        step: TotpStep::Password,
        busy: false,
        error: None,
        task: None,
    });
    let shown = dialog.clone();
    window.open_dialog(cx, move |modal, _, cx| {
        let next = shown.downgrade();
        let busy = shown.read(cx).busy;
        let done = shown.read(cx).step == TotpStep::Done;
        let row = h_flex().w_full().justify_end().gap(metrics::SPACE_2);
        let row = if done {
            row.child(
                ControlButton::new("sync-totp-close", ControlKind::Accent)
                    .label(strings::CLOSE)
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
        } else {
            row.child(
                ControlButton::new("sync-totp-cancel", ControlKind::Neutral)
                    .label(strings::CANCEL)
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .child(
                ControlButton::new("sync-totp-next", ControlKind::Accent)
                    .label(strings::CHECK)
                    .disabled(busy)
                    .on_click(move |_, window, cx| {
                        let _ = next.update(cx, |dialog, cx| dialog.next(window, cx));
                    }),
            )
        };
        modal
            .w(metrics::scaled(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(strings::TOTP_TITLE))
            .child(shown.clone())
            .footer(row)
    });
    dialog
}

// ---------------------------------------------------------------------------
// Change master password (UI 13.4), with "My old password may be
// compromised" (a personal vault rotation, SP 5.1); fixed on after a
// "Lost or stolen" revoke (UI 12.2, Devices)
// ---------------------------------------------------------------------------

pub struct PasswordChangeDialog {
    state: Entity<AppState>,
    current: Entity<InputState>,
    new: Entity<InputState>,
    repeat: Entity<InputState>,
    /// "My old password may be compromised".
    pub compromised: bool,
    /// After "Lost or stolen": the option is on and cannot be turned off.
    pub fixed: bool,
    busy: bool,
    done: bool,
    error: Option<&'static str>,
    task: Option<Task<()>>,
}

impl PasswordChangeDialog {
    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let new = input_text(&self.new, cx);
        if new != input_text(&self.repeat, cx) {
            self.error = Some(strings::PASSWORDS_DIFFER);
            cx.notify();
            return;
        }
        if new.chars().count() < 8 {
            self.error = Some(strings::PASSWORD_SHORT);
            cx.notify();
            return;
        }
        let current = secret(&self.current, cx);
        let new = SecretString::from(new);
        let rotate = self.compromised || self.fixed;
        self.busy = true;
        self.error = None;
        cx.notify();
        let done = cx.background_spawn(async move {
            if rotate {
                core.password_change_rotating(current, new)
            } else {
                core.password_change(current, new)
            }
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = done.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.busy = false;
                match result {
                    Ok(()) => {
                        dialog.done = true;
                        dialog
                            .state
                            .update(cx, |state, cx| state.refresh_sync_status(cx));
                    }
                    Err(error) => dialog.error = Some(error_text(&error)),
                }
                cx.notify();
            });
        }));
    }
}

impl Render for PasswordChangeDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.done {
            return v_flex()
                .id("sync-password-change")
                .test_support()
                .w_full()
                .child(widgets::caption(
                    if self.compromised || self.fixed {
                        strings::PASSWORD_CHANGED_ROTATED
                    } else {
                        strings::PASSWORD_CHANGED
                    },
                    cx,
                ));
        }
        let fixed = self.fixed;
        v_flex()
            .id("sync-password-change")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .when(fixed, |this| {
                this.child(widgets::caption(strings::LOST_TEXT, cx))
            })
            .child(widgets::field(
                strings::CURRENT_PASSWORD,
                widgets::text_input(&self.current, "sync-current-password", false, window, cx),
                None,
                cx,
            ))
            .child(widgets::field(
                strings::NEW_PASSWORD,
                widgets::text_input(&self.new, "sync-new-password", false, window, cx),
                None,
                cx,
            ))
            .child(widgets::field(
                strings::REPEAT_PASSWORD,
                widgets::text_input(&self.repeat, "sync-repeat-password", false, window, cx),
                None,
                cx,
            ))
            .child(
                Checkbox::new("sync-compromised")
                    .label(strings::COMPROMISED)
                    .checked(self.compromised || fixed)
                    .disabled(fixed)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.compromised = *checked;
                        cx.notify();
                    })),
            )
            .child(widgets::caption(strings::COMPROMISED_HINT, cx))
            .children(error_line(self.error, cx))
    }
}

debug_opaque!(PasswordChangeDialog);

/// Opens the password change; `lost`: after a "Lost or stolen" revoke.
pub fn open_password_change(
    state: Entity<AppState>,
    lost: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<PasswordChangeDialog> {
    let dialog = cx.new(|cx| PasswordChangeDialog {
        state,
        current: cx.new(|cx| InputState::new(window, cx).masked(true)),
        new: cx.new(|cx| InputState::new(window, cx).masked(true)),
        repeat: cx.new(|cx| InputState::new(window, cx).masked(true)),
        compromised: lost,
        fixed: lost,
        busy: false,
        done: false,
        error: None,
        task: None,
    });
    let shown = dialog.clone();
    window.open_dialog(cx, move |modal, _, cx| {
        let run = shown.downgrade();
        let busy = shown.read(cx).busy;
        let done = shown.read(cx).done;
        let row = h_flex().w_full().justify_end().gap(metrics::SPACE_2);
        let row = if done {
            row.child(
                ControlButton::new("sync-password-close", ControlKind::Accent)
                    .label(strings::CLOSE)
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
        } else {
            row.child(
                ControlButton::new("sync-password-cancel", ControlKind::Neutral)
                    .label(strings::CANCEL)
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .child(
                ControlButton::new("sync-password-run", ControlKind::Accent)
                    .label(strings::CHANGE_PASSWORD_RUN)
                    .disabled(busy)
                    .on_click(move |_, window, cx| {
                        let _ = run.update(cx, |dialog, cx| dialog.run(window, cx));
                    }),
            )
        };
        modal
            .w(metrics::scaled(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(strings::CHANGE_PASSWORD_TITLE))
            .child(shown.clone())
            .footer(row)
    });
    dialog
}

// ---------------------------------------------------------------------------
// Forgot password? → Restore from sync (UI 13.2; SP 5.2)
// ---------------------------------------------------------------------------

pub struct RecoveryDialog {
    state: Entity<AppState>,
    key: Entity<InputState>,
    email: Entity<InputState>,
    new: Entity<InputState>,
    repeat: Entity<InputState>,
    totp: Entity<InputState>,
    busy: bool,
    /// The server could not be reached or refused: the local recovery is
    /// offered.
    pub offer_local: bool,
    error: Option<&'static str>,
    task: Option<Task<()>>,
}

impl RecoveryDialog {
    fn checked(&mut self, cx: &mut Context<Self>) -> Option<(SecretString, SecretString)> {
        let new = input_text(&self.new, cx);
        if new != input_text(&self.repeat, cx) {
            self.error = Some(strings::PASSWORDS_DIFFER);
            cx.notify();
            return None;
        }
        if new.chars().count() < 8 {
            self.error = Some(strings::PASSWORD_SHORT);
            cx.notify();
            return None;
        }
        Some((secret(&self.key, cx), SecretString::from(new)))
    }

    /// Through the server: the account password changes too.
    fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let Some((key, new)) = self.checked(cx) else {
            return;
        };
        let email = input_text(&self.email, cx);
        let code = input_text(&self.totp, cx);
        self.busy = true;
        self.error = None;
        cx.notify();
        let done = cx.background_spawn(async move {
            core.recovery_unlock_server(key, new, &email, (!code.trim().is_empty()).then_some(code))
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = done.await;
            let _ = this.update_in(cx, |dialog, window, cx| {
                dialog.busy = false;
                match result {
                    Ok(_) => window.close_dialog(cx),
                    Err(error) => {
                        dialog.error = Some(match &error {
                            CoreError::InvalidRecoveryKey => strings::RECOVERY_KEY_WRONG,
                            CoreError::Sync(HttpError::Api { status: 401, .. }) => {
                                strings::RECOVERY_REFUSED
                            }
                            other => error_text(other),
                        });
                        // Anything but a wrong key or a TOTP step offers the
                        // local recovery (UI 13.2).
                        dialog.offer_local = !matches!(error, CoreError::InvalidRecoveryKey)
                            && error_text(&error) != strings::ERROR_TOTP;
                    }
                }
                cx.notify();
            });
        }));
    }

    /// This device only: the new password opens the local copy; the
    /// device is signed out of the account.
    fn run_local(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let Some((key, new)) = self.checked(cx) else {
            return;
        };
        self.busy = true;
        cx.notify();
        let done = cx.background_spawn(async move { core.recovery_unlock(key, new) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = done.await;
            let _ = this.update_in(cx, |dialog, window, cx| {
                dialog.busy = false;
                match result {
                    Ok(_) => window.close_dialog(cx),
                    Err(error) => dialog.error = Some(error_text(&error)),
                }
                cx.notify();
            });
        }));
    }
}

impl Render for RecoveryDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .id("sync-recovery")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(widgets::caption(strings::RECOVERY_TEXT, cx))
            .child(widgets::field(
                strings::RECOVERY_KEY,
                widgets::text_input(&self.key, "sync-recovery-key", false, window, cx),
                None,
                cx,
            ))
            .child(widgets::field(
                strings::EMAIL,
                widgets::text_input(&self.email, "sync-recovery-email", false, window, cx),
                None,
                cx,
            ))
            .child(widgets::field(
                strings::NEW_PASSWORD,
                widgets::text_input(&self.new, "sync-recovery-new", false, window, cx),
                None,
                cx,
            ))
            .child(widgets::field(
                strings::REPEAT_PASSWORD,
                widgets::text_input(&self.repeat, "sync-recovery-repeat", false, window, cx),
                None,
                cx,
            ))
            .child(widgets::field(
                strings::RECOVERY_TOTP,
                widgets::text_input(&self.totp, "sync-recovery-totp", false, window, cx),
                None,
                cx,
            ))
            .children(error_line(self.error, cx))
            .when(self.offer_local, |this| {
                this.child(widgets::caption(strings::RECOVER_LOCAL_HINT, cx))
            })
    }
}

debug_opaque!(RecoveryDialog);

/// Opens "Restore from sync" of the unlock screen (bound devices).
pub fn open_recovery(
    state: Entity<AppState>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<RecoveryDialog> {
    let email = state
        .read(cx)
        .core()
        .and_then(|core| core.sync_status().ok())
        .and_then(|status| status.email)
        .unwrap_or_default();
    let dialog = cx.new(|cx| RecoveryDialog {
        state,
        key: cx.new(|cx| InputState::new(window, cx)),
        email: cx.new(|cx| InputState::new(window, cx).default_value(email)),
        new: cx.new(|cx| InputState::new(window, cx).masked(true)),
        repeat: cx.new(|cx| InputState::new(window, cx).masked(true)),
        totp: cx.new(|cx| InputState::new(window, cx)),
        busy: false,
        offer_local: false,
        error: None,
        task: None,
    });
    let shown = dialog.clone();
    window.open_dialog(cx, move |modal, _, cx| {
        let run = shown.downgrade();
        let local = shown.downgrade();
        let busy = shown.read(cx).busy;
        let offer_local = shown.read(cx).offer_local;
        let row = h_flex()
            .w_full()
            .justify_end()
            .gap(metrics::SPACE_2)
            .child(
                ControlButton::new("sync-recovery-cancel", ControlKind::Neutral)
                    .label(strings::CANCEL)
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
            .when(offer_local, |row| {
                row.child(
                    ControlButton::new("sync-recovery-local", ControlKind::Neutral)
                        .label(strings::RECOVER_LOCAL)
                        .disabled(busy)
                        .on_click(move |_, window, cx| {
                            let _ = local.update(cx, |dialog, cx| dialog.run_local(window, cx));
                        }),
                )
            })
            .child(
                ControlButton::new("sync-recovery-run", ControlKind::Accent)
                    .label(strings::RECOVERY_RUN)
                    .disabled(busy)
                    .on_click(move |_, window, cx| {
                        let _ = run.update(cx, |dialog, cx| dialog.run(window, cx));
                    }),
            );
        modal
            .w(metrics::scaled(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(strings::RESTORE_FROM_SYNC))
            .child(shown.clone())
            .footer(row)
    });
    dialog
}

/// The QR code of the `otpauth://` text (UI 13.5): dark modules on white
/// whatever the theme, with the 4-module quiet zone scanners need.
fn qr_code(text: &str) -> Option<AnyElement> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let width = code.width();
    let colors = code.to_colors();
    let module = metrics::rpx(4.);
    let quiet = 4;
    let white = gpui::rgb(0xffffff);
    let black = gpui::rgb(0x000000);
    let mut rows = v_flex()
        .id("sync-totp-qr")
        .test_support()
        .bg(white)
        .p(module * quiet as f32);
    for row in colors.chunks(width) {
        let mut line = h_flex();
        for color in row {
            let fill = if *color == qrcode::Color::Dark {
                black
            } else {
                white
            };
            line = line.child(div().w(module).h(module).bg(fill));
        }
        rows = rows.child(line);
    }
    Some(h_flex().child(rows).into_any_element())
}
