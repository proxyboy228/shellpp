//! Platform implementations handed to the core (docs/architecture.md,
//! section 5.1): the real desktop services, or the mocks of the `e2e-mocks`
//! build (section 5.4).
//!
//! | Trait | Desktop | `e2e-mocks` |
//! |---|---|---|
//! | `SecretStore` | [`keychain::KeyringSecretStore`] | [`mocks::FileSecretStore`] |
//! | `Clipboard` | [`clipboard::DesktopClipboard`] | [`mocks::MemoryClipboard`] |
//! | `ExternalLauncher` | [`launcher::OpenerLauncher`] | [`mocks::RecordingLauncher`] |
//! | `EventSink` | [`crate::events::TauriEventSink`] | same |
//! | `Clock` | `SystemClock` | [`mocks::ManualClock`] |
//!
//! `FileDialogs` arrives with import and export (stage 3).

pub mod base64url;
pub mod clipboard;
pub mod device_name;
pub mod keychain;
pub mod launcher;
#[cfg(any(test, feature = "e2e-mocks"))]
pub mod mocks;

use std::path::Path;
use std::sync::Arc;

#[cfg(not(feature = "e2e-mocks"))]
use shellpp_core::platform::SystemClock;
use shellpp_core::platform::{OsKind, Platform, PlatformClass};
use tauri::{AppHandle, Runtime};

use crate::events::TauriEventSink;
use crate::links::LinkQueue;
use crate::windows::WindowRegistry;

/// Host-side handles to platform services that are not part of [`Platform`].
#[derive(Clone)]
pub struct PlatformExtras<R: Runtime> {
    /// The real clipboard, released at exit (absent in e2e builds).
    pub desktop_clipboard: Option<Arc<clipboard::DesktopClipboard<R>>>,
    /// The mocks driven by `e2e_mock`.
    #[cfg(feature = "e2e-mocks")]
    pub e2e: mocks::E2eHandles,
}

impl<R: Runtime> std::fmt::Debug for PlatformExtras<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PlatformExtras { .. }")
    }
}

/// The operating system of this build.
pub const fn current_os() -> OsKind {
    if cfg!(target_os = "windows") {
        OsKind::Windows
    } else if cfg!(target_os = "macos") {
        OsKind::Macos
    } else if cfg!(target_os = "ios") {
        OsKind::Ios
    } else if cfg!(target_os = "android") {
        OsKind::Android
    } else {
        OsKind::Linux
    }
}

/// Builds the platform for the core.
pub fn install<R: Runtime>(
    app: &AppHandle<R>,
    data_dir: &Path,
    links: Arc<LinkQueue>,
    windows: Arc<WindowRegistry>,
) -> (Platform, PlatformExtras<R>) {
    let os = current_os();
    let events = Arc::new(TauriEventSink::new(app.clone(), links, windows));
    let device_name = device_name::device_name(os);

    #[cfg(feature = "e2e-mocks")]
    {
        let e2e = mocks::E2eHandles::new(data_dir);
        let platform = Platform {
            secret_store: e2e.secret_store.clone(),
            clipboard: e2e.clipboard.clone(),
            launcher: e2e.launcher.clone(),
            events,
            clock: e2e.clock.clone(),
            class: PlatformClass::Desktop,
            os,
            device_name,
        };
        let extras = PlatformExtras {
            desktop_clipboard: None,
            e2e,
        };
        (platform, extras)
    }

    #[cfg(not(feature = "e2e-mocks"))]
    {
        let _ = data_dir;
        let desktop_clipboard = Arc::new(clipboard::DesktopClipboard::new(app.clone()));
        let secret_store = Arc::new(keychain::KeyringSecretStore::new());
        // Probe the keychain once on a runtime thread: later calls (even from
        // the main thread during startup) use the cached answer, and the Linux
        // Secret Service client must not run on the main thread.
        let probe = secret_store.clone();
        let _ = tauri::async_runtime::block_on(tauri::async_runtime::spawn_blocking(move || {
            use shellpp_core::platform::SecretStore;
            probe.availability()
        }));
        let platform = Platform {
            secret_store,
            clipboard: desktop_clipboard.clone(),
            launcher: Arc::new(launcher::OpenerLauncher::new(app.clone())),
            events,
            clock: Arc::new(SystemClock),
            class: PlatformClass::Desktop,
            os,
            device_name,
        };
        let extras = PlatformExtras {
            desktop_clipboard: Some(desktop_clipboard),
        };
        (platform, extras)
    }
}
