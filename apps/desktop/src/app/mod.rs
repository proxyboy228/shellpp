//! Startup into a window and shutdown.
//!
//! [`run`] takes an already opened core (or the reason it did not open),
//! starts GPUI with gpui-kit, installs the theme, creates the [`AppState`]
//! entity, the session layer (`crate::sessions::ui::install`; the user's
//! `~/.ssh` keys and SSH agent, none for `--demo` and test runs) and the main
//! window (1280x800, frameless, our own title bar), and hooks shutdown:
//! closing the last window quits, and quitting saves the open tabs for
//! Restore, ends the sessions and runs [`AppState::shutdown`] (which also
//! deletes a `--demo` profile). More windows open from the shell
//! ([`crate::shell::windows`]) with [`open_window_with`]; all of them share
//! the app state, the session layer and the lock.

pub mod assets;
pub mod identity;
pub mod root;

use gpui::{
    AnyWindowHandle, App, AppContext as _, Entity, TitlebarOptions, WindowBounds, WindowOptions,
    px, size,
};
use gpui_kit::component::TitleBar;

use crate::i18n;
use crate::sessions::KeySources;
use crate::state::{AppState, OpenedCore};
use crate::theme::{self, Appearance};

pub use root::RootView;

/// Default size of the main window (logical pixels).
pub const DEFAULT_WINDOW_WIDTH: f32 = 1280.;
pub const DEFAULT_WINDOW_HEIGHT: f32 = 800.;
/// Smallest size the window can be resized to (owner's step 1.4, lead
/// decision 4): at 960x600 nothing is clipped or overlaps, with and without
/// the 380 px details panel (the Hosts action bar collapses its labels and
/// wraps; `hosts::tests::min_window_*` check it).
pub const MIN_WINDOW_WIDTH: f32 = 960.;
pub const MIN_WINDOW_HEIGHT: f32 = 600.;
/// Application id (Wayland app id, X11 class).
pub const APP_ID: &str = "shellpp";

/// The app's identity for system notifications (the Windows
/// AppUserModelID).
pub const APP_USER_MODEL_ID: &str = "Shellpp.Desktop";

/// What `main` hands to [`run`].
#[derive(Debug)]
pub struct Launch {
    /// The opened core, or the message to show instead of the app.
    pub opened: Result<OpenedCore, String>,
    pub appearance: Appearance,
}

/// Runs the app until the last window closes. Never returns on macOS.
pub fn run(launch: Launch) {
    let Launch { opened, appearance } = launch;
    gpui_kit::application()
        .with_assets(assets::AppAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            theme::install(appearance, cx);
            root::bind_keys(cx);
            crate::terminal::init(cx);

            // The app's identity (D70; D83, item 4): set at every start and
            // registered for the user at the first one, for the taskbar and
            // the system notifications (the signature question while the
            // window is not focused, D68). A temporary profile or a test run
            // sets and registers none, so it shows no notification.
            let real_profile = matches!(&opened, Ok(opened) if opened.temp_profile.is_none())
                && !crate::dirs::is_test_run();
            identity::apply(real_profile, &identity::System(cx));

            // A temporary profile (`--demo`) or a test run never uses the
            // user's ~/.ssh keys or SSH agent, its Known Hosts import
            // suggests no file instead of the real ~/.ssh/known_hosts, and
            // the Keychain's "Import from ~/.ssh" lists no folder.
            let keys = match &opened {
                Ok(opened) if opened.temp_profile.is_none() && !crate::dirs::is_test_run() => {
                    KeySources::system()
                }
                _ => {
                    crate::known_hosts::set_import_source(None, cx);
                    // Neither the import dialog (UI 2.5) suggests ~/.ssh/config.
                    crate::import_sources::set_default_path(None, cx);
                    crate::keychain::set_ssh_folder(None, cx);
                    KeySources::none()
                }
            };
            let state = cx.new(|cx| match opened {
                Ok(opened) => {
                    let mut state = AppState::new(opened, cx);
                    state.start_tick(cx);
                    state
                }
                Err(message) => AppState::failed(message),
            });
            // The interface zoom of the last launch (D96), before the first
            // window: the lock screen opens at it too.
            if let Some(core) = state.read(cx).core().cloned() {
                theme::zoom::apply_stored(&core, cx);
            }
            // The session layer (its own Tokio runtime) over the core: the
            // session tabs connect through it. Nothing to start without a core.
            if state.read(cx).core().is_some() {
                let options = crate::sessions::ui::options(keys);
                crate::sessions::ui::install(&state, options, cx);
            }

            let Ok((_window, _)) = open_main_window(state.clone(), cx) else {
                tracing::error!("main window did not open");
                state.update(cx, |state, _| state.shutdown());
                cx.quit();
                return;
            };
            // The cold start measurement (dev-tools, SHELLPP_BENCH_STARTUP=1).
            #[cfg(feature = "dev-tools")]
            crate::dev::startup::arm(_window, cx);

            cx.on_window_closed(|cx, _| {
                if cx.windows().is_empty() {
                    cx.quit();
                }
            })
            .detach();
            let quit_state = state.clone();
            cx.on_app_quit(move |cx| {
                // The windows are still open here: edits waiting for the
                // autosave are saved before the core closes, and the open
                // tabs are saved for Restore (UI 6; nothing when the last
                // window closed first, which saved them already).
                crate::shell::flush_all_panels(cx);
                crate::shell::restore::save_last_session(&quit_state, None, cx);
                // Sessions end (their connection log entries close) and let
                // go of the core before it closes.
                crate::shell::release_all_sessions(cx);
                quit_state.update(cx, |state, _| state.shutdown());
                async {}
            })
            .detach();
            cx.activate(true);
        });
}

/// Options of the main window: centered 1280x800, frameless (the title bar
/// is drawn by the app), at least 960x600.
pub fn main_window_options(cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::centered(
            size(px(DEFAULT_WINDOW_WIDTH), px(DEFAULT_WINDOW_HEIGHT)),
            cx,
        )),
        window_min_size: Some(size(px(MIN_WINDOW_WIDTH), px(MIN_WINDOW_HEIGHT))),
        // The OS-level title (taskbar, Alt+Tab); the bar itself is ours.
        titlebar: Some(TitlebarOptions {
            title: Some(i18n::APP_NAME.into()),
            ..TitleBar::title_bar_options()
        }),
        app_id: Some(APP_ID.to_owned()),
        ..TitleBar::window_options()
    }
}

/// Opens the main window with the root view over `state`.
pub fn open_main_window(
    state: Entity<AppState>,
    cx: &mut App,
) -> gpui::Result<(AnyWindowHandle, Entity<RootView>)> {
    let options = main_window_options(cx);
    open_window_with(state, options, cx)
}

/// Opens a window with the root view over `state` and the given options
/// (tests pass a fixed size).
pub fn open_window_with(
    state: Entity<AppState>,
    options: WindowOptions,
    cx: &mut App,
) -> gpui::Result<(AnyWindowHandle, Entity<RootView>)> {
    // A click on a system notification finds its window (D68; the last
    // registration replaces the earlier ones).
    cx.on_system_notification_response(|response, cx| {
        crate::shell::open_notification(&response.tag, cx)
    });
    gpui_kit::open_window(options, cx, |window, cx| {
        cx.new(|cx| RootView::new(state, window, cx))
    })
}
