//! macOS native menu bar (UI 1.4: "On macOS, the system menu bar with the
//! same items is used instead of this menu"; plan decision 20).
//!
//! Item ids are the app menu test ids of the frontend (`app-menu-...`). A
//! click on one of them sends the window event `menu:action {id}` to the
//! focused (or last focused) window, which runs the same action as its in-app
//! menu. The Edit items are the standard system items. Verified by the manual
//! macOS checklist (no tauri-driver on macOS).

use std::sync::Arc;

use serde::Serialize;
use tauri::menu::{MenuBuilder, MenuItem, MenuItemBuilder, SubmenuBuilder};
use tauri::{AppHandle, Runtime};

use crate::windows::WindowRegistry;

/// Window event name of a native menu click.
pub const MENU_ACTION_EVENT: &str = "menu:action";

#[derive(Clone, Debug, Serialize)]
struct MenuAction {
    id: String,
}

fn item<R: Runtime>(
    app: &AppHandle<R>,
    id: &str,
    text: &str,
    accelerator: Option<&str>,
) -> tauri::Result<MenuItem<R>> {
    let mut builder = MenuItemBuilder::with_id(id, text);
    if let Some(accelerator) = accelerator {
        builder = builder.accelerator(accelerator);
    }
    builder.build(app)
}

/// Installs the menu bar and its click handler.
pub fn install<R: Runtime>(app: &AppHandle<R>, windows: Arc<WindowRegistry>) -> tauri::Result<()> {
    let vaults = item(app, "app-menu-vaults", "Vaults", Some("CmdOrCtrl+1"))?;
    let settings = item(app, "app-menu-settings", "Settings…", Some("CmdOrCtrl+,"))?;
    let local_terminal = item(
        app,
        "app-menu-new-local-terminal",
        "New Local Terminal",
        Some("CmdOrCtrl+Shift+L"),
    )?;
    let lock = item(app, "app-menu-lock", "Lock", None)?;
    let exit = item(app, "app-menu-exit", "Quit Shell++", Some("CmdOrCtrl+Q"))?;
    let app_menu = SubmenuBuilder::new(app, "Shell++")
        .item(&vaults)
        .item(&settings)
        .separator()
        .item(&local_terminal)
        .separator()
        .item(&lock)
        .separator()
        .hide()
        .hide_others()
        .show_all()
        .separator()
        .item(&exit)
        .build()?;

    let edit_menu = SubmenuBuilder::new(app, "Edit")
        .undo()
        .redo()
        .separator()
        .cut()
        .copy()
        .paste()
        .select_all()
        .build()?;

    let zoom_in = item(app, "app-menu-view-zoom-in", "Zoom In", Some("CmdOrCtrl+="))?;
    let zoom_out = item(
        app,
        "app-menu-view-zoom-out",
        "Zoom Out",
        Some("CmdOrCtrl+-"),
    )?;
    let zoom_reset = item(
        app,
        "app-menu-view-zoom-reset",
        "Actual Size",
        Some("CmdOrCtrl+0"),
    )?;
    let fullscreen = item(
        app,
        "app-menu-view-fullscreen",
        "Toggle Full Screen",
        Some("Ctrl+Cmd+F"),
    )?;
    let sidebar = item(
        app,
        "app-menu-view-toggle-sidebar",
        "Show or Hide Sidebar",
        None,
    )?;
    let view_menu = SubmenuBuilder::new(app, "View")
        .item(&zoom_in)
        .item(&zoom_out)
        .item(&zoom_reset)
        .separator()
        .item(&fullscreen)
        .item(&sidebar)
        .build()?;

    let new_window = item(app, "app-menu-window-new-window", "New Window", None)?;
    // Tab switching has no shortcut on macOS (Alt+arrows are Windows and
    // Linux only, UI 12.5); Close Tab keeps Cmd+Shift+W.
    let next_tab = item(app, "app-menu-window-next-tab", "Next Tab", None)?;
    let previous_tab = item(app, "app-menu-window-previous-tab", "Previous Tab", None)?;
    let close_tab = item(
        app,
        "app-menu-window-close-tab",
        "Close Tab",
        Some("CmdOrCtrl+Shift+W"),
    )?;
    let window_menu = SubmenuBuilder::new(app, "Window")
        .item(&new_window)
        .separator()
        .item(&next_tab)
        .item(&previous_tab)
        .item(&close_tab)
        .separator()
        .minimize()
        .maximize()
        .build()?;

    let documentation = item(app, "app-menu-help-documentation", "Documentation", None)?;
    let report = item(app, "app-menu-help-report-issue", "Report an Issue", None)?;
    let about = item(app, "app-menu-help-about", "About Shell++", None)?;
    let help_menu = SubmenuBuilder::new(app, "Help")
        .item(&documentation)
        .item(&report)
        .separator()
        .item(&about)
        .build()?;

    let menu = MenuBuilder::new(app)
        .items(&[&app_menu, &edit_menu, &view_menu, &window_menu, &help_menu])
        .build()?;
    app.set_menu(menu)?;

    let handle = app.clone();
    app.on_menu_event(move |_app, event| {
        let id: &str = event.id().as_ref();
        if !id.starts_with("app-menu-") {
            return;
        }
        let action = MenuAction { id: id.to_owned() };
        if !windows.emit_to_target(&handle, MENU_ACTION_EVENT, action) {
            tracing::warn!("menu action dropped: no window");
        }
    });
    Ok(())
}
