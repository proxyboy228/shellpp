//! Startup sequence (docs/architecture.md, section 4.1; plan 4.7):
//!
//! 1. Resolve and create the directories; empty `tmp/`.
//! 2. Start logging; apply process hardening.
//! 3. Build the platform (real services or e2e mocks).
//! 4. Open the core. A failure is kept as the startup error that every
//!    command returns (`app_info` and `window_open` still work).
//! 5. If the core is `locked`: try the device key ("remember on this
//!    device") before any window exists, so the first window never flashes
//!    the unlock screen.
//!    Then the session manager (stage 2), following the lock state.
//! 6. Create window `w-1`; register the macOS menu bar.
//! 7. Start the 1-second tick (auto-lock, clipboard clearing).
//! 8. Queue a `shellpp://` link from the command line until unlock.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use shellpp_core::error::CoreError;
use shellpp_core::service::dto::LockState;
use shellpp_core::{Core, CoreConfig};
use tauri::{App, AppHandle, Manager, Wry};

use crate::dirs::{self, AppDirs};
use crate::error::CmdError;
use crate::events::{SessionSlot, TauriSessionEvents, receive_link};
use crate::links::{LinkQueue, find_link};
use crate::sessions::{ManagerParts, SessionManager, SshConnector};
use crate::state::AppState;
use crate::windows::{self, WindowRegistry};
use crate::{hardening, logging, platform};

/// Runs the startup sequence inside Tauri's `setup`.
pub fn setup(app: &mut App<Wry>) -> Result<(), Box<dyn std::error::Error>> {
    let handle = app.handle().clone();

    // 1-2. Directories, logging, hardening.
    let dirs = match dirs::resolve_current() {
        Ok(dirs) => match dirs.create() {
            Ok(()) => Some(dirs),
            Err(_) => None,
        },
        Err(_) => None,
    };
    match &dirs {
        Some(dirs) => logging::init(&dirs.logs),
        None => logging::init_without_file(),
    }
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        debug = cfg!(debug_assertions),
        e2e = cfg!(feature = "e2e-mocks"),
        "starting"
    );
    if let Some(dirs) = &dirs {
        let failed = dirs.empty_tmp();
        if failed > 0 {
            tracing::warn!(failed, "tmp entries could not be removed");
        }
    } else {
        tracing::error!("data directory unavailable");
    }
    let hardening = hardening::apply();
    tracing::info!(
        core_dumps_disabled = hardening.core_dumps_disabled,
        not_dumpable = hardening.not_dumpable,
        "hardening"
    );

    let windows = Arc::new(WindowRegistry::new());
    let links = Arc::new(LinkQueue::new(false));
    let session_slot: SessionSlot = Arc::new(OnceLock::new());

    // 3-5. Platform, core, device unlock.
    let (core, platform, extras) = match &dirs {
        Some(dirs) => {
            let (platform, extras) = platform::install(
                &handle,
                &dirs.data,
                links.clone(),
                windows.clone(),
                session_slot.clone(),
            );
            let core = open_core(dirs, platform.clone());
            (core, Some(platform), Some(extras))
        }
        None => (Err(CmdError::code("io_error")), None, None),
    };
    let mut sessions = None;
    if let Ok(core) = &core {
        try_device_unlock(core);
        let unlocked = lock_state(core) == Some(LockState::Unlocked);
        links.set_unlocked(unlocked);
        let manager = Arc::new(new_session_manager(&handle, core.clone(), unlocked));
        let _ = session_slot.set(manager.clone());
        // A lock change between the read above and `set` is caught here.
        manager.set_unlocked(lock_state(core) == Some(LockState::Unlocked));
        sessions = Some(manager);
    }

    let tick_core = core.as_ref().ok().cloned();
    let webview_dir = dirs.as_ref().and_then(|dirs| dirs.webview.clone());
    app.manage(AppState::new(
        core,
        platform,
        extras,
        windows.clone(),
        links.clone(),
        dirs,
        sessions,
    ));

    // 6. First window, menu bar.
    let label = windows.next_label();
    windows::build_window(&handle, &label, webview_dir.as_deref())?;
    tracing::info!(label = %label, "window opened");
    #[cfg(target_os = "macos")]
    if crate::menu_macos::install(&handle, windows.clone()).is_err() {
        tracing::error!("menu bar not installed");
    }

    // 7. Tick.
    if let Some(core) = tick_core {
        spawn_tick(core);
    }

    // 8. A link from the command line.
    // `args_os`: a non-Unicode argument must not panic the app.
    let args = std::env::args_os()
        .skip(1)
        .map(|arg| arg.to_string_lossy().into_owned());
    if let Some(link) = find_link(args) {
        receive_link(&handle, &links, &windows, link);
    }
    Ok(())
}

/// The session manager with the real SSH connector and local terminal.
fn new_session_manager(handle: &AppHandle<Wry>, core: Arc<Core>, unlocked: bool) -> SessionManager {
    SessionManager::new(ManagerParts {
        events: Arc::new(TauriSessionEvents::new(handle.clone())),
        core,
        // e2e builds never use the developer's system agent.
        connector: Arc::new(SshConnector::new(!cfg!(feature = "e2e-mocks"))),
        #[cfg(desktop)]
        local: Arc::new(crate::sessions::local::PtyBackend::new()),
        runtime: tauri::async_runtime::handle().inner().clone(),
        unlocked,
    })
}

/// Opens the core; a panic during development is reported as `internal`.
fn open_core(
    dirs: &AppDirs,
    platform: shellpp_core::platform::Platform,
) -> Result<Arc<Core>, CmdError> {
    let config = CoreConfig {
        data_dir: dirs.data.clone(),
        app_version: env!("CARGO_PKG_VERSION").to_owned(),
    };
    match catch_unwind(AssertUnwindSafe(|| Core::open(config, platform))) {
        Ok(Ok(core)) => {
            let core = Arc::new(core);
            let state = lock_state(&core);
            tracing::info!(state = ?state, "core opened");
            Ok(core)
        }
        Ok(Err(error)) => {
            tracing::error!(code = error.code(), "core did not open");
            Err(CmdError::from(&error))
        }
        Err(_) => {
            tracing::error!("core panicked while opening");
            Err(CmdError::from(CoreError::Internal("core open panicked")))
        }
    }
}

fn lock_state(core: &Core) -> Option<LockState> {
    catch_unwind(AssertUnwindSafe(|| core.lock_status().state)).ok()
}

/// Step 5. Runs on a runtime thread: keychain calls must not run on the main
/// thread (the Linux Secret Service client can deadlock there).
fn try_device_unlock(core: &Arc<Core>) {
    if lock_state(core) != Some(LockState::Locked) {
        return;
    }
    let core = core.clone();
    let result = tauri::async_runtime::block_on(tauri::async_runtime::spawn_blocking(move || {
        core.try_device_unlock()
    }));
    match result {
        Ok(Ok(true)) => tracing::info!("unlocked with the device key"),
        Ok(Ok(false)) => tracing::info!("device key not used"),
        Ok(Err(error)) => tracing::warn!(code = error.code(), "device unlock failed"),
        Err(_) => tracing::error!("device unlock panicked"),
    }
}

/// Step 7: `core.tick()` about once per second on a blocking thread.
fn spawn_tick(core: Arc<Core>) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut reported = false;
        loop {
            interval.tick().await;
            let core = core.clone();
            let result = tauri::async_runtime::spawn_blocking(move || core.tick()).await;
            if result.is_err() && !reported {
                reported = true;
                tracing::error!("tick panicked");
            }
        }
    });
}

/// A second launch handed over its arguments (single instance): bring the
/// app to the front and take its link, if any.
pub fn on_second_instance(app: &AppHandle<Wry>, argv: Vec<String>) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    windows::bring_to_front(app, &state.windows);
    if let Some(link) = find_link(argv.iter().skip(1)) {
        receive_link(app, &state.links, &state.windows, link);
    }
}
