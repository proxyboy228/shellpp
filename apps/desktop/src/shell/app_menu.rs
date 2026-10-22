//! The app menu (UI 1.4): the button with three bars, first on the left of
//! the top bar. The composition is data ([`entries`]), so its order is
//! tested without opening a menu; [`build`] turns it into a gpui-kit popup
//! menu. Every item dispatches a shell action; shortcut hints are taken from
//! the key bindings of those actions (`Ctrl+1`, `Ctrl+2`, `Ctrl+,`, ...).
//!
//! Items whose target is not built yet stay visible in spec order and open
//! the "In development" placeholder (audit decision 1). The Edit items act on
//! the focused text field.

use gpui::{Action, Context, FocusHandle, Window};
use gpui_kit::component::input;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};

use super::actions::*;
use crate::i18n::menu as text;

/// One entry of the app menu.
#[derive(Debug)]
pub enum MenuEntry {
    Item {
        /// Stable id (tests, logs).
        id: &'static str,
        label: &'static str,
        target: Target,
    },
    Separator,
    Submenu {
        id: &'static str,
        label: &'static str,
        items: Vec<MenuEntry>,
    },
}

/// Where an item sends its action.
#[derive(Clone, Copy)]
pub enum Target {
    /// A shell action (dispatched to the shell; its binding is the hint).
    Shell(fn() -> Box<dyn Action>),
    /// A text editing action for the field that had focus before the menu
    /// opened.
    FocusedField(fn() -> Box<dyn Action>),
}

impl std::fmt::Debug for Target {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (kind, action) = match self {
            Target::Shell(action) => ("Shell", action()),
            Target::FocusedField(action) => ("FocusedField", action()),
        };
        write!(f, "{kind}({})", action.name())
    }
}

impl MenuEntry {
    fn item(id: &'static str, label: &'static str, action: fn() -> Box<dyn Action>) -> Self {
        Self::Item {
            id,
            label,
            target: Target::Shell(action),
        }
    }

    fn edit(id: &'static str, label: &'static str, action: fn() -> Box<dyn Action>) -> Self {
        Self::Item {
            id,
            label,
            target: Target::FocusedField(action),
        }
    }

    /// The id of an item or submenu; `None` for a separator.
    pub fn id(&self) -> Option<&'static str> {
        match self {
            MenuEntry::Item { id, .. } | MenuEntry::Submenu { id, .. } => Some(id),
            MenuEntry::Separator => None,
        }
    }
}

/// The app menu of UI 1.4, top to bottom.
pub fn entries() -> Vec<MenuEntry> {
    use MenuEntry::{Separator, Submenu};
    vec![
        MenuEntry::item("menu-vaults", text::VAULTS, || Box::new(ShowVaults)),
        MenuEntry::item("menu-sftp", text::SFTP, || Box::new(ShowSftp)),
        MenuEntry::item("menu-settings", text::SETTINGS, || Box::new(OpenSettings)),
        Separator,
        MenuEntry::item("menu-new-local-terminal", text::NEW_LOCAL_TERMINAL, || {
            Box::new(NewLocalTerminal)
        }),
        MenuEntry::item("menu-new-serial", text::NEW_SERIAL_CONNECTION, || {
            Box::new(NewSerialConnection)
        }),
        Separator,
        MenuEntry::item("menu-import-data", text::IMPORT_DATA, || {
            Box::new(ImportData)
        }),
        MenuEntry::item("menu-export-data", text::EXPORT_DATA, || {
            Box::new(ExportData)
        }),
        Separator,
        Submenu {
            id: "menu-edit",
            label: text::EDIT,
            items: vec![
                MenuEntry::edit("menu-undo", text::UNDO, || Box::new(input::Undo)),
                MenuEntry::edit("menu-redo", text::REDO, || Box::new(input::Redo)),
                Separator,
                MenuEntry::edit("menu-cut", text::CUT, || Box::new(input::Cut)),
                MenuEntry::edit("menu-copy", text::COPY, || Box::new(input::Copy)),
                MenuEntry::edit("menu-paste", text::PASTE, || Box::new(input::Paste)),
                MenuEntry::edit("menu-select-all", text::SELECT_ALL, || {
                    Box::new(input::SelectAll)
                }),
            ],
        },
        Submenu {
            id: "menu-view",
            label: text::VIEW,
            items: vec![
                MenuEntry::item("menu-zoom-in", text::ZOOM_IN, || Box::new(ZoomIn)),
                MenuEntry::item("menu-zoom-out", text::ZOOM_OUT, || Box::new(ZoomOut)),
                MenuEntry::item("menu-actual-size", text::ACTUAL_SIZE, || {
                    Box::new(ActualSize)
                }),
                Separator,
                MenuEntry::item("menu-full-screen", text::FULL_SCREEN, || {
                    Box::new(ToggleFullScreen)
                }),
                MenuEntry::item("menu-toggle-sidebar", text::TOGGLE_SIDEBAR, || {
                    Box::new(ToggleSidebar)
                }),
            ],
        },
        Submenu {
            id: "menu-window",
            label: text::WINDOW,
            items: vec![
                MenuEntry::item("menu-new-window", text::NEW_WINDOW, || Box::new(NewWindow)),
                Separator,
                MenuEntry::item("menu-next-tab", text::NEXT_TAB, || Box::new(NextTab)),
                MenuEntry::item("menu-previous-tab", text::PREVIOUS_TAB, || {
                    Box::new(PreviousTab)
                }),
                MenuEntry::item("menu-close-tab", text::CLOSE_TAB, || Box::new(CloseTab)),
            ],
        },
        Submenu {
            id: "menu-help",
            label: text::HELP,
            items: vec![
                MenuEntry::item("menu-documentation", text::DOCUMENTATION, || {
                    Box::new(OpenDocumentation)
                }),
                MenuEntry::item("menu-report-issue", text::REPORT_ISSUE, || {
                    Box::new(ReportIssue)
                }),
                MenuEntry::item("menu-check-updates", text::CHECK_FOR_UPDATES, || {
                    Box::new(CheckForUpdates)
                }),
                Separator,
                MenuEntry::item("menu-about", text::ABOUT, || Box::new(About)),
            ],
        },
        MenuEntry::item("menu-lock", text::LOCK, || Box::new(LockApp)),
        Separator,
        MenuEntry::item("menu-exit", text::EXIT, || Box::new(Exit)),
    ]
}

/// Builds the popup menu. `shell_focus` is the shell's focus handle: shell
/// actions are dispatched there and their shortcut hints resolved there.
pub fn build(
    menu: PopupMenu,
    shell_focus: FocusHandle,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    add_entries(
        menu.action_context(shell_focus)
            .min_w(crate::theme::metrics::scaled(240., cx)),
        entries(),
        window,
        cx,
    )
}

fn add_entries(
    menu: PopupMenu,
    entries: Vec<MenuEntry>,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    entries.into_iter().fold(menu, |menu, entry| match entry {
        MenuEntry::Separator => menu.separator(),
        MenuEntry::Item { label, target, .. } => menu.item(match target {
            Target::Shell(action) => PopupMenuItem::new(label).action(action()),
            Target::FocusedField(action) => {
                PopupMenuItem::new(label).on_click(move |_, window, cx| {
                    // After the menu closes, focus is back in the field
                    // that had it; the action goes there.
                    window.defer(cx, move |window, cx| {
                        window.dispatch_action(action(), cx);
                    });
                })
            }
        }),
        MenuEntry::Submenu { label, items, .. } => {
            let items = std::cell::RefCell::new(Some(items));
            menu.submenu(label, window, cx, move |menu, window, cx| {
                match items.borrow_mut().take() {
                    Some(items) => add_entries(menu, items, window, cx),
                    None => menu,
                }
            })
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(entries: &[MenuEntry]) -> Vec<&'static str> {
        entries
            .iter()
            .map(|entry| entry.id().unwrap_or("—"))
            .collect()
    }

    fn submenu<'a>(entries: &'a [MenuEntry], id: &str) -> &'a [MenuEntry] {
        entries
            .iter()
            .find_map(|entry| match entry {
                MenuEntry::Submenu { id: sub, items, .. } if *sub == id => Some(items.as_slice()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("{id} missing"))
    }

    /// UI 1.4, items 1-13 in order, with Import Data and Export Data after
    /// the new connections (D38).
    #[test]
    fn app_menu_has_the_spec_items_in_order() {
        let entries = entries();
        assert_eq!(
            ids(&entries),
            [
                "menu-vaults",
                "menu-sftp",
                "menu-settings",
                "—",
                "menu-new-local-terminal",
                "menu-new-serial",
                "—",
                "menu-import-data",
                "menu-export-data",
                "—",
                "menu-edit",
                "menu-view",
                "menu-window",
                "menu-help",
                "menu-lock",
                "—",
                "menu-exit",
            ]
        );
        let labels: Vec<&str> = entries
            .iter()
            .filter_map(|entry| match entry {
                MenuEntry::Item { label, .. } | MenuEntry::Submenu { label, .. } => Some(*label),
                MenuEntry::Separator => None,
            })
            .collect();
        assert_eq!(
            labels,
            [
                "Vaults",
                "SFTP",
                "Settings",
                "New Local Terminal",
                "New Serial Connection",
                "Import Data…",
                "Export Data…",
                "Edit",
                "View",
                "Window",
                "Help",
                "Lock",
                "Exit"
            ]
        );
    }

    #[test]
    fn submenus_have_the_spec_items() {
        let entries = entries();
        assert_eq!(
            ids(submenu(&entries, "menu-edit")),
            [
                "menu-undo",
                "menu-redo",
                "—",
                "menu-cut",
                "menu-copy",
                "menu-paste",
                "menu-select-all"
            ]
        );
        assert_eq!(
            ids(submenu(&entries, "menu-view")),
            [
                "menu-zoom-in",
                "menu-zoom-out",
                "menu-actual-size",
                "—",
                "menu-full-screen",
                "menu-toggle-sidebar"
            ]
        );
        assert_eq!(
            ids(submenu(&entries, "menu-window")),
            [
                "menu-new-window",
                "—",
                "menu-next-tab",
                "menu-previous-tab",
                "menu-close-tab"
            ]
        );
        assert_eq!(
            ids(submenu(&entries, "menu-help")),
            [
                "menu-documentation",
                "menu-report-issue",
                "menu-check-updates",
                "—",
                "menu-about"
            ]
        );
    }

    #[test]
    fn items_dispatch_the_shell_actions() {
        let entries = entries();
        let action_of = |id: &str| {
            entries
                .iter()
                .find_map(|entry| match entry {
                    MenuEntry::Item {
                        id: item,
                        target: Target::Shell(action),
                        ..
                    } if *item == id => Some(action()),
                    _ => None,
                })
                .unwrap()
        };
        assert!(action_of("menu-vaults").partial_eq(&ShowVaults));
        assert!(action_of("menu-sftp").partial_eq(&ShowSftp));
        assert!(action_of("menu-settings").partial_eq(&OpenSettings));
        assert!(action_of("menu-import-data").partial_eq(&ImportData));
        assert!(action_of("menu-export-data").partial_eq(&ExportData));
        assert!(action_of("menu-lock").partial_eq(&LockApp));
        assert!(action_of("menu-exit").partial_eq(&Exit));
    }
}
