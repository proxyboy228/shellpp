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
use crate::sessions::SessionManager;
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
    /// Terminal sessions (absent when the core did not open).
    pub sessions: Option<Arc<SessionManager>>,
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
        sessions: Option<Arc<SessionManager>>,
    ) -> Self {
        Self {
            core,
            platform,
            extras,
            windows,
            links,
            dirs,
            sessions,
            shut_down: AtomicBool::new(false),
        }
    }

    /// The session manager, or the startup error.
    pub fn sessions(&self) -> Result<&Arc<SessionManager>, CmdError> {
        match (&self.sessions, &self.core) {
            (Some(sessions), _) => Ok(sessions),
            (None, Err(error)) => Err(error.clone()),
            (None, Ok(_)) => Err(CmdError::internal()),
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

    /// Exit work, run once: closes the sessions (their connection log
    /// entries end), clears sensitive clipboard content (core), empties
    /// `tmp/` and releases the clipboard handle.
    pub fn shutdown(&self) {
        if self.shut_down.swap(true, Ordering::SeqCst) {
            return;
        }
        let sessions = self.sessions.clone();
        if let Ok(core) = &self.core {
            let core = core.clone();
            // `on_exit` may block on the clipboard; never on the main thread.
            let _ = std::thread::spawn(move || {
                let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if let Some(sessions) = sessions {
                        let logs = sessions.close_all();
                        sessions.end_logs_now(&logs);
                    }
                    core.on_exit();
                }));
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
