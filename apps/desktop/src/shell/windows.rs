//! Several app windows (UI 1.4; lead decision L3). Every window has its own
//! shell: top bar, Vaults and SFTP tabs, session tabs, details panel and
//! terminal sidebar. The app state, the session layer and the lock are
//! shared, so locking covers every window (each root view follows the lock
//! state).
//!
//! A new window opens with Window > New Window, Duplicate in a New Window
//! (UI 4.4) and Alt held on a connect action (UI 2.4). It has the size of
//! the window it was opened from and is placed `--space-6` further right
//! and down [decision]. Closing a window with active sessions asks first
//! (UI 4.6, [`super::confirm_close_window`]); the app quits when its last
//! window closes (`crate::app::run`), and the last window saves its tabs
//! for Restore before it goes ([`before_close`]).

use gpui::{AnyWindowHandle, App, Bounds, Context, Entity, Window, WindowBounds, point};

use super::{SessionId, Shell, TopTab, all_shells, all_windows, restore};
use crate::state::AppState;

/// How far a new window is placed from the one it was opened from, right
/// and down [decision]: `--space-6`.
pub const WINDOW_OFFSET: gpui::Pixels = gpui::px(32.);

/// Opens a new app window over `state`, at the size of `from` and offset by
/// [`WINDOW_OFFSET`], and runs `then` on its shell (to open a session tab
/// there). Runs after the current update; nothing runs when the window
/// shows no shell (the app is locked meanwhile).
pub fn open_in_new_window(
    state: Entity<AppState>,
    from: &Window,
    cx: &mut App,
    then: impl FnOnce(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
) {
    open_in_new_window_at(state, from, 1, cx, then);
}

/// [`open_in_new_window`] placed `step` offsets away from `from` (Restore
/// opens its windows as a cascade).
pub fn open_in_new_window_at(
    state: Entity<AppState>,
    from: &Window,
    step: usize,
    cx: &mut App,
    then: impl FnOnce(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
) {
    let from = from.bounds();
    let offset = WINDOW_OFFSET * step as f32;
    let bounds = Bounds::new(from.origin + point(offset, offset), from.size);
    cx.defer(move |cx| {
        let mut options = crate::app::main_window_options(cx);
        options.window_bounds = Some(WindowBounds::Windowed(bounds));
        let (handle, root) = match crate::app::open_window_with(state, options, cx) {
            Ok(opened) => opened,
            Err(_) => {
                tracing::warn!("new window did not open");
                return;
            }
        };
        let Some(shell) = root.read(cx).shell().cloned() else {
            return;
        };
        let _ = handle.update(cx, |_, window, cx| {
            shell.update(cx, |shell, cx| then(shell, window, cx));
            window.activate_window();
        });
    });
}

/// A window is about to close (no sessions, or the user confirmed): the
/// last window saves the open tabs for Restore (UI 6), then the details
/// panel's pending edit is saved and the window's sessions end.
pub(super) fn before_close(shell: &Entity<Shell>, cx: &mut App) {
    if all_shells(cx).len() <= 1 {
        let state = shell.read(cx).state.clone();
        restore::save_last_session(&state, None, cx);
    }
    shell.update(cx, |shell, cx| {
        shell.flush_panel(cx);
        shell.shutdown_sessions(cx);
    });
}

/// Brings window `handle` to the front with its session tab `tab` active
/// (the search's open tabs, UI 6). Call it outside that window's update.
pub fn activate_tab_in(handle: AnyWindowHandle, tab: SessionId, cx: &mut App) {
    let Some(shell) = all_windows(cx)
        .into_iter()
        .find(|(window, _)| window.window_id() == handle.window_id())
        .map(|(_, shell)| shell)
    else {
        return;
    };
    let _ = handle.update(cx, |_, window, cx| {
        window.activate_window();
        shell.update(cx, |shell, cx| {
            shell.set_top_tab(TopTab::Session(tab), cx);
            shell.focus_active_tab(cx);
        });
    });
}
