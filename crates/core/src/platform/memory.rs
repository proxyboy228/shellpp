//! In-memory platform implementations for tests and for the `e2e-mocks`
//! build of `shellpp-desktop` (docs/architecture.md, section 5.4).
//!
//! Everything here is `Send + Sync`. None of these types is used by a release
//! build of the desktop app.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, SystemTime};

use secrecy::{ExposeSecret, SecretBox, SecretString};
use zeroize::Zeroizing;

use crate::crypto::PlatformClass;
use crate::platform::{
    Availability, Clipboard, Clock, EventSink, ExternalLauncher, OsKind, Platform, PlatformError,
    SecretStore, SecretStoreError, UnavailableReason,
};
use crate::service::dto::CoreEvent;

fn guard<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// ---------------------------------------------------------------------------
// SecretStore
// ---------------------------------------------------------------------------

struct SecretState {
    available: bool,
    /// `get` of a stored item answers `AuthenticationRequired` (D218).
    requires_authentication: bool,
    items: BTreeMap<String, Zeroizing<[u8; 32]>>,
}

/// A boxed copy of a stored key, written in place (no stack copy).
fn boxed_copy(value: &Zeroizing<[u8; 32]>) -> SecretBox<[u8; 32]> {
    SecretBox::init_with_mut(|buf: &mut [u8; 32]| buf.copy_from_slice(&value[..]))
}

/// In-memory keychain. `set_available(false)` makes it report
/// `Unavailable(Unsupported)` and refuse every call.
pub struct MemorySecretStore {
    state: Mutex<SecretState>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(SecretState {
                available: true,
                requires_authentication: false,
                items: BTreeMap::new(),
            }),
        }
    }

    pub fn set_available(&self, available: bool) {
        guard(&self.state).available = available;
    }

    /// Makes `get` of a stored item answer `AuthenticationRequired`, as the
    /// Android store does until the biometric prompt passed (D218).
    pub fn set_requires_authentication(&self, required: bool) {
        guard(&self.state).requires_authentication = required;
    }

    pub fn is_available(&self) -> bool {
        guard(&self.state).available
    }

    /// Account names only, never values.
    pub fn accounts(&self) -> Vec<String> {
        guard(&self.state).items.keys().cloned().collect()
    }

    /// Every item, for wrappers that persist the mock keychain between runs
    /// of an e2e test (the values are test keys of a mock).
    pub fn entries(&self) -> Vec<(String, SecretBox<[u8; 32]>)> {
        guard(&self.state)
            .items
            .iter()
            .map(|(account, value)| (account.clone(), boxed_copy(value)))
            .collect()
    }

    /// Inserts an item regardless of availability (restoring persisted state).
    pub fn insert(&self, account: &str, secret: &SecretBox<[u8; 32]>) {
        guard(&self.state)
            .items
            .insert(account.to_owned(), Zeroizing::new(*secret.expose_secret()));
    }
}

impl Default for MemorySecretStore {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for MemorySecretStore {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = guard(&self.state);
        f.debug_struct("MemorySecretStore")
            .field("available", &state.available)
            .field("items", &state.items.len())
            .finish()
    }
}

impl SecretStore for MemorySecretStore {
    fn availability(&self) -> Availability {
        if self.is_available() {
            Availability::Available
        } else {
            Availability::Unavailable(UnavailableReason::Unsupported)
        }
    }

    fn get(&self, account: &str) -> Result<Option<SecretBox<[u8; 32]>>, SecretStoreError> {
        let state = guard(&self.state);
        if !state.available {
            return Err(SecretStoreError::Unavailable);
        }
        match state.items.get(account) {
            Some(_) if state.requires_authentication => {
                Err(SecretStoreError::AuthenticationRequired)
            }
            item => Ok(item.map(boxed_copy)),
        }
    }

    fn set(&self, account: &str, secret: &SecretBox<[u8; 32]>) -> Result<(), SecretStoreError> {
        let mut state = guard(&self.state);
        if !state.available {
            return Err(SecretStoreError::Unavailable);
        }
        state
            .items
            .insert(account.to_owned(), Zeroizing::new(*secret.expose_secret()));
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<(), SecretStoreError> {
        let mut state = guard(&self.state);
        if !state.available {
            return Err(SecretStoreError::Unavailable);
        }
        state.items.remove(account);
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Clipboard
// ---------------------------------------------------------------------------

#[derive(Default)]
struct ClipboardState {
    text: Option<Zeroizing<String>>,
    sensitive: bool,
    available: bool,
}

/// A clipboard holding one string.
pub struct MemoryClipboard {
    state: Mutex<ClipboardState>,
}

impl MemoryClipboard {
    pub fn new() -> Self {
        Self {
            state: Mutex::new(ClipboardState {
                available: true,
                ..ClipboardState::default()
            }),
        }
    }

    /// The current content.
    pub fn text(&self) -> Option<String> {
        guard(&self.state)
            .text
            .as_ref()
            .map(|text| text.to_string())
    }

    /// `true` when the current content was written with `write_sensitive`.
    pub fn last_was_sensitive(&self) -> bool {
        guard(&self.state).sensitive
    }

    /// Makes every call fail with `PlatformError::Unavailable`.
    pub fn set_available(&self, available: bool) {
        guard(&self.state).available = available;
    }

    /// Simulates another program writing to the clipboard.
    pub fn set_text(&self, text: Option<&str>) {
        let mut state = guard(&self.state);
        state.text = text.map(|text| Zeroizing::new(text.to_owned()));
        state.sensitive = false;
    }
}

impl Default for MemoryClipboard {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for MemoryClipboard {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = guard(&self.state);
        f.debug_struct("MemoryClipboard")
            .field("has_text", &state.text.is_some())
            .field("sensitive", &state.sensitive)
            .finish()
    }
}

impl Clipboard for MemoryClipboard {
    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        let state = guard(&self.state);
        if !state.available {
            return Err(PlatformError::Unavailable);
        }
        Ok(state.text.as_ref().map(|text| text.to_string()))
    }

    fn write_text(&self, text: &str) -> Result<(), PlatformError> {
        let mut state = guard(&self.state);
        if !state.available {
            return Err(PlatformError::Unavailable);
        }
        state.text = Some(Zeroizing::new(text.to_owned()));
        state.sensitive = false;
        Ok(())
    }

    fn write_sensitive(&self, text: &SecretString) -> Result<(), PlatformError> {
        let mut state = guard(&self.state);
        if !state.available {
            return Err(PlatformError::Unavailable);
        }
        state.text = Some(Zeroizing::new(text.expose_secret().to_owned()));
        state.sensitive = true;
        Ok(())
    }

    fn clear(&self) -> Result<(), PlatformError> {
        let mut state = guard(&self.state);
        if !state.available {
            return Err(PlatformError::Unavailable);
        }
        state.text = None;
        state.sensitive = false;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// ExternalLauncher
// ---------------------------------------------------------------------------

/// Records every opened URL.
#[derive(Default)]
pub struct RecordingLauncher {
    urls: Mutex<Vec<String>>,
}

impl RecordingLauncher {
    pub fn new() -> Self {
        Self::default()
    }

    /// Every URL passed to `open_url`, in order.
    pub fn urls(&self) -> Vec<String> {
        guard(&self.urls).clone()
    }
}

impl fmt::Debug for RecordingLauncher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RecordingLauncher")
            .field("urls", &guard(&self.urls).len())
            .finish()
    }
}

impl ExternalLauncher for RecordingLauncher {
    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        guard(&self.urls).push(url.to_owned());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// EventSink
// ---------------------------------------------------------------------------

/// Collects emitted events.
#[derive(Default)]
pub struct CollectingEvents {
    events: Mutex<Vec<CoreEvent>>,
}

impl CollectingEvents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns and forgets the events collected so far.
    pub fn take(&self) -> Vec<CoreEvent> {
        std::mem::take(&mut *guard(&self.events))
    }

    /// The events collected so far.
    pub fn events(&self) -> Vec<CoreEvent> {
        guard(&self.events).clone()
    }
}

impl fmt::Debug for CollectingEvents {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CollectingEvents")
            .field("events", &guard(&self.events).len())
            .finish()
    }
}

impl EventSink for CollectingEvents {
    fn emit(&self, event: &CoreEvent) {
        guard(&self.events).push(event.clone());
    }
}

// ---------------------------------------------------------------------------
// Clock
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug)]
enum ClockBase {
    System,
    Fixed(SystemTime),
}

#[derive(Clone, Copy, Debug)]
struct ClockState {
    base: ClockBase,
    /// Signed offset in nanoseconds.
    offset_ns: i128,
}

/// System time plus an adjustable offset, or a fixed time plus the offset.
#[derive(Debug)]
pub struct ManualClock {
    state: Mutex<ClockState>,
}

impl ManualClock {
    /// The system clock plus an offset that starts at zero.
    pub fn system() -> Self {
        Self {
            state: Mutex::new(ClockState {
                base: ClockBase::System,
                offset_ns: 0,
            }),
        }
    }

    /// A clock that stands still at `time` until advanced.
    pub fn fixed(time: SystemTime) -> Self {
        Self {
            state: Mutex::new(ClockState {
                base: ClockBase::Fixed(time),
                offset_ns: 0,
            }),
        }
    }

    /// Moves the clock forward.
    pub fn advance(&self, by: Duration) {
        guard(&self.state).offset_ns += by.as_nanos() as i128;
    }

    /// Moves the clock backward (a wall clock that was set back).
    pub fn rewind(&self, by: Duration) {
        guard(&self.state).offset_ns -= by.as_nanos() as i128;
    }
}

impl Clock for ManualClock {
    fn now(&self) -> SystemTime {
        let state = *guard(&self.state);
        let base = match state.base {
            ClockBase::System => SystemTime::now(),
            ClockBase::Fixed(time) => time,
        };
        let magnitude =
            Duration::from_nanos(u64::try_from(state.offset_ns.unsigned_abs()).unwrap_or(u64::MAX));
        if state.offset_ns >= 0 {
            base.checked_add(magnitude).unwrap_or(base)
        } else {
            base.checked_sub(magnitude).unwrap_or(base)
        }
    }
}

// ---------------------------------------------------------------------------
// A complete in-memory platform
// ---------------------------------------------------------------------------

/// A [`Platform`] built from the in-memory implementations, with handles to
/// each of them.
#[derive(Debug, Clone)]
pub struct MemoryPlatform {
    pub secret_store: Arc<MemorySecretStore>,
    pub clipboard: Arc<MemoryClipboard>,
    pub launcher: Arc<RecordingLauncher>,
    pub events: Arc<CollectingEvents>,
    pub clock: Arc<ManualClock>,
}

impl MemoryPlatform {
    /// Every service in memory; the clock follows system time plus an offset.
    pub fn new() -> Self {
        Self {
            secret_store: Arc::new(MemorySecretStore::new()),
            clipboard: Arc::new(MemoryClipboard::new()),
            launcher: Arc::new(RecordingLauncher::new()),
            events: Arc::new(CollectingEvents::new()),
            clock: Arc::new(ManualClock::system()),
        }
    }

    /// The [`Platform`] value for [`crate::service::Core::open`]: desktop KDF
    /// preset, the given OS and device name.
    pub fn platform(&self, os: OsKind, device_name: &str) -> Platform {
        Platform {
            secret_store: self.secret_store.clone(),
            clipboard: self.clipboard.clone(),
            launcher: self.launcher.clone(),
            events: self.events.clone(),
            clock: self.clock.clone(),
            class: PlatformClass::Desktop,
            os,
            device_name: device_name.to_owned(),
        }
    }
}

impl Default for MemoryPlatform {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_store_round_trip_and_availability() {
        let store = MemorySecretStore::new();
        assert_eq!(store.availability(), Availability::Available);
        let secret = SecretBox::new(Box::new([7u8; 32]));
        store.set("device-key:x", &secret).unwrap();
        assert_eq!(store.accounts(), vec!["device-key:x".to_owned()]);
        assert_eq!(
            *store.get("device-key:x").unwrap().unwrap().expose_secret(),
            [7u8; 32]
        );
        assert!(store.get("other").unwrap().is_none());
        store.set_available(false);
        assert_eq!(
            store.availability(),
            Availability::Unavailable(UnavailableReason::Unsupported)
        );
        assert!(store.get("device-key:x").is_err());
        store.set_available(true);
        store.delete("device-key:x").unwrap();
        assert!(store.accounts().is_empty());
        assert!(!format!("{store:?}").contains('7'));
    }

    #[test]
    fn clipboard_tracks_sensitivity() {
        let clipboard = MemoryClipboard::new();
        clipboard.write_text("plain").unwrap();
        assert!(!clipboard.last_was_sensitive());
        clipboard
            .write_sensitive(&SecretString::from("hidden"))
            .unwrap();
        assert!(clipboard.last_was_sensitive());
        assert_eq!(clipboard.read_text().unwrap().as_deref(), Some("hidden"));
        clipboard.clear().unwrap();
        assert_eq!(clipboard.text(), None);
        clipboard.set_available(false);
        assert!(clipboard.write_text("x").is_err());
    }

    #[test]
    fn manual_clock_moves_both_ways() {
        let start = SystemTime::UNIX_EPOCH + Duration::from_secs(1000);
        let clock = ManualClock::fixed(start);
        assert_eq!(clock.now(), start);
        clock.advance(Duration::from_secs(61));
        assert_eq!(clock.now(), start + Duration::from_secs(61));
        clock.rewind(Duration::from_secs(100));
        assert_eq!(clock.now(), start - Duration::from_secs(39));
        let system = ManualClock::system();
        system.advance(Duration::from_secs(3600));
        assert!(system.now() > SystemTime::now() + Duration::from_secs(3500));
    }

    #[test]
    fn launcher_records_urls() {
        let launcher = RecordingLauncher::new();
        launcher.open_url("https://example.com/a").unwrap();
        assert_eq!(launcher.urls(), vec!["https://example.com/a".to_owned()]);
    }
}
