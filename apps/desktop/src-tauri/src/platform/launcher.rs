//! `ExternalLauncher` through `tauri-plugin-opener`: opens `https` links in
//! the system browser (docs/architecture.md, section 3.7). The caller has
//! validated the URL (`commands::app::validate_external_url`).

use shellpp_core::platform::{ExternalLauncher, PlatformError};
use tauri::{AppHandle, Runtime};
use tauri_plugin_opener::OpenerExt;

/// The system browser.
pub struct OpenerLauncher<R: Runtime> {
    app: AppHandle<R>,
}

impl<R: Runtime> std::fmt::Debug for OpenerLauncher<R> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OpenerLauncher")
    }
}

impl<R: Runtime> OpenerLauncher<R> {
    pub fn new(app: AppHandle<R>) -> Self {
        Self { app }
    }
}

impl<R: Runtime> ExternalLauncher for OpenerLauncher<R> {
    fn open_url(&self, url: &str) -> Result<(), PlatformError> {
        self.app
            .opener()
            .open_url(url, None::<&str>)
            // The opener error can contain the URL; report only the failure.
            .map_err(|_| PlatformError::Failed("opener failed".to_owned()))
    }
}
