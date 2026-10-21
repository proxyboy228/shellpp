//! The stage 6 states of the screenshot run (UI 11, 12.6; D67): the Logs
//! screen with the demo's connection log and an entry's details, and the
//! Logs section of the Settings window.
//!
//! The log entries are written into the demo profile for the run (as the
//! session layer writes them); nothing connects.

use gpui::{AnyWindowHandle, AppContext as _, AsyncApp, Entity};
use gpui_kit::test::TestWindowExt as _;
use shellpp_core::service::dto::RecordRef;
use shellpp_core::service::host_api::{ConnectionEndReason, ConnectionLogStart};

use super::{ScreenshotError, ShotState, settle, shell_of};
use crate::app::RootView;
use crate::shell::{Section, TopTab};
use crate::state::AppState;

/// The states of this module.
pub const STATES: [ShotState; 2] = [ShotState::Logs, ShotState::SettingsLogs];

fn not_reached(state: ShotState, reason: &str) -> ScreenshotError {
    ScreenshotError::state(state, reason)
}

/// A few sessions of the demo's hosts, once: ended, dropped, one still
/// open and one saved.
fn seed(state: &Entity<AppState>, cx: &mut gpui::App) -> Option<()> {
    let core = state.read(cx).core()?.clone();
    if !core.connection_log_list().ok()?.is_empty() {
        return Some(());
    }
    let hosts: Vec<(RecordRef, String, String)> = state
        .read(cx)
        .hosts()
        .iter()
        .take(4)
        .map(|host| {
            (
                RecordRef {
                    vault_id: host.vault_id,
                    record_id: host.record_id,
                },
                host.title().to_owned(),
                host.address.clone(),
            )
        })
        .collect();
    let ends = [
        Some((ConnectionEndReason::Closed, None)),
        Some((ConnectionEndReason::ExitStatus, Some(1))),
        Some((ConnectionEndReason::Dropped, None)),
        None,
    ];
    let mut first = None;
    for ((host, title, address), end) in hosts.into_iter().zip(ends) {
        let id = core
            .connection_log_start(ConnectionLogStart {
                host: Some(host),
                host_label: Some(title),
                address,
                port: 22,
                username: Some("deploy".into()),
                os: None,
            })
            .ok()?;
        first.get_or_insert(id);
        if let Some((reason, status)) = end {
            core.connection_log_end(id, reason, status);
        }
    }
    core.connection_log_set_saved(first?, true).ok()
}

/// From the reset demo window: the state's own steps.
pub async fn reach(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let shell = shell_of(cx, root, shot)?;
    cx.update(|cx| seed(state, cx))
        .ok_or_else(|| not_reached(shot, "the demo log could not be written"))?;
    cx.update(|cx| {
        shell.update(cx, |shell, cx| {
            shell.set_top_tab(TopTab::Vaults, cx);
            shell.set_section(Section::Logs, cx);
        })
    });
    let screen = cx.update(|cx| shell.read(cx).logs_screen().clone());
    cx.update(|cx| screen.update(cx, |screen, cx| screen.reload(cx)));
    settle(cx, handle).await?;
    match shot {
        ShotState::Logs => {
            let first =
                cx.update(|cx| screen.read(cx).shown().first().map(|entry| entry.record_id));
            let first = first.ok_or_else(|| not_reached(shot, "the log is empty"))?;
            cx.update(|cx| screen.update(cx, |screen, cx| screen.select(Some(first), cx)));
            settle(cx, handle).await
        }
        ShotState::SettingsLogs => {
            let state = state.clone();
            cx.update_window(handle, |_, window, cx| {
                screen.update(cx, |screen, cx| screen.select(None, cx));
                crate::settings::open(state, window, cx);
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await?;
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click("settings-nav-logs", cx);
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await
        }
        _ => Err(not_reached(shot, "not a stage 6 state")),
    }
}

/// Checks that the window shows `shot`.
pub fn check(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let wanted: &[&str] = match shot {
        ShotState::Logs => &["log-rows", "log-panel"],
        ShotState::SettingsLogs => &["settings-logs"],
        _ => return Err(not_reached(shot, "not a stage 6 state")),
    };
    for id in wanted {
        let shown = cx
            .update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.try_find(*id).is_some_and(|found| found.visible())
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
        if !shown {
            return Err(not_reached(shot, &format!("{id} is not shown")));
        }
    }
    Ok(())
}
