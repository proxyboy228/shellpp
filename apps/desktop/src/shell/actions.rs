//! Shell actions (app menu UI 1.4, shortcuts UI 12.5) and their key
//! bindings. The bindings live in the `Shell` key context, so they work
//! anywhere in an unlocked window; a focused text field keeps its own keys
//! (its context is deeper).

use gpui::{App, KeyBinding};

gpui_kit::actions!(
    shell,
    [
        /// Ctrl+1: the Vaults tab.
        ShowVaults,
        /// Ctrl+2: the SFTP tab.
        ShowSftp,
        /// Ctrl+,: settings (placeholder).
        OpenSettings,
        /// Ctrl+L, Ctrl+Shift+L: new local terminal tab.
        NewLocalTerminal,
        /// Ctrl+Alt+S: new serial connection tab.
        NewSerialConnection,
        /// "+" of the top bar: the new tab page.
        NewTab,
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
        /// Ctrl+Shift+W.
        CloseTab,
        OpenDocumentation,
        ReportIssue,
        CheckForUpdates,
        About,
        /// Lock the app now (UI 13.3).
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
        KeyBinding::new("secondary-shift-l", NewLocalTerminal, context),
        KeyBinding::new("secondary-alt-s", NewSerialConnection, context),
        KeyBinding::new("secondary-shift-w", CloseTab, context),
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
}
