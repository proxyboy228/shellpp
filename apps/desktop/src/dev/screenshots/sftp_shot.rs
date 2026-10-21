//! The stage 4 states of the screenshot run (UI 5, 7.3; plan of run 3,
//! section 7): the SFTP tab with a local folder and the empty panel, the
//! host selection, the transfer queue under the name conflict dialog, the
//! Actions menu, the Delete confirmation and the Edit Permissions dialog;
//! the Keychain's Export to host and a key with a certificate.
//!
//! The files are made for the run inside the profile's temporary folder
//! (`tmp/shots`): the user's own folders are never listed. Keys and the CA
//! of the certificate are generated at run time.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use gpui::{AnyWindowHandle, AppContext as _, AsyncApp, Entity};
use gpui_kit::test::TestWindowExt as _;

use super::{ScreenshotError, ShotState, click, settle, shell_of};
use crate::app::RootView;
use crate::sftp::panel::{FilePanel, Load, Side};
use crate::shell::{Section, TopTab};
use crate::state::AppState;

const TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(50);

/// The states of this module.
pub const STATES: [ShotState; 8] = [
    ShotState::SftpLocal,
    ShotState::SftpPicker,
    ShotState::SftpQueue,
    ShotState::SftpActions,
    ShotState::SftpDelete,
    ShotState::SftpPermissions,
    ShotState::KeychainExport,
    ShotState::KeychainCertificate,
];

fn not_reached(state: ShotState, reason: &str) -> ScreenshotError {
    ScreenshotError::state(state, reason)
}

/// The run's folders: a project with a few files and folders, and an empty
/// backup folder for the copies.
fn folders(state: &AppState) -> Option<(PathBuf, PathBuf)> {
    let root = state.tmp_dir()?.join("shots");
    let project = root.join("website");
    let backup = root.join("backup");
    let _ = std::fs::remove_dir_all(&backup);
    std::fs::create_dir_all(&backup).ok()?;
    if !project.is_dir() {
        std::fs::create_dir_all(project.join("assets")).ok()?;
        std::fs::create_dir_all(project.join("src")).ok()?;
        let files: [(&str, usize); 6] = [
            ("index.html", 4_812),
            ("README.md", 1_240),
            ("deploy.sh", 612),
            ("release-2026-10.tar.gz", 8_734_002),
            ("styles.css", 22_480),
            ("favicon.ico", 15_086),
        ];
        for (name, size) in files {
            std::fs::write(project.join(name), vec![b'x'; size]).ok()?;
        }
        std::fs::write(project.join("src").join("main.ts"), b"export {}").ok()?;
    }
    Some((project, backup))
}

/// Polls `condition` after a fresh frame until it holds.
async fn wait_for(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    state: ShotState,
    what: &str,
    mut condition: impl FnMut(&mut gpui::App) -> bool,
) -> Result<(), ScreenshotError> {
    let deadline = Instant::now() + TIMEOUT;
    loop {
        cx.update_window(handle, |_, window, cx| window.render_frame(cx))
            .map_err(|_| ScreenshotError::WindowClosed)?;
        if cx.update(|cx| condition(cx)) {
            return Ok(());
        }
        if Instant::now() > deadline {
            return Err(not_reached(state, &format!("timeout: {what}")));
        }
        cx.background_executor().timer(POLL).await;
    }
}

fn listed(panel: &Entity<FilePanel>, cx: &gpui::App) -> bool {
    let panel = panel.read(cx);
    *panel.load() == Load::Idle && !panel.rows().is_empty()
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
    match shot {
        ShotState::KeychainExport | ShotState::KeychainCertificate => {
            return keychain(cx, handle, root, state, shot).await;
        }
        _ => {}
    }
    let (project, backup) = cx
        .update(|cx| folders(state.read(cx)))
        .ok_or_else(|| not_reached(shot, "no temporary folder"))?;
    cx.update(|cx| shell.update(cx, |shell, cx| shell.set_top_tab(TopTab::Sftp, cx)));
    settle(cx, handle).await?;
    let screen = cx.update(|cx| shell.read(cx).sftp_screen().clone());
    let left = cx.update(|cx| screen.read(cx).panel(Side::Left).clone());
    let right = cx.update(|cx| screen.read(cx).panel(Side::Right).clone());
    cx.update(|cx| {
        left.update(cx, |panel, cx| {
            panel.open_local(project.clone(), cx);
        });
        right.update(cx, |panel, cx| panel.close(cx));
        let transfers = screen.read(cx).transfers().clone();
        transfers.update(cx, |transfers, cx| transfers.clear_all(cx));
    });
    wait_for(cx, handle, shot, "the project listed", |cx| {
        listed(&left, cx)
    })
    .await?;
    match shot {
        ShotState::SftpLocal => {
            cx.update(|cx| {
                left.update(cx, |panel, cx| {
                    panel.click(
                        "index.html",
                        crate::state::selection::SelectMode::Replace,
                        cx,
                    )
                })
            });
            settle(cx, handle).await
        }
        ShotState::SftpPicker => {
            click(cx, handle, "sftp-right-choose-host")?;
            settle(cx, handle).await
        }
        ShotState::SftpQueue => {
            cx.update(|cx| right.update(cx, |panel, cx| panel.open_local(backup.clone(), cx)));
            wait_for(cx, handle, shot, "the backup folder", |cx| {
                *right.read(cx).load() == Load::Idle
            })
            .await?;
            let entries = |cx: &gpui::App, names: &[&str]| -> Vec<_> {
                left.read(cx)
                    .rows()
                    .iter()
                    .filter(|row| names.contains(&row.name.as_str()))
                    .cloned()
                    .collect()
            };
            let first = cx.update(|cx| entries(cx, &["src", "README.md", "styles.css"]));
            cx.update_window(handle, |_, window, cx| {
                screen.update(cx, |screen, cx| screen.copy(Side::Left, first, window, cx))
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            wait_for(cx, handle, shot, "the first copies", |cx| {
                screen.read(cx).transfers().read(cx).active() == 0
            })
            .await?;
            // The same file again: the name conflict dialog.
            let again = cx.update(|cx| entries(cx, &["README.md"]));
            cx.update_window(handle, |_, window, cx| {
                screen.update(cx, |screen, cx| screen.copy(Side::Left, again, window, cx))
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            // The conflict dialog opens once the target is checked.
            for _ in 0..40 {
                settle(cx, handle).await?;
                let shown = cx
                    .update_window(handle, |_, window, _| {
                        window
                            .try_find("sftp-conflict")
                            .is_some_and(|found| found.visible())
                    })
                    .map_err(|_| ScreenshotError::WindowClosed)?;
                if shown {
                    break;
                }
                cx.background_executor().timer(POLL).await;
            }
            settle(cx, handle).await
        }
        ShotState::SftpActions => {
            click(cx, handle, "sftp-left-actions")?;
            settle(cx, handle).await
        }
        ShotState::SftpDelete | ShotState::SftpPermissions => {
            let entry = cx
                .update(|cx| {
                    left.read(cx)
                        .rows()
                        .iter()
                        .find(|row| row.name == "deploy.sh")
                        .cloned()
                })
                .ok_or_else(|| not_reached(shot, "no deploy.sh"))?;
            cx.update(|cx| {
                left.update(cx, |panel, cx| {
                    panel.click(
                        &entry.name,
                        crate::state::selection::SelectMode::Replace,
                        cx,
                    )
                })
            });
            cx.update_window(handle, |_, window, cx| {
                if shot == ShotState::SftpDelete {
                    crate::sftp::menu::open_delete(left.clone(), vec![entry.clone()], window, cx);
                } else {
                    let mut entry = entry.clone();
                    entry.permissions = Some(0o755);
                    crate::sftp::menu::open_permissions(left.clone(), vec![entry], window, cx);
                }
            })
            .map_err(|_| ScreenshotError::WindowClosed)?;
            settle(cx, handle).await
        }
        _ => Err(not_reached(shot, "not a stage 4 state")),
    }
}

/// The Keychain states: Export to host over the first key's panel, and a
/// key with a certificate (expired, so the warning shows).
async fn keychain(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    click(cx, handle, Section::Keychain.element_id().as_ref())?;
    settle(cx, handle).await?;
    let shell = shell_of(cx, root, shot)?;
    let screen = cx.update(|cx| shell.read(cx).keychain_screen().clone());
    let key = cx
        .update(|cx| screen.read(cx).shown_key_ids().first().copied())
        .ok_or_else(|| not_reached(shot, "no demo key"))?;
    if shot == ShotState::KeychainCertificate {
        let text = cx.update(|cx| state.read(cx).reveal_private_key(key).ok());
        let text = text.ok_or_else(|| not_reached(shot, "the key cannot be read"))?;
        let ca = shellpp_core::keys::generate::generate(
            shellpp_core::keys::generate::KeyKind::Ed25519,
            "ca@example",
            None,
        )
        .map_err(|_| not_reached(shot, "no CA key"))?;
        let line = certificate_of(secrecy::ExposeSecret::expose_secret(&text), &ca.to_string())
            .ok_or_else(|| not_reached(shot, "no certificate"))?;
        let added =
            cx.update(|cx| state.update(cx, |state, cx| state.import_certificate(&line, cx)));
        if added.is_err() {
            return Err(not_reached(shot, "the certificate was refused"));
        }
    }
    cx.update_window(handle, |_, window, cx| {
        screen.update(cx, |screen, cx| screen.open_key(key, window, cx))
    })
    .map_err(|_| ScreenshotError::WindowClosed)?;
    settle(cx, handle).await?;
    if shot == ShotState::KeychainExport {
        let line = cx
            .update(|cx| state.read(cx).key_public_key(key).ok())
            .ok_or_else(|| not_reached(shot, "no public key"))?;
        cx.update_window(handle, |_, window, cx| {
            crate::keychain::export_host::open(state.clone(), line, window, cx)
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    } else {
        // The certificate block, at the bottom of the panel.
        let _ = super::scroll_to_bottom(cx, handle, "kc-key-panel-body");
    }
    settle(cx, handle).await
}

/// An expired user certificate of `subject` by `ca` (both OpenSSH texts).
fn certificate_of(subject: &str, ca: &str) -> Option<String> {
    use ssh_key::certificate::{Builder, CertType};
    let subject = ssh_key::PrivateKey::from_openssh(subject.as_bytes()).ok()?;
    let ca = ssh_key::PrivateKey::from_openssh(ca.as_bytes()).ok()?;
    let mut builder = Builder::new(
        vec![5u8; 32],
        subject.public_key().key_data().clone(),
        1_767_225_600,
        1_769_904_000,
    )
    .ok()?;
    builder.cert_type(CertType::User).ok()?;
    builder.key_id("deploy@ci").ok()?;
    builder.valid_principal("deploy").ok()?;
    builder.valid_principal("www-data").ok()?;
    builder.sign(&ca).ok()?.to_openssh().ok()
}

/// Checks that the window shows `shot`.
pub fn check(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let wanted = match shot {
        ShotState::SftpLocal => "sftp-left-rows",
        ShotState::SftpPicker => "sftp-right-picker",
        ShotState::SftpQueue => "sftp-queue",
        ShotState::SftpActions => "sftp-left-actions",
        ShotState::SftpDelete => "sftp-delete",
        ShotState::SftpPermissions => "sftp-permissions",
        ShotState::KeychainExport => "kc-export",
        ShotState::KeychainCertificate => "kc-key-block-certificate",
        _ => return Err(not_reached(shot, "not a stage 4 state")),
    };
    let shown = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(wanted).is_some_and(|found| found.visible())
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    if shown {
        Ok(())
    } else {
        Err(not_reached(shot, &format!("{wanted} is not shown")))
    }
}
