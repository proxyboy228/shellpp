//! Rendering of the Snippets screen: the action bar, packages and snippet
//! cards, the empty states, the snippet panel and the dialogs (removal,
//! package name). Cards and the panel follow the Port Forwarding screen.

use std::cell::RefCell;
use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, DragMoveEvent, Entity, Focusable as _,
    FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ParentElement as _, Render, SharedString, StatefulInteractiveElement as _, Styled as _, Window,
    canvas, div, prelude::FluentBuilder as _, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::input::{Input, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    WindowExt as _, h_flex, v_flex,
};
use shellpp_core::service::dto::TagColor;
use shellpp_core::service::snippets::{SnippetPackageView, SnippetView};
use uuid::Uuid;

use super::{
    FormMode, SnippetsScreen, card_id, card_title, conflict_badge_id, package_conflict_badge_id,
    package_id, strings,
};
use crate::hosts::controls::{self, ControlButton, ControlKind, ICON_SIZE};
use crate::hosts::layout as grid;
use crate::panels::widgets;
use crate::shell::details::{self, DetailsHeader, ResizeDetails, ResizePreview};
use crate::theme::focus::FocusRingExt as _;
use crate::theme::{self, metrics};

pub(super) const CONTENT_PADDING: gpui::Rems = metrics::SPACE_5;
const SEARCH_MIN_WIDTH: gpui::Rems = metrics::rpx(160.);
const SEARCH_MAX_WIDTH: gpui::Rems = metrics::rpx(320.);
const DIALOG_WIDTH: gpui::Rems = metrics::rpx(420.);

/// A plain menu item with a stable id; `danger` items use `--danger`.
fn menu_item(id: &'static str, label: &'static str, danger: bool, cx: &App) -> AnyElement {
    let theme = cx.theme();
    div()
        .id(id)
        .test_support()
        .flex_1()
        .text_color(if danger {
            theme.danger
        } else {
            theme.foreground
        })
        .child(label)
        .into_any_element()
}

/// The snippet menu: Run, Edit, Remove.
fn snippet_menu(menu: PopupMenu, screen: gpui::WeakEntity<SnippetsScreen>, id: Uuid) -> PopupMenu {
    let run = screen.clone();
    let edit = screen.clone();
    menu.min_w(px(200.))
        .item(
            PopupMenuItem::element(|_, cx| menu_item("snippet-menu-run", strings::RUN, false, cx))
                .on_click(move |_, window, cx| {
                    let _ = run.update(cx, |screen, cx| screen.run_from_menu(id, window, cx));
                }),
        )
        .item(
            PopupMenuItem::element(|_, cx| {
                menu_item("snippet-menu-edit", strings::EDIT, false, cx)
            })
            .on_click(move |_, window, cx| {
                let _ = edit.update(cx, |screen, cx| screen.select(Some(id), window, cx));
            }),
        )
        .separator()
        .item(
            PopupMenuItem::element(|_, cx| {
                menu_item("snippet-menu-remove", strings::REMOVE, true, cx)
            })
            .on_click(move |_, window, cx| {
                let _ = screen.update(cx, |screen, cx| screen.confirm_remove(id, window, cx));
            }),
        )
}

/// The package menu: Open, Rename, Remove.
fn package_menu(menu: PopupMenu, screen: gpui::WeakEntity<SnippetsScreen>, id: Uuid) -> PopupMenu {
    let open = screen.clone();
    let rename = screen.clone();
    menu.min_w(px(200.))
        .item(
            PopupMenuItem::element(|_, cx| {
                menu_item("package-menu-open", strings::OPEN, false, cx)
            })
            .on_click(move |_, _, cx| {
                let _ = open.update(cx, |screen, cx| screen.enter_package(Some(id), cx));
            }),
        )
        .item(
            PopupMenuItem::element(|_, cx| {
                menu_item("package-menu-rename", strings::RENAME, false, cx)
            })
            .on_click(move |_, window, cx| {
                let _ = rename.update(cx, |screen, cx| screen.edit_package(Some(id), window, cx));
            }),
        )
        .separator()
        .item(
            PopupMenuItem::element(|_, cx| {
                menu_item("package-menu-remove", strings::REMOVE, true, cx)
            })
            .on_click(move |_, window, cx| {
                let _ = screen.update(cx, |screen, cx| {
                    screen.confirm_remove_package(id, window, cx)
                });
            }),
        )
}

/// A confirmation with Cancel and a `--danger` Remove; `id` prefixes the
/// element ids (`<id>-dialog`, `<id>-confirm`).
pub(crate) fn open_confirmation(
    id: &'static str,
    title: &'static str,
    body: String,
    window: &mut Window,
    cx: &mut App,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
) {
    let on_confirm = Rc::new(on_confirm);
    let body: SharedString = body.into();
    window.open_dialog(cx, move |dialog, _, cx| {
        let confirm = on_confirm.clone();
        let on_ok = on_confirm.clone();
        dialog
            .w(metrics::to_px(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .id(SharedString::from(format!("{id}-dialog")))
                    .test_support()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .text_color(cx.theme().muted_foreground)
                    .child(body.clone()),
            )
            .on_ok(move |_, window, cx| {
                on_ok(window, cx);
                true
            })
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
                            SharedString::from(format!("{id}-confirm")),
                            ControlKind::Danger,
                        )
                        .label(strings::REMOVE)
                        .on_click(move |_, window, cx| {
                            window.close_dialog(cx);
                            confirm(window, cx);
                        }),
                    ),
            )
    });
}

/// What was chosen on leaving the panel with changes not saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum LeaveChoice {
    Save,
    Discard,
}

type LeaveAction = Rc<RefCell<Option<Box<dyn FnOnce(LeaveChoice, &mut Window, &mut App)>>>>;

/// A button's handler: closes the dialog, then acts once (the action may
/// open a dialog of its own).
fn leave_choice(
    action: &LeaveAction,
    choice: LeaveChoice,
) -> impl Fn(&mut Window, &mut App) + 'static {
    let action = action.clone();
    move |window, cx| {
        window.close_dialog(cx);
        if let Some(act) = action.borrow_mut().take() {
            act(choice, window, cx);
        }
    }
}

fn dialog_title(title: SharedString) -> impl IntoElement {
    div()
        .text_size(metrics::FONT_SIZE_TITLE)
        .font_weight(FontWeight::SEMIBOLD)
        .child(title)
}

/// Leaving the panel with changes not saved (D68): Save, Discard changes
/// or Stay. Enter saves; Esc stays.
pub(super) fn open_leave_dialog(
    title: Option<String>,
    window: &mut Window,
    cx: &mut App,
    on_choice: impl FnOnce(LeaveChoice, &mut Window, &mut App) + 'static,
) {
    let action: LeaveAction = Rc::new(RefCell::new(Some(Box::new(on_choice))));
    let heading: SharedString = strings::leave_title(title.as_deref()).into();
    window.open_dialog(cx, move |dialog, _, cx| {
        let save = leave_choice(&action, LeaveChoice::Save);
        let on_ok = leave_choice(&action, LeaveChoice::Save);
        let discard = leave_choice(&action, LeaveChoice::Discard);
        dialog
            .w(metrics::to_px(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(dialog_title(heading.clone()))
            .child(
                div()
                    .id("snippet-leave-dialog")
                    .test_support()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .text_color(cx.theme().muted_foreground)
                    .child(strings::LEAVE_BODY),
            )
            .on_ok(move |_, window, cx| {
                on_ok(window, cx);
                false
            })
            .footer(
                h_flex()
                    .w_full()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("snippet-leave-discard", ControlKind::Danger)
                            .label(strings::DISCARD_CHANGES)
                            .on_click(move |_, window, cx| discard(window, cx)),
                    )
                    .child(div().flex_1())
                    .child(
                        ControlButton::new("snippet-leave-stay", ControlKind::Neutral)
                            .label(strings::STAY)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("snippet-leave-save", ControlKind::Accent)
                            .label(strings::SAVE)
                            .on_click(move |_, window, cx| save(window, cx)),
                    ),
            )
    });
}

/// Run with changes not saved (D68): Save and run, or Cancel. Enter saves
/// and runs.
pub(super) fn open_save_and_run(
    title: Option<String>,
    window: &mut Window,
    cx: &mut App,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
) {
    let on_confirm = Rc::new(on_confirm);
    let heading: SharedString = strings::save_run_title(title.as_deref()).into();
    window.open_dialog(cx, move |dialog, _, cx| {
        let confirm = on_confirm.clone();
        let on_ok = on_confirm.clone();
        dialog
            .w(metrics::to_px(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(dialog_title(heading.clone()))
            .child(
                div()
                    .id("snippet-save-run-dialog")
                    .test_support()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .text_color(cx.theme().muted_foreground)
                    .child(strings::SAVE_RUN_BODY),
            )
            // Closed before the run opens its own dialogs.
            .on_ok(move |_, window, cx| {
                window.close_dialog(cx);
                on_ok(window, cx);
                false
            })
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("snippet-save-run-cancel", ControlKind::Neutral)
                            .label(strings::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("snippet-save-run-confirm", ControlKind::Accent)
                            .icon(Lucide::Play)
                            .label(strings::SAVE_AND_RUN)
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                confirm(window, cx);
                            }),
                    ),
            )
    });
}

/// New snippet package / Rename: the name; `save` returns whether it was
/// taken (the dialog closes then).
pub(super) fn open_package_dialog(
    id: Option<Uuid>,
    name: String,
    window: &mut Window,
    cx: &mut App,
    save: impl Fn(String, &mut Window, &mut App) -> bool + 'static,
) {
    let input: Entity<InputState> = cx.new(|cx| {
        InputState::new(window, cx)
            .default_value(name)
            .placeholder(strings::PACKAGE_NAME_PLACEHOLDER)
    });
    input.update(cx, |input, cx| input.focus(window, cx));
    let save = Rc::new(save);
    let (title, button) = match id {
        None => (strings::PACKAGE_TITLE_NEW, strings::CREATE),
        Some(_) => (strings::PACKAGE_TITLE_RENAME, strings::SAVE),
    };
    window.open_dialog(cx, move |dialog, window, cx| {
        let submit = {
            let input = input.clone();
            let save = save.clone();
            move |window: &mut Window, cx: &mut App| {
                let name = input.read(cx).value().to_string();
                if save(name, window, cx) {
                    window.close_dialog(cx);
                }
            }
        };
        let submit = Rc::new(submit);
        let on_ok = submit.clone();
        dialog
            .w(metrics::to_px(DIALOG_WIDTH, cx))
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .id("package-dialog")
                    .test_support()
                    .child(widgets::field(
                        strings::PACKAGE_NAME,
                        widgets::text_input(&input, "package-name", false, window, cx),
                        None,
                        cx,
                    )),
            )
            .on_ok(move |_, window, cx| {
                on_ok(window, cx);
                false
            })
            .footer(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap(metrics::SPACE_2)
                    .child(
                        ControlButton::new("package-cancel", ControlKind::Neutral)
                            .label(strings::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("package-save", ControlKind::Accent)
                            .label(button)
                            .on_click(move |_, window, cx| submit(window, cx)),
                    ),
            )
    });
}

impl SnippetsScreen {
    // -- Action bar --------------------------------------------------------------

    fn render_search(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let focused = self
            .search
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx);
        h_flex()
            .id("snippet-search-field")
            .test_support()
            .relative()
            .flex_1()
            .min_w(SEARCH_MIN_WIDTH)
            .max_w(SEARCH_MAX_WIDTH)
            .h(metrics::HEIGHT_CONTROL)
            .gap(metrics::SPACE_2)
            .px(metrics::SPACE_3)
            .rounded(metrics::RADIUS_CONTROL)
            .bg(tokens.bg_input.hsla())
            .border_1()
            .border_color(tokens.border_strong.hsla())
            .child(
                Icon::new(IconName::Search)
                    .size(ICON_SIZE)
                    .text_color(theme.muted_foreground),
            )
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .child(theme::controls::muted_placeholder(&self.search, cx))
                    .child(
                        Input::new(&self.search)
                            .id("snippet-search")
                            .appearance(false)
                            .cleanable(true)
                            .pl_0()
                            .text_size(metrics::FONT_SIZE_BODY),
                    ),
            )
            .when(focused, |this| {
                this.child(controls::focus_ring(
                    metrics::RADIUS_CONTROL,
                    metrics::rpx(1.),
                    cx,
                ))
            })
    }

    fn render_action_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let screen = cx.entity().downgrade();
        let new_snippet = ControlButton::new("snippet-new", ControlKind::Neutral)
            .icon(Lucide::Plus)
            .label(strings::NEW_SNIPPET)
            .rounded_sides(true, false)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.new_snippet(window, cx)));
        let new_menu = ControlButton::new("snippet-new-menu", ControlKind::Neutral)
            .icon(IconName::ChevronDown)
            .tooltip(strings::NEW_MENU)
            .rounded_sides(false, true)
            .dropdown_menu(move |menu, _, cx| {
                let screen = screen.clone();
                menu.min_w(metrics::scaled(220., cx)).item(
                    PopupMenuItem::element(|_, cx| {
                        menu_item("snippet-new-package", strings::NEW_PACKAGE, false, cx)
                    })
                    .on_click(move |_, window, cx| {
                        let _ =
                            screen.update(cx, |screen, cx| screen.edit_package(None, window, cx));
                    }),
                )
            });
        let showing_history = self.history.is_some();
        let history = ControlButton::new("snippet-shell-history", ControlKind::Neutral)
            .icon(Lucide::RotateCcwClock)
            .label(strings::SHELL_HISTORY)
            .selected(showing_history)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.show_history(!showing_history, window, cx)
            }));
        h_flex()
            .id("snippet-action-bar")
            .test_support()
            .w_full()
            .flex_wrap()
            .items_center()
            .gap(metrics::SPACE_2)
            .child(
                h_flex()
                    .flex_none()
                    .child(new_snippet)
                    .child(
                        div()
                            .w(metrics::rpx(1.))
                            .h(metrics::HEIGHT_CONTROL)
                            .bg(theme.border),
                    )
                    .child(new_menu),
            )
            .child(history)
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .justify_end()
                    // Shell History has its own search.
                    .when(!showing_history, |this| {
                        this.child(self.render_search(window, cx))
                    }),
            )
    }

    /// The way back from a package or Shell History: "Snippets › Deploy".
    fn render_breadcrumb(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let name = if self.history.is_some() {
            strings::SHELL_HISTORY.to_owned()
        } else {
            self.package
                .and_then(|id| self.package_view(id))?
                .name
                .clone()
        };
        let theme = cx.theme();
        Some(
            h_flex()
                .id("snippet-breadcrumb")
                .test_support()
                .px(CONTENT_PADDING)
                .pb(metrics::SPACE_3)
                .gap(metrics::SPACE_2)
                .items_center()
                .text_size(metrics::FONT_SIZE_BODY)
                .child(
                    widgets::text_button("snippet-back", strings::ALL_SNIPPETS, true, cx).on_click(
                        cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.show_history(false, window, cx);
                            this.enter_package(None, cx)
                        }),
                    ),
                )
                .child(
                    Icon::new(IconName::ChevronRight)
                        .small()
                        .text_color(theme.muted_foreground),
                )
                .child(div().font_weight(FontWeight::MEDIUM).child(name))
                .into_any_element(),
        )
    }

    // -- Content -------------------------------------------------------------------

    fn section_title(title: &'static str, cx: &App) -> impl IntoElement {
        div()
            .w_full()
            .pb(metrics::SPACE_2)
            .text_size(metrics::FONT_SIZE_CAPTION)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().muted_foreground)
            .child(title)
    }

    fn card_frame(
        &self,
        id: SharedString,
        selected: bool,
        width: f32,
        cx: &App,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = cx.theme();
        let hover = theme.secondary_hover;
        let pressed = theme.secondary_active;
        div()
            .id(id)
            .relative()
            .flex_none()
            .w(metrics::rpx(width))
            .h(metrics::HEIGHT_HOST_CARD)
            .flex()
            .items_center()
            .gap(metrics::SPACE_3)
            .px(metrics::SPACE_3)
            .rounded(metrics::RADIUS_CARD)
            .bg(theme.secondary)
            .border_1()
            .border_color(if selected {
                theme.primary
            } else {
                theme.border
            })
            .cursor_default()
            .hover(move |style| style.bg(hover))
            .active(move |style| style.bg(pressed))
    }

    fn texts(title: String, subtitle: String, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
            .flex_1()
            .min_w_0()
            .gap(metrics::rpx(2.))
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.foreground)
                    .truncate()
                    .child(title),
            )
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(subtitle),
            )
    }

    fn render_package(
        &self,
        package: &SnippetPackageView,
        width: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = package.record.record_id;
        let screen = cx.entity().downgrade();
        // With nothing selected the ring marks the first folder, where the
        // arrows start.
        let first = self.selected.is_none()
            && self.package_cursor.is_none()
            && self.shown_package_ids().first() == Some(&id);
        let keyboard_cursor = (self.package_cursor == Some(id) || first)
            && self.focus.is_focused(window)
            && window.last_input_was_keyboard();
        self.card_frame(package_id(id), false, width, cx)
            .test_support()
            .when(keyboard_cursor, |this| {
                this.child(controls::focus_ring(
                    metrics::RADIUS_CARD,
                    metrics::rpx(1.),
                    cx,
                ))
            })
            .child(widgets::tile(
                Lucide::Folder,
                TagColor::Amber,
                metrics::HOST_TILE_SIZE,
            ))
            .child(Self::texts(
                package.name.clone(),
                strings::snippet_count(self.package_size(id)),
                cx,
            ))
            // Sync kept another version of the package (UI 13.6).
            .when(
                self.state.read(cx).sync_conflicted().contains(&id),
                |this| {
                    this.child(crate::sync_ui::compare::conflict_badge(
                        package_conflict_badge_id(id),
                        &self.state,
                        id,
                        cx,
                    ))
                },
            )
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.focus.focus(window, cx);
                this.enter_package(Some(id), cx);
            }))
            .context_menu(move |menu, _, _| package_menu(menu, screen.clone(), id))
            .into_any_element()
    }

    fn render_snippet(
        &self,
        snippet: &SnippetView,
        width: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = snippet.record.record_id;
        let selected = self.selected == Some(id);
        // With nothing selected the ring marks the first card, where the
        // arrows start.
        let first = self.selected.is_none()
            && self.package_cursor.is_none()
            && self.shown_package_ids().is_empty()
            && self.shown_ids().first() == Some(&id);
        let keyboard_cursor = (selected || first)
            && self.focus.is_focused(window)
            && window.last_input_was_keyboard();
        let package = snippet
            .package_id
            .filter(|_| self.package.is_none())
            .and_then(|id| self.package_view(id))
            .map(|package| package.name.clone());
        let subtitle = match package {
            Some(name) => format!(
                "{name} · {}",
                strings::targets(snippet.target_host_ids.len())
            ),
            None => strings::targets(snippet.target_host_ids.len()),
        };
        let screen = cx.entity().downgrade();
        self.card_frame(card_id(id), selected, width, cx)
            .test_support()
            .child(widgets::tile(
                Lucide::Braces,
                TagColor::Graphite,
                metrics::HOST_TILE_SIZE,
            ))
            .child(Self::texts(card_title(snippet), subtitle, cx))
            // Sync kept another version of the snippet (UI 13.6).
            .when(
                self.state.read(cx).sync_conflicted().contains(&id),
                |this| {
                    this.child(crate::sync_ui::compare::conflict_badge(
                        conflict_badge_id(id),
                        &self.state,
                        id,
                        cx,
                    ))
                },
            )
            .when(keyboard_cursor, |this| {
                this.child(controls::focus_ring(
                    metrics::RADIUS_CARD,
                    metrics::rpx(1.),
                    cx,
                ))
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.focus.focus(window, cx);
                if event.click_count() >= 2 {
                    this.run(id, window, cx);
                } else if this.selected != Some(id) {
                    this.select(Some(id), window, cx);
                }
            }))
            .context_menu(move |menu, window, cx| {
                let _ = screen.update(cx, |screen, cx| {
                    if screen.selected != Some(id) {
                        screen.select(Some(id), window, cx);
                    }
                });
                snippet_menu(menu, screen.clone(), id)
            })
            .into_any_element()
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let searching = !self.search.read(cx).value().trim().is_empty();
        let (id, title, body) = if searching {
            ("snippet-no-matches", strings::NO_MATCHES, None)
        } else if self.package.is_some() {
            (
                "snippet-empty-package",
                strings::EMPTY_TITLE,
                Some(strings::EMPTY_PACKAGE),
            )
        } else {
            (
                "snippet-empty",
                strings::EMPTY_TITLE,
                Some(strings::EMPTY_BODY),
            )
        };
        v_flex()
            .id(id)
            .test_support()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(metrics::SPACE_2)
            .px(CONTENT_PADDING)
            .child(
                div()
                    .text_center()
                    .text_size(metrics::FONT_SIZE_HEADING)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(title),
            )
            .when_some(body, |this, body| {
                this.child(
                    div()
                        .max_w(metrics::rpx(420.))
                        .text_center()
                        .text_color(theme.muted_foreground)
                        .child(body),
                )
            })
            .when(!searching && self.form.is_none(), |this| {
                this.child(
                    h_flex().pt(metrics::SPACE_2).child(
                        ControlButton::new("snippet-empty-new", ControlKind::Accent)
                            .icon(Lucide::Plus)
                            .label(strings::NEW_SNIPPET)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.new_snippet(window, cx)
                            })),
                    ),
                )
            })
            .into_any_element()
    }

    fn render_grid(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let width = self.grid_width.unwrap_or(0.).max(1.);
        let columns = grid::columns_for(width);
        let card_width = grid::card_width(width, columns);
        let screen = cx.entity().downgrade();
        let measure = canvas(
            move |bounds, _, cx| {
                let width = metrics::unscaled(bounds.size.width, cx).as_f32();
                let _ = screen.update(cx, |screen, cx| {
                    if screen
                        .grid_width
                        .is_none_or(|measured| (measured - width).abs() >= 0.5)
                    {
                        screen.grid_width = Some(width);
                        screen.columns = grid::columns_for(width);
                        cx.notify();
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let packages: Vec<SnippetPackageView> = self
            .shown_packages
            .iter()
            .map(|index| self.packages[*index].clone())
            .collect();
        let snippets: Vec<SnippetView> = self
            .shown
            .iter()
            .map(|index| self.snippets[*index].clone())
            .collect();
        let wrap = || {
            div()
                .w_full()
                .flex()
                .flex_wrap()
                .gap(metrics::HOST_CARD_GAP)
        };
        let package_section = (!packages.is_empty()).then(|| {
            v_flex()
                .id("snippet-packages")
                .test_support()
                .w_full()
                .pb(metrics::SPACE_4)
                .child(Self::section_title(strings::PACKAGES, cx))
                .child(
                    wrap().children(
                        packages
                            .iter()
                            .map(|package| self.render_package(package, card_width, window, cx)),
                    ),
                )
        });
        let snippet_section = (!snippets.is_empty()).then(|| {
            v_flex()
                .id("snippet-list")
                .test_support()
                .w_full()
                .child(Self::section_title(strings::SNIPPETS, cx))
                .child(
                    wrap().children(
                        snippets
                            .iter()
                            .map(|snippet| self.render_snippet(snippet, card_width, window, cx)),
                    ),
                )
        });
        div()
            .id("snippet-grid")
            .test_support()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .px(CONTENT_PADDING)
            .pb(metrics::SPACE_4)
            .child(
                v_flex()
                    .relative()
                    .w_full()
                    .child(measure)
                    .children(package_section)
                    .children(snippet_section),
            )
            .into_any_element()
    }

    // -- Panel ---------------------------------------------------------------------

    fn render_panel(&self, total: f32, over: bool, cx: &mut Context<Self>) -> Option<AnyElement> {
        let form = self.form.clone()?;
        let theme = cx.theme();
        let mode = form.read(cx).mode();
        let can_run = form.read(cx).can_run(cx);
        let screen = cx.entity().downgrade();
        let edited = match mode {
            FormMode::Edit(id) => self.snippet(id).map(|_| id),
            FormMode::New => None,
        };
        let title = match mode {
            FormMode::New => strings::PANEL_NEW,
            FormMode::Edit(_) => strings::PANEL_EDIT,
        };
        let mut header = DetailsHeader::new(title).on_collapse({
            let screen = screen.clone();
            move |_, window, cx| {
                let _ = screen.update(cx, |screen, cx| screen.leave_panel(window, cx));
            }
        });
        if form.read(cx).is_dirty(cx) {
            header = header.note(strings::NOT_SAVED);
        }
        if let Some(id) = edited {
            let screen = screen.clone();
            header = header.menu(move |menu, _, _| snippet_menu(menu, screen.clone(), id));
        }
        let danger = theme.danger;
        let danger_hover = theme.danger_hover;
        let hover = theme.secondary_hover;
        let pressed = theme.secondary_active;
        let remove = edited.map(|id| {
            div()
                .id("snippet-panel-remove")
                .test_support()
                .tab_index(0)
                .focus_ring_visible(theme.colors.list, cx)
                .flex()
                .flex_none()
                .items_center()
                .justify_center()
                .h(metrics::HEIGHT_CONTROL)
                .px(metrics::SPACE_3)
                .rounded(metrics::RADIUS_CONTROL)
                .border_1()
                .border_color(theme.input)
                .text_size(metrics::FONT_SIZE_BODY)
                .font_weight(FontWeight::MEDIUM)
                .text_color(danger)
                .cursor_default()
                .hover(move |style| style.bg(hover).text_color(danger_hover))
                .active(move |style| style.bg(pressed).text_color(danger_hover))
                .child(strings::REMOVE)
                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                    this.confirm_remove(id, window, cx)
                }))
        });
        let save = ControlButton::new(
            if edited.is_some() {
                "snippet-save"
            } else {
                "snippet-create"
            },
            ControlKind::Neutral,
        )
        .label(if edited.is_some() {
            strings::SAVE
        } else {
            strings::CREATE
        })
        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
            this.save(false, window, cx);
        }));
        // Changes not saved: Save and run first (9.3: disabled without a
        // script or a target; D68).
        let run = ControlButton::new("snippet-run", ControlKind::Accent)
            .icon(Lucide::Play)
            .label(strings::RUN)
            .disabled(!can_run)
            .on_click(
                cx.listener(|this, _: &ClickEvent, window, cx| this.run_from_panel(window, cx)),
            );
        let room = px(total) - details::MAIN_AREA_MIN;
        let handle_hover = theme.drag_border;
        // Too narrow for the panel beside the list (175 and 200 %, D100): it
        // covers the list's right, with a shadow.
        let (frame, width) = if over {
            (
                details::over_area(div().id("snippet-panel"), cx),
                details::overlay_width(self.panel_width, px(total)),
            )
        } else {
            (
                div().id("snippet-panel").relative(),
                details::clamp_width(self.panel_width)
                    .min(room.max(metrics::at_100(metrics::WIDTH_DETAILS_MIN))),
            )
        };
        Some(
            frame
                .test_support()
                .w(metrics::rems_of(width))
                .flex_none()
                .h_full()
                .border_l_1()
                .border_color(theme.border)
                .bg(theme.colors.list)
                .child(
                    v_flex()
                        .size_full()
                        .child(header)
                        .child(
                            v_flex()
                                .id("snippet-panel-body")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .px(metrics::SPACE_4)
                                .pb(metrics::SPACE_4)
                                .child(form),
                        )
                        .child(
                            h_flex()
                                .id("snippet-panel-footer")
                                .test_support()
                                .flex_none()
                                .w_full()
                                .gap(metrics::SPACE_2)
                                .px(metrics::SPACE_4)
                                .py(metrics::SPACE_3)
                                .border_t_1()
                                .border_color(theme.border)
                                .children(remove)
                                .child(div().flex_1())
                                .child(save)
                                .child(run),
                        ),
                )
                .child(
                    div()
                        .id("snippet-panel-resize")
                        .test_support()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(metrics::rpx(-3.))
                        .w(metrics::rpx(6.))
                        .cursor_col_resize()
                        .hover(move |style| style.bg(handle_hover.opacity(0.5)))
                        .on_drag(ResizeDetails, |_, _, _, cx| cx.new(|_| ResizePreview)),
                )
                .into_any_element(),
        )
    }

    /// Arrows move over the folders, then the snippets; Enter opens the
    /// folder or runs the selected snippet; Delete removes it; Esc closes
    /// the panel, then leaves the package.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return;
        }
        let packages = self.shown_package_ids();
        let ids: Vec<Uuid> = packages.iter().copied().chain(self.shown_ids()).collect();
        let current = self
            .package_cursor
            .or(self.selected)
            .and_then(|id| ids.iter().position(|shown| *shown == id));
        let arrow = match keystroke.key.as_str() {
            "enter" if self.package_cursor.is_some() => {
                cx.stop_propagation();
                self.enter_package(self.package_cursor, cx);
                return;
            }
            "enter" => {
                if let Some(id) = self.selected {
                    cx.stop_propagation();
                    self.run(id, window, cx);
                }
                return;
            }
            "delete" => {
                if let Some(id) = self.selected {
                    cx.stop_propagation();
                    self.confirm_remove(id, window, cx);
                }
                return;
            }
            "escape" if self.form.is_some() => {
                cx.stop_propagation();
                self.leave_panel(window, cx);
                return;
            }
            "escape" if self.package.is_some() => {
                cx.stop_propagation();
                self.enter_package(None, cx);
                return;
            }
            key => match crate::hosts::layout::Arrow::from_key(key) {
                Some(arrow) => arrow,
                None => return,
            },
        };
        let Some(next) = crate::hosts::sections_target(
            packages.len(),
            ids.len() - packages.len(),
            current,
            self.columns,
            arrow,
        ) else {
            return;
        };
        cx.stop_propagation();
        if next < packages.len() {
            // A folder: the cursor stands on it, the snippet panel closes.
            self.package_cursor = Some(ids[next]);
            self.select(None, window, cx);
            cx.notify();
        } else {
            self.package_cursor = None;
            self.select(Some(ids[next]), window, cx);
        }
    }
}

impl Render for SnippetsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let total = self.measured_width.unwrap_or_else(|| {
            (metrics::unscaled(window.viewport_size().width, cx)
                - metrics::at_100(metrics::WIDTH_SIDEBAR))
            .as_f32()
        });
        let screen = cx.entity().downgrade();
        let measure = canvas(
            move |bounds, _, cx| {
                let width = metrics::unscaled(bounds.size.width, cx).as_f32();
                let _ = screen.update(cx, |screen, cx| {
                    if screen
                        .measured_width
                        .is_none_or(|measured| (measured - width).abs() >= 0.5)
                    {
                        screen.measured_width = Some(width);
                        cx.notify();
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
        let history = self.history.clone();
        let content = if history.is_some() {
            div().into_any_element()
        } else if self.shown.is_empty() && self.shown_packages.is_empty() {
            self.render_empty(cx)
        } else {
            self.render_grid(window, cx)
        };
        let main = v_flex()
            .id("snippet-main")
            .test_support()
            .flex_1()
            .min_w_0()
            .h_full()
            .child(
                div()
                    .flex_none()
                    .px(CONTENT_PADDING)
                    .pt(metrics::SPACE_4)
                    .pb(metrics::SPACE_3)
                    .child(self.render_action_bar(window, cx)),
            )
            .children(self.render_breadcrumb(cx))
            .when(history.is_none(), |this| {
                this.child(self.duplicates_hint.clone())
            })
            // Shell History has its own fields: the grid's focus handling
            // (which takes the focus on a click) is not around it.
            .when_some(history.clone(), |this, history| {
                this.child(div().flex().flex_col().flex_1().min_h_0().child(history))
            })
            .when(history.is_none(), |this| {
                this.child(
                    div()
                        .id("snippet-content")
                        .test_support()
                        .key_context("SnippetsGrid")
                        .track_focus(&self.focus)
                        .relative()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .w_full()
                        .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                            this.on_key_down(event, window, cx)
                        }))
                        .on_mouse_down(
                            MouseButton::Left,
                            cx.listener(|this, _, window, cx| this.focus.focus(window, cx)),
                        )
                        .child(content),
                )
            });
        let panel = self.render_panel(total, details::overlays_in(window), cx);
        h_flex()
            .id("snippets-screen")
            .test_support()
            .relative()
            // A panel over the list (D100) closes with Esc wherever the focus is.
            .when(
                self.form.is_some() && details::overlays_in(window),
                |screen| {
                    screen.on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                        if event.keystroke.key == "escape" && !event.keystroke.modifiers.modified()
                        {
                            cx.stop_propagation();
                            this.leave_panel(window, cx);
                        }
                    }))
                },
            )
            .size_full()
            .bg(cx.theme().colors.list)
            .text_color(cx.theme().foreground)
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<ResizeDetails>, _, cx| {
                    let width = details::width_for_drag(
                        metrics::unscaled(event.bounds.right(), cx),
                        metrics::unscaled(event.event.position.x, cx),
                    );
                    this.set_panel_width(width, cx);
                }),
            )
            .child(measure)
            .child(main)
            .children(panel)
    }
}
