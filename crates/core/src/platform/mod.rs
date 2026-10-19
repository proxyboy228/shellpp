//! Platform traits (docs/architecture.md, section 5.1). The desktop host builds
//! one [`Platform`] value with an `Arc<dyn ...>` per trait and passes it to
//! [`crate::service::Core::open`]. The `e2e-mocks` build of `shellpp-desktop`
//! passes mock implementations instead.
//!
//! Stage 1 traits: [`SecretStore`] (the device key of "remember on this
//! device"), [`Clipboard`], [`ExternalLauncher`], [`EventSink`] (broadcast
//! events, including the `NotificationSink` role of the architecture table) and
//! [`Clock`] (wall clock for auto-lock, replaceable in e2e builds).
//! `FileDialogs`, `PtyBackend` and `SerialBackend` arrive in later stages.
//!
//! Owner: W2. The trait signatures below are part of the W2/W3 contract; W2
//! adds the in-memory implementations in `platform::memory` (see the plan).

use std::fmt;
use std::sync::Arc;
use std::time::SystemTime;

use secrecy::{SecretBox, SecretString};

pub use crate::crypto::PlatformClass;
use crate::service::dto::CoreEvent;

pub mod memory;

/// Operating system the app runs on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OsKind {
    Windows,
    Macos,
    Linux,
    Ios,
    Android,
}

/// Why the OS keychain cannot be used (docs/architecture.md, section 5.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnavailableReason {
    /// Linux without a Secret Service provider.
    NoSecretService,
    /// macOS build without a Developer ID signature.
    UnsignedBuild,
    /// No implementation on this platform.
    Unsupported,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    Unavailable(UnavailableReason),
}

/// Errors of [`SecretStore`]. Never carries the secret.
#[derive(Debug, thiserror::Error)]
pub enum SecretStoreError {
    #[error("secret store unavailable")]
    Unavailable,
    #[error("stored secret is malformed")]
    Malformed,
    #[error("secret store failure: {0}")]
    Backend(String),
}

/// OS keychain access for the device key (docs/architecture.md, section 5.2).
/// Blocking; the host calls it from a blocking thread. Service `shellpp`,
/// account `device-key:<db_id>` ([`crate::ids::keychain_account`]); the
/// implementation stores the 32 bytes as base64url without padding.
pub trait SecretStore: Send + Sync {
    fn availability(&self) -> Availability;
    fn get(&self, account: &str) -> Result<Option<SecretBox<[u8; 32]>>, SecretStoreError>;
    fn set(&self, account: &str, secret: &SecretBox<[u8; 32]>) -> Result<(), SecretStoreError>;
    fn delete(&self, account: &str) -> Result<(), SecretStoreError>;
}

/// Errors of the other platform services.
#[derive(Debug, thiserror::Error)]
pub enum PlatformError {
    #[error("platform service unavailable")]
    Unavailable,
    #[error("platform service failed: {0}")]
    Failed(String),
}

/// The OS clipboard (docs/security.md, section 9.4).
pub trait Clipboard: Send + Sync {
    /// Current text, or `None` when the clipboard holds no text.
    fn read_text(&self) -> Result<Option<String>, PlatformError>;
    /// Writes ordinary text (Copy Link, Edit > Copy).
    fn write_text(&self, text: &str) -> Result<(), PlatformError>;
    /// Writes a sensitive value (password, recovery key), marked where the
    /// platform allows it so that clipboard history and cloud clipboard skip it.
    fn write_sensitive(&self, text: &SecretString) -> Result<(), PlatformError>;
    /// Empties the clipboard.
    fn clear(&self) -> Result<(), PlatformError>;
}

/// Opens `http(s)` links in the system browser (and, from stage 4, files).
pub trait ExternalLauncher: Send + Sync {
    /// `url` was already validated by the caller (https only in stage 1).
    fn open_url(&self, url: &str) -> Result<(), PlatformError>;
}

/// Receives every broadcast event of the core. The desktop host emits it to
/// all windows under [`crate::service::dto::CoreEventKind::name`] with
/// [`CoreEvent::payload`] as the payload.
pub trait EventSink: Send + Sync {
    fn emit(&self, event: &CoreEvent);
}

/// Wall clock. Auto-lock compares wall-clock time with the last input, so
/// that sleep counts as inactivity (docs/security.md, section 8).
pub trait Clock: Send + Sync {
    fn now(&self) -> SystemTime;
}

/// The real wall clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> SystemTime {
        SystemTime::now()
    }
}

/// Platform services handed to the core at startup.
#[derive(Clone)]
pub struct Platform {
    pub secret_store: Arc<dyn SecretStore>,
    pub clipboard: Arc<dyn Clipboard>,
    pub launcher: Arc<dyn ExternalLauncher>,
    pub events: Arc<dyn EventSink>,
    pub clock: Arc<dyn Clock>,
    /// Selects the KDF preset for new passwords and the KDF ceiling.
    pub class: PlatformClass,
    pub os: OsKind,
    /// Default `device_info.name`: the OS computer name, else `<Platform> device`.
    pub device_name: String,
}

impl fmt::Debug for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Platform")
            .field("class", &self.class)
            .field("os", &self.os)
            .finish_non_exhaustive()
    }
}
