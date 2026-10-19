//! Mock launcher: records URLs, opens nothing (e2e builds).

use std::sync::Mutex;

use shellpp_core::platform::{ExternalLauncher, PlatformError};

/// The e2e launcher.
#[derive(Debug, Default)]
pub struct RecordingLauncher {
    urls: Mutex<Vec<String>>,
}

impl RecordingLauncher {
    /// Every URL passed to the launcher, in order (`launcher_requests`).
    pub fn urls(&self) -> Vec<String> {
        self.urls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }
}

impl ExternalLauncher for RecordingLauncher {
    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        self.urls
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(url.to_owned());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_in_order() {
        let launcher = RecordingLauncher::default();
        launcher.open_url("https://example.com/a").unwrap();
        launcher.open_url("https://example.org/b").unwrap();
        assert_eq!(
            launcher.urls(),
            vec!["https://example.com/a", "https://example.org/b"]
        );
    }
}
