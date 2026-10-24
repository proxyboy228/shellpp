//! The run 10 states of the screenshot run (plan, section 3.5): the second
//! step of log out with "Also delete the local copy" (D197), with the
//! set-aside copies and backups it lists, and the connect wizard's account
//! step after the server's key derivation settings changed (SP 4.2, D200).
//! Made-up files and settings: nothing is deleted, no server is asked.

use gpui::{AnyWindowHandle, AsyncApp, Entity};

use super::{ScreenshotError, ShotState, settle, shell_of};
use crate::app::RootView;
use crate::state::AppState;

/// The states of this module.
pub const STATES: [ShotState; 2] = [ShotState::SyncSignOutConfirm, ShotState::SyncKdfChanged];

/// From the reset demo window: the state's own steps.
pub async fn reach(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    shell_of(cx, root, shot)?;
    let state = state.clone();
    match shot {
        ShotState::SyncSignOutConfirm => {
            cx.update_window(handle, move |_, window, cx| {
                let dialog = crate::sync_ui::open_sign_out(state, window, cx);
                dialog.update(cx, |dialog, cx| {
                    dialog.delete_local = true;
                    dialog.confirming = Some(vec![
                        shellpp_core::service::LocalCopyFile {
                            name: "shellpp.db.20261020T101500Z.old".into(),
                            set_aside: true,
                            modified: Some("2026-10-20T10:15:00Z".into()),
                            bytes: 3_276_800,
                        },
                        shellpp_core::service::LocalCopyFile {
                            name: "backups/shellpp-20261024T080000Z.db".into(),
                            set_aside: false,
                            modified: Some("2026-10-24T08:00:00Z".into()),
                            bytes: 2_949_120,
                        },
                    ]);
                    cx.notify();
                });
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await
        }
        ShotState::SyncKdfChanged => {
            cx.update_window(handle, move |_, window, cx| {
                let wizard = crate::sync_ui::open_connect(state, false, window, cx);
                wizard.update(cx, |wizard, cx| {
                    wizard.origin = Some("https://sync.home.arpa".into());
                    wizard.step = crate::sync_ui::ConnectStep::Account;
                    wizard.kdf_changed = true;
                    wizard.error = Some(crate::sync_ui::strings::ERROR_KDF_CHANGED);
                    cx.notify();
                });
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await
        }
        _ => Err(ScreenshotError::state(shot, "not a run 10 state")),
    }
}

pub fn check(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let shown = |cx: &mut AsyncApp, id: &'static str| {
        cx.update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(gpui::SharedString::from(id))
                .is_some_and(|found| found.visible())
        })
        .map_err(|_| ScreenshotError::WindowClosed)
    };
    match shot {
        ShotState::SyncSignOutConfirm if shown(cx, "sync-delete-local-files")? => Ok(()),
        ShotState::SyncKdfChanged if shown(cx, "sync-password")? => Ok(()),
        ShotState::SyncSignOutConfirm | ShotState::SyncKdfChanged => Err(ScreenshotError::state(
            shot,
            "the run 10 dialog is not shown",
        )),
        _ => Err(ScreenshotError::state(shot, "not a run 10 state")),
    }
}
