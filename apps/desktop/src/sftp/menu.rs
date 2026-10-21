//! The context menu of a file and the Actions menu of a panel (UI 5.4,
//! 5.5), and their dialogs: New Folder and Rename (a name), Delete (a
//! confirmation naming what goes) and Edit Permissions (rwx checkboxes with
//! the octal value kept in step, "apply to everything inside" for folders).
//!
//! Items that need a selection are disabled without one; Edit Permissions
//! is hidden in a local panel on Windows; Delete and Close are drawn in
//! `--danger`.

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Entity, FontWeight, IntoElement, SharedString, Window, div, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::component::checkbox::Checkbox;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::menu::PopupMenu;
use gpui_kit::component::{ActiveTheme as _, Disableable as _, WindowExt as _, h_flex, v_flex};

use super::listing::{Entry, Kind};
use super::panel::{FilePanel, PanelEvent, valid_name};
use super::strings;
use crate::hosts::controls::{ControlButton, ControlKind};
use crate::panels::{menus, widgets};
use crate::theme::metrics;

/// Width of the dialogs.
const DIALOG_WIDTH: f32 = 420.;

/// What a menu item does with its panel.
type PanelAction = Rc<dyn Fn(&Entity<FilePanel>, &mut Window, &mut App)>;

/// The menu of `panel`: for the file under the pointer (`entry`, the
/// context menu) or for the panel's selection (the Actions menu, which adds
/// Show Hidden Files, Select All and Close).
pub fn panel_menu(
    menu: PopupMenu,
    panel: &Entity<FilePanel>,
    entry: Option<Entry>,
    _window: &mut Window,
    cx: &mut App,
) -> PopupMenu {
    let read = panel.read(cx);
    let selection = read.selection();
    let some = !selection.is_empty();
    let single = selection.first().cloned().filter(|_| selection.len() == 1);
    let local = read.is_local();
    let permissions = !(local && cfg!(windows));
    let show_hidden = read.show_hidden();
    let actions = entry.is_none();
    let weak = panel.downgrade();
    let on = |run: PanelAction| {
        let weak = weak.clone();
        move |window: &mut Window, cx: &mut App| {
            if let Some(panel) = weak.upgrade() {
                run(&panel, window, cx);
            }
        }
    };

    let open_entry = single.clone();
    let open_with_entry = single.clone().filter(|entry| !entry.is_folder_like());
    let rename_entry = single.clone();
    let copied = selection.clone();
    let deleted = selection.clone();
    let changed = selection.clone();
    let mut menu = menu
        .min_w(px(220.))
        .item(menus::menu_item(
            strings::MENU_OPEN,
            false,
            single.is_none(),
            on(Rc::new(move |panel, _, cx| {
                if let Some(entry) = &open_entry {
                    panel.update(cx, |panel, cx| panel.open_entry(entry, cx));
                }
            })),
        ))
        .item(menus::menu_item(
            strings::MENU_OPEN_WITH,
            false,
            open_with_entry.is_none(),
            on(Rc::new(move |panel, _, cx| {
                if let Some(entry) = &open_with_entry {
                    let entry = entry.clone();
                    panel.update(cx, |_, cx| cx.emit(PanelEvent::OpenWith(entry)));
                }
            })),
        ))
        .item(menus::menu_item(
            strings::MENU_COPY,
            false,
            !some,
            on(Rc::new(move |panel, _, cx| {
                let entries = copied.clone();
                panel.update(cx, |_, cx| cx.emit(PanelEvent::Copy(entries)));
            })),
        ))
        .item(menus::menu_item(
            strings::MENU_RENAME,
            false,
            single.is_none(),
            on(Rc::new(move |panel, window, cx| {
                if let Some(entry) = &rename_entry {
                    open_rename(panel.clone(), entry.name.clone(), window, cx);
                }
            })),
        ))
        .item(menus::menu_item(
            strings::MENU_DELETE,
            true,
            !some,
            on(Rc::new(move |panel, window, cx| {
                open_delete(panel.clone(), deleted.clone(), window, cx);
            })),
        ))
        .separator()
        .item(menus::menu_item(
            strings::MENU_REFRESH,
            false,
            false,
            on(Rc::new(|panel, _, cx| {
                panel.update(cx, |panel, cx| panel.refresh(cx))
            })),
        ))
        .item(menus::menu_item(
            strings::MENU_NEW_FOLDER,
            false,
            false,
            on(Rc::new(|panel, window, cx| {
                open_new_folder(panel.clone(), window, cx)
            })),
        ));
    if permissions {
        menu = menu.item(menus::menu_item(
            strings::MENU_PERMISSIONS,
            false,
            !some,
            on(Rc::new(move |panel, window, cx| {
                open_permissions(panel.clone(), changed.clone(), window, cx);
            })),
        ));
    }
    if actions {
        menu = menu
            .separator()
            .item(
                menus::menu_item(
                    strings::MENU_SHOW_HIDDEN,
                    false,
                    false,
                    on(Rc::new(move |panel, _, cx| {
                        panel.update(cx, |panel, cx| panel.set_show_hidden(!show_hidden, cx));
                    })),
                )
                .checked(show_hidden),
            )
            .item(menus::menu_item(
                strings::MENU_SELECT_ALL,
                false,
                false,
                on(Rc::new(|panel, _, cx| {
                    panel.update(cx, |panel, cx| panel.select_all(cx))
                })),
            ));
        if !local {
            menu = menu.item(menus::menu_item(
                strings::MENU_CLOSE,
                true,
                false,
                on(Rc::new(|panel, _, cx| {
                    panel.update(cx, |panel, cx| panel.close(cx))
                })),
            ));
        }
    }
    menu
}

fn title(text: impl Into<SharedString>) -> impl IntoElement {
    div()
        .text_size(metrics::FONT_SIZE_TITLE)
        .font_weight(FontWeight::SEMIBOLD)
        .child(text.into())
}

/// A dialog asking for a name: New Folder and Rename.
fn open_name_dialog(
    id: &'static str,
    heading: &'static str,
    confirm: &'static str,
    initial: String,
    window: &mut Window,
    cx: &mut App,
    on_name: impl Fn(String, &mut Window, &mut App) + 'static,
) {
    let input = cx.new(|cx| {
        let mut input = InputState::new(window, cx).placeholder(strings::NEW_FOLDER_PLACEHOLDER);
        input.set_value(initial, window, cx);
        input
    });
    input.update(cx, |input, cx| input.focus(window, cx));
    let on_name = Rc::new(on_name);
    let submit = {
        let input = input.clone();
        let on_name = on_name.clone();
        move |window: &mut Window, cx: &mut App| -> bool {
            let name = input.read(cx).value().to_string();
            if !valid_name(&name) {
                return false;
            }
            on_name(name.trim().to_owned(), window, cx);
            true
        }
    };
    let submit = Rc::new(submit);
    // Enter in the field submits.
    let enter = submit.clone();
    let subscription = window.subscribe(&input, cx, move |_, event: &InputEvent, window, cx| {
        if let InputEvent::PressEnter { .. } = event
            && enter(window, cx)
        {
            window.close_dialog(cx);
        }
    });
    window.open_dialog(cx, move |dialog, window, cx| {
        let _ = &subscription;
        let name = input.read(cx).value().to_string();
        let invalid = !name.trim().is_empty() && !valid_name(&name);
        let ok = submit.clone();
        dialog
            .w(px(DIALOG_WIDTH))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(heading))
            .child(
                v_flex()
                    .id(id)
                    .test_support()
                    .gap(metrics::SPACE_1)
                    .child(widgets::text_input(
                        &input,
                        SharedString::from(format!("{id}-name")),
                        invalid,
                        window,
                        cx,
                    ))
                    .when(invalid, |this| {
                        this.child(widgets::error_text(strings::NAME_INVALID, cx))
                    }),
            )
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new(
                            SharedString::from(format!("{id}-cancel")),
                            ControlKind::Neutral,
                        )
                        .label(strings::CANCEL)
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new(
                            SharedString::from(format!("{id}-ok")),
                            ControlKind::Accent,
                        )
                        .label(confirm)
                        .disabled(!valid_name(&name))
                        .on_click(move |_: &ClickEvent, window, cx| {
                            if ok(window, cx) {
                                window.close_dialog(cx);
                            }
                        }),
                    ),
            )
    });
}

fn open_new_folder(panel: Entity<FilePanel>, window: &mut Window, cx: &mut App) {
    open_name_dialog(
        "sftp-new-folder",
        strings::NEW_FOLDER_TITLE,
        strings::CREATE,
        String::new(),
        window,
        cx,
        move |name, window, cx| {
            panel.update(cx, |panel, cx| panel.new_folder(&name, window, cx));
        },
    );
}

fn open_rename(panel: Entity<FilePanel>, from: String, window: &mut Window, cx: &mut App) {
    let old = from.clone();
    open_name_dialog(
        "sftp-rename",
        strings::RENAME_TITLE,
        strings::RENAME,
        from,
        window,
        cx,
        move |name, window, cx| {
            panel.update(cx, |panel, cx| panel.rename(&old, &name, window, cx));
        },
    );
}

/// Delete: `Delete "name"?` with what it removes; Delete in `--danger`.
pub(crate) fn open_delete(
    panel: Entity<FilePanel>,
    entries: Vec<Entry>,
    window: &mut Window,
    cx: &mut App,
) {
    if entries.is_empty() {
        return;
    }
    let names: Vec<String> = entries.iter().map(|entry| entry.name.clone()).collect();
    let folders = entries
        .iter()
        .filter(|entry| matches!(entry.kind, Kind::Folder))
        .count();
    let heading: SharedString = strings::delete_title(&names).into();
    let text: SharedString = strings::delete_text(folders).into();
    let entries = Rc::new(entries);
    window.open_dialog(cx, move |dialog, _, cx| {
        let panel = panel.clone();
        let entries = entries.clone();
        dialog
            .w(px(DIALOG_WIDTH))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(heading.clone()))
            .child(
                div()
                    .id("sftp-delete")
                    .test_support()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .text_color(cx.theme().muted_foreground)
                    .child(text.clone()),
            )
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("sftp-delete-cancel", ControlKind::Neutral)
                            .label(strings::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("sftp-delete-confirm", ControlKind::Danger)
                            .label(strings::DELETE)
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                let entries = (*entries).clone();
                                panel.update(cx, |panel, cx| panel.delete(entries, window, cx));
                            }),
                    ),
            )
    });
}

/// The rwx checkboxes of Edit Permissions with the octal field in step.
pub struct PermissionsForm {
    mode: u32,
    recursive: bool,
    octal: Entity<InputState>,
    /// The octal field shows a value the user typed that is not valid.
    invalid: bool,
    _subscription: gpui::Subscription,
}

impl std::fmt::Debug for PermissionsForm {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PermissionsForm")
            .field("mode", &format_args!("{:03o}", self.mode))
            .field("recursive", &self.recursive)
            .finish_non_exhaustive()
    }
}

impl PermissionsForm {
    pub fn new(mode: u32, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let octal = cx.new(|cx| {
            let mut input = InputState::new(window, cx);
            input.set_value(format!("{:03o}", mode & 0o777), window, cx);
            input
        });
        let subscription =
            cx.subscribe(&octal, |this: &mut Self, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(cx).value().to_string();
                    match parse_octal(&text) {
                        Some(mode) => {
                            this.mode = mode;
                            this.invalid = false;
                        }
                        None => this.invalid = true,
                    }
                    cx.notify();
                }
            });
        Self {
            mode: mode & 0o777,
            recursive: false,
            octal,
            invalid: false,
            _subscription: subscription,
        }
    }

    pub fn mode(&self) -> u32 {
        self.mode
    }

    pub fn recursive(&self) -> bool {
        self.recursive
    }

    pub fn is_valid(&self) -> bool {
        !self.invalid
    }

    /// One checkbox: `bit` of the mode.
    pub fn set_bit(&mut self, bit: u32, on: bool, window: &mut Window, cx: &mut Context<Self>) {
        if on {
            self.mode |= bit;
        } else {
            self.mode &= !bit;
        }
        self.invalid = false;
        let text = format!("{:03o}", self.mode);
        self.octal
            .update(cx, |input, cx| input.set_value(text, window, cx));
        cx.notify();
    }
}

/// `755` or `0755` as a mode; anything else is `None`.
pub fn parse_octal(text: &str) -> Option<u32> {
    let text = text.trim();
    if text.is_empty() || text.len() > 4 || !text.chars().all(|c| ('0'..='7').contains(&c)) {
        return None;
    }
    u32::from_str_radix(text, 8)
        .ok()
        .filter(|mode| *mode <= 0o777)
}

impl Render for PermissionsForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let rows = [
            (strings::PERMISSIONS_OWNER, 6, "owner"),
            (strings::PERMISSIONS_GROUP, 3, "group"),
            (strings::PERMISSIONS_OTHERS, 0, "others"),
        ];
        let bits = [
            (strings::PERMISSIONS_READ, 4, "r"),
            (strings::PERMISSIONS_WRITE, 2, "w"),
            (strings::PERMISSIONS_EXECUTE, 1, "x"),
        ];
        v_flex()
            .id("sftp-permissions")
            .test_support()
            .gap(metrics::SPACE_3)
            .children(rows.into_iter().map(|(label, shift, who)| {
                h_flex()
                    .gap(metrics::SPACE_4)
                    .child(
                        div()
                            .w(px(72.))
                            .text_size(metrics::FONT_SIZE_BODY)
                            .text_color(theme.foreground)
                            .child(label),
                    )
                    .children(bits.into_iter().map(|(name, value, letter)| {
                        let bit = value << shift;
                        Checkbox::new(SharedString::from(format!("sftp-perm-{who}-{letter}")))
                            .checked(self.mode & bit != 0)
                            .label(name)
                            .on_click(cx.listener(move |this, checked: &bool, window, cx| {
                                this.set_bit(bit, *checked, window, cx)
                            }))
                    }))
            }))
            .child(widgets::field(
                strings::PERMISSIONS_OCTAL,
                widgets::text_input(&self.octal, "sftp-perm-octal", self.invalid, window, cx),
                None,
                cx,
            ))
            .child(
                Checkbox::new("sftp-perm-recursive")
                    .checked(self.recursive)
                    .label(strings::PERMISSIONS_RECURSIVE)
                    .on_click(cx.listener(|this, checked: &bool, _, cx| {
                        this.recursive = *checked;
                        cx.notify();
                    })),
            )
    }
}

/// Edit Permissions: the first entry's mode to start with; Apply sets the
/// mode on every entry (into folders with "apply to everything inside").
pub(crate) fn open_permissions(
    panel: Entity<FilePanel>,
    entries: Vec<Entry>,
    window: &mut Window,
    cx: &mut App,
) {
    let Some(first) = entries.first() else {
        return;
    };
    let start = first.permissions.unwrap_or(0o644);
    let form = cx.new(|cx| PermissionsForm::new(start, window, cx));
    let entries = Rc::new(entries);
    window.open_dialog(cx, move |dialog, _, cx| {
        let panel = panel.clone();
        let entries = entries.clone();
        let read = form.clone();
        let valid = form.read(cx).is_valid();
        dialog
            .w(px(DIALOG_WIDTH))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(title(strings::PERMISSIONS_TITLE))
            .child(form.clone())
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("sftp-perm-cancel", ControlKind::Neutral)
                            .label(strings::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("sftp-perm-apply", ControlKind::Accent)
                            .label(strings::APPLY)
                            .disabled(!valid)
                            .on_click(move |_, window, cx| {
                                let (mode, recursive) = {
                                    let form = read.read(cx);
                                    (form.mode(), form.recursive())
                                };
                                window.close_dialog(cx);
                                let entries = (*entries).clone();
                                panel.update(cx, |panel, cx| {
                                    panel.set_permissions(entries, mode, recursive, window, cx)
                                });
                            }),
                    ),
            )
    });
}
