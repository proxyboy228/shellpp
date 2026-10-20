//! The new tab page and its search (UI 6, 4.7; stage 2b lead decisions L1
//! and L4). Three views share one search ([`picker`]):
//!
//! - [`NewTabPage`]: the content of a "New Tab" tab ("+", Ctrl+T outside the
//!   terminal, Ctrl+Shift+T). At the top the search field with the Ctrl+K
//!   hint, focused when the page opens; below it Recent connections with
//!   Restore (enabled while a snapshot of the previous launch exists) and
//!   "Create a workspace" (disabled until stage 9, with a tooltip).
//! - [`SearchOverlay`]: Ctrl+K outside the terminal and Ctrl+Shift+K
//!   everywhere: the same search over the current window. Esc or a click
//!   outside closes it ([`NewTabEvent::Dismissed`]) and gives the focus
//!   back to where it was.
//! - [`CompactPicker`]: the host selection of a new pane (UI 4.7): the
//!   search and one list whose first row is the split pane's target
//!   ([`PickerOrigin`]), so Enter opens another session to it. Esc asks to
//!   close the pane ([`NewTabEvent::Dismissed`]).
//!
//! The lists ([`model`]): with an empty search, Recent connections (the
//! core's connection log: newest first, one row per target, at most 10);
//! while typing, open tabs of every window first, then hosts, and a
//! "Connect to ..." row when the text is an address (UI 2.1) that no host
//! matches exactly. Up and Down move the selection, Enter chooses it,
//! Alt+Enter (or Alt + click) opens a session in a new window (UI 2.4).
//!
//! Data in: hosts and groups from the app state; recent connections and
//! the vault's name from the core (read on a background thread, again
//! whenever records change); the open tabs ([`OpenTab`]) and the Restore
//! flag from the caller. Data out: [`NewTabEvent`]s; the window shell and
//! the panes act on them. Nothing here logs hosts or search text.

mod compact;
pub mod model;
mod overlay;
mod page;
mod picker;
pub mod strings;

#[cfg(test)]
mod tests;

use gpui::{AnyWindowHandle, App, Global, KeyBinding, SharedString};
use gpui_kit::assets::IconName;
use uuid::Uuid;

use crate::hosts::QuickTarget;
use crate::shell::SessionId;

pub use compact::CompactPicker;
pub use overlay::SearchOverlay;
pub use page::NewTabPage;

/// Key context of the search (field and list) of all three views.
pub const KEY_CONTEXT: &str = "NewTabSearch";

gpui_kit::actions!(
    new_tab,
    [
        /// Alt+Enter: the selected row, in a new window (UI 2.4).
        ConfirmInNewWindow,
        /// Esc: closes the overlay, or the new pane of the pane picker.
        Dismiss,
    ]
);

/// The keys of the search. Up and Down are taken from the search field
/// before it moves its caret (`capture_action` in [`picker`]); Enter is the
/// field's own `PressEnter`. Registered once, by the first view.
pub fn bind_keys(cx: &mut App) {
    struct Bound;
    impl Global for Bound {}
    if cx.has_global::<Bound>() {
        return;
    }
    cx.set_global(Bound);
    let context = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("alt-enter", ConfirmInNewWindow, context),
        KeyBinding::new("escape", Dismiss, context),
    ]);
}

/// What the views ask the shell or the pane to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NewTabEvent {
    /// Connect to a saved host: in this tab (the page), in this pane (the
    /// picker), in a new tab (the overlay), or in a new window.
    OpenHost { host_id: Uuid, new_window: bool },
    /// Connect to an address without saving it (UI 2.1).
    QuickConnect {
        target: QuickTarget,
        new_window: bool,
    },
    /// A new local terminal (the pane picker's first row for a local
    /// terminal); `None`: the default shell.
    OpenLocal {
        shell_id: Option<String>,
        new_window: bool,
    },
    /// Bring an open tab and its window to the front.
    ActivateTab {
        window: AnyWindowHandle,
        tab: SessionId,
    },
    /// Restore the windows, tabs and splits of the previous launch.
    Restore,
    /// Esc or a click outside (the overlay; the focus is back where it
    /// was), or Esc in a new pane's picker (the pane closes).
    Dismissed,
}

/// An open session tab offered by the search (any window).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenTab {
    window: AnyWindowHandle,
    tab: SessionId,
    title: SharedString,
    icon: IconName,
    host: Option<Uuid>,
}

impl OpenTab {
    /// The tab `tab` of `window`, shown as `title` (the tab's own title).
    pub fn new(window: AnyWindowHandle, tab: SessionId, title: impl Into<SharedString>) -> Self {
        Self {
            window,
            tab,
            title: title.into(),
            icon: IconName::Server,
            host: None,
        }
    }

    /// The tab's left icon (the OS icon; the server icon by default).
    pub fn icon(mut self, icon: IconName) -> Self {
        self.icon = icon;
        self
    }

    /// The saved host of the tab's active pane: the search finds the tab by
    /// the host's name, address, user and tags too.
    pub fn host(mut self, host: Option<Uuid>) -> Self {
        self.host = host;
        self
    }

    pub fn window(&self) -> AnyWindowHandle {
        self.window
    }

    pub fn tab(&self) -> SessionId {
        self.tab
    }

    pub fn title(&self) -> &SharedString {
        &self.title
    }

    pub fn icon_name(&self) -> IconName {
        self.icon
    }

    pub fn host_id(&self) -> Option<Uuid> {
        self.host
    }
}

/// The target of the pane that was split: the first row of the new pane's
/// picker (UI 4.7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PickerOrigin {
    /// A saved host: another session to it.
    Host(Uuid),
    /// A quick connect: the same address again.
    Quick(QuickTarget),
    /// A local terminal: a new local terminal with the same shell; `title`
    /// is the shell's display name.
    Local {
        shell_id: Option<String>,
        title: SharedString,
    },
}
