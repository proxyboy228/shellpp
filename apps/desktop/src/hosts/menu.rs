//! Context menus of the Hosts screen: the host card menu of UI 2.4 (the
//! order matters) and the group card menu of UI 3.4. The menus are built
//! from a plain model ([`host_menu`], [`group_menu`]) so the composition and
//! the order are unit-tested; [`build_popup`] turns a model into a
//! gpui-kit `PopupMenu` whose items carry stable element ids (`menu-*`).
//!
//! Items that act on one record only (Edit Host Details, Collaborate,
//! Duplicate, Copy Link, Open, Edit) stay in place but are inactive while
//! several cards are selected; the others act on every selected card
//! (UI 2.3). Targets that are not built yet open an "In development"
//! placeholder (lead decision 1). Move to ▸ and Copy to ▸ list the top
//! level and every group of this vault (lead decision 7); Move to ▸ ends
//! with "Another vault…" (dialog 2.6, a placeholder), while other vaults
//! join Copy to ▸ only with team vaults (stage 8, UI 2.4).

use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, ElementId, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _};
use uuid::Uuid;

use super::strings;
use crate::theme::{self, metrics};

/// What a menu item does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuCommand {
    QuickConnect,
    ConnectSsh,
    ConnectSftp,
    /// Connect ▸ via Telnet: the hosts that have a Telnet block (D87).
    ConnectTelnet,
    EditHost,
    Collaborate,
    /// Move into a group (`None`: the top level).
    MoveTo(Option<Uuid>),
    /// Move to another vault (dialog 2.6, placeholder).
    MoveToVault,
    /// Copy into a group of this vault (`None`: the top level): a copy with
    /// the same label.
    CopyTo(Option<Uuid>),
    Duplicate,
    CopyLink,
    Remove,
    OpenGroup,
    EditGroup,
    /// Add to Workspace ▸ (UI 4.5): a tab per host in that workspace;
    /// `None`: a new workspace (its name is asked).
    AddToWorkspace(Option<Uuid>),
}

/// One entry of a menu model.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MenuEntry {
    Command {
        id: SharedString,
        label: SharedString,
        /// Key hint on the right (`Enter`, `E`).
        hint: Option<&'static str>,
        /// Explanation behind an (i) icon (Copy Link).
        info: Option<&'static str>,
        /// Indentation level (groups in Move to).
        depth: usize,
        danger: bool,
        enabled: bool,
        command: MenuCommand,
    },
    Submenu {
        id: SharedString,
        label: SharedString,
        entries: Vec<MenuEntry>,
    },
    Separator,
    /// A non-interactive line (the Alt hint, "In development").
    Note {
        id: SharedString,
        label: SharedString,
    },
}

impl MenuEntry {
    fn command(id: &str, label: &str, command: MenuCommand) -> Self {
        Self::Command {
            id: SharedString::from(id.to_owned()),
            label: SharedString::from(label.to_owned()),
            hint: None,
            info: None,
            depth: 0,
            danger: false,
            enabled: true,
            command,
        }
    }

    fn hint(mut self, key: &'static str) -> Self {
        if let Self::Command { hint, .. } = &mut self {
            *hint = Some(key);
        }
        self
    }

    fn info(mut self, text: &'static str) -> Self {
        if let Self::Command { info, .. } = &mut self {
            *info = Some(text);
        }
        self
    }

    fn danger(mut self) -> Self {
        if let Self::Command { danger, .. } = &mut self {
            *danger = true;
        }
        self
    }

    fn enabled(mut self, value: bool) -> Self {
        if let Self::Command { enabled, .. } = &mut self {
            *enabled = value;
        }
        self
    }

    fn submenu(id: &str, label: &str, entries: Vec<MenuEntry>) -> Self {
        Self::Submenu {
            id: SharedString::from(id.to_owned()),
            label: SharedString::from(label.to_owned()),
            entries,
        }
    }

    fn note(id: &str, label: &str) -> Self {
        Self::Note {
            id: SharedString::from(id.to_owned()),
            label: SharedString::from(label.to_owned()),
        }
    }

    /// The element id (`None` for a separator).
    pub fn id(&self) -> Option<&SharedString> {
        match self {
            Self::Command { id, .. } | Self::Submenu { id, .. } | Self::Note { id, .. } => Some(id),
            Self::Separator => None,
        }
    }

    /// The visible label (`None` for a separator).
    pub fn label(&self) -> Option<&SharedString> {
        match self {
            Self::Command { label, .. }
            | Self::Submenu { label, .. }
            | Self::Note { label, .. } => Some(label),
            Self::Separator => None,
        }
    }
}

/// A group the selection can move into.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MoveTarget {
    /// `None`: the top level.
    pub group: Option<Uuid>,
    pub name: String,
    pub depth: usize,
    /// False for the place the records already are, or a group inside a
    /// moved group.
    pub enabled: bool,
}

/// Element id of a Move to target.
pub fn move_target_id(group: Option<Uuid>) -> SharedString {
    match group {
        Some(id) => format!("menu-move-{id}").into(),
        None => "menu-move-top".into(),
    }
}

/// Element id of a Copy to target.
pub fn copy_target_id(group: Option<Uuid>) -> SharedString {
    match group {
        Some(id) => format!("menu-copy-{id}").into(),
        None => "menu-copy-top".into(),
    }
}

/// Copy to ▸: the top level and every group (a copy may go next to its
/// original, so every target is active). Other vaults join with team
/// vaults (stage 8, UI 2.4).
fn copy_to_entries(targets: &[MoveTarget]) -> Vec<MenuEntry> {
    targets
        .iter()
        .map(|target| {
            let mut entry = MenuEntry::command(
                &copy_target_id(target.group),
                &target.name,
                MenuCommand::CopyTo(target.group),
            );
            if let MenuEntry::Command { depth, .. } = &mut entry {
                *depth = target.depth;
            }
            entry
        })
        .collect()
}

/// The workspaces by name, then New Workspace (UI 4.5).
fn workspace_entries(workspaces: &[(Uuid, String)]) -> Vec<MenuEntry> {
    let mut entries: Vec<MenuEntry> = workspaces
        .iter()
        .map(|(id, name)| {
            MenuEntry::command(
                &format!("menu-workspace-{id}"),
                name,
                MenuCommand::AddToWorkspace(Some(*id)),
            )
        })
        .collect();
    if !entries.is_empty() {
        entries.push(MenuEntry::Separator);
    }
    entries.push(MenuEntry::command(
        "menu-workspace-new",
        strings::MENU_NEW_WORKSPACE,
        MenuCommand::AddToWorkspace(None),
    ));
    entries
}

fn move_to_entries(targets: &[MoveTarget]) -> Vec<MenuEntry> {
    let mut entries: Vec<MenuEntry> = targets
        .iter()
        .map(|target| {
            let mut entry = MenuEntry::command(
                &move_target_id(target.group),
                &target.name,
                MenuCommand::MoveTo(target.group),
            )
            .enabled(target.enabled);
            if let MenuEntry::Command { depth, .. } = &mut entry {
                *depth = target.depth;
            }
            entry
        })
        .collect();
    entries.push(MenuEntry::Separator);
    entries.push(MenuEntry::command(
        "menu-move-vault",
        strings::MENU_VAULT,
        MenuCommand::MoveToVault,
    ));
    entries
}

/// The host card menu of UI 2.4, in this order: Quick Connect, Connect ▸,
/// Add to Workspace ▸, Edit Host Details, Collaborate, Move to ▸, Copy to ▸,
/// Duplicate, Copy Link, Remove. `count` is the number of selected cards the
/// menu acts on.
pub fn host_menu(
    count: usize,
    move_targets: &[MoveTarget],
    workspaces: &[(Uuid, String)],
    telnet: bool,
) -> Vec<MenuEntry> {
    let single = count <= 1;
    vec![
        MenuEntry::command(
            "menu-quick-connect",
            strings::MENU_QUICK_CONNECT,
            MenuCommand::QuickConnect,
        )
        .hint(strings::KEY_ENTER),
        MenuEntry::submenu(
            "menu-connect",
            strings::MENU_CONNECT,
            vec![
                MenuEntry::command(
                    "menu-connect-ssh",
                    strings::MENU_VIA_SSH,
                    MenuCommand::ConnectSsh,
                ),
                // SFTP session tabs come in a later stage: shown disabled (D30, D31).
                MenuEntry::command(
                    "menu-connect-sftp",
                    strings::MENU_VIA_SFTP,
                    MenuCommand::ConnectSftp,
                )
                .enabled(false),
                // The hosts with a Telnet block (UI 3.1 "Add Telnet", D87).
                MenuEntry::command(
                    "menu-connect-telnet",
                    strings::MENU_VIA_TELNET,
                    MenuCommand::ConnectTelnet,
                )
                .enabled(telnet),
                MenuEntry::Separator,
                MenuEntry::note("menu-connect-alt-hint", strings::MENU_NEW_WINDOW_HINT),
            ],
        ),
        MenuEntry::submenu(
            "menu-add-to-workspace",
            strings::MENU_ADD_TO_WORKSPACE,
            workspace_entries(workspaces),
        ),
        MenuEntry::command(
            "menu-edit-host",
            strings::MENU_EDIT_HOST,
            MenuCommand::EditHost,
        )
        .hint(strings::KEY_E)
        .enabled(single),
        MenuEntry::command(
            "menu-collaborate",
            strings::MENU_COLLABORATE,
            MenuCommand::Collaborate,
        )
        .enabled(single),
        MenuEntry::submenu(
            "menu-move-to",
            strings::MENU_MOVE_TO,
            move_to_entries(move_targets),
        ),
        MenuEntry::submenu(
            "menu-copy-to",
            strings::MENU_COPY_TO,
            copy_to_entries(move_targets),
        ),
        MenuEntry::command(
            "menu-duplicate",
            strings::MENU_DUPLICATE,
            MenuCommand::Duplicate,
        )
        .enabled(single),
        MenuEntry::command(
            "menu-copy-link",
            strings::MENU_COPY_LINK,
            MenuCommand::CopyLink,
        )
        .info(strings::MENU_COPY_LINK_INFO)
        .enabled(single),
        MenuEntry::command("menu-remove", strings::MENU_REMOVE, MenuCommand::Remove).danger(),
    ]
}

/// The group card menu of UI 3.4: Open, Edit, Move to ▸, Remove.
pub fn group_menu(count: usize, move_targets: &[MoveTarget]) -> Vec<MenuEntry> {
    let single = count <= 1;
    vec![
        MenuEntry::command(
            "menu-open-group",
            strings::MENU_OPEN,
            MenuCommand::OpenGroup,
        )
        .enabled(single),
        MenuEntry::command(
            "menu-edit-group",
            strings::MENU_EDIT,
            MenuCommand::EditGroup,
        )
        .enabled(single),
        MenuEntry::submenu(
            "menu-move-to",
            strings::MENU_MOVE_TO,
            move_to_entries(move_targets),
        ),
        MenuEntry::command("menu-remove", strings::MENU_REMOVE, MenuCommand::Remove).danger(),
    ]
}

/// The ids of the top-level entries, separators skipped (tests, debugging).
pub fn entry_ids(entries: &[MenuEntry]) -> Vec<&str> {
    entries
        .iter()
        .filter_map(|entry| entry.id().map(SharedString::as_ref))
        .collect()
}

/// Runs a menu command.
pub type CommandHandler = Rc<dyn Fn(MenuCommand, &mut Window, &mut App)>;

/// Fills a `PopupMenu` from a model.
pub fn build_popup(
    menu: PopupMenu,
    entries: Vec<MenuEntry>,
    on_command: CommandHandler,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let mut menu = menu.min_w(metrics::scaled(240., cx));
    for entry in entries {
        menu = match entry {
            MenuEntry::Separator => menu.separator(),
            MenuEntry::Note { id, label } => menu.item(
                PopupMenuItem::element(move |_, cx| note_label(id.clone(), label.clone(), cx))
                    .disabled(true),
            ),
            MenuEntry::Submenu { label, entries, .. } => {
                let on_command = on_command.clone();
                menu.submenu(label, window, cx, move |menu, window, cx| {
                    build_popup(menu, entries.clone(), on_command.clone(), window, cx)
                })
            }
            MenuEntry::Command {
                id,
                label,
                hint,
                info,
                depth,
                danger,
                enabled,
                command,
            } => {
                let on_command = on_command.clone();
                menu.item(
                    PopupMenuItem::element(move |_, cx| {
                        item_label(
                            id.clone(),
                            label.clone(),
                            ItemStyle {
                                hint,
                                info,
                                depth,
                                danger,
                                enabled,
                            },
                            cx,
                        )
                    })
                    .disabled(!enabled)
                    .on_click(move |_, window, cx| on_command(command, window, cx)),
                )
            }
        };
    }
    menu
}

/// Height of the content row of a gpui-kit menu item (medium size).
const KIT_ITEM_HEIGHT: gpui::Rems = metrics::rpx(26.);

struct ItemStyle {
    hint: Option<&'static str>,
    info: Option<&'static str>,
    depth: usize,
    danger: bool,
    enabled: bool,
}

/// The label of a menu item: the text, an optional (i) icon with its
/// explanation, and a key hint on the right in `--text-secondary`.
/// Dangerous items use `--danger` (`--danger-hover` under the pointer);
/// inactive items use `--text-disabled` at full opacity (design-system 6).
/// The text keeps the kit's menu item size, which the kit also gives the
/// submenu labels (Connect ▸, Move to ▸): one size in the whole menu.
fn item_label(id: SharedString, label: SharedString, style: ItemStyle, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let danger_hover = theme.danger_hover;
    let text = if !style.enabled {
        tokens.text_disabled.hsla()
    } else if style.danger {
        theme.danger
    } else {
        theme.foreground
    };
    let hint_color = if style.enabled {
        theme.muted_foreground
    } else {
        tokens.text_disabled.hsla()
    };
    div()
        .id(ElementId::Name(id))
        .test_support()
        .flex()
        .flex_1()
        .items_center()
        .justify_between()
        .gap(metrics::SPACE_3)
        .pl(metrics::SPACE_3 * style.depth as f32)
        .text_color(text)
        .when(style.danger && style.enabled, |this| {
            // The hover area covers the whole item (its 8 px / 4 px padding
            // included), so the hovered item never shows plain `--danger` on
            // `--bg-overlay-hover` (design-system 6).
            this.mx(-metrics::SPACE_2)
                .px(metrics::SPACE_2)
                .my(-metrics::SPACE_1)
                .py(metrics::SPACE_1)
                .min_h(KIT_ITEM_HEIGHT)
                .hover(move |hovered| hovered.text_color(danger_hover))
        })
        .child(
            div()
                .flex()
                .items_center()
                .gap(metrics::SPACE_1)
                .child(label)
                .when_some(style.info, |this, info| {
                    this.child(
                        div()
                            .id("menu-copy-link-info")
                            .child(
                                Icon::new(IconName::Info)
                                    .xsmall()
                                    .text_color(theme.muted_foreground),
                            )
                            .tooltip(move |window, cx| {
                                gpui_kit::component::tooltip::Tooltip::new(info).build(window, cx)
                            }),
                    )
                }),
        )
        .when_some(style.hint, |this, hint| {
            this.child(
                div()
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(hint_color)
                    .child(hint),
            )
        })
        .into_any_element()
}

fn note_label(id: SharedString, label: SharedString, cx: &App) -> AnyElement {
    div()
        .id(ElementId::Name(id))
        .test_support()
        .text_size(metrics::FONT_SIZE_CAPTION)
        .text_color(cx.theme().muted_foreground)
        .child(label)
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn targets() -> Vec<MoveTarget> {
        vec![
            MoveTarget {
                group: None,
                name: strings::MENU_TOP_LEVEL.to_owned(),
                depth: 0,
                enabled: false,
            },
            MoveTarget {
                group: Some(Uuid::nil()),
                name: "Production".to_owned(),
                depth: 0,
                enabled: true,
            },
        ]
    }

    fn submenu<'a>(entries: &'a [MenuEntry], id: &str) -> &'a [MenuEntry] {
        entries
            .iter()
            .find_map(|entry| match entry {
                MenuEntry::Submenu {
                    id: found, entries, ..
                } if found == id => Some(entries.as_slice()),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn host_menu_has_the_order_of_ui_2_4() {
        let entries = host_menu(1, &targets(), &[], false);
        assert_eq!(
            entry_ids(&entries),
            [
                "menu-quick-connect",
                "menu-connect",
                "menu-add-to-workspace",
                "menu-edit-host",
                "menu-collaborate",
                "menu-move-to",
                "menu-copy-to",
                "menu-duplicate",
                "menu-copy-link",
                "menu-remove",
            ]
        );
        let labels: Vec<&str> = entries
            .iter()
            .filter_map(|entry| entry.label().map(SharedString::as_ref))
            .collect();
        assert_eq!(
            labels,
            [
                "Quick Connect",
                "Connect",
                "Add to Workspace",
                "Edit Host Details",
                "Collaborate",
                "Move to",
                "Copy to",
                "Duplicate",
                "Copy Link",
                "Remove",
            ]
        );
        // Connect ▸: via SSH, via SFTP, via Telnet (D87), a separator, the
        // Alt hint.
        let connect = submenu(&entries, "menu-connect");
        assert_eq!(connect.len(), 5);
        assert_eq!(
            entry_ids(connect),
            [
                "menu-connect-ssh",
                "menu-connect-sftp",
                "menu-connect-telnet",
                "menu-connect-alt-hint"
            ]
        );
        assert_eq!(connect[3], MenuEntry::Separator);
        // Telnet only for hosts with a Telnet block.
        assert!(matches!(
            &connect[2],
            MenuEntry::Command { enabled: false, .. }
        ));
        let telnet = host_menu(1, &targets(), &[], true);
        assert!(matches!(
            &submenu(&telnet, "menu-connect")[2],
            MenuEntry::Command { enabled: true, .. }
        ));
        // SFTP comes in a later stage (D30, D31): shown, disabled.
        assert!(matches!(
            &connect[1],
            MenuEntry::Command { enabled: false, .. }
        ));
        // Move to ▸: the groups, a separator, another vault.
        assert_eq!(
            entry_ids(submenu(&entries, "menu-move-to")),
            [
                "menu-move-top",
                "menu-move-00000000-0000-0000-0000-000000000000",
                "menu-move-vault"
            ]
        );
        // Copy to ▸ (lead decision 7): the top level and the groups, all
        // active (the place the host is in included); no other vault before
        // team vaults (stage 8, UI 2.4).
        let copy = submenu(&entries, "menu-copy-to");
        assert_eq!(
            entry_ids(copy),
            [
                "menu-copy-top",
                "menu-copy-00000000-0000-0000-0000-000000000000"
            ]
        );
        assert!(!copy.contains(&MenuEntry::Separator));
        assert!(matches!(
            &copy[0],
            MenuEntry::Command {
                enabled: true,
                command: MenuCommand::CopyTo(None),
                ..
            }
        ));
        // Hints, Remove is the danger item.
        assert!(matches!(
            &entries[0],
            MenuEntry::Command {
                hint: Some("Enter"),
                ..
            }
        ));
        assert!(matches!(
            &entries[3],
            MenuEntry::Command {
                hint: Some("E"),
                ..
            }
        ));
        assert!(matches!(
            &entries[9],
            MenuEntry::Command { danger: true, .. }
        ));
        assert!(matches!(
            &entries[8],
            MenuEntry::Command { info: Some(_), .. }
        ));
    }

    #[test]
    fn single_record_items_are_inactive_for_a_multi_selection() {
        let enabled = |entries: &[MenuEntry]| -> Vec<String> {
            entries
                .iter()
                .filter_map(|entry| match entry {
                    MenuEntry::Command {
                        id, enabled: true, ..
                    } => Some(id.to_string()),
                    MenuEntry::Submenu { id, .. } => Some(id.to_string()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            enabled(&host_menu(3, &targets(), &[], false)),
            [
                "menu-quick-connect",
                "menu-connect",
                "menu-add-to-workspace",
                "menu-move-to",
                "menu-copy-to",
                "menu-remove",
            ]
        );
        assert_eq!(enabled(&host_menu(1, &targets(), &[], false)).len(), 10);
    }

    #[test]
    fn group_menu_has_the_order_of_ui_3_4() {
        assert_eq!(
            entry_ids(&group_menu(1, &targets())),
            [
                "menu-open-group",
                "menu-edit-group",
                "menu-move-to",
                "menu-remove"
            ]
        );
    }
}
