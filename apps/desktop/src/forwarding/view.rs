//! Rendering of the Port Forwarding screen: the action bar, the card grid,
//! the empty states, the rule panel and the removal confirmation. Cards and
//! the panel follow the Hosts and Known Hosts screens (design-system 6).

use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, DragMoveEvent, Focusable as _,
    FontWeight, Hsla, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, canvas, div, prelude::FluentBuilder as _, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName, Sizable as _, WindowExt as _, h_flex, v_flex,
};
use shellpp_core::service::dto::TagColor;
use shellpp_core::service::snippets::{ForwardType, ForwardingRuleView};
use uuid::Uuid;

use super::{
    FormMode, ForwardingScreen, StatusTone, card_id, card_title, status_id, status_text, strings,
    type_letter,
};
use crate::hosts::controls::{self, ControlButton, ControlKind, ICON_SIZE};
use crate::hosts::layout as grid;
use crate::shell::details::{self, DetailsHeader, ResizeDetails, ResizePreview};
use crate::theme::{self, metrics, palette};

/// Horizontal padding of the content (`--space-5`), as on the Hosts screen.
const CONTENT_PADDING: Pixels = metrics::SPACE_5;
/// A card holds three lines: title, summary, status.
const CARD_HEIGHT: Pixels = px(80.);
const SEARCH_MIN_WIDTH: Pixels = px(160.);
const SEARCH_MAX_WIDTH: Pixels = px(320.);
const DIALOG_WIDTH: Pixels = px(420.);

/// The tile color of a rule type.
fn type_color(kind: ForwardType) -> TagColor {
    match kind {
        ForwardType::Local => TagColor::Ocean,
        ForwardType::Remote => TagColor::Plum,
        ForwardType::Dynamic => TagColor::Jade,
    }
}

/// The square badge with the type letter (UI 8.2).
fn badge(kind: ForwardType, size: Pixels) -> impl IntoElement {
    div()
        .flex_none()
        .size(size)
        .rounded(metrics::RADIUS_TILE)
        .bg(palette::tag_color(type_color(kind)).hsla())
        .flex()
        .items_center()
        .justify_center()
        .text_size(metrics::FONT_SIZE_BODY)
        .font_weight(FontWeight::BOLD)
        .text_color(palette::TEXT_ON_TAG.hsla())
        .child(type_letter(kind))
}

/// A theme color (the literal scan of `theme::literals` reads `Hsla {` as
/// a struct literal).
type Color = Hsla;

fn tone_color(tone: StatusTone, cx: &App) -> Color {
    let theme = cx.theme();
    match tone {
        StatusTone::Stopped => theme.muted_foreground,
        StatusTone::Busy => theme.warning,
        StatusTone::Running => theme.success,
        StatusTone::Error => theme.danger,
    }
}

/// The status indicator: a dot and the words.
/// A card cuts it to one line; the panel shows all of it (`wrap`).
fn status_line(
    id: SharedString,
    tone: StatusTone,
    text: String,
    wrap: bool,
    cx: &App,
) -> impl IntoElement {
    let color = tone_color(tone, cx);
    h_flex()
        .id(id)
        .test_support()
        .min_w_0()
        .gap(metrics::SPACE_2)
        .when(wrap, |this| this.items_start())
        .when(!wrap, |this| this.items_center())
        .child(
            div()
                .flex_none()
                .when(wrap, |this| this.mt(px(5.)))
                .size(px(8.))
                .rounded_full()
                .bg(color),
        )
        .child(
            div()
                .min_w_0()
                .when(!wrap, |this| this.truncate())
                .text_size(metrics::FONT_SIZE_CAPTION)
                .text_color(if tone == StatusTone::Stopped {
                    cx.theme().muted_foreground
                } else {
                    color
                })
                .child(text),
        )
}

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

/// The card menu (8.2): Start/Stop, Edit, Duplicate, Remove.
fn card_menu(
    menu: PopupMenu,
    screen: gpui::WeakEntity<ForwardingScreen>,
    id: Uuid,
    started: bool,
) -> PopupMenu {
    let toggle = screen.clone();
    let edit = screen.clone();
    let duplicate = screen.clone();
    let (toggle_id, toggle_label) = if started {
        ("fwd-menu-stop", strings::STOP)
    } else {
        ("fwd-menu-start", strings::START)
    };
    menu.min_w(px(200.))
        .item(
            PopupMenuItem::element(move |_, cx| menu_item(toggle_id, toggle_label, false, cx))
                .on_click(move |_, window, cx| {
                    let _ = toggle.update(cx, |screen, cx| screen.toggle(id, window, cx));
                }),
        )
        .item(
            PopupMenuItem::element(|_, cx| menu_item("fwd-menu-edit", strings::EDIT, false, cx))
                .on_click(move |_, window, cx| {
                    let _ = edit.update(cx, |screen, cx| screen.select(Some(id), window, cx));
                }),
        )
        .item(
            PopupMenuItem::element(|_, cx| {
                menu_item("fwd-menu-duplicate", strings::DUPLICATE, false, cx)
            })
            .on_click(move |_, window, cx| {
                let _ = duplicate.update(cx, |screen, cx| screen.duplicate(id, window, cx));
            }),
        )
        .separator()
        .item(
            PopupMenuItem::element(|_, cx| menu_item("fwd-menu-remove", strings::REMOVE, true, cx))
                .on_click(move |_, window, cx| {
                    let _ = screen.update(cx, |screen, cx| screen.confirm_remove(id, window, cx));
                }),
        )
}

/// "Remove forwarding rule?" with Cancel and Remove (`--danger`).
pub(super) fn open_remove_confirmation(
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
            .w(DIALOG_WIDTH)
            .bg(cx.theme().popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(strings::REMOVE_TITLE),
            )
            .child(
                div()
                    .id("fwd-remove-dialog")
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
                        ControlButton::new("fwd-remove-cancel", ControlKind::Neutral)
                            .label(strings::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("fwd-remove-confirm", ControlKind::Danger)
                            .label(strings::REMOVE)
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                confirm(window, cx);
                            }),
                    ),
            )
    });
}

impl ForwardingScreen {
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
            .id("fwd-search-field")
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
                    .with_size(ICON_SIZE)
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
                            .id("fwd-search")
                            .appearance(false)
                            .cleanable(true)
                            .pl_0()
                            .text_size(metrics::FONT_SIZE_BODY),
                    ),
            )
            .when(focused, |this| {
                this.child(controls::focus_ring(metrics::RADIUS_CONTROL, px(1.), cx))
            })
    }

    /// Split New forwarding (▾: Local, Remote, Dynamic) on the left, search
    /// on the right (8.1).
    fn render_action_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let screen = cx.entity().downgrade();
        let new_rule = ControlButton::new("fwd-new", ControlKind::Neutral)
            .icon(Lucide::Plus)
            .label(strings::NEW_RULE)
            .rounded_sides(true, false)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.new_rule(ForwardType::Local, window, cx)
            }));
        let new_menu = ControlButton::new("fwd-new-menu", ControlKind::Neutral)
            .icon(IconName::ChevronDown)
            .tooltip(strings::NEW_RULE_MENU)
            .rounded_sides(false, true)
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.min_w(px(220.));
                for (id, label, kind) in [
                    ("fwd-new-local", strings::NEW_LOCAL, ForwardType::Local),
                    ("fwd-new-remote", strings::NEW_REMOTE, ForwardType::Remote),
                    (
                        "fwd-new-dynamic",
                        strings::NEW_DYNAMIC,
                        ForwardType::Dynamic,
                    ),
                ] {
                    let screen = screen.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, cx| menu_item(id, label, false, cx))
                            .on_click(move |_, window, cx| {
                                let _ = screen
                                    .update(cx, |screen, cx| screen.new_rule(kind, window, cx));
                            }),
                    );
                }
                menu
            });
        h_flex()
            .id("fwd-action-bar")
            .test_support()
            .w_full()
            .flex_wrap()
            .items_center()
            .gap(metrics::SPACE_2)
            .child(
                h_flex()
                    .flex_none()
                    .child(new_rule)
                    .child(div().w(px(1.)).h(metrics::HEIGHT_CONTROL).bg(theme.border))
                    .child(new_menu),
            )
            .child(
                h_flex()
                    .flex_1()
                    .min_w_0()
                    .justify_end()
                    .child(self.render_search(window, cx)),
            )
    }

    // -- Content -------------------------------------------------------------------

    fn render_card(
        &self,
        rule: &ForwardingRuleView,
        width: f32,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let id = rule.record.record_id;
        let theme = cx.theme();
        let selected = self.selected == Some(id);
        let keyboard_cursor =
            selected && self.focus.is_focused(window) && window.last_input_was_keyboard();
        let hover = theme.secondary_hover;
        let pressed = theme.secondary_active;
        let (tone, text) = if self.is_starting(id) {
            (StatusTone::Busy, strings::STARTING.to_owned())
        } else {
            status_text(self.status(id))
        };
        let screen = cx.entity().downgrade();
        div()
            .id(card_id(id))
            .test_support()
            .relative()
            .flex_none()
            .w(px(width))
            .h(CARD_HEIGHT)
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
            .child(badge(rule.kind, metrics::HOST_TILE_SIZE))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap(px(2.))
                    .child(
                        div()
                            .text_size(metrics::FONT_SIZE_BODY)
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme.foreground)
                            .truncate()
                            .child(card_title(rule)),
                    )
                    .child(
                        div()
                            .text_size(metrics::FONT_SIZE_CAPTION)
                            .text_color(theme.muted_foreground)
                            .truncate()
                            .child(rule.summary.clone()),
                    )
                    .child(status_line(status_id(id), tone, text, false, cx)),
            )
            .when(keyboard_cursor, |this| {
                this.child(controls::focus_ring(metrics::RADIUS_CARD, px(1.), cx))
            })
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.focus.focus(window, cx);
                if event.click_count() >= 2 {
                    this.toggle(id, window, cx);
                } else if this.selected != Some(id) {
                    this.select(Some(id), window, cx);
                }
            }))
            .context_menu(move |menu, window, cx| {
                // The menu acts on this card and shows it in the panel.
                let started = screen
                    .update(cx, |screen, cx| {
                        if screen.selected != Some(id) {
                            screen.select(Some(id), window, cx);
                        }
                        screen.is_running(id)
                    })
                    .unwrap_or(false);
                card_menu(menu, screen.clone(), id, started)
            })
            .into_any_element()
    }

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let none = self.items.is_empty();
        let (id, title, body) = if none {
            ("fwd-empty", strings::EMPTY_TITLE, Some(strings::EMPTY_BODY))
        } else {
            ("fwd-no-matches", strings::NO_MATCHES, None)
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
                    .max_w_full()
                    .text_center()
                    .text_size(metrics::FONT_SIZE_HEADING)
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.foreground)
                    .child(title),
            )
            .when_some(body, |this, body| {
                this.child(
                    div()
                        .max_w(px(420.))
                        .text_center()
                        .text_color(theme.muted_foreground)
                        .child(body),
                )
            })
            .when(none && self.form.is_none(), |this| {
                this.child(
                    h_flex().pt(metrics::SPACE_2).child(
                        ControlButton::new("fwd-empty-new", ControlKind::Accent)
                            .icon(Lucide::Plus)
                            .label(strings::NEW_RULE)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.new_rule(ForwardType::Local, window, cx)
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
                let width = bounds.size.width.as_f32();
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
        let rules: Vec<ForwardingRuleView> = self
            .shown
            .iter()
            .map(|index| self.items[*index].clone())
            .collect();
        div()
            .id("fwd-grid")
            .test_support()
            .flex_1()
            .min_h_0()
            .w_full()
            .overflow_y_scroll()
            .px(CONTENT_PADDING)
            .pb(metrics::SPACE_4)
            .child(
                div()
                    .relative()
                    .w_full()
                    .flex()
                    .flex_wrap()
                    .gap(metrics::HOST_CARD_GAP)
                    .child(measure)
                    .children(
                        rules
                            .iter()
                            .map(|rule| self.render_card(rule, card_width, window, cx)),
                    ),
            )
            .into_any_element()
    }

    // -- Panel ---------------------------------------------------------------------

    fn render_panel(&self, total: f32, cx: &mut Context<Self>) -> Option<AnyElement> {
        let form = self.form.clone()?;
        let theme = cx.theme();
        let mode = form.read(cx).mode();
        let screen = cx.entity().downgrade();
        let title = match mode {
            FormMode::New => strings::PANEL_NEW,
            FormMode::Edit(_) => strings::PANEL_EDIT,
        };
        let edited = match mode {
            FormMode::Edit(id) => self.rule(id).map(|_| id),
            FormMode::New => None,
        };
        let mut header = DetailsHeader::new(title).on_collapse({
            let screen = screen.clone();
            move |_, _, cx| {
                let _ = screen.update(cx, |screen, cx| screen.close_panel(cx));
            }
        });
        if let Some(id) = edited {
            let started = self.is_running(id);
            let screen = screen.clone();
            header = header.menu(move |menu, _, _| card_menu(menu, screen.clone(), id, started));
        }
        // The rule's status with Start or Stop.
        let status = edited.map(|id| {
            let started = self.is_running(id);
            let (tone, text) = if self.is_starting(id) {
                (StatusTone::Busy, strings::STARTING.to_owned())
            } else {
                status_text(self.status(id))
            };
            let button = if started {
                ControlButton::new("fwd-panel-stop", ControlKind::Neutral)
                    .icon(Lucide::Square)
                    .label(strings::STOP)
            } else {
                ControlButton::new("fwd-panel-start", ControlKind::Neutral)
                    .icon(Lucide::Play)
                    .label(strings::START)
            };
            crate::panels::widgets::block("fwd-panel-status", None, cx).child(
                h_flex()
                    .w_full()
                    .gap(metrics::SPACE_3)
                    .items_center()
                    .child(div().flex_1().min_w_0().child(status_line(
                        "fwd-panel-status-line".into(),
                        tone,
                        text,
                        true,
                        cx,
                    )))
                    .child(button.on_click(cx.listener(
                        move |this, _: &ClickEvent, window, cx| this.toggle(id, window, cx),
                    ))),
            )
        });
        let danger = theme.danger;
        let danger_hover = theme.danger_hover;
        let hover = theme.secondary_hover;
        let pressed = theme.secondary_active;
        let save =
            match mode {
                FormMode::New => vec![
                    ControlButton::new("fwd-create", ControlKind::Neutral)
                        .label(strings::CREATE)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.save(false, window, cx)
                        })),
                    ControlButton::new("fwd-create-start", ControlKind::Accent)
                        .label(strings::CREATE_AND_START)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.save(true, window, cx)
                        })),
                ],
                FormMode::Edit(_) => vec![
                    ControlButton::new("fwd-save", ControlKind::Accent)
                        .label(strings::SAVE)
                        .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                            this.save(false, window, cx)
                        })),
                ],
            };
        let remove = edited.map(|id| {
            div()
                .id("fwd-panel-remove")
                .test_support()
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
        let room = px(total) - details::MAIN_AREA_MIN;
        let width =
            details::clamp_width(self.panel_width).min(room.max(metrics::WIDTH_DETAILS_MIN));
        let handle_hover = theme.drag_border;
        Some(
            div()
                .id("fwd-panel")
                .test_support()
                .relative()
                .w(width)
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
                                .id("fwd-panel-body")
                                .flex_1()
                                .min_h_0()
                                .overflow_y_scroll()
                                .px(metrics::SPACE_4)
                                .pb(metrics::SPACE_4)
                                .gap(metrics::SPACE_3)
                                .children(status)
                                .child(form),
                        )
                        // The actions stay in view below the long form.
                        .child(
                            h_flex()
                                .id("fwd-panel-footer")
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
                                .children(save),
                        ),
                )
                .child(
                    div()
                        .id("fwd-panel-resize")
                        .test_support()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(-3.))
                        .w(px(6.))
                        .cursor_col_resize()
                        .hover(move |style| style.bg(handle_hover.opacity(0.5)))
                        .on_drag(ResizeDetails, |_, _, _, cx| cx.new(|_| ResizePreview)),
                )
                .into_any_element(),
        )
    }

    /// Arrows move the selection; Enter starts or stops the selected rule;
    /// Delete removes it; Esc closes the panel.
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return;
        }
        let ids = self.shown_ids();
        let current = self
            .selected
            .and_then(|id| ids.iter().position(|shown| *shown == id));
        let columns = self.columns.max(1) as isize;
        let step = match keystroke.key.as_str() {
            "left" => -1,
            "right" => 1,
            "up" => -columns,
            "down" => columns,
            "enter" => {
                if let Some(id) = self.selected {
                    cx.stop_propagation();
                    self.toggle(id, window, cx);
                }
                return;
            }
            "delete" | "backspace" => {
                if let Some(id) = self.selected {
                    cx.stop_propagation();
                    self.confirm_remove(id, window, cx);
                }
                return;
            }
            "escape" if self.form.is_some() => {
                cx.stop_propagation();
                self.close_panel(cx);
                return;
            }
            _ => return,
        };
        if ids.is_empty() {
            return;
        }
        cx.stop_propagation();
        let next = match current {
            None => 0,
            Some(index) => (index as isize + step).clamp(0, ids.len() as isize - 1) as usize,
        };
        self.select(Some(ids[next]), window, cx);
    }
}

impl Render for ForwardingScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Shown: the statuses are read again (rules that autostarted
        // elsewhere show up), and polled while any rule runs.
        self.read_statuses(cx);
        self.poll_if_needed(cx);
        let total = self
            .measured_width
            .unwrap_or_else(|| (window.viewport_size().width - metrics::WIDTH_SIDEBAR).as_f32());
        let screen = cx.entity().downgrade();
        let measure = canvas(
            move |bounds, _, cx| {
                let width = bounds.size.width.as_f32();
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
        let content = if self.shown.is_empty() {
            self.render_empty(cx)
        } else {
            self.render_grid(window, cx)
        };
        let main = v_flex()
            .id("fwd-main")
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
            .child(
                div()
                    .id("fwd-content")
                    .test_support()
                    .key_context("ForwardingGrid")
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
            );
        let panel = self.render_panel(total, cx);
        h_flex()
            .id("forwarding-screen")
            .test_support()
            .relative()
            .size_full()
            .bg(cx.theme().colors.list)
            .text_color(cx.theme().foreground)
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<ResizeDetails>, _, cx| {
                    let width =
                        details::width_for_drag(event.bounds.right(), event.event.position.x);
                    this.set_panel_width(width, cx);
                }),
            )
            .child(measure)
            .child(main)
            .children(panel)
    }
}
