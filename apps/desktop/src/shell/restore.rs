//! Restore (UI 6; lead decision L5): the windows, tabs, pane trees and the
//! titles set with Rename of the previous launch come back, and their
//! sessions connect. The first saved window is restored into the window
//! where Restore was chosen (its tabs are added), the others open as new
//! windows.
//!
//! The app saves the set when it exits (`crate::app::run`), when its last
//! window closes ([`super::windows::before_close`]) and before it locks
//! (the keys are needed, so a quit while locked keeps what the lock saved)
//! [decision for the lock]. It holds only structure and targets (host
//! record ids, quick-connect `user@host:port`, local shell ids, custom
//! titles, the pane trees with their shares), never secrets or terminal
//! content, in the core's encrypted `last_session` record, which is never
//! synced (docs/data-model.md, section 16.3).
//!
//! What Restore offers is read once, at the first unlock of the run
//! ([`load_previous_launch`]): the saves of this run replace the record,
//! not that copy. Restore can run once per launch [decision].

use gpui::{App, BorrowAppContext as _, Context, Entity, EntityId, Global, Window};
use shellpp_core::service::last_session::{
    LastSession, MAX_PANES, MAX_TABS, MAX_WINDOWS, SavedPane, SavedTab, SavedWindow, check_tab,
};

use super::{Shell, all_shells, sessions, strings, windows};
use crate::panes::{Layout, PaneTarget};
use crate::state::AppState;

/// One window's tabs for Restore: each tab's pane tree and its title set
/// with Rename.
pub type WindowSnapshot = Vec<(Layout<PaneTarget>, Option<String>)>;

/// The session of the previous launch, read at the first unlock.
#[derive(Debug, Default)]
struct PreviousLaunch {
    loaded: bool,
    session: Option<LastSession>,
    used: bool,
}

impl Global for PreviousLaunch {}

/// Reads what Restore offers, once per run (the first unlock; later calls
/// do nothing).
pub fn load_previous_launch(state: &Entity<AppState>, cx: &mut App) {
    if cx
        .try_global::<PreviousLaunch>()
        .is_some_and(|previous| previous.loaded)
    {
        return;
    }
    let Some(core) = state.read(cx).core().cloned() else {
        return;
    };
    if !state.read(cx).is_unlocked() {
        return;
    }
    let session = match core.last_session() {
        Ok(session) => session.filter(LastSession::has_tabs),
        Err(error) => {
            tracing::warn!(code = error.code(), "last session not read");
            None
        }
    };
    cx.set_global(PreviousLaunch {
        loaded: true,
        session,
        used: false,
    });
}

/// Whether Restore has something to bring back (UI 6: the button is
/// disabled otherwise).
pub fn can_restore(cx: &App) -> bool {
    cx.try_global::<PreviousLaunch>()
        .is_some_and(|previous| !previous.used && previous.session.is_some())
}

/// Takes the previous launch's session for Restore (once).
fn take_previous_launch(cx: &mut App) -> Option<LastSession> {
    let previous = cx.try_global::<PreviousLaunch>()?;
    if previous.used {
        return None;
    }
    let session = previous.session.clone();
    cx.update_global::<PreviousLaunch, _>(|previous, _| previous.used = true);
    session
}

/// The saved form of a pane tree (the shapes are the same:
/// docs/data-model.md, section 12).
fn saved_pane(layout: &Layout<PaneTarget>) -> Option<SavedPane> {
    let value = serde_json::to_value(layout).ok()?;
    serde_json::from_value(value).ok()
}

/// The pane tree of a saved tab without the saved hosts removed since
/// (UI 6: they are not opened; their siblings take the space). `None` when
/// no pane is left.
fn layout_of(pane: &SavedPane, state: &AppState) -> Option<Layout<PaneTarget>> {
    let value = serde_json::to_value(pane).ok()?;
    let layout: Layout<PaneTarget> = serde_json::from_value(value).ok()?;
    layout.filter_map(&mut |target| match target {
        PaneTarget::Host { record_id, .. } if state.host(*record_id).is_none() => None,
        _ => Some(target.clone()),
    })
}

/// The saved windows of the snapshots, within the limits of the record
/// (windows, tabs per window, panes in all). Titles are cleaned as Rename
/// keeps them, and a tab the core would refuse is left out, so one bad tab
/// does not lose the whole save.
pub(super) fn saved_windows(snapshots: Vec<WindowSnapshot>) -> Vec<SavedWindow> {
    let mut panes = 0;
    let mut windows = Vec::new();
    for snapshot in snapshots.into_iter().take(MAX_WINDOWS) {
        let mut tabs = Vec::new();
        for (layout, title) in snapshot {
            if tabs.len() == MAX_TABS || panes + layout.leaf_count() > MAX_PANES {
                break;
            }
            let Some(root) = saved_pane(&layout) else {
                tracing::warn!("a tab could not be saved for Restore");
                continue;
            };
            let tab = SavedTab {
                title: title.as_deref().and_then(sessions::clean_title),
                root,
            };
            match check_tab(&tab) {
                Ok(count) => {
                    panes += count;
                    tabs.push(tab);
                }
                Err(_) => tracing::warn!("a tab could not be saved for Restore"),
            }
        }
        windows.push(SavedWindow { tabs });
    }
    windows
}

/// Saves the tabs of every window for Restore. `own` is the snapshot of a
/// shell that is being updated (its entity cannot be read now). Does
/// nothing while locked, and when no window is left (the app quits after
/// its last window closed, which saved already).
pub fn save_last_session(
    state: &Entity<AppState>,
    own: Option<(EntityId, WindowSnapshot)>,
    cx: &mut App,
) {
    if !state.read(cx).is_unlocked() {
        return;
    }
    let Some(core) = state.read(cx).core().cloned() else {
        return;
    };
    let shells = all_shells(cx);
    if shells.is_empty() && own.is_none() {
        return;
    }
    let mut own = own;
    let mut snapshots: Vec<WindowSnapshot> = Vec::new();
    for shell in shells {
        match own.take() {
            Some((id, snapshot)) if id == shell.entity_id() => snapshots.push(snapshot),
            other => {
                own = other;
                snapshots.push(shell.read(cx).snapshot(cx));
            }
        }
    }
    if let Some((_, snapshot)) = own {
        snapshots.push(snapshot);
    }
    if let Err(error) = core.last_session_save(saved_windows(snapshots)) {
        tracing::warn!(code = error.code(), "tabs not saved for Restore");
    }
}

/// [`save_last_session`] from inside a shell's update: `own` is that
/// shell's snapshot.
pub(super) fn save_from(shell: &Shell, cx: &mut Context<Shell>) {
    let own = (cx.entity_id(), shell.snapshot(cx));
    let state = shell.state.clone();
    save_last_session(&state, Some(own), cx);
}

/// Restore (UI 6): the previous launch's first window into `shell`, the
/// others as new windows; their sessions connect. Returns the number of
/// tabs opened in this window. A notification (UI 1.5: the notification
/// center and a toast) says when nothing could be restored.
pub fn restore_previous_launch(
    shell: &mut Shell,
    window: &mut Window,
    cx: &mut Context<Shell>,
) -> usize {
    let Some(session) = take_previous_launch(cx) else {
        return 0;
    };
    let state = shell.state.clone();
    let mut windows: Vec<Vec<(Layout<PaneTarget>, Option<String>)>> = session
        .windows
        .into_iter()
        .map(|saved| {
            saved
                .tabs
                .into_iter()
                .filter_map(|tab| Some((layout_of(&tab.root, state.read(cx))?, tab.title)))
                .collect()
        })
        .collect();
    windows.retain(|tabs| !tabs.is_empty());
    let mut windows_left = windows.into_iter();
    let mut opened = 0;
    if let Some(first) = windows_left.next() {
        for (layout, title) in first {
            shell.open_snapshot(layout, title, cx);
            opened += 1;
        }
    }
    let mut others = 0;
    for (step, tabs) in windows_left.enumerate() {
        others += tabs.len();
        windows::open_in_new_window_at(
            shell.state.clone(),
            window,
            step + 1,
            cx,
            move |shell, _, cx| {
                for (layout, title) in tabs {
                    shell.open_snapshot(layout, title, cx);
                }
            },
        );
    }
    if opened + others == 0 {
        super::notify(strings::RESTORE_NOTHING, window, cx);
    }
    opened
}

/// Tests: forget what was read, so the next unlock reads again (a new run).
#[cfg(test)]
pub(crate) fn forget_previous_launch(cx: &mut App) {
    cx.set_global(PreviousLaunch::default());
}

#[cfg(test)]
mod tests {
    use shellpp_core::service::last_session::{MAX_TITLE, check_windows};

    use super::*;
    use crate::hosts::QuickTarget;

    fn local() -> Layout<PaneTarget> {
        Layout::leaf(PaneTarget::Local { shell_id: None })
    }

    /// One bad tab does not lose the save: titles are cleaned, and a tab the
    /// core refuses is left out while the others are kept.
    #[test]
    fn a_bad_tab_is_left_out_and_titles_are_cleaned() {
        let refused = Layout::leaf(PaneTarget::Quick(QuickTarget {
            username: None,
            host: "bad host".into(),
            port: None,
        }));
        let windows = saved_windows(vec![vec![
            (local(), Some("prod\tdb".to_owned())),
            (local(), Some("t".repeat(MAX_TITLE + 5))),
            (refused, Some("refused".to_owned())),
            (local(), Some("\u{7}".to_owned())),
        ]]);
        assert!(check_windows(&windows).is_ok());
        let titles: Vec<Option<usize>> = windows[0]
            .tabs
            .iter()
            .map(|tab| tab.title.as_ref().map(|title| title.chars().count()))
            .collect();
        assert_eq!(titles, vec![Some(6), Some(MAX_TITLE), None]);
        assert_eq!(windows[0].tabs[0].title.as_deref(), Some("proddb"));
    }
}
