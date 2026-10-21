//! Rendering of the Known Hosts screen: the action bar, the card grid or the
//! list, the empty states, the details panel and the removal confirmation.
//! Sizes and colors come from docs/design-system.md through
//! `crate::theme` (cards and rows as on the Hosts screen, the panel as the
//! details panel of UI 1.3).

use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, DragMoveEvent, Focusable as _,
    FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, canvas, div, prelude::FluentBuilder as _, px, size,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    WindowExt as _, h_flex, v_flex, v_virtual_list,
};
use shellpp_core::service::dto::{KnownHostView, TagColor};
use uuid::Uuid;

use super::{
    KnownHostsLayout, KnownHostsScreen, KnownHostsSort, card_id, card_subtitle, card_title,
    date_text, hashed_title_id, panel_host_id, port_text, revoked_badge_id, strings,
};
use crate::hosts::controls::{self, ControlButton, ControlKind, ICON_SIZE};
use crate::hosts::layout as grid;
use crate::shell::details::{self, DetailsHeader, ResizeDetails, ResizePreview};
use crate::theme::{self, metrics, palette};

/// Horizontal padding of the content (`--space-5`), as on the Hosts screen.
const CONTENT_PADDING: Pixels = metrics::SPACE_5;
/// The tile of a list row (the 36 px card tile does not fit a 44 px row).
const LIST_TILE_SIZE: Pixels = px(28.);
/// The search field of the action bar: it takes the room left, between
/// these widths (decision of stage 2b, on the 4 px grid).
const SEARCH_MIN_WIDTH: Pixels = px(160.);
const SEARCH_MAX_WIDTH: Pixels = px(320.);
/// Fixed list columns (decision of stage 2b): key type and date added.
const KEY_TYPE_COLUMN: Pixels = px(200.);
const ADDED_COLUMN: Pixels = px(148.);
/// Width of the small dialogs (as the Hosts screen's).
const DIALOG_WIDTH: Pixels = px(420.);

/// The element id of a card's title: "Hashed host" titles have their own.
fn title_id(view: &KnownHostView) -> SharedString {
    if view.hashed {
        hashed_title_id(view.record_id)
    } else {
        format!("known-host-title-{}", view.record_id).into()
    }
}

/// The fingerprint tile (`--tag-graphite`, white icon), as host tiles.
fn tile(tile_size: Pixels) -> impl IntoElement {
    div()
        .flex_none()
        .size(tile_size)
        .rounded(metrics::RADIUS_TILE)
        .bg(palette::tag_color(TagColor::Graphite).hsla())
        .flex()
        .items_center()
        .justify_center()
        .child(
            Icon::new(Lucide::FingerprintPattern)
                .with_size(ICON_SIZE)
                .text_color(palette::TEXT_ON_TAG.hsla()),
        )
}

/// The "revoked" badge: `--danger` text and border, no fill (as a chip,
/// design-system 3.4, in the danger color of 2).
fn revoked_badge(id: impl Into<gpui::ElementId>, cx: &App) -> impl IntoElement {
    let danger = cx.theme().danger;
    div()
        .id(id.into())
        .test_support()
        .flex_none()
        .flex()
        .items_center()
        .h(px(20.))
        .px(metrics::SPACE_2)
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .border_color(danger)
        .text_color(danger)
        .text_size(metrics::FONT_SIZE_CAPTION)
        .whitespace_nowrap()
        .child(strings::REVOKED)
}

/// A plain menu item with a stable id; `danger` items use `--danger`
/// (`--danger-hover` under the pointer, design-system 6).
fn menu_item(id: &'static str, label: &'static str, danger: bool, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let hover = theme.danger_hover;
    div()
        .id(id)
        .test_support()
        .flex_1()
        .text_color(if danger {
            theme.danger
        } else {
            theme.foreground
        })
        .when(danger, |this| {
            this.mx(-metrics::SPACE_2)
                .px(metrics::SPACE_2)
                .my(-metrics::SPACE_1)
                .py(metrics::SPACE_1)
                .hover(move |style| style.text_color(hover))
        })
        .child(label)
        .into_any_element()
}

/// The card menu (10.1): Copy fingerprint, Remove.
fn card_menu(menu: PopupMenu, screen: gpui::WeakEntity<KnownHostsScreen>, id: Uuid) -> PopupMenu {
    let copy = screen.clone();
    menu.min_w(px(200.))
        .item(
            PopupMenuItem::element(|_, cx| {
                menu_item(
                    "kh-menu-copy-fingerprint",
                    strings::COPY_FINGERPRINT,
                    false,
                    cx,
                )
            })
            .on_click(move |_, window, cx| {
                let _ = copy.update(cx, |screen, cx| screen.copy_fingerprint(id, window, cx));
            }),
        )
        .separator()
        .item(
            PopupMenuItem::element(|_, cx| menu_item("kh-menu-remove", strings::REMOVE, true, cx))
                .on_click(move |_, window, cx| {
                    let _ = screen.update(cx, |screen, cx| screen.confirm_remove(id, window, cx));
                }),
        )
}

/// "Remove known host?" with Cancel and Remove (`--danger`). Remove or Enter
/// confirms; Esc and Cancel close without removing.
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
                    .id("kh-remove-dialog")
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
                        ControlButton::new("kh-remove-cancel", ControlKind::Neutral)
                            .label(strings::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("kh-remove-confirm", ControlKind::Danger)
                            .label(strings::REMOVE)
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                confirm(window, cx);
                            }),
                    ),
            )
    });
}

impl KnownHostsScreen {
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
            .id("kh-search-field")
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
                            .id("kh-search")
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

    fn render_view_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let button = |id: &'static str, icon: Lucide, tooltip: &'static str, mode| {
            ControlButton::new(id, ControlKind::Ghost)
                .icon(icon)
                .tooltip(tooltip)
                .selected(self.layout == mode)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_layout(mode, cx)))
        };
        h_flex()
            .id("kh-view-toggle")
            .test_support()
            .flex_none()
            .gap(metrics::SPACE_1)
            .child(button(
                "kh-view-tiles",
                Lucide::LayoutGrid,
                strings::VIEW_TILES,
                KnownHostsLayout::Tiles,
            ))
            .child(button(
                "kh-view-list",
                Lucide::List,
                strings::VIEW_LIST,
                KnownHostsLayout::List,
            ))
    }

    fn render_sort_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = cx.entity().downgrade();
        let current = self.sort;
        ControlButton::new("kh-sort", ControlKind::Ghost)
            .icon(Lucide::ArrowUpDown)
            .tooltip(strings::SORT)
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.min_w(px(200.));
                for sort in KnownHostsSort::ALL {
                    let (id, label) = match sort {
                        KnownHostsSort::Address => ("kh-sort-address", strings::SORT_ADDRESS),
                        KnownHostsSort::Added => ("kh-sort-added", strings::SORT_ADDED),
                        KnownHostsSort::KeyType => ("kh-sort-key-type", strings::SORT_KEY_TYPE),
                    };
                    let screen = screen.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, cx| menu_item(id, label, false, cx))
                            .checked(sort == current)
                            .on_click(move |_, _, cx| {
                                let _ = screen.update(cx, |screen, cx| screen.set_sort(sort, cx));
                            }),
                    );
                }
                menu
            })
    }

    /// Import and Export on the left; search, view toggle and sort on the
    /// right (10.1). In a narrow area the right group wraps below.
    fn render_action_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let empty = self.items.is_empty();
        // One group: both as wide as the wider (design-system 6).
        let width = controls::group_width(window, &[strings::IMPORT, strings::EXPORT]);
        let import = ControlButton::new("kh-import", ControlKind::Neutral)
            .icon(Lucide::Import)
            .label(strings::IMPORT)
            .min_w(width)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_import(window, cx)));
        let export = ControlButton::new("kh-export", ControlKind::Neutral)
            .icon(Lucide::Upload)
            .label(strings::EXPORT)
            .min_w(width)
            .disabled(empty)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.export(window, cx)));
        h_flex()
            .id("kh-action-bar")
            .test_support()
            .w_full()
            .flex_wrap()
            .items_center()
            .gap(metrics::SPACE_2)
            .child(
                h_flex()
                    .flex_none()
                    .gap(metrics::SPACE_2)
                    .child(import)
                    .child(export),
            )
            // No spacer that grows: the right group takes all the room left,
            // so the search field widens up to its maximum before the
            // placeholder is cut.
            .child(
                h_flex()
                    .id("kh-action-bar-right")
                    .test_support()
                    .flex_1()
                    .min_w_0()
                    .justify_end()
                    .items_center()
                    .gap(metrics::SPACE_2)
                    .child(self.render_search(window, cx))
                    .child(self.render_view_toggle(cx))
                    .child(self.render_sort_menu(cx)),
            )
    }

    // -- Content -------------------------------------------------------------------

    /// The width of the grid area (the screen without its panel).
    fn grid_width(&self, window: &Window) -> f32 {
        let total = self
            .measured_width
            .unwrap_or_else(|| (window.viewport_size().width - metrics::WIDTH_SIDEBAR).as_f32());
        let panel = if self.has_panel() {
            self.fitted_panel_width(total).as_f32()
        } else {
            0.
        };
        (total - panel - CONTENT_PADDING.as_f32() * 2.).max(1.)
    }

    /// The panel on display: the chosen width, narrowed so the grid keeps
    /// 360 px, never below 320 px (design-system 6).
    fn fitted_panel_width(&self, total: f32) -> Pixels {
        let room = px(total) - details::MAIN_AREA_MIN;
        details::clamp_width(self.panel_width).min(room.max(metrics::WIDTH_DETAILS_MIN))
    }

    fn rebuild_rows(&mut self, width: f32) {
        let columns = match self.layout {
            KnownHostsLayout::Tiles => grid::columns_for(width),
            KnownHostsLayout::List => 1,
        };
        let key = (columns, self.shown.len(), width as i32);
        if key == self.layout_key {
            return;
        }
        self.layout_key = key;
        self.columns = columns;
        self.card_width = match self.layout {
            KnownHostsLayout::Tiles => grid::card_width(width, columns),
            KnownHostsLayout::List => width,
        };
        let height = match self.layout {
            KnownHostsLayout::Tiles => {
                metrics::HEIGHT_HOST_CARD.as_f32() + metrics::HOST_CARD_GAP.as_f32()
            }
            KnownHostsLayout::List => metrics::HEIGHT_ROW.as_f32(),
        };
        let rows = self.shown.len().div_ceil(columns.max(1));
        self.row_sizes = Rc::new(vec![size(px(width), px(height)); rows]);
    }

    fn render_texts(&self, view: &KnownHostView, cx: &App) -> AnyElement {
        let theme = cx.theme();
        v_flex()
            .flex_1()
            .min_w_0()
            .gap(px(2.))
            .child(
                div()
                    .id(title_id(view))
                    .test_support()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(theme.foreground)
                    .truncate()
                    .child(card_title(view)),
            )
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(card_subtitle(view)),
            )
            .into_any_element()
    }

    fn render_list_cells(&self, view: &KnownHostView, cx: &App) -> Vec<AnyElement> {
        let theme = cx.theme();
        let id = view.record_id;
        let secondary = |text: String| {
            div()
                .min_w_0()
                .truncate()
                .text_color(theme.muted_foreground)
                .child(text)
        };
        vec![
            h_flex()
                .flex_1()
                .min_w_0()
                .gap(metrics::SPACE_3)
                .items_center()
                .child(tile(LIST_TILE_SIZE))
                .child(
                    div()
                        .id(title_id(view))
                        .test_support()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.foreground)
                        .child(card_title(view)),
                )
                .when(view.revoked, |this| {
                    this.child(revoked_badge(revoked_badge_id(id), cx))
                })
                .into_any_element(),
            secondary(view.key_type.clone())
                .flex_none()
                .w(KEY_TYPE_COLUMN)
                .into_any_element(),
            crate::panels::widgets::fingerprint_line(view.fingerprint.clone())
                .flex_1()
                .text_color(theme.muted_foreground)
                .into_any_element(),
            secondary(date_text(&view.created_at))
                .id(SharedString::from(format!("kh-added-{id}")))
                .when_some(
                    crate::dates::timestamp_tooltip(&view.created_at),
                    |this, utc| this.tooltip(crate::dates::utc_tooltip(utc)),
                )
                .flex_none()
                .w(ADDED_COLUMN)
                .into_any_element(),
        ]
    }

    fn render_card(&self, index: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let view = &self.items[index];
        let id = view.record_id;
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let selected = self.selected == Some(id);
        let keyboard_cursor =
            selected && self.focus.is_focused(window) && window.last_input_was_keyboard();
        let hover = theme.secondary_hover;
        let pressed = theme.secondary_active;
        let frame = div()
            .id(card_id(id))
            .test_support()
            .relative()
            .flex()
            .items_center()
            .gap(metrics::SPACE_3)
            .px(metrics::SPACE_3)
            .cursor_default();
        let (frame, radius) = match self.layout {
            KnownHostsLayout::Tiles => (
                frame
                    .flex_none()
                    .w(px(self.card_width))
                    .h(metrics::HEIGHT_HOST_CARD)
                    .rounded(metrics::RADIUS_CARD)
                    .bg(theme.secondary)
                    .border_1()
                    .border_color(if selected {
                        theme.primary
                    } else {
                        theme.border
                    })
                    .hover(move |style| style.bg(hover))
                    .active(move |style| style.bg(pressed))
                    .child(tile(metrics::HOST_TILE_SIZE))
                    .child(self.render_texts(view, cx))
                    .when(view.revoked, |this| {
                        this.child(revoked_badge(revoked_badge_id(id), cx))
                    }),
                metrics::RADIUS_CARD,
            ),
            KnownHostsLayout::List => (
                frame
                    .w_full()
                    .h(metrics::HEIGHT_ROW)
                    .border_b_1()
                    .border_color(theme.border)
                    .text_size(metrics::FONT_SIZE_DENSE)
                    .map(|this| {
                        if selected {
                            this.bg(tokens.accent_subtle.hsla())
                        } else {
                            this.hover(move |style| style.bg(hover))
                                .active(move |style| style.bg(pressed))
                        }
                    })
                    .children(self.render_list_cells(view, cx)),
                Pixels::ZERO,
            ),
        };
        let screen = cx.entity().downgrade();
        frame
            .when(keyboard_cursor, |this| {
                this.child(controls::focus_ring(radius, px(1.), cx))
            })
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.focus.focus(window, cx);
                this.select(Some(id), cx);
            }))
            .context_menu(move |menu, _, cx| {
                // The menu acts on this card and shows its details.
                let _ = screen.update(cx, |screen, cx| screen.select(Some(id), cx));
                card_menu(menu, screen.clone(), id)
            })
            .into_any_element()
    }

    fn render_row(&self, row: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let columns = match self.layout {
            KnownHostsLayout::Tiles => self.columns.max(1),
            KnownHostsLayout::List => 1,
        };
        let start = row * columns;
        let end = (start + columns).min(self.shown.len());
        let tiles = self.layout == KnownHostsLayout::Tiles;
        let mut container = h_flex()
            .w_full()
            .when(tiles, |this| {
                this.h(metrics::HEIGHT_HOST_CARD + metrics::HOST_CARD_GAP)
                    .pb(metrics::HOST_CARD_GAP)
                    .gap(metrics::HOST_CARD_GAP)
            })
            .when(!tiles, |this| this.h(metrics::HEIGHT_ROW));
        for position in start..end {
            container = container.child(self.render_card(self.shown[position], window, cx));
        }
        container.into_any_element()
    }

    /// The list layout's column names, aligned with the rows.
    fn render_list_header(&self, cx: &App) -> impl IntoElement {
        let theme = cx.theme();
        let cell = |label: &'static str| div().min_w_0().truncate().child(label);
        h_flex()
            .id("kh-list-header")
            .test_support()
            .flex_none()
            .w_full()
            .h(metrics::HEIGHT_CONTROL)
            .px(metrics::SPACE_3)
            .gap(metrics::SPACE_3)
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .text_size(metrics::FONT_SIZE_CAPTION)
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.muted_foreground)
            .child(
                cell(strings::COLUMN_ADDRESS)
                    .flex_1()
                    .pl(LIST_TILE_SIZE + metrics::SPACE_3),
            )
            .child(
                cell(strings::COLUMN_KEY_TYPE)
                    .flex_none()
                    .w(KEY_TYPE_COLUMN),
            )
            .child(cell(strings::COLUMN_FINGERPRINT).flex_1())
            .child(cell(strings::COLUMN_ADDED).flex_none().w(ADDED_COLUMN))
    }

    /// No entries at all (with Import as the screen's accent button while
    /// no panel is open), or none matching the search.
    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let none = self.items.is_empty();
        let (id, title, body) = if none {
            ("kh-empty", strings::EMPTY_TITLE, Some(strings::EMPTY_BODY))
        } else {
            ("kh-no-matches", strings::NO_MATCHES, None)
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
                        .child(crate::panels::widgets::text_with_paths(body, cx)),
                )
            })
            .when(none, |this| {
                this.child(
                    h_flex().pt(metrics::SPACE_2).child(
                        ControlButton::new("kh-empty-import", ControlKind::Accent)
                            .icon(Lucide::Import)
                            .label(strings::EMPTY_IMPORT)
                            .path_tooltip(strings::KNOWN_HOSTS_PATH)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.open_import(window, cx)
                            })),
                    ),
                )
            })
            .into_any_element()
    }

    // -- Details panel -------------------------------------------------------------

    fn render_panel(&self, view: &KnownHostView, total: f32, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let id = view.record_id;
        let screen = cx.entity().downgrade();
        let header = DetailsHeader::new(strings::PANEL_TITLE)
            .menu({
                let screen = screen.clone();
                move |menu, _, _| card_menu(menu, screen.clone(), id)
            })
            .on_collapse({
                let screen = screen.clone();
                move |_, _, cx| {
                    let _ = screen.update(cx, |screen, cx| screen.close_panel(cx));
                }
            });
        let row = |row_id: &'static str, label: &'static str, value: AnyElement| {
            v_flex()
                .id(row_id)
                .test_support()
                .w_full()
                .gap(metrics::SPACE_1)
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(theme.muted_foreground)
                        .child(label),
                )
                .child(value)
        };
        let text = |value: String| {
            div()
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(theme.foreground)
                .child(value)
                .into_any_element()
        };
        let note = |value: &'static str| {
            div()
                .text_size(metrics::FONT_SIZE_CAPTION)
                .text_color(theme.muted_foreground)
                .child(value)
        };
        let address = v_flex()
            .gap(metrics::SPACE_1)
            .child(
                h_flex()
                    .gap(metrics::SPACE_2)
                    .items_center()
                    .child(
                        div()
                            .min_w_0()
                            .text_size(metrics::FONT_SIZE_BODY)
                            .text_color(theme.foreground)
                            .child(card_title(view)),
                    )
                    .when(view.revoked, |this| {
                        this.child(revoked_badge("kh-panel-revoked", cx))
                    }),
            )
            .when(view.hashed, |this| this.child(note(strings::HASHED_NOTE)))
            .when(view.revoked, |this| this.child(note(strings::REVOKED_NOTE)))
            .into_any_element();
        let fingerprint = h_flex()
            .w_full()
            .gap(metrics::SPACE_2)
            .items_center()
            .child(
                crate::panels::widgets::fingerprint_line(view.fingerprint.clone())
                    .id("kh-fingerprint")
                    .test_support()
                    .flex_1()
                    .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                    .text_size(metrics::FONT_SIZE_DENSE)
                    .text_color(theme.foreground),
            )
            .child(
                ControlButton::new("kh-copy-fingerprint", ControlKind::Ghost)
                    .icon(Lucide::Copy)
                    .tooltip(strings::COPY_FINGERPRINT)
                    .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                        this.copy_fingerprint(id, window, cx)
                    })),
            )
            .into_any_element();
        let state = self.state.read(cx);
        let hosts: Vec<(Uuid, String)> = view
            .host_ids
            .iter()
            .filter_map(|host| {
                state
                    .host(*host)
                    .map(|item| (*host, item.title().to_owned()))
            })
            .collect();
        let hosts = if hosts.is_empty() {
            text(strings::NO_SAVED_HOSTS.to_owned())
        } else {
            v_flex()
                .gap(metrics::SPACE_1)
                .children(hosts.into_iter().map(|(host, title)| {
                    h_flex()
                        .id(panel_host_id(host))
                        .test_support()
                        .gap(metrics::SPACE_2)
                        .items_center()
                        .child(
                            Icon::new(Lucide::Server)
                                .with_size(ICON_SIZE)
                                .text_color(theme.muted_foreground),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_size(metrics::FONT_SIZE_BODY)
                                .text_color(theme.foreground)
                                .child(title),
                        )
                }))
                .into_any_element()
        };
        let block = v_flex()
            .id("kh-panel-block")
            .test_support()
            .w_full()
            .flex_none()
            .p(metrics::SPACE_4)
            .gap(metrics::SPACE_3)
            .rounded(metrics::RADIUS_CARD)
            .bg(theme.secondary)
            .child(row("kh-panel-address", strings::FIELD_ADDRESS, address))
            .child(row(
                "kh-panel-port",
                strings::FIELD_PORT,
                text(port_text(view)),
            ))
            .child(row(
                "kh-panel-key-type",
                strings::FIELD_KEY_TYPE,
                text(view.key_type.clone()),
            ))
            .child(row(
                "kh-panel-fingerprint",
                strings::FIELD_FINGERPRINT,
                fingerprint,
            ))
            .child(
                row(
                    "kh-panel-added",
                    strings::FIELD_ADDED,
                    text(date_text(&view.created_at)),
                )
                .when_some(
                    crate::dates::timestamp_tooltip(&view.created_at),
                    |this, utc| this.tooltip(crate::dates::utc_tooltip(utc)),
                ),
            )
            .when_some(view.comment.clone(), |this, comment| {
                this.child(row(
                    "kh-panel-comment",
                    strings::FIELD_COMMENT,
                    text(comment),
                ))
            })
            .child(row("kh-panel-hosts", strings::FIELD_HOSTS, hosts));
        let danger = theme.danger;
        let danger_hover = theme.danger_hover;
        let hover = theme.secondary_hover;
        let pressed = theme.secondary_active;
        // Remove: a neutral button with the danger text (design-system 2, 6);
        // the confirmation's Remove carries the danger fill.
        let remove = div()
            .id("kh-panel-remove")
            .test_support()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .gap(metrics::SPACE_2)
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
            }));
        let width = self.fitted_panel_width(total);
        let handle_hover = theme.drag_border;
        div()
            .id("kh-panel")
            .test_support()
            .relative()
            .w(width)
            .flex_none()
            .h_full()
            .border_l_1()
            .border_color(theme.border)
            .bg(theme.colors.list)
            .child(
                v_flex().size_full().child(header).child(
                    v_flex()
                        .id("kh-panel-body")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scroll()
                        .px(metrics::SPACE_4)
                        .pb(metrics::SPACE_4)
                        .gap(metrics::SPACE_3)
                        .child(block)
                        .child(h_flex().child(remove)),
                ),
            )
            .child(
                div()
                    .id("kh-panel-resize")
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
            .into_any_element()
    }
}

impl Render for KnownHostsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Drawn means shown: a stale list loads now (in the background).
        self.load_if_stale(cx);
        let width = self.grid_width(window);
        self.rebuild_rows(width);
        let total = self
            .measured_width
            .unwrap_or_else(|| (window.viewport_size().width - metrics::WIDTH_SIDEBAR).as_f32());
        let screen = cx.entity().downgrade();
        // Measures the whole screen (the shell's sidebar and window size set
        // it); a new width lays the grid out again on the next frame.
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

        let content = if self.shown.is_empty() && self.items.is_empty() && self.is_loading() {
            // No "No known hosts" while the list is still on its way.
            div().flex_1().into_any_element()
        } else if self.shown.is_empty() {
            self.render_empty(cx)
        } else {
            div()
                .size_full()
                .px(CONTENT_PADDING)
                .child(
                    v_virtual_list(
                        cx.entity(),
                        "kh-grid",
                        self.row_sizes.clone(),
                        |this: &mut Self, range, window, cx| {
                            range
                                .map(|row| this.render_row(row, window, cx))
                                .collect::<Vec<_>>()
                        },
                    )
                    .track_scroll(&self.scroll)
                    .size_full(),
                )
                .into_any_element()
        };
        let header = (self.layout == KnownHostsLayout::List && !self.shown.is_empty()).then(|| {
            div()
                .flex_none()
                .w_full()
                .px(CONTENT_PADDING)
                .child(self.render_list_header(cx))
        });
        let rows = v_flex()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(content)
            .when(!self.shown.is_empty(), |this| {
                this.child(Scrollbar::vertical(&self.scroll))
            });
        let main = v_flex()
            .id("kh-main")
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
                    .id("kh-content")
                    .test_support()
                    .key_context("KnownHostsGrid")
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
                    .children(header)
                    .child(rows),
            );
        let panel = self
            .selected_view()
            .cloned()
            .map(|view| self.render_panel(&view, total, cx));
        h_flex()
            .id("known-hosts-screen")
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
