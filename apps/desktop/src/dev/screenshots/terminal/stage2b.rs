//! The session states of stage 2b (owner's step 5.1) in the screenshot run:
//! splits, the tab menu, the new tab page, the terminal sidebar and the
//! connecting tab. Like the stage 2a states, each starts from the reset
//! demo window (no session tabs, the sidebar hidden) and opens SSH tabs to
//! the test server as Connect does; the UI is driven the way a user does
//! it where that is reliable offscreen: the split keys (Ctrl+Shift+D,
//! Ctrl+Shift+E) and Enter in the new pane's host list, a right click on
//! the tab, the "+" button, typing into the search fields, Ctrl+Shift+F,
//! the sidebar's button and tabs. Which pane is active between two splits
//! is set through the pane group (a click in a terminal could start a
//! selection).
//!
//! - **Connecting**: the test server holds new connections
//!   ([`Rig::freeze`]: OpenSSH's container paused, `hold on` on the russh
//!   server) until the next state starts ([`super::reach`] thaws it).
//! - **History**: the commands are typed into the session, so the terminal
//!   captures them (docs/security.md 10.1) and the core keeps them for the
//!   test server's saved host; the hint card shows the most frequent ones.
//!
//! Nothing here logs addresses, user names, secrets or terminal text.

use std::time::{Duration, Instant};

use gpui::{App, AppContext as _, AsyncApp, Entity, Window};
use gpui_kit::test::TestWindowExt as _;

use super::super::{
    NEW_TAB_SEARCH, ScreenshotError, ShotState, click, press, settle, shell_of, type_into, visible,
};
use super::{
    HTOP_MARKER, LS_LONG_MARKER, LS_MARKER, RUSSH_ALT_MARKER, RUSSH_COLOR_MARKER, Rig,
    STEP_TIMEOUT, active_pane, clear, colored_output, connect, open_ssh, render, run, shows, sleep,
    ssh_session, type_line, wait_prompt, wait_until,
};
use crate::app::RootView;
use crate::panes::{PaneGroup, PaneId, PaneStatus};
use crate::session_ui::{Phase, SessionPane};
use crate::shell::{SessionId, SessionKind, Shell, TopTab};
use crate::state::AppState;
use crate::terminal_sidebar::SidebarTab;

/// The states of this module.
pub const STATES: [ShotState; 11] = [
    ShotState::TerminalConnecting,
    ShotState::TerminalSplits,
    ShotState::SplitPicker,
    ShotState::TabMenu,
    ShotState::NewTab,
    ShotState::NewTabSearch,
    ShotState::SidebarTools,
    ShotState::SidebarSnippets,
    ShotState::SidebarHistory,
    ShotState::SidebarAppearance,
    ShotState::SidebarFont,
];

/// Ctrl+Shift+D: a new pane to the right (UI 12.5).
const SPLIT_RIGHT: &str = "ctrl-shift-d";
/// Ctrl+Shift+E: a new pane below.
const SPLIT_DOWN: &str = "ctrl-shift-e";
/// Ctrl+Shift+F: the sidebar's Tools with the search focused.
const FIND: &str = "ctrl-shift-f";
/// The top bar button of the terminal sidebar.
const SIDEBAR_BUTTON: &str = "top-bar-terminal-sidebar";
/// The "+" of the session tabs.
const NEW_TAB_BUTTON: &str = "tab-new";
/// The search field of the sidebar's Tools.
const TOOLS_SEARCH: &str = "terminal-search";
/// The search field of the new tab page.
const NEW_TAB_SEARCH_FIELD: &str = "new-tab-search";
/// What Tools searches for: several matches in the `ls` output on OpenSSH
/// (the `projects` folder and the `current` link to it), in the color
/// sample on the russh server.
const TOOLS_QUERY_REAL: &str = "projects";
const TOOLS_QUERY_RUSSH: &str = "3";
/// Commands of the History state (typed after the colored output: `clear`,
/// `ls --color`, `ls -l --color` on OpenSSH), each with what its output
/// shows; some repeat, so the hint has an order.
const HISTORY_REAL: [(&str, &str); 4] = [
    ("pwd", "/home/pwuser"),
    ("uptime", "load average"),
    ("ls --color", LS_MARKER),
    ("uptime", "load average"),
];
const HISTORY_RUSSH: [(&str, &str); 4] = [
    ("echo deploy done", "deploy done"),
    ("term", "xterm"),
    ("color", RUSSH_COLOR_MARKER),
    ("echo deploy done", "deploy done"),
];
/// Commands the History state waits for in the list.
const HISTORY_MIN: usize = 3;
/// How long the held connection is left alone before the capture.
const HOLD: Duration = Duration::from_millis(1500);

fn failed(shot: ShotState, reason: &str) -> ScreenshotError {
    ScreenshotError::state(shot, reason)
}

/// The panes of session tab `id`.
fn group_of(
    cx: &mut AsyncApp,
    shell: &Entity<Shell>,
    id: SessionId,
    shot: ShotState,
) -> Result<Entity<PaneGroup>, ScreenshotError> {
    cx.update(|cx| shell.read(cx).group(id).cloned())
        .ok_or_else(|| failed(shot, "the tab has no panes"))
}

/// Makes pane `pane` of `group` active (its terminal takes the focus).
fn activate(
    cx: &mut AsyncApp,
    handle: gpui::AnyWindowHandle,
    group: &Entity<PaneGroup>,
    pane: PaneId,
) -> Result<(), ScreenshotError> {
    cx.update(|cx| group.update(cx, |group, cx| group.activate(pane, cx)));
    render(cx, handle)
}

/// Presses a split key in the active pane of tab `id`: a new pane shows
/// the host list, active. Returns the new pane.
async fn split_key(
    cx: &mut AsyncApp,
    handle: gpui::AnyWindowHandle,
    group: &Entity<PaneGroup>,
    key: &str,
    shot: ShotState,
) -> Result<PaneId, ScreenshotError> {
    let before = cx.update(|cx| group.read(cx).pane_count());
    render(cx, handle)?;
    press(cx, handle, key)?;
    wait_until(
        cx,
        handle,
        shot,
        "the new pane's host list",
        STEP_TIMEOUT,
        |cx| {
            let group = group.read(cx);
            Ok(group.pane_count() == before + 1 && group.shows_picker(group.active_pane()))
        },
    )
    .await?;
    // The list is created and takes the focus when drawn.
    settle(cx, handle).await?;
    Ok(cx.update(|cx| group.read(cx).active_pane()))
}

/// [`split_key`], then Enter in the new pane's host list (its first row is
/// the split pane's host): a session to the same host, connected and at
/// its prompt.
async fn split_and_open(
    cx: &mut AsyncApp,
    handle: gpui::AnyWindowHandle,
    group: &Entity<PaneGroup>,
    key: &str,
    rig: &Rig,
    shot: ShotState,
) -> Result<(PaneId, Entity<SessionPane>), ScreenshotError> {
    let pane_id = split_key(cx, handle, group, key, shot).await?;
    press(cx, handle, "enter")?;
    wait_until(
        cx,
        handle,
        shot,
        "a session in the new pane",
        STEP_TIMEOUT,
        |cx| Ok(group.read(cx).session(pane_id).is_some()),
    )
    .await?;
    let pane = cx
        .update(|cx| group.read(cx).session(pane_id).cloned())
        .ok_or_else(|| failed(shot, "the new pane has no session"))?;
    connect(cx, handle, &pane, Some(rig), None, shot).await?;
    wait_prompt(cx, handle, &pane, shot).await?;
    if rig.real_shell() {
        // The login banner goes, as in the first pane.
        clear(cx, handle, &pane, shot, "clear").await?;
    }
    Ok((pane_id, pane))
}

/// Brings the reset demo window into the stage 2b session state `shot`.
pub async fn reach(
    cx: &mut AsyncApp,
    handle: gpui::AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    rig: &Rig,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    if shot == ShotState::TerminalConnecting {
        rig.freeze()?;
        let (_, pane) = open_ssh(cx, handle, root, state, rig, 0, shot).await?;
        sleep(cx, HOLD).await;
        render(cx, handle)?;
        let phase = cx.update(|cx| pane.read(cx).phase().clone());
        return if matches!(phase, Phase::Connecting(_)) {
            Ok(())
        } else {
            Err(failed(shot, "the session is not connecting"))
        };
    }
    let shell = shell_of(cx, root, shot)?;
    if matches!(
        shot,
        ShotState::SplitPicker | ShotState::NewTab | ShotState::NewTabSearch
    ) {
        seed_recent_connections(cx, handle, root, state, rig, &shell, shot).await?;
    }
    let (id, first) = ssh_session(cx, handle, root, state, rig, 0, shot).await?;
    colored_output(cx, handle, &first, rig, shot).await?;
    let real = rig.real_shell();
    match shot {
        ShotState::TerminalSplits => {
            let group = group_of(cx, &shell, id, shot)?;
            let top_left = cx.update(|cx| group.read(cx).active_pane());
            // Top right: a full-screen program.
            let (top_right, pane) =
                split_and_open(cx, handle, &group, SPLIT_RIGHT, rig, shot).await?;
            let (program, marker) = if real {
                ("htop", HTOP_MARKER)
            } else {
                ("alt-screen", RUSSH_ALT_MARKER)
            };
            type_line(cx, handle, &pane, program)?;
            wait_until(
                cx,
                handle,
                shot,
                "the full-screen program",
                STEP_TIMEOUT,
                |cx| Ok(shows(&pane, marker, cx)),
            )
            .await?;
            // Bottom left and bottom right.
            activate(cx, handle, &group, top_left)?;
            let (_, pane) = split_and_open(cx, handle, &group, SPLIT_DOWN, rig, shot).await?;
            let (line, marker) = if real {
                ("ls -l --color", LS_LONG_MARKER)
            } else {
                ("unicode", "wide:")
            };
            run(cx, handle, &pane, shot, line, marker).await?;
            activate(cx, handle, &group, top_right)?;
            let (_, pane) = split_and_open(cx, handle, &group, SPLIT_DOWN, rig, shot).await?;
            let (line, marker) = if real {
                ("uptime", "load average")
            } else {
                ("color", RUSSH_COLOR_MARKER)
            };
            run(cx, handle, &pane, shot, line, marker).await?;
            activate(cx, handle, &group, top_left)
        }
        ShotState::SplitPicker => {
            let group = group_of(cx, &shell, id, shot)?;
            split_key(cx, handle, &group, SPLIT_RIGHT, shot)
                .await
                .map(|_| ())
        }
        ShotState::TabMenu => {
            let group = group_of(cx, &shell, id, shot)?;
            split_and_open(cx, handle, &group, SPLIT_RIGHT, rig, shot).await?;
            settle(cx, handle).await?;
            let tab = cx
                .update(|cx| {
                    shell
                        .read(cx)
                        .sessions()
                        .get(id)
                        .map(|tab| tab.element_id())
                })
                .ok_or_else(|| failed(shot, "no tab"))?;
            cx.update_window(handle, |_, window, cx| {
                window.render_frame(cx);
                window.right_click(tab, cx);
            })
            .map_err(|_| ScreenshotError::WindowClosed)
        }
        ShotState::NewTab | ShotState::NewTabSearch => {
            click(cx, handle, NEW_TAB_BUTTON)?;
            settle(cx, handle).await?;
            if shot == ShotState::NewTabSearch {
                type_into(cx, handle, NEW_TAB_SEARCH_FIELD, NEW_TAB_SEARCH)?;
            }
            Ok(())
        }
        ShotState::SidebarTools => {
            // The focus is in the terminal (the last command was typed
            // there): Ctrl+Shift+F opens the sidebar on Tools.
            press(cx, handle, FIND)?;
            settle(cx, handle).await?;
            let query = if real {
                TOOLS_QUERY_REAL
            } else {
                TOOLS_QUERY_RUSSH
            };
            type_into(cx, handle, TOOLS_SEARCH, query)?;
            wait_until(cx, handle, shot, "the search matches", STEP_TIMEOUT, |cx| {
                Ok(match_count(&shell, cx) >= 2)
            })
            .await
        }
        ShotState::SidebarSnippets
        | ShotState::SidebarHistory
        | ShotState::SidebarAppearance
        | ShotState::SidebarFont => {
            if shot == ShotState::SidebarHistory {
                let commands = if real { HISTORY_REAL } else { HISTORY_RUSSH };
                for (line, marker) in commands {
                    run(cx, handle, &first, shot, line, marker).await?;
                }
            }
            open_sidebar(cx, handle, &shell, shot).await?;
            let tab = match shot {
                ShotState::SidebarSnippets => SidebarTab::Snippets,
                ShotState::SidebarHistory => SidebarTab::History,
                _ => SidebarTab::Appearance,
            };
            click(cx, handle, tab.element_id())?;
            settle(cx, handle).await?;
            match shot {
                ShotState::SidebarFont => click(cx, handle, "terminal-appearance-font"),
                ShotState::SidebarHistory => wait_history(cx, handle, &shell, shot).await,
                _ => Ok(()),
            }
        }
        _ => Err(failed(shot, "not a stage 2b session state")),
    }
}

/// Recent connections for the host lists (UI 6, 4.7): one session each to
/// the test server's other hosts, closed again; once per run (a run limited
/// to these states has no earlier sessions to them).
async fn seed_recent_connections(
    cx: &mut AsyncApp,
    handle: gpui::AnyWindowHandle,
    root: &Entity<RootView>,
    state: &Entity<AppState>,
    rig: &Rig,
    shell: &Entity<Shell>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    if rig.recents_seeded.get() {
        return Ok(());
    }
    for index in 1..super::HOST_LABELS.len() {
        let (id, _) = ssh_session(cx, handle, root, state, rig, index, shot).await?;
        cx.update(|cx| shell.update(cx, |shell, cx| shell.close_session(id, cx)));
        render(cx, handle)?;
    }
    rig.recents_seeded.set(true);
    settle(cx, handle).await
}

/// Shows the terminal sidebar with its top bar button (once).
async fn open_sidebar(
    cx: &mut AsyncApp,
    handle: gpui::AnyWindowHandle,
    shell: &Entity<Shell>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let shown =
        |cx: &mut AsyncApp| cx.update(|cx| shell.read(cx).terminal_sidebar().read(cx).is_visible());
    if !shown(cx) {
        click(cx, handle, SIDEBAR_BUTTON)?;
        settle(cx, handle).await?;
    }
    if shown(cx) {
        Ok(())
    } else {
        Err(failed(shot, "the terminal sidebar did not open"))
    }
}

/// Waits until the History tab lists the typed commands and the hint.
/// The entries are written by the core after each command; the tab is
/// asked to read them again now and then.
async fn wait_history(
    cx: &mut AsyncApp,
    handle: gpui::AnyWindowHandle,
    shell: &Entity<Shell>,
    shot: ShotState,
) -> Result<(), ScreenshotError> {
    let history = cx.update(|cx| shell.read(cx).terminal_sidebar().read(cx).history().clone());
    let deadline = Instant::now() + STEP_TIMEOUT;
    let mut next_reload = Instant::now() + Duration::from_secs(1);
    loop {
        render(cx, handle)?;
        let (entries, hint) = cx.update(|cx| {
            let history = history.read(cx);
            (history.commands().len(), history.hint_commands().len())
        });
        if entries >= HISTORY_MIN && hint > 0 {
            return settle(cx, handle).await;
        }
        if Instant::now() > deadline {
            return Err(failed(
                shot,
                &format!("timeout: the history lists {entries} commands, hint {hint}"),
            ));
        }
        if Instant::now() >= next_reload {
            cx.update(|cx| history.update(cx, |history, cx| history.reload(cx)));
            next_reload = Instant::now() + Duration::from_secs(1);
        }
        sleep(cx, Duration::from_millis(50)).await;
    }
}

/// The matches of the sidebar's search.
fn match_count(shell: &Entity<Shell>, cx: &App) -> usize {
    shell
        .read(cx)
        .terminal_sidebar()
        .read(cx)
        .tools()
        .read(cx)
        .summary()
        .map_or(0, |summary| summary.count)
}

/// The active session tab's id.
fn active_tab(shell: &Entity<Shell>, cx: &App) -> Option<SessionId> {
    match shell.read(cx).top_tab() {
        TopTab::Session(id) => Some(id),
        TopTab::Vaults | TopTab::Sftp => None,
    }
}

/// The active tab's state dot shows `status` (design-system 3.3).
pub fn check_dot(
    window: &mut Window,
    cx: &mut App,
    shell: &Entity<Shell>,
    status: PaneStatus,
) -> Result<(), &'static str> {
    let id = active_tab(shell, cx).ok_or("no session tab is active")?;
    if shell.read(cx).tab_status(id, cx) != Some(status) {
        return Err("the tab's state is not the expected one");
    }
    let dot = shell
        .read(cx)
        .sessions()
        .get(id)
        .map(|tab| tab.dot_element_id())
        .ok_or("no tab")?;
    if visible(window, dot.as_ref()) {
        Ok(())
    } else {
        Err("the tab's state dot is not on screen")
    }
}

/// Checks the stage 2b session state `shot` in a fresh frame.
pub fn check(
    window: &mut Window,
    cx: &mut App,
    shell: &Entity<Shell>,
    real_shell: bool,
    shot: ShotState,
) -> Result<(), &'static str> {
    let need = |ok: bool, reason: &'static str| if ok { Ok(()) } else { Err(reason) };
    let id = active_tab(shell, cx).ok_or("no session tab is active")?;
    let pane = active_pane(shell, cx);
    let running = pane
        .as_ref()
        .is_some_and(|pane| *pane.read(cx).phase() == Phase::Running);
    let group = shell.read(cx).group(id).cloned();
    let sidebar = shell.read(cx).terminal_sidebar().clone();
    match shot {
        ShotState::TerminalConnecting => {
            need(
                pane.as_ref()
                    .is_some_and(|pane| matches!(pane.read(cx).phase(), Phase::Connecting(_))),
                "the session is not connecting",
            )?;
            check_dot(window, cx, shell, PaneStatus::Connecting)
        }
        ShotState::TerminalSplits => {
            let group = group.ok_or("the tab has no panes")?;
            let group = group.read(cx);
            need(group.pane_count() == 4, "not four panes")?;
            need(
                group.sessions().len() == 4
                    && group
                        .sessions()
                        .iter()
                        .all(|session| *session.read(cx).phase() == Phase::Running),
                "not four running sessions",
            )?;
            let active = group.active_pane();
            need(
                visible(window, &format!("pane-{active}-active")),
                "the active pane is not marked",
            )?;
            check_dot(window, cx, shell, PaneStatus::Connected)
        }
        ShotState::SplitPicker => {
            let group = group.ok_or("the tab has no panes")?;
            let group = group.read(cx);
            need(group.pane_count() == 2, "not two panes")?;
            need(
                group.shows_picker(group.active_pane()),
                "the new pane shows no host list",
            )?;
            need(visible(window, "pane-picker"), "no host list on screen")?;
            let host = group.sessions().first().and_then(|session| {
                crate::panes::PaneTarget::from_open(session.read(cx).request()).host_id()
            });
            need(
                host.is_some_and(|host| visible(window, &format!("pane-picker-row-origin-{host}"))),
                "the split pane's host is not the first row",
            )
        }
        ShotState::TabMenu => {
            need(
                group.is_some_and(|group| group.read(cx).pane_count() == 2),
                "not two panes",
            )?;
            for (item, reason) in [
                ("tab-menu-duplicate", "no Duplicate"),
                ("tab-menu-duplicate-window", "no Duplicate in a new window"),
                ("tab-menu-rename", "no Rename"),
                ("tab-menu-split-horizontal", "no Split horizontally"),
                ("tab-menu-split-vertical", "no Split vertically"),
                ("tab-menu-broadcast", "no Broadcast input"),
                ("tab-menu-close", "no Close"),
            ] {
                need(visible(window, item), reason)?;
            }
            Ok(())
        }
        ShotState::NewTab | ShotState::NewTabSearch => {
            let tab_kind = shell.read(cx).sessions().get(id).map(|tab| tab.kind());
            need(
                tab_kind == Some(SessionKind::NewTab),
                "the active tab is not a new tab",
            )?;
            need(visible(window, "new-tab-page"), "no new tab page")?;
            let page = shell
                .read(cx)
                .new_tab_page(id)
                .cloned()
                .ok_or("no new tab page")?;
            let page = page.read(cx);
            need(!page.rows().is_empty(), "the page lists nothing")?;
            if shot == ShotState::NewTabSearch {
                need(
                    page.query().trim() == NEW_TAB_SEARCH,
                    "the search is not typed",
                )?;
                need(
                    !visible(window, "new-tab-empty"),
                    "the search found nothing",
                )
            } else {
                need(page.query().is_empty(), "a search is typed")
            }
        }
        ShotState::SidebarTools
        | ShotState::SidebarSnippets
        | ShotState::SidebarHistory
        | ShotState::SidebarAppearance
        | ShotState::SidebarFont => {
            need(running, "the session does not run")?;
            need(
                sidebar.read(cx).is_visible() && visible(window, "terminal-sidebar"),
                "no terminal sidebar",
            )?;
            match shot {
                ShotState::SidebarTools => {
                    need(sidebar.read(cx).tab() == SidebarTab::Tools, "not on Tools")?;
                    need(match_count(shell, cx) >= 2, "fewer than two matches")?;
                    let query = if real_shell {
                        TOOLS_QUERY_REAL
                    } else {
                        TOOLS_QUERY_RUSSH
                    };
                    need(
                        sidebar.read(cx).tools().read(cx).query(cx).as_ref() == query,
                        "another search",
                    )?;
                    need(visible(window, "terminal-search-count"), "no match counter")
                }
                ShotState::SidebarSnippets => need(
                    visible(window, "sidebar-snippets-list")
                        || visible(window, "terminal-sidebar-snippets-empty"),
                    "no Snippets tab",
                ),
                ShotState::SidebarHistory => {
                    let history = sidebar.read(cx).history().clone();
                    let history = history.read(cx);
                    need(
                        history.commands().len() >= HISTORY_MIN,
                        "too few commands in the history",
                    )?;
                    need(visible(window, "history-list"), "no history list")?;
                    need(visible(window, "history-hint"), "no hint card")
                }
                // The theme list sits in the Appearance page (the list itself
                // has no test id).
                ShotState::SidebarAppearance => {
                    need(
                        sidebar.read(cx).tab() == SidebarTab::Appearance,
                        "not on Appearance",
                    )?;
                    need(
                        visible(window, "terminal-sidebar-appearance"),
                        "no theme list",
                    )?;
                    need(
                        !visible(window, "terminal-sidebar-font"),
                        "the Font page is open",
                    )
                }
                _ => need(visible(window, "terminal-sidebar-font"), "no Font page"),
            }
        }
        _ => Err("not a stage 2b session state"),
    }
}
