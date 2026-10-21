//! The data transfer states of the screenshot run (night plan, stage 3,
//! items 5-6; UI 1.4 and 2.5): Export Data with encryption turned off (the
//! clear-text warning), Export Data as OpenSSH files (D43), the summary of
//! Import Data and the preview of an OpenSSH config in the import dialog.
//!
//! The files are written into a temporary folder (inside `TEMP`, which the
//! scripts point at their temporary folder) and deleted as soon as the
//! dialog has read them:
//! - Import Data reads a plain export of the demo profile itself, so every
//!   record is already there: the summary must show nothing added and no
//!   conflict copies (the demo of the later states stays the same, and the
//!   run fails if the round trip ever changes the profile);
//! - the import dialog reads [`sample_config`] from a `.ssh` folder whose
//!   parent is `~` for it: documentation addresses, a key file made at run
//!   time and protected by a passphrase, a key file that does not exist, a
//!   `ProxyCommand` (not imported) and a `Host *` template. Nothing is
//!   imported.

use std::time::{Duration, Instant};

use gpui::{AnyWindowHandle, App, AppContext as _, AsyncApp, Entity};
use gpui_kit::test::TestWindowExt as _;
use secrecy::SecretString;
use shellpp_core::keys::generate::{KeyKind, generate};
use shellpp_core::service::dto::DataExportInput;

use super::{ScreenshotError, ShotState, click, settle};
use crate::data_transfer::{self, ImportStep};
use crate::import_sources::{self, Step};
use crate::profile::TempProfile;
use crate::state::AppState;

const TIMEOUT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(50);
/// The switch of Export Data.
const EXPORT_ENCRYPT: &str = "data-export-encrypt";
/// Next of the import dialog.
const IMPORT_NEXT: &str = "import-next";
/// The made-up passphrase of the sample key file (never typed in).
const SAMPLE_PASSPHRASE: &str = "screenshot passphrase";

/// The OpenSSH config of [`ShotState::ImportSshConfig`].
pub fn sample_config() -> String {
    [
        "# Shell++ screenshots: documentation addresses",
        "Host bastion",
        "  HostName 203.0.113.10",
        "  User ops",
        "  IdentityFile ~/.ssh/id_bastion",
        "",
        "Host app-1 app-2",
        "  HostName %h.internal.example.com",
        "  User deploy",
        "  ProxyJump bastion",
        "",
        "Host backup",
        "  HostName 198.51.100.7",
        "  Port 2222",
        "  IdentityFile ~/.ssh/id_backup",
        "",
        "Host legacy",
        "  HostName 192.0.2.50",
        "  ProxyCommand nc %h %p",
        "",
        "Host *",
        "  ServerAliveInterval 30",
        "",
    ]
    .join("\n")
}

/// From the reset demo window: the dialog of `shot_state` in the state the
/// picture shows.
pub async fn reach(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    state: &Entity<AppState>,
    shot_state: ShotState,
) -> Result<(), ScreenshotError> {
    match shot_state {
        ShotState::ExportData => export_default(cx, handle, state).await,
        ShotState::ExportDataPlain => export_plain(cx, handle, state).await,
        ShotState::ExportOpenSsh => export_openssh(cx, handle, state).await,
        ShotState::ImportData => import_summary(cx, handle, state).await,
        ShotState::ImportSshConfig => config_preview(cx, handle, state).await,
        _ => Err(ScreenshotError::state(
            shot_state,
            "not a data transfer state",
        )),
    }
}

/// Export Data as it opens: encrypted with a password (the default).
async fn export_default(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    state: &Entity<AppState>,
) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        data_transfer::open_export(state.clone(), window, cx);
    })
    .map_err(|_| ScreenshotError::WindowClosed)?;
    settle(cx, handle).await
}

/// Export Data, "Encrypt with a password" turned off: the warning and
/// "I understand".
async fn export_plain(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    state: &Entity<AppState>,
) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        data_transfer::open_export(state.clone(), window, cx);
    })
    .map_err(|_| ScreenshotError::WindowClosed)?;
    settle(cx, handle).await?;
    click(cx, handle, EXPORT_ENCRYPT)
}

/// Export Data with the format "OpenSSH files": the key files and their
/// passphrase fields.
async fn export_openssh(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    state: &Entity<AppState>,
) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        let dialog = data_transfer::open_export(state.clone(), window, cx);
        dialog.update(cx, |dialog, cx| {
            dialog.set_format(data_transfer::ExportFormat::OpenSsh, cx)
        });
    })
    .map_err(|_| ScreenshotError::WindowClosed)?;
    settle(cx, handle).await
}

/// Import Data of a plain export of the demo itself, up to its summary.
async fn import_summary(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    state: &Entity<AppState>,
) -> Result<(), ScreenshotError> {
    let shot = ShotState::ImportData;
    let not_reached = |reason: &str| ScreenshotError::state(shot, reason);
    let core = cx
        .update(|cx| state.read(cx).core().cloned())
        .ok_or_else(|| not_reached("no core"))?;
    let export = core
        .data_export(DataExportInput::default())
        .map_err(|error| not_reached(&format!("export: {error}")))?;
    let folder = TempProfile::create("shellpp-shots-import-data")
        .map_err(|error| not_reached(&format!("temporary folder: {error}")))?;
    let file = folder.path().join(&export.file_name);
    std::fs::write(&file, secrecy::ExposeSecret::expose_secret(&export.text))
        .map_err(|error| not_reached(&format!("export file: {error}")))?;
    drop(export);
    let dialog = cx
        .update_window(handle, |_, window, cx| {
            let dialog = data_transfer::open_import(state.clone(), window, cx);
            dialog.update(cx, |dialog, cx| {
                dialog.set_path(&file, window, cx);
                dialog.run(window, cx);
            });
            dialog
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    let reached = wait_for(cx, handle, shot, "the import summary", |cx| {
        matches!(dialog.read(cx).step(), ImportStep::Summary(_))
    })
    .await;
    // The file holds the demo's secrets in clear text: gone right away.
    drop(folder);
    reached?;
    let summary = cx.update(|cx| match dialog.read(cx).step() {
        ImportStep::Summary(summary) => Some(summary.clone()),
        _ => None,
    });
    let Some(summary) = summary else {
        return Err(not_reached("no summary"));
    };
    if summary.existing == 0 || summary.added != 0 || summary.conflicts != 0 {
        return Err(not_reached(&format!(
            "the demo's own export changed the profile: {} added, {} already here, {} copies",
            summary.added, summary.existing, summary.conflicts
        )));
    }
    if !summary.refused.is_empty() {
        return Err(not_reached("the import refused records of the demo"));
    }
    Ok(())
}

/// The import dialog on [`sample_config`], up to its preview.
async fn config_preview(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    state: &Entity<AppState>,
) -> Result<(), ScreenshotError> {
    let shot = ShotState::ImportSshConfig;
    let not_reached = |reason: &str| ScreenshotError::state(shot, reason);
    let folder = TempProfile::create("shellpp-shots-ssh-config")
        .map_err(|error| not_reached(&format!("temporary folder: {error}")))?;
    let ssh = folder.path().join(".ssh");
    let passphrase = SecretString::from(SAMPLE_PASSPHRASE);
    let key = generate(KeyKind::Ed25519, "bastion", Some(&passphrase))
        .map_err(|error| not_reached(&format!("sample key: {error}")))?;
    let written = std::fs::create_dir_all(&ssh)
        .and_then(|()| std::fs::write(ssh.join("config"), sample_config()))
        .and_then(|()| std::fs::write(ssh.join("id_bastion"), key.as_bytes()));
    written.map_err(|error| not_reached(&format!("sample files: {error}")))?;
    let config = ssh.join("config");
    let dialog = cx
        .update_window(handle, |_, window, cx| {
            import_sources::open(state.clone(), window, cx)
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    let reached = async {
        settle(cx, handle).await?;
        // Source (OpenSSH config) -> File: the sample instead of the
        // suggested ~/.ssh/config, then the preview.
        click(cx, handle, IMPORT_NEXT)?;
        settle(cx, handle).await?;
        cx.update_window(handle, |_, window, cx| {
            dialog.update(cx, |dialog, cx| dialog.set_path(&config, window, cx));
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
        click(cx, handle, IMPORT_NEXT)?;
        wait_for(cx, handle, shot, "the preview", |cx| {
            *dialog.read(cx).step() == Step::Preview
        })
        .await
    }
    .await;
    drop(folder);
    reached?;
    let hosts = cx.update(|cx| {
        dialog
            .read(cx)
            .preview()
            .map_or(0, |preview| preview.hosts.len())
    });
    if hosts < 4 {
        return Err(not_reached("the preview lists fewer hosts than the sample"));
    }
    Ok(())
}

/// Polls `condition` after a fresh frame until it holds.
async fn wait_for(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    shot: ShotState,
    what: &str,
    mut condition: impl FnMut(&mut App) -> bool,
) -> Result<(), ScreenshotError> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .map_err(|_| ScreenshotError::WindowClosed)?;
        if cx.update(|cx| condition(cx)) {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(ScreenshotError::state(shot, &format!("timeout: {what}")));
        }
        cx.background_executor().timer(POLL).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sample config names the hosts the state expects: four concrete
    /// aliases plus the `Host *` template, with documentation addresses.
    #[test]
    fn the_sample_config_has_every_kind_of_host() {
        let text = sample_config();
        for alias in ["bastion", "app-1 app-2", "backup", "legacy", "*"] {
            assert!(text.contains(&format!("Host {alias}\n")), "{alias}");
        }
        assert!(text.contains("ProxyCommand"));
        assert!(text.contains("id_backup"), "a key file that is missing");
    }
}
