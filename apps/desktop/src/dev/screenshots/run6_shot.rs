//! The run 6 states of the screenshot run (plan, sections 5 and 6): the new
//! tab page with a workspace that links a host that is gone (D94).
//!
//! The state saves a workspace of its own, `Ops (lab)`, linking two demo
//! hosts and one host that does not exist, named "old-db (lab)" as if it had
//! been deleted after the save; the reset removes it
//! ([`remove_workspace`]), so the other states keep the demo as it is.

use gpui::{AnyWindowHandle, AppContext as _, AsyncApp, Entity};
use gpui_kit::test::TestWindowExt as _;
use serde_json::json;

use super::{ScreenshotError, ShotState, click, settle, shell_of};
use crate::app::RootView;
use crate::state::AppState;

/// The states of this module.
pub const STATES: [ShotState; 1] = [ShotState::NewTabWorkspacesMissing];

/// The name of the workspace of these states.
const WORKSPACE_NAME: &str = "Ops (lab)";
/// The host that is gone, as the workspace saved it.
const MISSING_HOST: &str = "old-db (lab)";
/// A host id no record has.
const MISSING_ID: &str = "0199f6a1-0000-7000-8000-00000000dead";

fn failed(shot: ShotState, reason: &str) -> ScreenshotError {
    ScreenshotError::state(shot, reason)
}

/// Saves the workspace of these states; returns its record id.
fn save_workspace(
    state: &Entity<AppState>,
    cx: &mut AsyncApp,
    shot: ShotState,
) -> Result<uuid::Uuid, ScreenshotError> {
    cx.update(|cx| {
        let state = state.read(cx);
        let core = state.core()?.clone();
        let vault_id = state.vault_id()?;
        let hosts: Vec<_> = state
            .hosts()
            .iter()
            .take(2)
            .map(|host| host.record_id)
            .collect();
        let link = |id: String| json!({"vault_id": vault_id.to_string(), "record_id": id});
        let mut tabs: Vec<_> = hosts
            .iter()
            .map(|id| json!({"root": {"session": {"kind": "ssh", "host": link(id.to_string())}}}))
            .collect();
        tabs.push(
            json!({"root": {"session": {"kind": "ssh", "host": link(MISSING_ID.to_owned())}}}),
        );
        let data = json!({
            "name": WORKSPACE_NAME,
            "tabs": tabs,
            "host_names": [{"host": link(MISSING_ID.to_owned()), "name": MISSING_HOST}],
        });
        core.workspace_save(shellpp_core::service::snippets::RecordInput {
            vault_id,
            record_id: None,
            data: data.as_object().cloned().unwrap_or_default(),
        })
        .ok()
        .map(|view| view.record.record_id)
    })
    .ok_or_else(|| failed(shot, "the workspace was not saved"))
}

/// Removes the workspace of these states, if a state saved it (the reset of
/// the demo window calls it).
pub fn remove_workspace(state: &Entity<AppState>, cx: &mut gpui::App) {
    let ids: Vec<uuid::Uuid> = state
        .read(cx)
        .workspaces()
        .iter()
        .filter(|workspace| workspace.name.as_ref() == WORKSPACE_NAME)
        .map(|workspace| workspace.id)
        .collect();
    if !ids.is_empty() {
        let _ = state.update(cx, |state, cx| state.delete_records(ids, cx));
    }
}

/// From the reset demo window: the state's own steps.
pub async fn reach(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    shell_of(cx, root, shot)?;
    match shot {
        ShotState::NewTabWorkspacesMissing => {
            save_workspace(state, cx, shot)?;
            cx.update(|cx| state.update(cx, |state, cx| state.reload_records(cx)));
            settle(cx, handle).await?;
            click(cx, handle, "tab-new")?;
            settle(cx, handle).await
        }
        _ => Err(failed(shot, "not a run 6 state")),
    }
}

/// Checks that the window shows `shot`.
pub fn check(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    state: &Entity<AppState>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let id = cx
        .update(|cx| {
            state
                .read(cx)
                .workspaces()
                .iter()
                .find(|workspace| workspace.name.as_ref() == WORKSPACE_NAME)
                .map(|workspace| workspace.id)
        })
        .ok_or_else(|| failed(shot, "no workspace"))?;
    let shown = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(gpui::SharedString::from(format!(
                    "new-tab-workspace-missing-{id}"
                )))
                .is_some_and(|found| found.visible())
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    if shown {
        Ok(())
    } else {
        Err(failed(shot, "the workspace's mark is not on screen"))
    }
}
