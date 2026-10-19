//! Right-hand panels (UI 1.3, 3): the host panel and the group panel.
//!
//! The panels fill the details panel container of the shell (380 px,
//! resizable 320-560 px) and draw everything inside it: the header (title,
//! "…" menu `panel-menu`, collapse button `panel-close`), the scrolling body
//! with the blocks on `--bg-raised`, and for hosts the pinned Connect button.
//! The panel background is `--bg-surface`.
//!
//! Saving rule (UI 1.3): a new object is a draft until its required field is
//! valid, then it is created; after that every valid change is saved
//! automatically after [`crate::theme::metrics::AUTOSAVE_DEBOUNCE`]; there is
//! no save button; an invalid value is outlined, explained under its field and
//! not saved. [`autosave::Autosave`] holds the debounce and
//! [`crate::state::records_edit`] the rules.
//!
//! Events: [`PanelEvent`] (close, toast) and [`PanelRequest`] (what the shell
//! does for the panel: connect, show another record, removed records).

pub mod autosave;
pub mod group_panel;
pub mod host_panel;
pub mod menus;
pub mod picker;
pub mod strings;
pub(crate) mod widgets;

#[cfg(test)]
mod tests;

use gpui::SharedString;
use uuid::Uuid;

pub use group_panel::GroupPanel;
pub use host_panel::HostPanel;

/// What a panel asks the shell to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelEvent {
    /// Close the panel (its pending edits were saved or the draft discarded).
    Close,
    /// Show a toast at the bottom of the main area (also the "In development"
    /// placeholder of targets that are not built yet).
    Toast(SharedString),
}

/// Requests a panel makes of the shell and the Hosts screen (subscribe to
/// them next to [`PanelEvent`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PanelRequest {
    /// Connect to this host (a session tab; a placeholder until terminals exist).
    Connect(Uuid),
    /// Show this host in the panel and select it (after Duplicate).
    OpenHost(Uuid),
    /// Show this group in the group panel.
    OpenGroup(Uuid),
    /// These records were removed after confirmation (the panel also sends
    /// `Close`); the shell may offer Undo through `AppState::restore_records`.
    Removed(Vec<Uuid>),
}
