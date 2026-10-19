//! `Clipboard` for the core on the desktop (docs/security.md, section 9.4),
//! through `arboard`.
//!
//! The core uses it for sensitive values (`secret_copy`: passwords, the
//! recovery key) and to clear them again after 30 seconds. Sensitive values
//! are marked where the platform has a convention, so clipboard history and
//! cloud clipboard skip them (best effort): the Windows history and cloud
//! exclusion formats, the macOS concealed type, the KDE password-manager hint
//! on Linux. Ordinary copy and paste in the UI uses GPUI's own clipboard
//! (`App::write_to_clipboard`).
//!
//! Every method blocks; the core calls them from its own call sites (tick,
//! `secret_copy`), never from inside a GPUI render.

use std::sync::Mutex;

use secrecy::{ExposeSecret, SecretString};
use shellpp_core::platform::{Clipboard, PlatformError};

/// The OS clipboard.
#[derive(Default)]
pub struct DesktopClipboard {
    /// Kept for the life of the process: on X11 the owner must stay alive to
    /// serve the content. Dropped at exit ([`DesktopClipboard::shutdown`]).
    handle: Mutex<Option<arboard::Clipboard>>,
}

impl std::fmt::Debug for DesktopClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DesktopClipboard")
    }
}

impl DesktopClipboard {
    pub fn new() -> Self {
        Self::default()
    }

    fn with_handle<T>(
        &self,
        f: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
    ) -> Result<T, arboard::Error> {
        let mut guard = self
            .handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if guard.is_none() {
            *guard = Some(arboard::Clipboard::new()?);
        }
        match guard.as_mut() {
            Some(clipboard) => f(clipboard),
            None => Err(arboard::Error::ClipboardNotSupported),
        }
    }

    /// Releases the clipboard handle (called once at exit).
    pub fn shutdown(&self) {
        let mut guard = self
            .handle
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.take();
    }
}

fn failed(error: arboard::Error) -> PlatformError {
    match error {
        arboard::Error::ClipboardNotSupported => PlatformError::Unavailable,
        // arboard messages never contain clipboard content.
        other => PlatformError::Failed(other.to_string()),
    }
}

impl Clipboard for DesktopClipboard {
    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        match self.with_handle(|clipboard| clipboard.get_text()) {
            Ok(text) => Ok(Some(text)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(error) => Err(failed(error)),
        }
    }

    fn write_text(&self, text: &str) -> Result<(), PlatformError> {
        self.with_handle(|clipboard| clipboard.set_text(text))
            .map_err(failed)
    }

    fn write_sensitive(&self, text: &SecretString) -> Result<(), PlatformError> {
        let value = text.expose_secret();
        self.with_handle(|clipboard| {
            let set = clipboard.set();
            #[cfg(target_os = "windows")]
            let set = {
                use arboard::SetExtWindows;
                set.exclude_from_history().exclude_from_cloud()
            };
            #[cfg(target_os = "macos")]
            let set = {
                use arboard::SetExtApple;
                set.exclude_from_history()
            };
            #[cfg(all(
                unix,
                not(any(target_os = "macos", target_os = "android", target_os = "ios"))
            ))]
            let set = {
                use arboard::SetExtLinux;
                set.exclude_from_history()
            };
            set.text(value)
        })
        .map_err(failed)
    }

    fn clear(&self) -> Result<(), PlatformError> {
        self.with_handle(|clipboard| clipboard.clear())
            .map_err(failed)
    }
}
