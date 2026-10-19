//! `Clipboard` for the desktop (docs/security.md, section 9.4).
//!
//! Ordinary text (Copy Link, Edit ▸ Copy) and clearing go through
//! `tauri-plugin-clipboard-manager`. Reading uses `arboard` directly so that
//! "no text on the clipboard" can be told apart from a failure. Sensitive
//! values (passwords, the recovery key) are written with `arboard` and marked
//! where the platform has a convention, so clipboard history and cloud
//! clipboard skip them (best effort): the Windows history and cloud exclusion
//! formats, the macOS concealed type, the KDE password-manager hint on Linux.
//!
//! Every method blocks and must not run on the main thread (Linux clipboard
//! reads can deadlock there); the core and the commands call it from
//! `spawn_blocking` threads.

use std::sync::Mutex;

use secrecy::{ExposeSecret, SecretString};
use shellpp_core::platform::{Clipboard, PlatformError};
use tauri::{AppHandle, Runtime};
use tauri_plugin_clipboard_manager::ClipboardExt;

/// The OS clipboard.
pub struct DesktopClipboard<R: Runtime> {
    app: AppHandle<R>,
    /// Kept for the life of the process: on X11 the owner must stay alive to
    /// serve the content. Dropped at exit (`shutdown`).
    direct: Mutex<Option<arboard::Clipboard>>,
}

impl<R: Runtime> std::fmt::Debug for DesktopClipboard<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DesktopClipboard")
    }
}

impl<R: Runtime> DesktopClipboard<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self {
            app,
            direct: Mutex::new(None),
        }
    }

    fn with_direct<T>(
        &self,
        f: impl FnOnce(&mut arboard::Clipboard) -> Result<T, arboard::Error>,
    ) -> Result<T, arboard::Error> {
        let mut guard = self
            .direct
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

    /// Releases the direct clipboard handle (called once at exit).
    pub fn shutdown(&self) {
        let mut guard = self
            .direct
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        guard.take();
    }
}

fn failed(error: impl std::fmt::Display) -> PlatformError {
    // arboard and plugin messages never contain clipboard content.
    PlatformError::Failed(error.to_string())
}

impl<R: Runtime> Clipboard for DesktopClipboard<R> {
    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        match self.with_direct(|clipboard| clipboard.get_text()) {
            Ok(text) => Ok(Some(text)),
            Err(arboard::Error::ContentNotAvailable) => Ok(None),
            Err(arboard::Error::ClipboardNotSupported) => Err(PlatformError::Unavailable),
            Err(error) => Err(failed(error)),
        }
    }

    fn write_text(&self, text: &str) -> Result<(), PlatformError> {
        self.app.clipboard().write_text(text).map_err(failed)
    }

    fn write_sensitive(&self, text: &SecretString) -> Result<(), PlatformError> {
        let value = text.expose_secret();
        self.with_direct(|clipboard| {
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
        .map_err(|error| match error {
            arboard::Error::ClipboardNotSupported => PlatformError::Unavailable,
            other => failed(other),
        })
    }

    fn clear(&self) -> Result<(), PlatformError> {
        self.app.clipboard().clear().map_err(failed)
    }
}
