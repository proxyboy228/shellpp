//! The run 7 states of the screenshot run (plan, sections 2 and 3.5): the
//! top bar with the update button of an update found (D100, item 4), and the
//! sync interface of stage 7: the connect wizard, the certificate trust
//! dialog (a made-up certificate: no server is asked), Settings ▸ Account &
//! Sync of a bound account (the status set by hand), the Recently deleted
//! screen (UI 14) after two hosts of the demo were deleted, Review, and the
//! duplicates of UI 13.7: a copy of a demo host as another device would
//! have imported it, listed as a pair as a pull would list it
//! (`Core::sync_duplicates_scan`), with the Hosts screen's hint and the
//! pair side by side.
//!
//! The update state sets the update found of the run by hand (no update
//! source is asked); the reset clears it ([`reset`]), so the other states
//! keep a top bar without the button.

use gpui::{AnyWindowHandle, AppContext as _, AsyncApp, Entity};
use gpui_kit::test::TestWindowExt as _;

use super::{ScreenshotError, ShotState, settle, shell_of};
use crate::app::RootView;
use crate::state::AppState;

/// The states of this module.
pub const STATES: [ShotState; 8] = [
    ShotState::TopBarUpdate,
    ShotState::SyncConnect,
    ShotState::SyncTrust,
    ShotState::SyncAccount,
    ShotState::RecentlyDeleted,
    ShotState::SyncReview,
    ShotState::HostsDuplicates,
    ShotState::SyncDuplicates,
];

/// The version the update state shows.
const VERSION: &str = "0.2.0";

fn failed(shot: ShotState, reason: &str) -> ScreenshotError {
    ScreenshotError::state(shot, reason)
}

/// What the copy of the duplicates states adds to the demo host's label.
const COPY_SUFFIX: &str = " (laptop)";

/// Clears what these states set (the reset of the demo window calls it).
pub fn reset(cx: &mut gpui::App) {
    cx.set_global(crate::updates::Found(None));
}

/// Removes the copy of the duplicates states for good, and keeps any other
/// pair the scan listed (the reset of the demo window calls it).
pub fn remove_duplicate(state: &Entity<AppState>, cx: &mut gpui::App) {
    let ids: Vec<uuid::Uuid> = state
        .read(cx)
        .hosts()
        .iter()
        .filter(|host| host.title().ends_with(COPY_SUFFIX))
        .map(|host| host.record_id)
        .collect();
    if !ids.is_empty() {
        let _ = state.update(cx, |state, cx| state.delete_records(ids.clone(), cx));
    }
    if let Some(core) = state.read(cx).core().cloned() {
        if !ids.is_empty() {
            let _ = core.recently_deleted_delete(&ids);
        }
        for pair in core.sync_duplicates().unwrap_or_default() {
            let _ = core.sync_duplicates_keep(pair.record_id, pair.other_record_id);
        }
    }
    state.update(cx, |state, cx| state.refresh_sync_status(cx));
}

/// A copy of the first demo host with another label, a note and a saved
/// password (generated), then the pairs listed.
fn add_duplicate(state: &Entity<AppState>, cx: &mut gpui::App) {
    use shellpp_core::service::dto::{
        QueryScope, RecordSaveInput, RecordType, RecordView, RecordsQuery, SortMode,
    };
    let Some(core) = state.read(cx).core().cloned() else {
        return;
    };
    let Some(RecordView::Host(host)) = core
        .records_query(RecordsQuery {
            vault_id: None,
            types: vec![RecordType::Host],
            scope: QueryScope::All,
            text: None,
            tags: Vec::new(),
            sort: SortMode::Name,
        })
        .ok()
        .and_then(|hosts| hosts.into_iter().next())
    else {
        return;
    };
    let mut data = host.data.as_object().cloned().unwrap_or_default();
    data.remove("order_key");
    let label = data
        .get("label")
        .and_then(serde_json::Value::as_str)
        .filter(|label| !label.is_empty())
        .or_else(|| data.get("address").and_then(serde_json::Value::as_str))
        .unwrap_or_default()
        .to_owned();
    data.insert("label".into(), format!("{label}{COPY_SUFFIX}").into());
    data.insert(
        "notes".into(),
        "Imported from ~/.ssh/config on the laptop".into(),
    );
    let password = shellpp_core::crypto::random_array::<12>()
        .map(|bytes| bytes.iter().map(|b| format!("{b:02x}")).collect::<String>())
        .unwrap_or_default();
    let secrets = std::collections::BTreeMap::from([(
        "ssh.password".to_owned(),
        Some(secrecy::SecretString::from(password)),
    )]);
    let _ = core.record_save(RecordSaveInput {
        vault_id: host.vault_id,
        record_id: None,
        record_type: RecordType::Host,
        data,
        secrets,
    });
    let _ = core.sync_duplicates_scan();
    state.update(cx, |state, cx| state.refresh_sync_status(cx));
}

/// The bound account the Account & Sync state shows.
fn bound_status() -> shellpp_core::service::SyncStatus {
    shellpp_core::service::SyncStatus {
        bound: true,
        signed_in: true,
        server_url: Some("https://sync.example.com".into()),
        email: Some("demo@example.com".into()),
        last_sync_at: Some("2026-10-23T09:15:00Z".into()),
        pending: 1,
        conflicts: 1,
        ..Default::default()
    }
}

/// A made-up certificate for the trust dialog.
fn certificate() -> shellpp_core::sync::TlsProblem {
    let der = vec![0x30, 0x82, 0x01, 0x0a];
    shellpp_core::sync::TlsProblem::UnknownIssuer(shellpp_core::sync::CertInfo {
        subject: "CN=sync.home.arpa".into(),
        issuer: "CN=Caddy Local Authority - ECC Intermediate".into(),
        not_before: "2026-10-23T00:00:00Z".into(),
        not_after: "2026-10-23T12:00:00Z".into(),
        sha256: shellpp_core::sync::tls::sha256(&der),
        der,
    })
}

/// From the reset demo window: the state's own steps.
pub async fn reach(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    _state: &Entity<AppState>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    shell_of(cx, root, shot)?;
    match shot {
        ShotState::TopBarUpdate => {
            cx.update(|cx| {
                cx.set_global(crate::updates::Found(Some((
                    VERSION.to_owned(),
                    format!("https://github.com/proxyboy228/shellpp/releases/tag/v{VERSION}"),
                ))));
            });
            settle(cx, handle).await
        }
        ShotState::SyncConnect | ShotState::SyncTrust => {
            let state = _state.clone();
            let trust = shot == ShotState::SyncTrust;
            cx.update_window(handle, move |_, window, cx| {
                let dialog = crate::sync_ui::open_connect(state, false, window, cx);
                if trust {
                    dialog.update(cx, |dialog, cx| {
                        dialog.step = crate::sync_ui::ConnectStep::Trust(certificate());
                        cx.notify();
                    });
                }
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await
        }
        ShotState::SyncAccount => {
            let state = _state.clone();
            cx.update_window(handle, move |_, window, cx| {
                state.update(cx, |state, cx| state.set_sync_status(bound_status(), cx));
                crate::settings::open(state, window, cx);
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await?;
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click(gpui::SharedString::from("settings-nav-account"), cx);
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await
        }
        ShotState::RecentlyDeleted => {
            // Two hosts of the demo deleted, the screen opened from the
            // sidebar, the newest one selected: its read-only panel.
            let state = _state.clone();
            let deleted = cx
                .update_window(handle, move |_, window, cx| {
                    let core = state.read(cx).core().cloned()?;
                    let vault = core.vaults().ok()?.first()?.vault_id;
                    let hosts = core
                        .records_query(shellpp_core::service::dto::RecordsQuery {
                            vault_id: None,
                            types: vec![shellpp_core::service::dto::RecordType::Host],
                            scope: shellpp_core::service::dto::QueryScope::All,
                            text: None,
                            tags: Vec::new(),
                            sort: shellpp_core::service::dto::SortMode::Name,
                        })
                        .ok()?;
                    let mut last = None;
                    for host in hosts.iter().take(2) {
                        core.record_delete(shellpp_core::service::dto::RecordsRef {
                            vault_id: vault,
                            record_ids: vec![host.record_id()],
                        })
                        .ok()?;
                        last = Some(host.record_id());
                    }
                    crate::shell::open_section(crate::shell::Section::RecentlyDeleted, window, cx);
                    last
                })
                .map_err(|_| ScreenshotError::WindowClosed)?
                .ok_or_else(|| failed(shot, "no demo host to delete"))?;
            settle(cx, handle).await?;
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.click(crate::trash::row_id(deleted), cx);
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await
        }
        ShotState::SyncReview => {
            let state = _state.clone();
            cx.update_window(handle, move |_, window, cx| {
                crate::sync_ui::open_review(state, window, cx);
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await
        }
        ShotState::HostsDuplicates | ShotState::SyncDuplicates => {
            let state = _state.clone();
            cx.update(|cx| add_duplicate(&state, cx));
            settle(cx, handle).await?;
            if shot == ShotState::SyncDuplicates {
                cx.update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    window.click(gpui::SharedString::from("hosts-duplicates-review"), cx);
                })
                .map_err(|_| ScreenshotError::WindowClosed)?;
                settle(cx, handle).await?;
            }
            Ok(())
        }
        _ => Err(failed(shot, "not a run 7 state")),
    }
}

/// Checks that the window shows `shot`.
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
        ShotState::TopBarUpdate if shown(cx, "top-bar-update")? => Ok(()),
        ShotState::TopBarUpdate => Err(failed(shot, "no update button in the top bar")),
        ShotState::SyncConnect if shown(cx, "sync-server")? => Ok(()),
        ShotState::SyncTrust if shown(cx, "sync-fingerprint")? => Ok(()),
        ShotState::SyncAccount if shown(cx, "sync-account-email")? => Ok(()),
        ShotState::RecentlyDeleted if shown(cx, "trash-panel")? => Ok(()),
        ShotState::SyncReview if shown(cx, "sync-review")? => Ok(()),
        ShotState::HostsDuplicates if shown(cx, "hosts-duplicates")? => Ok(()),
        ShotState::SyncDuplicates if shown(cx, "sync-duplicates")? => Ok(()),
        ShotState::SyncConnect
        | ShotState::SyncTrust
        | ShotState::SyncAccount
        | ShotState::RecentlyDeleted
        | ShotState::SyncReview
        | ShotState::HostsDuplicates
        | ShotState::SyncDuplicates => Err(failed(shot, "the sync state is not shown")),
        _ => Err(failed(shot, "not a run 7 state")),
    }
}
