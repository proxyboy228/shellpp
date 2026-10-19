//! Platform mocks of the `e2e-mocks` build (docs/architecture.md, section 5.4).
//!
//! WebDriver cannot see the OS keychain, the clipboard or the system browser,
//! and an e2e run must not touch the developer's real ones. These mocks are
//! driven by the `e2e_mock` command:
//!
//! - [`FileSecretStore`]: an in-memory keychain persisted to
//!   `<data_dir>/e2e-keychain.json`, so a restart in the same data directory
//!   sees the device key ("remember on this device" scenarios).
//! - [`MemoryClipboard`]: one string, readable by the test.
//! - [`RecordingLauncher`]: records URLs, launches nothing.
//! - [`ManualClock`]: wall clock plus an offset the test advances.
//!
//! They live in the host (not in `shellpp_core::platform::memory`) because
//! the keychain mock persists to a file and the host must not wait for the
//! core's test doubles.

mod clipboard;
mod clock;
mod launcher;
mod secret_store;

use std::sync::Arc;

pub use clipboard::MemoryClipboard;
pub use clock::ManualClock;
pub use launcher::RecordingLauncher;
pub use secret_store::FileSecretStore;

/// The mock instances, kept by the host for `e2e_mock`.
#[derive(Clone, Debug)]
pub struct E2eHandles {
    pub secret_store: Arc<FileSecretStore>,
    pub clipboard: Arc<MemoryClipboard>,
    pub launcher: Arc<RecordingLauncher>,
    pub clock: Arc<ManualClock>,
}

impl E2eHandles {
    /// Creates the mocks; the keychain file lives in `data_dir`.
    pub fn new(data_dir: &std::path::Path) -> Self {
        Self {
            secret_store: Arc::new(FileSecretStore::open(
                data_dir.join(secret_store::FILE_NAME),
            )),
            clipboard: Arc::new(MemoryClipboard::default()),
            launcher: Arc::new(RecordingLauncher::default()),
            clock: Arc::new(ManualClock::default()),
        }
    }
}
