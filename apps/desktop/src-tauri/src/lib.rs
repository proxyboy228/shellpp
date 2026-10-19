//! Shell++ desktop host (crate `shellpp-desktop`): windows, commands, events and
//! platform implementations around `shellpp-core` (docs/architecture.md, sections
//! 2.4, 3 and 5).
//!
//! Module map:
//! - [`startup`]: the startup sequence (directories, logging, hardening,
//!   platform, core, device unlock, first window, tick).
//! - [`commands`]: every IPC command, its lock guard and [`error::CmdError`].
//! - [`events`]: core events to all windows; `shellpp://` link delivery;
//!   window-targeted session events.
//! - [`sessions`]: SSH sessions and local terminals (stage 2).
//! - [`windows`]: `w-<n>` windows, focus tracking, navigation guard, intents.
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
pub mod sessions;
pub mod startup;
pub mod state;
pub mod windows;

use tauri::webview::PageLoadEvent;
use tauri::{Manager, RunEvent, WindowEvent};

pub use windows::FIRST_WINDOW_LABEL;

use crate::state::AppState;

/// Builds and runs the Tauri application.
pub fn run() {
    // The launch time tells a window which `session.last` is the previous launch's.
    let _ = commands::app::launched_at();
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
        // A reloaded page lost its output channels (stage 2 plan, decision 3).
        .on_page_load(|webview, payload| {
            if payload.event() == PageLoadEvent::Started
                && let Some(state) = webview.try_state::<AppState>()
                && let Some(sessions) = &state.sessions
            {
                sessions.detach_window(webview.label());
            }
        })
        .on_window_event(|window, event| {
            let Some(state) = window.try_state::<AppState>() else {
                return;
            };
            match event {
                WindowEvent::Focused(true) => state.windows.focused(window.label()),
                // Closing a window with active sessions asks first (stage 2
                // plan, decision 17): the window shows its dialog and calls
                // `window_close` again with `confirmed`.
                WindowEvent::CloseRequested { api, .. } => {
                    if let Some(sessions) = &state.sessions
                        && sessions.active_count_of(window.label()) > 0
                    {
                        api.prevent_close();
                        events::emit_close_requested(window.app_handle(), window.label());
                    }
                }
                WindowEvent::Destroyed => {
                    state.windows.destroyed(window.label());
                    // A window's sessions end with it (D23, plan decision 28).
                    if let Some(sessions) = &state.sessions {
                        let logs = sessions.close_window(window.label());
                        sessions.end_logs_later(logs);
                    }
                }
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
            commands::app::window_close,
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
            // sessions (stage 2)
            commands::sessions::session_open,
            commands::sessions::session_attach,
            commands::sessions::session_write,
            commands::sessions::session_inject,
            commands::sessions::session_resize,
            commands::sessions::session_prompt_reply,
            commands::sessions::session_cancel,
            commands::sessions::session_close,
            commands::sessions::session_reconnect,
            commands::sessions::session_ack,
            // local terminal
            commands::local::local_shells,
            commands::local::local_open,
            // known hosts
            commands::knownhosts::known_hosts_list,
            commands::knownhosts::known_host_remove,
            // keys
            commands::keys::keys_list,
            commands::keys::key_import,
            // history and recent connections
            commands::history::history_list,
            commands::history::history_delete,
            commands::history::history_clear,
            commands::history::recent_connections,
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
