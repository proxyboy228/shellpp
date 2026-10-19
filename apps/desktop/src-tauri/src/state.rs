//! Application state managed by Tauri (`State<'_, AppState>` in commands).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use shellpp_core::Core;
use shellpp_core::platform::Platform;
use tauri::Wry;

use crate::dirs::AppDirs;
use crate::error::CmdError;
use crate::links::LinkQueue;
use crate::platform::PlatformExtras;
use crate::windows::WindowRegistry;

/// Everything the commands need.
pub struct AppState {
    /// The opened core, or the error that stopped it from opening
    /// (`newer_schema`, `storage_error`, `io_error`, ...). With an error every
    /// command except `app_info`, `window_open`, `activity` and `app_exit`
    /// returns it.
    core: Result<Arc<Core>, CmdError>,
    /// The platform given to the core; host-only commands use its clipboard
    /// and launcher (mocks in e2e builds).
    pub platform: Option<Platform>,
    pub extras: Option<PlatformExtras<Wry>>,
    pub windows: Arc<WindowRegistry>,
    pub links: Arc<LinkQueue>,
    pub dirs: Option<AppDirs>,
    shut_down: AtomicBool,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("core_open", &self.core.is_ok())
            .finish_non_exhaustive()
    }
}

impl AppState {
    pub fn new(
        core: Result<Arc<Core>, CmdError>,
        platform: Option<Platform>,
        extras: Option<PlatformExtras<Wry>>,
        windows: Arc<WindowRegistry>,
        links: Arc<LinkQueue>,
        dirs: Option<AppDirs>,
    ) -> Self {
        Self {
            core,
            platform,
            extras,
            windows,
            links,
            dirs,
            shut_down: AtomicBool::new(false),
        }
    }

    /// The core, or the startup error.
    pub fn core(&self) -> Result<Arc<Core>, CmdError> {
        self.core.clone()
    }

    /// The platform, or the startup error when the core never got one.
    pub fn platform(&self) -> Result<&Platform, CmdError> {
        match (&self.platform, &self.core) {
            (Some(platform), _) => Ok(platform),
            (None, Err(error)) => Err(error.clone()),
            (None, Ok(_)) => Err(CmdError::internal()),
        }
    }

    /// Exit work, run once: clears sensitive clipboard content (core), empties
    /// `tmp/` and releases the clipboard handle.
    pub fn shutdown(&self) {
        if self.shut_down.swap(true, Ordering::SeqCst) {
            return;
        }
        if let Ok(core) = &self.core {
            let core = core.clone();
            // `on_exit` may block on the clipboard; never on the main thread.
            let _ = std::thread::spawn(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| core.on_exit()));
            })
            .join();
        }
        if let Some(dirs) = &self.dirs {
            let failed = dirs.empty_tmp();
            if failed > 0 {
                tracing::warn!(failed, "tmp entries left for the next start");
            }
        }
        if let Some(extras) = &self.extras
            && let Some(clipboard) = &extras.desktop_clipboard
        {
            clipboard.shutdown();
        }
        tracing::info!("shutdown complete");
    }
}
