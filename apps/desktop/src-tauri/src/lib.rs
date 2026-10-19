//! Shell++ desktop host (crate `shellpp-desktop`): windows, commands, events and
//! platform implementations around `shellpp-core` (docs/architecture.md, sections
//! 2.4, 3 and 5).
//!
//! Module map:
//! - [`startup`]: the startup sequence (directories, logging, hardening,
//!   platform, core, device unlock, first window, tick).
//! - [`commands`]: every IPC command, its lock guard and [`error::CmdError`].
//! - [`events`]: core events to all windows; `shellpp://` link delivery.
//! - [`windows`]: `w-<n>` windows, focus tracking, navigation guard.
//! - [`links`], [`dirs`], [`logging`], [`hardening`], [`platform`].
//!
//! Owner: W3.

#[cfg(all(feature = "e2e-mocks", not(debug_assertions)))]
compile_error!("the e2e-mocks feature must never be enabled in a release build");

pub mod commands;
pub mod dirs;
pub mod error;
pub mod events;
pub mod hardening;
pub mod links;
pub mod logging;
#[cfg(any(target_os = "macos", test))]
#[cfg_attr(test, allow(dead_code))]
mod menu_macos;
pub mod platform;
pub mod startup;
pub mod state;
pub mod windows;

use tauri::{Manager, RunEvent, WindowEvent};

pub use windows::FIRST_WINDOW_LABEL;

use crate::state::AppState;

/// Builds and runs the Tauri application.
pub fn run() {
    let builder = tauri::Builder::default();

    // Single instance first: a second launch hands its arguments (a
    // `shellpp://` link) to this process and exits. Not in e2e builds, so a
    // test run never forwards its launch to a developer's running instance.
    #[cfg(not(feature = "e2e-mocks"))]
    let builder = builder.plugin(tauri_plugin_single_instance::init(|app, argv, _cwd| {
        startup::on_second_instance(app, argv);
    }));

    let app = builder
        .plugin(tauri_plugin_clipboard_manager::init())
        // No JS link interception: the webview opens links only through
        // `external_open_url`.
        .plugin(
            tauri_plugin_opener::Builder::new()
                .open_js_links_on_click(false)
                .build(),
        )
        .setup(startup::setup)
        .on_window_event(|window, event| {
            let Some(state) = window.try_state::<AppState>() else {
                return;
            };
            match event {
                WindowEvent::Focused(true) => state.windows.focused(window.label()),
                WindowEvent::Destroyed => state.windows.destroyed(window.label()),
                _ => {}
            }
        })
        // Keep this list in the order of COMMANDS in build.rs (a test checks it).
        .invoke_handler(tauri::generate_handler![
            // app
            commands::app::app_info,
            commands::app::state_snapshot,
            commands::app::window_open,
            commands::app::activity,
            commands::app::app_exit,
            commands::app::external_open_url,
            // lock
            commands::lock::lock_status,
            commands::lock::first_run_setup,
            commands::lock::unlock,
            commands::lock::lock,
            commands::lock::recovery_unlock,
            commands::lock::password_change,
            commands::lock::recovery_key_show,
            commands::lock::recovery_key_reissue,
            commands::lock::remember_device_set,
            // records
            commands::records::records_query,
            commands::records::record_get,
            commands::records::record_save,
            commands::records::record_delete,
            commands::records::record_restore,
            commands::records::record_move,
            commands::records::record_duplicate,
            commands::records::secret_reveal,
            commands::records::secret_copy,
            // settings
            commands::settings::settings_get,
            commands::settings::settings_set,
            // notifications
            commands::notifications::notifications_list,
            commands::notifications::notifications_mark_read,
            // clipboard
            commands::clipboard::clipboard_write_text,
            commands::clipboard::clipboard_read_text,
            // e2e only
            #[cfg(feature = "e2e-mocks")]
            commands::e2e::e2e_mock,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Shell++");

    app.run(|app, event| match event {
        RunEvent::ExitRequested { .. } | RunEvent::Exit => {
            if let Some(state) = app.try_state::<AppState>() {
                state.shutdown();
            }
        }
        #[cfg(target_os = "macos")]
        RunEvent::Opened { urls } => {
            if let Some(state) = app.try_state::<AppState>()
                && let Some(link) = links::find_link(urls.iter().map(|url| url.as_str()))
            {
                events::receive_link(app, &state.links, &state.windows, link);
            }
        }
        _ => {}
    });
}
