//! The session tab menu (UI 4.4; lead decision L2), opened with a right
//! click on a tab: Duplicate, Duplicate in a New Window, Rename, Split
//! Horizontally, Split Vertically, Broadcast Input (only on a tab with
//! splits, disabled until stage 9; D30), Close. Start multiplayer is not
//! shown (owner's step 1.2). The composition is data ([`entries`]), so it
//! is tested without opening a menu; [`build`] turns it into a gpui-kit
//! popup menu whose items carry stable element ids (`tab-menu-*`).

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{AnyElement, App, Context, ElementId, SharedString, Window, div};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::ActiveTheme as _;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};

use super::strings;
use crate::theme::{self, metrics};

/// What a tab menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabCommand {
    /// A new session to the active pane's target in a new tab next to it.
    Duplicate,
    /// The same in a new window (UI 1.4).
    DuplicateInNewWindow,
    /// Edit the tab title in place.
    Rename,
    /// A new pane below the active one (Ctrl+Shift+E).
    SplitHorizontally,
    /// A new pane to the right of the active one (Ctrl+Shift+D).
    SplitVertically,
    /// Broadcast input on or off, on a tab with splits (UI 4.4).
    BroadcastInput,
    /// Closes the tab with all its panes.
    Close,
}

/// One entry of the tab menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TabMenuEntry {
    Item {
        command: TabCommand,
        /// Stable element id (`tab-menu-*`).
        id: &'static str,
        label: &'static str,
        /// Key hint on the right.
        hint: Option<&'static str>,
        enabled: bool,
    },
    Separator,
}

impl TabMenuEntry {
    fn item(
        command: TabCommand,
        id: &'static str,
        label: &'static str,
        hint: Option<&'static str>,
        enabled: bool,
    ) -> Self {
        TabMenuEntry::Item {
            command,
            id,
            label,
            hint,
            enabled,
        }
    }
}

/// What the menu needs to know about its tab.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TabFacts {
    /// The active pane has a session target (SSH, Telnet, a local terminal
    /// or a serial port): the splits work.
    pub has_target: bool,
    /// Duplicate works: a target other than a serial port, which opens
    /// once (D88).
    pub can_duplicate: bool,
    /// The active pane can be split below / to the right (the tab has
    /// panes, and the new halves keep their minimum size).
    pub can_split_down: bool,
    pub can_split_right: bool,
    /// The tab has more than one pane.
    pub has_splits: bool,
    /// Other session tabs are open (Broadcast input over tabs, D86).
    pub other_tabs: bool,
    /// Broadcast input is on: the item stops it.
    pub broadcasting: bool,
}

/// The tab menu of UI 4.4, top to bottom. Items that do not apply to the
/// tab (Duplicate and the splits on the new tab page, SFTP and serial
/// tabs) stay in place, disabled.
pub fn entries(facts: TabFacts) -> Vec<TabMenuEntry> {
    use TabCommand::*;
    let mut entries = vec![
        TabMenuEntry::item(
            Duplicate,
            "tab-menu-duplicate",
            strings::MENU_DUPLICATE,
            None,
            facts.can_duplicate,
        ),
        TabMenuEntry::item(
            DuplicateInNewWindow,
            "tab-menu-duplicate-window",
            strings::MENU_DUPLICATE_NEW_WINDOW,
            None,
            facts.can_duplicate,
        ),
        TabMenuEntry::Separator,
        TabMenuEntry::item(Rename, "tab-menu-rename", strings::MENU_RENAME, None, true),
        TabMenuEntry::Separator,
        TabMenuEntry::item(
            SplitHorizontally,
            "tab-menu-split-horizontal",
            strings::MENU_SPLIT_HORIZONTALLY,
            Some("Ctrl+Shift+E"),
            facts.can_split_down,
        ),
        TabMenuEntry::item(
            SplitVertically,
            "tab-menu-split-vertical",
            strings::MENU_SPLIT_VERTICALLY,
            Some("Ctrl+Shift+D"),
            facts.can_split_right,
        ),
    ];
    // Broadcast input: to the panes of a split tab, or of tabs ticked in a
    // list when several are open (D86).
    if facts.has_splits || facts.other_tabs || facts.broadcasting {
        entries.push(TabMenuEntry::item(
            BroadcastInput,
            "tab-menu-broadcast",
            if facts.broadcasting {
                strings::MENU_BROADCAST_STOP
            } else {
                strings::MENU_BROADCAST_INPUT
            },
            // The key turns it off, or on for this tab only: the same as
            // the item unless the item opens the tab list.
            (facts.broadcasting || !facts.other_tabs).then_some("Ctrl+Shift+B"),
            true,
        ));
    }
    entries.push(TabMenuEntry::Separator);
    entries.push(TabMenuEntry::item(
        Close,
        "tab-menu-close",
        strings::MENU_CLOSE,
        None,
        true,
    ));
    entries
}

/// Runs a chosen command.
pub type TabCommandHandler = Rc<dyn Fn(TabCommand, &mut Window, &mut App)>;

/// Turns the model into the kit's popup menu.
pub fn build(
    menu: PopupMenu,
    entries: Vec<TabMenuEntry>,
    on_command: TabCommandHandler,
    _window: &mut Window,
    _cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let mut menu = menu.min_w(gpui::px(240.));
    for entry in entries {
        menu = match entry {
            TabMenuEntry::Separator => menu.separator(),
            TabMenuEntry::Item {
                command,
                id,
                label,
                hint,
                enabled,
            } => {
                let on_command = on_command.clone();
                // Close ends every session of the tab: `--danger` (design-system 2.5).
                let danger = command == TabCommand::Close;
                menu.item(
                    PopupMenuItem::element(move |_, cx| {
                        item_label(id, label, hint, enabled, danger, cx)
                    })
                    .disabled(!enabled)
                    .on_click(move |_, window, cx| on_command(command, window, cx)),
                )
            }
        };
    }
    menu
}

/// The label of an item: the text and a key hint on the right in
/// `--text-secondary`; inactive items use `--text-disabled` at full opacity
/// (design-system 6), like the host card menu. A dangerous item uses
/// `--danger`, and `--danger-hover` under the pointer.
fn item_label(
    id: &'static str,
    label: &'static str,
    hint: Option<&'static str>,
    enabled: bool,
    danger: bool,
    cx: &App,
) -> AnyElement {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let danger_hover = theme.danger_hover;
    let (text, hint_color) = if !enabled {
        (tokens.text_disabled.hsla(), tokens.text_disabled.hsla())
    } else if danger {
        (theme.danger, theme.muted_foreground)
    } else {
        (theme.foreground, theme.muted_foreground)
    };
    div()
        .id(ElementId::Name(SharedString::new_static(id)))
        .test_support()
        .flex()
        .flex_1()
        .items_center()
        .justify_between()
        .gap(metrics::SPACE_3)
        .text_color(text)
        .when(danger && enabled, |this| {
            // The hover area covers the whole item (its padding included), so
            // the hovered item never shows plain `--danger` on
            // `--bg-overlay-hover` (design-system 6), as in the host card menu.
            this.mx(-metrics::SPACE_2)
                .px(metrics::SPACE_2)
                .my(-metrics::SPACE_1)
                .py(metrics::SPACE_1)
                .hover(move |hovered| hovered.text_color(danger_hover))
        })
        .child(label)
        .when_some(hint, |this, hint| {
            this.child(
                div()
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(hint_color)
                    .child(hint),
            )
        })
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn commands(entries: &[TabMenuEntry]) -> Vec<(TabCommand, bool)> {
        entries
            .iter()
            .filter_map(|entry| match entry {
                TabMenuEntry::Item {
                    command, enabled, ..
                } => Some((*command, *enabled)),
                TabMenuEntry::Separator => None,
            })
            .collect()
    }

    /// UI 4.4: the order; Broadcast Input only with splits and disabled;
    /// no Start multiplayer.
    #[test]
    fn the_menu_follows_the_spec_order() {
        use TabCommand::*;
        let one_pane = TabFacts {
            has_target: true,
            can_duplicate: true,
            can_split_down: true,
            can_split_right: true,
            has_splits: false,
            other_tabs: false,
            broadcasting: false,
        };
        assert_eq!(
            commands(&entries(one_pane)),
            vec![
                (Duplicate, true),
                (DuplicateInNewWindow, true),
                (Rename, true),
                (SplitHorizontally, true),
                (SplitVertically, true),
                (Close, true),
            ]
        );
        // Another tab open: Broadcast input over ticked tabs (D86).
        let with_others = TabFacts {
            other_tabs: true,
            ..one_pane
        };
        assert!(commands(&entries(with_others)).contains(&(BroadcastInput, true)));
        let split = TabFacts {
            has_splits: true,
            ..one_pane
        };
        assert_eq!(
            commands(&entries(split)),
            vec![
                (Duplicate, true),
                (DuplicateInNewWindow, true),
                (Rename, true),
                (SplitHorizontally, true),
                (SplitVertically, true),
                (BroadcastInput, true),
                (Close, true),
            ]
        );
        // The new tab page: only Rename and Close act.
        assert_eq!(
            commands(&entries(TabFacts::default())),
            vec![
                (Duplicate, false),
                (DuplicateInNewWindow, false),
                (Rename, true),
                (SplitHorizontally, false),
                (SplitVertically, false),
                (Close, true),
            ]
        );
        // A serial port: no Duplicate, the splits work (D88).
        let serial = TabFacts {
            can_duplicate: false,
            ..one_pane
        };
        assert_eq!(
            commands(&entries(serial))[..5],
            [
                (Duplicate, false),
                (DuplicateInNewWindow, false),
                (Rename, true),
                (SplitHorizontally, true),
                (SplitVertically, true),
            ]
        );
    }
}
