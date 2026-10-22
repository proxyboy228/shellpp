//! Shell actions (app menu UI 1.4, shortcuts UI 12.5) and their key
//! bindings. The bindings live in the `Shell` key context, so they work
//! anywhere in an unlocked window; a focused text field keeps its own keys
//! (its context is deeper). Ctrl+Shift+L (lock) is bound in every context:
//! while a kit dialog such as Settings has the focus, the shell is not on
//! the dispatch path and a global handler locks instead (UI 13.3).

use gpui::{App, KeyBinding};

gpui_kit::actions!(
    shell,
    [
        /// Ctrl+1: the Vaults tab.
        ShowVaults,
        /// The Hosts screen (unassigned by default, UI 12.5).
        ShowHosts,
        /// Ctrl+2: the SFTP tab.
        ShowSftp,
        /// Ctrl+,: the Settings window (UI 12.1).
        OpenSettings,
        /// Ctrl+L: new local terminal tab.
        NewLocalTerminal,
        /// Ctrl+Alt+S: new serial connection tab.
        NewSerialConnection,
        /// Import Data… of the app menu (D38): a Shell++ export.
        ImportData,
        /// Export Data… of the app menu (D38): all data as JSON.
        ExportData,
        /// "+" of the top bar, Ctrl+T (outside a terminal) and Ctrl+Shift+T:
        /// the new tab page (UI 6).
        NewTab,
        /// Ctrl+K (outside a terminal) and Ctrl+Shift+K: the search over
        /// open tabs and hosts as an overlay (UI 6).
        OpenSearch,
        ZoomIn,
        ZoomOut,
        ActualSize,
        ToggleFullScreen,
        ToggleSidebar,
        NewWindow,
        /// Alt+Right.
        NextTab,
        /// Alt+Left.
        PreviousTab,
        /// Ctrl+Shift+W: the active pane of a tab with splits, else the tab
        /// (UI 4.7).
        CloseTab,
        /// Ctrl+Shift+B: Broadcast input on or off in the active tab
        /// (UI 4.4).
        ToggleBroadcast,
        OpenDocumentation,
        ReportIssue,
        CheckForUpdates,
        About,
        /// Ctrl+Shift+L: lock the app now (UI 13.3, decision D29).
        LockApp,
        /// Exit; asks first while sessions are open (UI 4.6).
        Exit,
        /// The terminal sidebar button (only on a session tab, UI 4.2).
        ToggleTerminalSidebar,
    ]
);

/// Ctrl+3..9: go to tab N (1 = Vaults, 2 = SFTP, 3.. = session tabs).
#[derive(Clone, Debug, PartialEq, Eq, gpui::Action)]
#[action(namespace = shell, no_json)]
pub struct ActivateTab(pub usize);

/// Key context of the shell.
pub const KEY_CONTEXT: &str = "Shell";

/// The shortcuts of UI 12.5 (Windows/Linux set; `secondary` is Cmd on macOS).
pub fn bind_keys(cx: &mut App) {
    let context = Some(KEY_CONTEXT);
    let mut bindings = vec![
        KeyBinding::new("secondary-1", ShowVaults, context),
        KeyBinding::new("secondary-2", ShowSftp, context),
        KeyBinding::new("secondary-,", OpenSettings, context),
        KeyBinding::new("secondary-l", NewLocalTerminal, context),
        KeyBinding::new("secondary-shift-l", LockApp, None),
        KeyBinding::new("secondary-alt-s", NewSerialConnection, context),
        KeyBinding::new("secondary-shift-w", CloseTab, context),
        KeyBinding::new("secondary-shift-b", ToggleBroadcast, context),
        // Ctrl+T and Ctrl+K reach the shell only outside a terminal (the
        // terminal keeps Ctrl+letter for the shell); the Ctrl+Shift keys
        // work everywhere (UI 12.5).
        KeyBinding::new("secondary-t", NewTab, context),
        KeyBinding::new("secondary-shift-t", NewTab, context),
        KeyBinding::new("secondary-k", OpenSearch, context),
        KeyBinding::new("secondary-shift-k", OpenSearch, context),
        KeyBinding::new("alt-right", NextTab, context),
        KeyBinding::new("alt-left", PreviousTab, context),
    ];
    for number in 3..=9 {
        bindings.push(KeyBinding::new(
            &format!("secondary-{number}"),
            ActivateTab(number),
            context,
        ));
    }
    cx.bind_keys(bindings);
    cx.on_action(super::lock_from_anywhere);
    // The interface zoom where no shell takes it (a dialog of the kit, the
    // lock screen).
    cx.on_action(|_: &ZoomIn, cx| super::zoom_from_anywhere(Some(true), cx));
    cx.on_action(|_: &ZoomOut, cx| super::zoom_from_anywhere(Some(false), cx));
    cx.on_action(|_: &ActualSize, cx| super::zoom_from_anywhere(None, cx));
}
