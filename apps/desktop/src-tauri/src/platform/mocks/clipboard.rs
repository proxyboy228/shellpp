//! Mock clipboard holding one string (e2e builds).

use std::sync::Mutex;

use secrecy::{ExposeSecret, SecretString};
use shellpp_core::platform::{Clipboard, PlatformError};

#[derive(Debug, Default)]
struct State {
    text: Option<String>,
    sensitive: bool,
}

/// The e2e clipboard.
#[derive(Default)]
pub struct MemoryClipboard {
    state: Mutex<State>,
}

impl std::fmt::Debug for MemoryClipboard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MemoryClipboard { .. }")
    }
}

impl MemoryClipboard {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// The current content (`e2e_mock {kind: "clipboard_read"}`).
    pub fn text(&self) -> Option<String> {
        self.lock().text.clone()
    }

    /// Sets the content as if the user copied it elsewhere (`clipboard_write`).
    pub fn set_text(&self, text: String) {
        let mut state = self.lock();
        state.text = Some(text);
        state.sensitive = false;
    }

    /// Whether the last write was a sensitive one.
    pub fn last_was_sensitive(&self) -> bool {
        self.lock().sensitive
    }
}

impl Clipboard for MemoryClipboard {
    fn read_text(&self) -> Result<Option<String>, PlatformError> {
        Ok(self.text())
    }

    fn write_text(&self, text: &str) -> Result<(), PlatformError> {
        self.set_text(text.to_owned());
        Ok(())
    }

    fn write_sensitive(&self, text: &SecretString) -> Result<(), PlatformError> {
        let mut state = self.lock();
        state.text = Some(text.expose_secret().to_owned());
        state.sensitive = true;
        Ok(())
    }

    fn clear(&self) -> Result<(), PlatformError> {
        let mut state = self.lock();
        state.text = None;
        state.sensitive = false;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn holds_one_string() {
        let clipboard = MemoryClipboard::default();
        assert_eq!(clipboard.read_text().unwrap(), None);
        clipboard.write_text("a").unwrap();
        assert_eq!(clipboard.text().as_deref(), Some("a"));
        assert!(!clipboard.last_was_sensitive());
        clipboard.write_sensitive(&SecretString::from("s")).unwrap();
        assert_eq!(clipboard.read_text().unwrap().as_deref(), Some("s"));
        assert!(clipboard.last_was_sensitive());
        clipboard.clear().unwrap();
        assert_eq!(clipboard.text(), None);
    }
}
