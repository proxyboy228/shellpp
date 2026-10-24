//! The recovery key and the master password (UI 13.4; "Lost or stolen",
//! UI 12.2; the wizard's key step, UI 13.5): Show and Reissue, and Change
//! master password with "My old password may be compromised".
//!
//! A new recovery key is shown with the required "I saved the recovery
//! key" before anything is sent (docs/sync-protocol.md, section 5.3, step
//! 4): the core keeps the prepared change (`password_change_prepare`,
//! `recovery_key_reissue_prepare`) until `key_change_confirm`. Nothing here
//! logs the key, a password or an address.

use gpui::{
    App, AppContext as _, Context, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, Render, Styled as _, Task, Window, div, prelude::FluentBuilder as _,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::InputState;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};
use secrecy::{ExposeSecret as _, SecretString};
use shellpp_core::service::KeyChangeFailure;
use shellpp_core::service::dto::SecretCopyTarget;
use uuid::Uuid;

use super::{DIALOG_WIDTH, error_line, error_text, input_text, secret, strings, title};
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::widgets;
use crate::state::AppState;
use crate::theme::metrics;

/// What the user is told after a confirmed change failed (D180); a
/// reissue alone changes no password, so its texts name none.
pub fn failure_text(failure: &KeyChangeFailure, password: bool) -> &'static str {
    match (failure, password) {
        (KeyChangeFailure::NotAccepted(_), _) => strings::KEY_NOT_ACCEPTED,
        (KeyChangeFailure::SignedOut(_), _) => strings::KEY_SIGNED_OUT,
        (KeyChangeFailure::Unknown(_), true) => strings::KEY_UNKNOWN,
        (KeyChangeFailure::Unknown(_), false) => strings::KEY_UNKNOWN_REISSUE,
        (KeyChangeFailure::NotStoredHere(_), true) => strings::KEY_NOT_STORED,
        (KeyChangeFailure::NotStoredHere(_), false) => strings::KEY_NOT_STORED_REISSUE,
    }
}

type CoreRef = std::sync::Arc<shellpp_core::Core>;

/// Groups of the key on one line: `RK1` and six groups, then the other five.
const KEY_GROUPS_PER_LINE: usize = 7;

/// The key in lines that break only between its groups (a hyphen ends every
/// line but the last), so a group is never split when it is written down.
pub(super) fn key_lines(key: &str) -> Vec<String> {
    let groups: Vec<&str> = key.split('-').collect();
    let lines: Vec<&[&str]> = groups.chunks(KEY_GROUPS_PER_LINE).collect();
    let last = lines.len().saturating_sub(1);
    lines
        .iter()
        .enumerate()
        .map(|(index, line)| {
            let text = line.join("-");
            if index < last { text + "-" } else { text }
        })
        .collect()
}

/// The key as text in the monospaced font, with "Copy" (the core copies it
/// as a sensitive value and clears the clipboard after 30 seconds).
pub(super) fn key_block(
    state: &Entity<AppState>,
    key: &SecretString,
    target: SecretCopyTarget,
    copied: Option<bool>,
    on_copied: impl Fn(&mut App, bool) + 'static,
    cx: &App,
) -> impl IntoElement {
    let state = state.clone();
    v_flex()
        .gap(metrics::SPACE_2)
        .child(
            div()
                .id("sync-key-text")
                .test_support()
                .px(metrics::SPACE_3)
                .py(metrics::SPACE_2)
                .rounded(metrics::RADIUS_CONTROL)
                .border_1()
                .border_color(cx.theme().border)
                .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(cx.theme().popover_foreground)
                .children(
                    key_lines(key.expose_secret())
                        .into_iter()
                        .map(|line| div().child(line)),
                ),
        )
        .child(
            h_flex()
                .gap(metrics::SPACE_2)
                .child(
                    ControlButton::new("sync-key-copy", ControlKind::Neutral)
                        .label(strings::COPY)
                        .on_click(move |_, _, cx| {
                            let Some(core) = state.read(cx).core().cloned() else {
                                return;
                            };
                            match core.secret_copy(target.clone()) {
                                Ok(()) => on_copied(cx, true),
                                Err(error) => {
                                    tracing::info!(code = error.code(), "recovery key not copied");
                                    on_copied(cx, false);
                                }
                            }
                        }),
                )
                .when_some(copied, |row, copied| {
                    row.child(if copied {
                        widgets::caption(strings::KEY_COPIED, cx)
                    } else {
                        widgets::error_text(strings::KEY_NOT_COPIED, cx)
                    })
                }),
        )
}

/// The required "I saved the recovery key" (UI 13.5, step 4).
fn saved_checkbox<V: 'static>(
    checked: bool,
    disabled: bool,
    cx: &mut Context<V>,
    set: impl Fn(&mut V, bool) + 'static,
) -> Checkbox {
    Checkbox::new("sync-key-saved")
        .label(strings::KEY_SAVED)
        .checked(checked)
        .disabled(disabled)
        .on_click(cx.listener(move |this, checked: &bool, _, cx| {
            set(this, *checked);
            cx.notify();
        }))
}

// ---------------------------------------------------------------------------
// Settings ▸ Security ▸ Recovery key: Show and Reissue (UI 13.4)
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecoveryKeyMode {
    Show,
    Reissue,
}

#[derive(Clone, Debug)]
pub enum RecoveryStep {
    Password,
    /// The key: the current one (Show) or the prepared new one (Reissue,
    /// with the id of the prepared change).
    Key(SecretString, Option<u64>),
    Done,
    Failed(&'static str),
}

pub struct RecoveryKeyDialog {
    state: Entity<AppState>,
    /// Drops a prepared key that was never confirmed when the dialog goes.
    core: Option<CoreRef>,
    pub mode: RecoveryKeyMode,
    password: Entity<InputState>,
    pub step: RecoveryStep,
    pub saved: bool,
    copied: Option<bool>,
    busy: bool,
    error: Option<&'static str>,
    task: Option<Task<()>>,
}

impl RecoveryKeyDialog {
    /// The master password step: Show reads the key, Reissue prepares a new
    /// one (nothing is sent yet).
    pub fn next(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        if self.busy {
            return;
        }
        let password = secret(&self.password, cx);
        let mode = self.mode;
        self.busy = true;
        self.error = None;
        cx.notify();
        let ran = cx.background_spawn(async move {
            match mode {
                RecoveryKeyMode::Show => core
                    .recovery_key_show(password)
                    .map(|shown| (shown.recovery_key, None)),
                RecoveryKeyMode::Reissue => core
                    .recovery_key_reissue_prepare(password)
                    .map(|prepared| (prepared.recovery_key, Some(prepared.id))),
            }
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = ran.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.busy = false;
                match result {
                    Ok((key, id)) => dialog.step = RecoveryStep::Key(key, id),
                    Err(error) => dialog.error = Some(error_text(&error)),
                }
                cx.notify();
            });
        }));
    }

    /// "Reissue" after "I saved the recovery key".
    pub fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let RecoveryStep::Key(_, Some(id)) = self.step else {
            return;
        };
        if !self.saved || self.busy {
            return;
        }
        self.busy = true;
        self.error = None;
        cx.notify();
        let ran = cx.background_spawn(async move { core.key_change_confirm(id) });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = ran.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.busy = false;
                dialog.step = match result {
                    Ok(()) => RecoveryStep::Done,
                    Err(failure) => {
                        tracing::info!(code = failure.error().code(), "reissue failed");
                        RecoveryStep::Failed(failure_text(&failure, false))
                    }
                };
                dialog
                    .state
                    .update(cx, |state, cx| state.refresh_sync_status(cx));
                cx.notify();
            });
        }));
    }

    /// Closing before "Reissue" drops the prepared key: nothing was sent.
    fn cancel(&self) {
        if let (RecoveryStep::Key(_, Some(id)), Some(core)) = (&self.step, &self.core) {
            core.key_change_cancel(*id);
        }
    }
}

impl Drop for RecoveryKeyDialog {
    /// Any way the dialog goes (Cancel, Escape, a lock) drops a prepared key
    /// that was never confirmed; a confirmed one is no longer pending.
    fn drop(&mut self) {
        self.cancel();
    }
}

impl Render for RecoveryKeyDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.step.clone() {
            RecoveryStep::Password => v_flex()
                .gap(metrics::SPACE_2)
                .child(widgets::caption(
                    match self.mode {
                        RecoveryKeyMode::Show => strings::KEY_SHOW_TEXT,
                        RecoveryKeyMode::Reissue => strings::KEY_REISSUE_TEXT,
                    },
                    cx,
                ))
                .child(widgets::field(
                    strings::PASSWORD,
                    widgets::text_input(
                        &self.password,
                        "sync-key-password",
                        self.error.is_some(),
                        window,
                        cx,
                    ),
                    None,
                    cx,
                ))
                .into_any_element(),
            RecoveryStep::Key(key, id) => {
                let target = match id {
                    None => SecretCopyTarget::RecoveryKey,
                    Some(id) => SecretCopyTarget::PendingRecoveryKey { id },
                };
                let this = cx.entity().downgrade();
                let block = key_block(
                    &self.state,
                    &key,
                    target,
                    self.copied,
                    move |cx, copied| {
                        let _ = this.update(cx, |dialog, cx| {
                            dialog.copied = Some(copied);
                            cx.notify();
                        });
                    },
                    cx,
                );
                let saved = self.saved;
                let busy = self.busy;
                v_flex()
                    .gap(metrics::SPACE_2)
                    .child(widgets::caption(
                        match self.mode {
                            RecoveryKeyMode::Show => strings::KEY_SHOWN_TEXT,
                            RecoveryKeyMode::Reissue => strings::KEY_NEW_TEXT,
                        },
                        cx,
                    ))
                    .child(block)
                    .when(self.mode == RecoveryKeyMode::Reissue, |this| {
                        this.child(saved_checkbox(saved, busy, cx, |dialog: &mut Self, on| {
                            dialog.saved = on
                        }))
                    })
                    .into_any_element()
            }
            RecoveryStep::Done => widgets::caption(strings::KEY_REISSUED, cx).into_any_element(),
            RecoveryStep::Failed(text) => div()
                .id("sync-key-failed")
                .test_support()
                .text_size(metrics::FONT_SIZE_CAPTION)
                .text_color(cx.theme().danger)
                .child(text)
                .into_any_element(),
        };
        v_flex()
            .id("sync-recovery-key")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(body)
            .children(error_line(self.error, cx))
    }
}

impl std::fmt::Debug for RecoveryKeyDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecoveryKeyDialog").finish_non_exhaustive()
    }
}

/// Opens Show or Reissue (Settings ▸ Security, with a sync account).
pub fn open_recovery_key(
    state: Entity<AppState>,
    mode: RecoveryKeyMode,
    window: &mut Window,
    cx: &mut App,
) -> Entity<RecoveryKeyDialog> {
    let core = state.read(cx).core().cloned();
    let dialog = cx.new(|cx| RecoveryKeyDialog {
        state,
        core,
        mode,
        password: cx.new(|cx| InputState::new(window, cx).masked(true)),
        step: RecoveryStep::Password,
        saved: false,
        copied: None,
        busy: false,
        error: None,
        task: None,
    });
    let shown = dialog.clone();
    window.open_dialog(cx, move |modal, _, cx| {
        let handle = shown.downgrade();
        let this = shown.read(cx);
        let (busy, saved, step) = (this.busy, this.saved, this.step.clone());
        let row = h_flex().w_full().justify_end().gap(metrics::SPACE_2);
        let finished = matches!(
            (&step, mode),
            (RecoveryStep::Done | RecoveryStep::Failed(_), _)
                | (RecoveryStep::Key(..), RecoveryKeyMode::Show)
        );
        let row = if finished {
            row.child(
                ControlButton::new("sync-key-close", ControlKind::Accent)
                    .label(strings::CLOSE)
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
        } else {
            let cancel = handle.clone();
            let row = row.child(
                ControlButton::new("sync-key-cancel", ControlKind::Neutral)
                    .label(strings::CANCEL)
                    .disabled(busy)
                    .on_click(move |_, window, cx| {
                        if let Some(dialog) = cancel.upgrade() {
                            dialog.read(cx).cancel();
                        }
                        window.close_dialog(cx);
                    }),
            );
            let (id, text, enabled) = match step {
                RecoveryStep::Password => (
                    "sync-key-next",
                    match mode {
                        RecoveryKeyMode::Show => strings::KEY_SHOW_RUN,
                        RecoveryKeyMode::Reissue => strings::CONTINUE,
                    },
                    !busy,
                ),
                _ => ("sync-key-reissue", strings::KEY_REISSUE_RUN, !busy && saved),
            };
            row.child(
                ControlButton::new(id, ControlKind::Accent)
                    .label(text)
                    .disabled(!enabled)
                    .on_click(move |_, window, cx| {
                        let _ = handle.update(cx, |dialog, cx| match dialog.step {
                            RecoveryStep::Password => dialog.next(window, cx),
                            RecoveryStep::Key(..) => dialog.confirm(window, cx),
                            _ => {}
                        });
                    }),
            )
        };
        modal
            .w(metrics::scaled(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            // Not while a request runs: its answer must reach the user.
            .keyboard(!busy)
            .overlay_closable(!busy)
            .title(title(match mode {
                RecoveryKeyMode::Show => strings::KEY_SHOW_TITLE,
                RecoveryKeyMode::Reissue => strings::KEY_REISSUE_TITLE,
            }))
            .child(shown.clone())
            .footer(row)
    });
    dialog
}

// ---------------------------------------------------------------------------
// Change master password (UI 13.4), with "My old password may be
// compromised" on by default (D173); fixed on after a "Lost or stolen"
// revoke (UI 12.2, Devices). With the option and an account, the new
// recovery key is shown and confirmed before the one request of SP 5.9.
// ---------------------------------------------------------------------------

#[derive(Clone, Debug)]
pub enum PasswordStep {
    Form,
    /// The prepared change's new recovery key and id, waiting for "I saved
    /// it".
    Key(SecretString, u64),
    Done,
    Failed(&'static str),
}

pub struct PasswordChangeDialog {
    state: Entity<AppState>,
    /// Drops a prepared change that was never confirmed when the dialog goes.
    core: Option<CoreRef>,
    current: Entity<InputState>,
    new: Entity<InputState>,
    repeat: Entity<InputState>,
    /// "My old password may be compromised"; on by default (D173).
    pub compromised: bool,
    /// After "Lost or stolen": the option is on and cannot be turned off.
    pub fixed: bool,
    /// The device reported lost or stolen: revoked already, and once more
    /// right before the change is sent (SP 4.7).
    lost_device: Option<Uuid>,
    pub step: PasswordStep,
    pub saved: bool,
    copied: Option<bool>,
    /// The recovery key was replaced too (the "Done" text says so).
    reissued: bool,
    /// The option as the change was started with (the "Done" text).
    rotated: bool,
    busy: bool,
    error: Option<&'static str>,
    task: Option<Task<()>>,
}

impl PasswordChangeDialog {
    fn rotate(&self) -> bool {
        self.compromised || self.fixed
    }

    fn bound(&self, cx: &App) -> bool {
        self.state
            .read(cx)
            .sync_status()
            .is_some_and(|status| status.bound)
    }

    /// The form: checks the fields, then changes the password (option off,
    /// or local mode) or prepares the change and shows the new key.
    pub fn run(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        if self.busy {
            return;
        }
        // The passwords stay in wiping buffers (D195).
        let new = zeroize::Zeroizing::new(input_text(&self.new, cx));
        if *new != *zeroize::Zeroizing::new(input_text(&self.repeat, cx)) {
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
        let new = SecretString::from(new.as_str().to_owned());
        let rotate = self.rotate();
        self.rotated = rotate;
        self.busy = true;
        self.error = None;
        cx.notify();
        let done = cx.background_spawn(async move {
            if rotate {
                core.password_change_prepare(current, new)
            } else {
                core.password_change(current, new).map(|()| None)
            }
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = done.await;
            let _ = this.update_in(cx, |dialog, window, cx| {
                dialog.busy = false;
                if result.is_ok() {
                    // The fields are not needed any more: no copy stays in them.
                    for input in [&dialog.current, &dialog.new, &dialog.repeat] {
                        input.update(cx, |input, cx| input.set_value("", window, cx));
                    }
                }
                match result {
                    Ok(Some(prepared)) => {
                        dialog.step = PasswordStep::Key(prepared.recovery_key, prepared.id)
                    }
                    Ok(None) => {
                        dialog.step = PasswordStep::Done;
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

    /// "Change password" on the key step, after "I saved the recovery key":
    /// the one request of SP 5.9.
    pub fn confirm(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(core) = self.state.read(cx).core().cloned() else {
            return;
        };
        let PasswordStep::Key(_, id) = self.step else {
            return;
        };
        if !self.saved || self.busy {
            return;
        }
        let lost = self.lost_device;
        self.busy = true;
        self.error = None;
        cx.notify();
        let ran = cx.background_spawn(async move {
            match lost {
                Some(device_id) => core.key_change_confirm_lost(id, device_id),
                None => core.key_change_confirm(id),
            }
        });
        self.task = Some(cx.spawn_in(window, async move |this, cx| {
            let result = ran.await;
            let _ = this.update(cx, |dialog, cx| {
                dialog.busy = false;
                dialog.step = match result {
                    Ok(()) => {
                        dialog.reissued = true;
                        PasswordStep::Done
                    }
                    Err(failure) => {
                        tracing::info!(code = failure.error().code(), "password change failed");
                        PasswordStep::Failed(failure_text(&failure, true))
                    }
                };
                dialog
                    .state
                    .update(cx, |state, cx| state.refresh_sync_status(cx));
                cx.notify();
            });
        }));
    }

    /// Cancel: nothing of the change is sent (after "Lost or stolen" the
    /// device is revoked already).
    fn cancel(&self) {
        if let (PasswordStep::Key(_, id), Some(core)) = (&self.step, &self.core) {
            core.key_change_cancel(*id);
        }
    }

    fn render_form(&mut self, window: &mut Window, cx: &mut Context<Self>) -> gpui::AnyElement {
        let fixed = self.fixed;
        let bound = self.bound(cx);
        v_flex()
            .gap(metrics::SPACE_3)
            .when(fixed, |this| {
                this.child(
                    div()
                        .id("sync-lost-text")
                        .test_support()
                        .child(widgets::caption(strings::LOST_TEXT, cx)),
                )
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
                    .checked(self.rotate())
                    .disabled(fixed || self.busy)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.compromised = *checked;
                        cx.notify();
                    })),
            )
            .child(widgets::caption(
                match (self.rotate(), bound) {
                    (true, true) => strings::COMPROMISED_HINT,
                    (true, false) => strings::COMPROMISED_HINT_LOCAL,
                    (false, _) => strings::COMPROMISED_OFF_HINT,
                },
                cx,
            ))
            .when(bound, |this| {
                this.child(widgets::caption(strings::SIGNED_OUT_WARNING, cx))
            })
            .into_any_element()
    }
}

impl Render for PasswordChangeDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.step.clone() {
            PasswordStep::Form => self.render_form(window, cx),
            PasswordStep::Key(key, id) => {
                let this = cx.entity().downgrade();
                let block = key_block(
                    &self.state,
                    &key,
                    SecretCopyTarget::PendingRecoveryKey { id },
                    self.copied,
                    move |cx, copied| {
                        let _ = this.update(cx, |dialog, cx| {
                            dialog.copied = Some(copied);
                            cx.notify();
                        });
                    },
                    cx,
                );
                let (saved, busy) = (self.saved, self.busy);
                v_flex()
                    .gap(metrics::SPACE_2)
                    .child(widgets::caption(strings::KEY_NEW_WITH_PASSWORD_TEXT, cx))
                    .child(block)
                    .child(saved_checkbox(saved, busy, cx, |dialog: &mut Self, on| {
                        dialog.saved = on
                    }))
                    .into_any_element()
            }
            PasswordStep::Done => widgets::caption(
                if self.reissued {
                    strings::PASSWORD_CHANGED_REISSUED
                } else if self.rotated {
                    strings::PASSWORD_CHANGED_ROTATED
                } else {
                    strings::PASSWORD_CHANGED
                },
                cx,
            )
            .into_any_element(),
            PasswordStep::Failed(text) => v_flex()
                .gap(metrics::SPACE_2)
                .child(
                    div()
                        .id("sync-key-failed")
                        .test_support()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(cx.theme().danger)
                        .child(text),
                )
                // The device is signed out, but it still holds the old
                // password and recovery key.
                .when(self.fixed, |this| {
                    this.child(widgets::caption(strings::LOST_NOT_CHANGED, cx))
                })
                .into_any_element(),
        };
        v_flex()
            .id("sync-password-change")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_3)
            .child(body)
            .children(error_line(self.error, cx))
    }
}

impl Drop for PasswordChangeDialog {
    /// Any way the dialog goes drops a prepared change never confirmed.
    fn drop(&mut self) {
        self.cancel();
    }
}

impl std::fmt::Debug for PasswordChangeDialog {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PasswordChangeDialog")
            .finish_non_exhaustive()
    }
}

/// Opens the password change; `lost`: the device reported lost or stolen
/// (Devices, UI 12.2), already revoked; revoked again right before the
/// change is sent.
pub fn open_password_change(
    state: Entity<AppState>,
    lost: Option<Uuid>,
    window: &mut Window,
    cx: &mut App,
) -> Entity<PasswordChangeDialog> {
    let core = state.read(cx).core().cloned();
    let dialog = cx.new(|cx| PasswordChangeDialog {
        state,
        core,
        current: cx.new(|cx| InputState::new(window, cx).masked(true)),
        new: cx.new(|cx| InputState::new(window, cx).masked(true)),
        repeat: cx.new(|cx| InputState::new(window, cx).masked(true)),
        compromised: true,
        fixed: lost.is_some(),
        lost_device: lost,
        step: PasswordStep::Form,
        saved: false,
        copied: None,
        reissued: false,
        rotated: false,
        busy: false,
        error: None,
        task: None,
    });
    let shown = dialog.clone();
    window.open_dialog(cx, move |modal, _, cx| {
        let handle = shown.downgrade();
        let this = shown.read(cx);
        let (busy, saved, step) = (this.busy, this.saved, this.step.clone());
        let row = h_flex().w_full().justify_end().gap(metrics::SPACE_2);
        let row = if matches!(step, PasswordStep::Done | PasswordStep::Failed(_)) {
            row.child(
                ControlButton::new("sync-password-close", ControlKind::Accent)
                    .label(strings::CLOSE)
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
        } else {
            let cancel = handle.clone();
            let enabled = !busy && (matches!(step, PasswordStep::Form) || saved);
            row.child(
                ControlButton::new("sync-password-cancel", ControlKind::Neutral)
                    .label(strings::CANCEL)
                    .disabled(busy)
                    .on_click(move |_, window, cx| {
                        if let Some(dialog) = cancel.upgrade() {
                            dialog.read(cx).cancel();
                        }
                        window.close_dialog(cx);
                    }),
            )
            .child(
                ControlButton::new("sync-password-run", ControlKind::Accent)
                    .label(strings::CHANGE_PASSWORD_RUN)
                    .disabled(!enabled)
                    .on_click(move |_, window, cx| {
                        let _ = handle.update(cx, |dialog, cx| match dialog.step {
                            PasswordStep::Form => dialog.run(window, cx),
                            PasswordStep::Key(..) => dialog.confirm(window, cx),
                            _ => {}
                        });
                    }),
            )
        };
        modal
            .w(metrics::scaled(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            // Not while a request runs: its answer must reach the user.
            .keyboard(!busy)
            .overlay_closable(!busy)
            .title(title(strings::CHANGE_PASSWORD_TITLE))
            .child(shown.clone())
            .footer(row)
    });
    dialog
}
