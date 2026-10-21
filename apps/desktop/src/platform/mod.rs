//! Platform services handed to the core (docs/architecture.md, section 5.1).
//!
//! | Trait | App | `--demo` (dev-tools) | Tests |
//! |---|---|---|---|
//! | `SecretStore` | [`keychain::KeyringSecretStore`] (OS keychain) | in memory | in memory |
//! | `Clipboard` | [`clipboard::DesktopClipboard`] | [`clipboard::DesktopClipboard`] | in memory |
//! | `ExternalLauncher` | [`NoLauncher`] | [`NoLauncher`] | in memory |
//! | `EventSink` | [`events::ChannelEventSink`] | same | same |
//! | `Clock` | `SystemClock` | `SystemClock` | manual clock (system time) |
//!
//! The in-memory implementations come from `shellpp_core::platform::memory`.
//! A temporary profile never writes a device key into the OS keychain.

pub mod base64url;
pub mod clipboard;
pub mod device_name;
pub mod drives;
pub mod events;
pub mod keychain;

use std::sync::Arc;

use shellpp_core::platform::memory::{MemoryPlatform, MemorySecretStore};
use shellpp_core::platform::{
    EventSink, ExternalLauncher, OsKind, Platform, PlatformClass, PlatformError, SystemClock,
};

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

/// The core does not open links itself in this stage; the UI opens them with
/// GPUI's `App::open_url`. Kept as the `ExternalLauncher` the core requires.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoLauncher;

impl ExternalLauncher for NoLauncher {
    fn open_url(&self, _url: &str) -> Result<(), PlatformError> {
        Err(PlatformError::Unavailable)
    }
}

/// Host-side handles to platform services that need work at exit.
#[derive(Clone, Debug, Default)]
pub struct PlatformExtras {
    /// The real clipboard (absent in tests).
    pub clipboard: Option<Arc<clipboard::DesktopClipboard>>,
}

impl PlatformExtras {
    /// Exit work: releases the clipboard handle.
    pub fn shutdown(&self) {
        if let Some(clipboard) = &self.clipboard {
            clipboard.shutdown();
        }
    }
}

/// The platform of a normal run: OS keychain, OS clipboard, system clock.
///
/// Probes the keychain once on a helper thread (the Linux Secret Service
/// client must not run on the main thread); later calls use the cached answer.
pub fn desktop(events: Arc<dyn EventSink>) -> (Platform, PlatformExtras) {
    let os = current_os();
    let secret_store = Arc::new(keychain::KeyringSecretStore::new());
    let probe = secret_store.clone();
    let _ = std::thread::spawn(move || {
        use shellpp_core::platform::SecretStore;
        probe.availability()
    })
    .join();
    let clipboard = Arc::new(clipboard::DesktopClipboard::new());
    let platform = Platform {
        secret_store,
        clipboard: clipboard.clone(),
        launcher: Arc::new(NoLauncher),
        events,
        clock: Arc::new(SystemClock),
        class: PlatformClass::Desktop,
        os,
        device_name: device_name::device_name(os),
    };
    (
        platform,
        PlatformExtras {
            clipboard: Some(clipboard),
        },
    )
}

/// The platform of a temporary profile (`--demo`): like [`desktop`], but the
/// device key ("remember on this device") stays in memory, never in the OS
/// keychain.
pub fn temporary(events: Arc<dyn EventSink>) -> (Platform, PlatformExtras) {
    let os = current_os();
    let clipboard = Arc::new(clipboard::DesktopClipboard::new());
    let platform = Platform {
        secret_store: Arc::new(MemorySecretStore::new()),
        clipboard: clipboard.clone(),
        launcher: Arc::new(NoLauncher),
        events,
        clock: Arc::new(SystemClock),
        class: PlatformClass::Desktop,
        os,
        device_name: device_name::device_name(os),
    };
    (
        platform,
        PlatformExtras {
            clipboard: Some(clipboard),
        },
    )
}

/// The platform of tests: every service in memory (no OS keychain, no OS
/// clipboard), events into `events`. The returned [`MemoryPlatform`] gives
/// access to the mocks (clipboard content, clock).
pub fn in_memory(events: Arc<dyn EventSink>) -> (Platform, MemoryPlatform) {
    let memory = MemoryPlatform::new();
    let mut platform = memory.platform(current_os(), "Test device");
    platform.events = events;
    (platform, memory)
}
