//! The Known Hosts state of the screenshot run (owner's stage 2b, step
//! 5.1; UI 10.1): the screen after an import of a temporary known_hosts
//! file, with the details panel of one entry open.
//!
//! The file ([`sample_file`]) is written into a temporary folder (inside
//! `TEMP`, which the scripts point at their temporary folder) and imported
//! through the screen's own import dialog (the import source is set to it,
//! so the real `~/.ssh/known_hosts` is never offered); the folder is
//! deleted right after. It holds documentation addresses only and keys
//! made up for the picture: a plain entry whose address is the first demo
//! host's (the panel lists that host), `[host]:port`, a line with two
//! patterns, a hashed entry (`|1|salt|hash`), a revoked key and a
//! `@cert-authority` line, which the import skips. A second import (the
//! light theme) finds only duplicates.

use std::time::{Duration, Instant};

use gpui::{AnyWindowHandle, AppContext as _, AsyncApp, Entity};
use gpui_kit::test::TestWindowExt as _;
use shellpp_core::known_hosts::{encode_blob, hash_host};
use uuid::Uuid;

use super::{ScreenshotError, ShotState, click, settle, shell_of, visible};
use crate::app::RootView;
use crate::known_hosts::{self, ImportStep, KnownHostsScreen};
use crate::profile::TempProfile;
use crate::shell::Section;
use crate::state::AppState;

const SHOT: ShotState = ShotState::KnownHosts;
/// The entry whose details panel is open: the first demo host's address.
pub const PANEL_ADDRESS: &str = "192.0.2.1";
/// The hashed entry's name (only its HMAC is in the file).
const HASHED_NAME: &str = "db-01.prod.example.com";
const TIMEOUT: Duration = Duration::from_secs(20);
const POLL: Duration = Duration::from_millis(50);
/// The import dialog and the Close of its summary.
const IMPORT_DIALOG: &str = "kh-import-dialog";
const IMPORT_CLOSE: &str = "kh-import-close";

/// A made-up `ssh-ed25519` public key in known_hosts form (base64 of the
/// key blob): 32 bytes from a small xorshift generator seeded with `seed`,
/// so every run shows the same fingerprints.
fn ed25519_key(seed: u64) -> String {
    let mut state = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut key = [0u8; 32];
    for byte in &mut key {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        *byte = (state >> 24) as u8;
    }
    let mut blob = Vec::with_capacity(51);
    for part in [b"ssh-ed25519".as_slice(), key.as_slice()] {
        blob.extend_from_slice(&(part.len() as u32).to_be_bytes());
        blob.extend_from_slice(part);
    }
    encode_blob(&blob)
}

/// The known_hosts file of the state.
pub fn sample_file() -> String {
    let salt: Vec<u8> = (1..=20).map(|n: u8| n.wrapping_mul(37)).collect();
    let hashed = hash_host(&salt, HASHED_NAME, 22);
    [
        "# Shell++ screenshots: documentation addresses, made-up keys".to_owned(),
        format!("{PANEL_ADDRESS} ssh-ed25519 {}", ed25519_key(1)),
        format!("[198.51.100.2]:2222 ssh-ed25519 {}", ed25519_key(2)),
        format!(
            "web-01.staging.example.com,192.0.2.3 ssh-ed25519 {}",
            ed25519_key(3)
        ),
        format!("{hashed} ssh-ed25519 {}", ed25519_key(4)),
        format!(
            "@revoked 198.51.100.9 ssh-ed25519 {} old key",
            ed25519_key(5)
        ),
        format!(
            "@cert-authority *.example.com ssh-ed25519 {}",
            ed25519_key(6)
        ),
        String::new(),
    ]
    .join("\n")
}

fn not_reached(reason: &str) -> ScreenshotError {
    ScreenshotError::state(SHOT, reason)
}

fn screen_of(
    cx: &mut AsyncApp,
    root: &Entity<RootView>,
) -> Result<Entity<KnownHostsScreen>, ScreenshotError> {
    let shell = shell_of(cx, root, SHOT)?;
    Ok(cx.update(|cx| shell.read(cx).known_hosts_screen().clone()))
}

/// Polls `condition` after a fresh frame until it holds.
async fn wait_for(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
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
            return Err(not_reached(&format!("timeout: {what}")));
        }
        cx.background_executor().timer(POLL).await;
    }
}

/// The entry of the details panel ([`PANEL_ADDRESS`], not hashed).
fn panel_entry(screen: &KnownHostsScreen) -> Option<Uuid> {
    screen
        .items()
        .iter()
        .find(|item| !item.hashed && item.host == PANEL_ADDRESS)
        .map(|item| item.record_id)
}

/// From the reset demo window: Known Hosts in the sidebar, Import of the
/// sample file through the dialog, then a click on the entry of
/// [`PANEL_ADDRESS`].
pub async fn reach(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
    _state: &Entity<AppState>,
) -> Result<(), ScreenshotError> {
    click(cx, handle, Section::KnownHosts.element_id().as_ref())?;
    settle(cx, handle).await?;
    let screen = screen_of(cx, root)?;
    let folder = TempProfile::create("shellpp-shots-known-hosts")
        .map_err(|error| not_reached(&format!("temporary folder: {error}")))?;
    let file = folder.path().join("known_hosts");
    std::fs::write(&file, sample_file())
        .map_err(|error| not_reached(&format!("known_hosts file: {error}")))?;
    let imported = import(cx, handle, &screen, file).await;
    drop(folder);
    imported?;
    wait_for(cx, handle, "the imported entries", |cx| {
        panel_entry(screen.read(cx)).is_some()
    })
    .await?;
    let entry = cx
        .update(|cx| panel_entry(screen.read(cx)))
        .ok_or_else(|| not_reached("no entry to open"))?;
    click(cx, handle, known_hosts::card_id(entry).as_ref())
}

/// Opens the import dialog on `file`, imports, waits for the summary and
/// closes the dialog.
async fn import(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    screen: &Entity<KnownHostsScreen>,
    file: std::path::PathBuf,
) -> Result<(), ScreenshotError> {
    cx.update_window(handle, |_, window, cx| {
        known_hosts::set_import_source(Some(file), cx);
        screen.update(cx, |screen, cx| screen.open_import(window, cx));
    })
    .map_err(|_| ScreenshotError::WindowClosed)?;
    // Never left pointing at a deleted file.
    let result = async {
        settle(cx, handle).await?;
        let dialog = cx
            .update(|cx| screen.read(cx).import_dialog())
            .ok_or_else(|| not_reached("no import dialog"))?;
        cx.update_window(handle, |_, window, cx| {
            dialog.update(cx, |dialog, cx| dialog.run(window, cx));
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
        wait_for(cx, handle, "the import summary", |cx| {
            matches!(dialog.read(cx).step(), ImportStep::Summary(_))
        })
        .await?;
        // The summary's Close.
        settle(cx, handle).await?;
        click(cx, handle, IMPORT_CLOSE)?;
        let deadline = Instant::now() + TIMEOUT;
        loop {
            let open = cx
                .update_window(handle, |_, window, cx| {
                    window.render_frame(cx);
                    visible(window, IMPORT_DIALOG)
                })
                .map_err(|_| ScreenshotError::WindowClosed)?;
            if !open {
                return Ok(());
            }
            if Instant::now() > deadline {
                return Err(not_reached("timeout: the import dialog to close"));
            }
            cx.background_executor().timer(POLL).await;
        }
    }
    .await;
    cx.update(|cx| known_hosts::set_import_source(None, cx));
    result
}

/// The Known Hosts screen with the entry's panel open, a hashed entry and a
/// revoked one among the cards.
pub fn check(
    cx: &mut AsyncApp,
    handle: AnyWindowHandle,
    root: &Entity<RootView>,
) -> Result<(), ScreenshotError> {
    let screen = screen_of(cx, root)?;
    let result = cx
        .update_window(handle, |_, window, cx| {
            window.render_frame(cx);
            let need = |ok: bool, reason: &'static str| if ok { Ok(()) } else { Err(reason) };
            let screen = screen.read(cx);
            need(
                visible(window, "known-hosts-screen"),
                "no Known Hosts screen",
            )?;
            need(
                screen.has_panel() && visible(window, "kh-panel"),
                "no details panel",
            )?;
            need(
                screen.selected() == panel_entry(screen),
                "the panel shows another entry",
            )?;
            let hashed = screen.items().iter().find(|item| item.hashed);
            need(
                hashed.is_some_and(|item| {
                    visible(
                        window,
                        known_hosts::hashed_title_id(item.record_id).as_ref(),
                    )
                }),
                "no hashed entry on screen",
            )?;
            let revoked = screen.items().iter().find(|item| item.revoked);
            need(
                revoked.is_some_and(|item| {
                    visible(
                        window,
                        known_hosts::revoked_badge_id(item.record_id).as_ref(),
                    )
                }),
                "no revoked entry on screen",
            )?;
            need(!visible(window, IMPORT_DIALOG), "the import dialog is open")
        })
        .map_err(|_| ScreenshotError::WindowClosed)?;
    result.map_err(not_reached)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sample file parses as the state expects: a plain entry for the
    /// panel, a hashed one, a revoked one, the `@cert-authority` line
    /// skipped, nothing invalid; the keys are the same every run.
    #[test]
    fn the_sample_file_has_every_kind_of_entry() {
        let parsed = shellpp_core::known_hosts::openssh::parse(sample_file().as_bytes());
        assert_eq!(parsed.invalid, 0);
        assert!(parsed.skipped >= 1, "the @cert-authority line");
        assert!(
            parsed
                .entries
                .iter()
                .any(|entry| entry.host.starts_with("|1|"))
        );
        assert!(parsed.entries.iter().any(|entry| entry.revoked));
        assert!(
            parsed
                .entries
                .iter()
                .any(|entry| entry.host == PANEL_ADDRESS)
        );
        assert_eq!(ed25519_key(1), ed25519_key(1), "the same keys every run");
        assert_ne!(ed25519_key(1), ed25519_key(2));
    }
}
