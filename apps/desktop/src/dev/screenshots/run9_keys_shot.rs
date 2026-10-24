//! The run 9 states of the recovery key and the password change (plan,
//! sections 1.3 and 3.2; UI 13.4, 12.2, 13.5 step 4): Show with the key,
//! Reissue with the new key and "I saved the recovery key", Change master
//! password with "My old password may be compromised", the same dialog
//! after "Lost or stolen", and the wizard's key step after a registration.
//!
//! The account is the status set by hand (no server is asked) and every
//! key shown is a random one made here, never the demo vault's own: the
//! dialogs get their step by hand, so nothing is prepared in the core and
//! the reset (which closes every dialog) leaves nothing behind.

use gpui::{AnyWindowHandle, AppContext as _, AsyncApp, Entity};
use gpui_kit::test::TestWindowExt as _;
use secrecy::SecretString;

use super::{ScreenshotError, ShotState, settle};
use crate::app::RootView;
use crate::state::AppState;
use crate::sync_ui::RecoveryKeyMode;
use crate::sync_ui::keys::RecoveryStep;

/// The states of this module.
pub const STATES: [ShotState; 5] = [
    ShotState::RecoveryKeyShow,
    ShotState::RecoveryKeyReissue,
    ShotState::PasswordChange,
    ShotState::LostOrStolen,
    ShotState::SyncRegisteredKey,
];

fn failed(shot: ShotState, reason: &str) -> ScreenshotError {
    ScreenshotError::state(shot, reason)
}

/// A key to show: random, in the format of a real one.
fn made_up_key() -> Result<SecretString, ScreenshotError> {
    let key = shellpp_core::crypto::Key32::generate()
        .map_err(|_| ScreenshotError::state(ShotState::RecoveryKeyShow, "no random key"))?;
    Ok(shellpp_core::crypto::encode_recovery_key(&key))
}

fn bound_status() -> shellpp_core::service::SyncStatus {
    shellpp_core::service::SyncStatus {
        bound: true,
        signed_in: true,
        server_url: Some("https://sync.example.com".into()),
        email: Some("demo@example.com".into()),
        last_sync_at: Some("2026-10-24T08:15:00Z".into()),
        ..Default::default()
    }
}

pub async fn reach(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    _root: &Entity<RootView>,
    state: &Entity<AppState>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let state = state.clone();
    let key = made_up_key()?;
    cx.update_window(handle, move |_, window, cx| {
        state.update(cx, |state, cx| state.set_sync_status(bound_status(), cx));
        match shot {
            ShotState::RecoveryKeyShow | ShotState::RecoveryKeyReissue => {
                let (mode, id) = if shot == ShotState::RecoveryKeyShow {
                    (RecoveryKeyMode::Show, None)
                } else {
                    (RecoveryKeyMode::Reissue, Some(0))
                };
                let dialog = crate::sync_ui::open_recovery_key(state, mode, window, cx);
                dialog.update(cx, |dialog, cx| {
                    dialog.step = RecoveryStep::Key(key, id);
                    cx.notify();
                });
            }
            ShotState::PasswordChange => {
                crate::sync_ui::open_password_change(state, None, window, cx);
            }
            ShotState::LostOrStolen => {
                crate::sync_ui::open_password_change(state, Some(uuid::Uuid::now_v7()), window, cx);
            }
            ShotState::SyncRegisteredKey => {
                let dialog = crate::sync_ui::open_connect(state, false, window, cx);
                dialog.update(cx, |dialog, cx| dialog.show_registered_key(key, cx));
            }
            _ => {}
        }
    })
    .map_err(|_| ScreenshotError::WindowClosed)?;
    settle(cx, handle).await
}

pub fn check(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let shown = |cx: &mut AsyncApp, id: &'static str| {
        cx.update_window(handle, move |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(gpui::SharedString::from(id))
                .is_some_and(|found| found.visible())
        })
        .map_err(|_| ScreenshotError::WindowClosed)
    };
    let evidence: &[&'static str] = match shot {
        ShotState::RecoveryKeyShow => &["sync-recovery-key", "sync-key-text", "sync-key-copy"],
        ShotState::RecoveryKeyReissue => &["sync-key-text", "sync-key-saved", "sync-key-reissue"],
        ShotState::PasswordChange => &["sync-password-change", "sync-compromised"],
        ShotState::LostOrStolen => &["sync-password-change", "sync-lost-text"],
        ShotState::SyncRegisteredKey => &["sync-registered-key", "sync-key-saved", "sync-finish"],
        _ => return Err(failed(shot, "not a run 9 key state")),
    };
    for id in evidence {
        if !shown(cx, id)? {
            return Err(failed(shot, "the key dialog is not shown"));
        }
    }
    Ok(())
}
