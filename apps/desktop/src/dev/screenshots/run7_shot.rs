//! The run 7 states of the screenshot run (plan, sections 2 and 3.5): the
//! top bar with the update button of an update found (D100, item 4), and the
//! sync interface of stage 7: the connect wizard, the certificate trust
//! dialog (a made-up certificate: no server is asked), Settings ▸ Account &
//! Sync of a bound account (the status set by hand), the Recently deleted
//! screen (UI 14) after two hosts of the demo were deleted, Review, the
//! duplicates of UI 13.7: a copy of a demo host as another device would
//! have imported it, listed as a pair as a pull would list it
//! (`Core::sync_duplicates_scan`), with the Hosts screen's hint and the
//! pair side by side, and the comparison of a sync conflict (UI 13.6) over
//! the host card's badge.
//!
//! The update state sets the update found of the run by hand (no update
//! source is asked); the reset clears it ([`reset`]), so the other states
//! keep a top bar without the button. The conflict state writes a conflict
//! copy through the core's test hook (no server); the reset keeps the
//! current version ([`drop_conflicts`]), so no other state shows a badge.

use gpui::{AnyWindowHandle, AppContext as _, AsyncApp, Entity};
use gpui_kit::test::TestWindowExt as _;

use super::{ScreenshotError, ShotState, settle, shell_of};
use crate::app::RootView;
use crate::state::AppState;

/// The states of this module.
pub const STATES: [ShotState; 9] = [
    ShotState::TopBarUpdate,
    ShotState::SyncConnect,
    ShotState::SyncTrust,
    ShotState::SyncAccount,
    ShotState::RecentlyDeleted,
    ShotState::SyncReview,
    ShotState::HostsDuplicates,
    ShotState::SyncDuplicates,
    ShotState::SyncConflict,
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

/// Resolves the conflicts of the conflict state ("Keep this"): the demo
/// is as before and no card shows a badge.
pub fn drop_conflicts(state: &Entity<AppState>, cx: &mut gpui::App) {
    let Some(core) = state.read(cx).core().cloned() else {
        return;
    };
    let conflicts = core.sync_conflicts().unwrap_or_default();
    if conflicts.is_empty() {
        return;
    }
    for item in conflicts {
        let _ = core.sync_conflict_resolve(
            item.conflict_id,
            shellpp_core::service::ConflictChoice::Current,
        );
    }
    state.update(cx, |state, cx| state.refresh_sync_status(cx));
}

/// The host of the conflict state: the last card of the grid's first row,
/// so its badge stays in view beside the dialog.
fn shown_host(shell: &Entity<crate::shell::Shell>, cx: &gpui::App) -> Option<uuid::Uuid> {
    let screen = shell.read(cx).hosts_screen().read(cx);
    let ids = screen.visible_host_ids();
    let last_of_row = screen.columns().clamp(1, ids.len().max(1)) - 1;
    ids.get(last_of_row).or(ids.first()).copied()
}

/// The stored host `id` (its data without secrets).
fn host_view(
    core: &shellpp_core::Core,
    id: uuid::Uuid,
) -> Option<shellpp_core::service::dto::HostView> {
    use shellpp_core::service::dto::{QueryScope, RecordType, RecordView, RecordsQuery, SortMode};
    core.records_query(RecordsQuery {
        vault_id: None,
        types: vec![RecordType::Host],
        scope: QueryScope::All,
        text: None,
        tags: Vec::new(),
        sort: SortMode::Manual,
    })
    .ok()?
    .into_iter()
    .find_map(|view| match view {
        RecordView::Host(host) if host.record_id == id => Some(host),
        _ => None,
    })
}

/// Sync kept another device's version of host `id`: renamed, on another
/// port, with a password of its own (shown masked).
fn write_conflict(state: &Entity<AppState>, id: uuid::Uuid, cx: &mut gpui::App) {
    let Some(core) = state.read(cx).core().cloned() else {
        return;
    };
    let Some(host) = host_view(&core, id) else {
        return;
    };
    let mut data = host.data.as_object().cloned().unwrap_or_default();
    let name = ["label", "address"]
        .iter()
        .find_map(|member| {
            data.get(*member)
                .and_then(serde_json::Value::as_str)
                .filter(|text| !text.is_empty())
        })
        .unwrap_or("host")
        .to_owned();
    data.insert("label".into(), format!("{name} (laptop)").into());
    let ssh = data
        .entry("ssh")
        .or_insert_with(|| serde_json::Value::Object(Default::default()));
    if let Some(ssh) = ssh.as_object_mut() {
        ssh.insert("port".into(), 2222.into());
        // A made-up password: the comparison shows it masked.
        ssh.insert("password".into(), uuid::Uuid::now_v7().to_string().into());
    }
    let changed = std::time::SystemTime::now() - std::time::Duration::from_secs(3 * 3600);
    if let Err(error) = core.sync_conflict_insert_for_test(
        host.record_id,
        Some(data),
        uuid::Uuid::now_v7(),
        changed,
    ) {
        tracing::warn!(code = error.code(), "demo conflict not written");
    }
}

/// The bound account the Account & Sync state shows.
pub(super) fn bound_status() -> shellpp_core::service::SyncStatus {
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
pub(super) fn certificate() -> shellpp_core::sync::TlsProblem {
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
                        dialog.show_trust("https://sync.home.arpa".to_owned(), certificate(), cx);
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
        ShotState::SyncConflict => {
            let shell = shell_of(cx, root, shot)?;
            let host = cx
                .update(|cx| shown_host(&shell, cx))
                .ok_or_else(|| failed(shot, "no host on the demo's grid"))?;
            let state = _state.clone();
            cx.update(|cx| write_conflict(&state, host, cx));
            // The badge comes with the status the core's event refreshes.
            settle(cx, handle).await?;
            let state = _state.clone();
            cx.update_window(handle, move |_, window, cx| {
                crate::sync_ui::compare::open_for_record(state, host, window, cx);
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await
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
        ShotState::SyncConflict if shown(cx, "sync-compare")? => Ok(()),
        ShotState::HostsDuplicates if shown(cx, "hosts-duplicates")? => Ok(()),
        ShotState::SyncDuplicates if shown(cx, "sync-duplicates")? => Ok(()),
        ShotState::SyncConnect
        | ShotState::SyncTrust
        | ShotState::SyncAccount
        | ShotState::RecentlyDeleted
        | ShotState::SyncReview
        | ShotState::HostsDuplicates
        | ShotState::SyncConflict
        | ShotState::SyncDuplicates => Err(failed(shot, "the sync state is not shown")),
        _ => Err(failed(shot, "not a run 7 state")),
    }
}
