//! Rendering of the Keychain screen (UI 7.1-7.3): the action bar, the Keys
//! and Identities sections as a card grid or a list, the empty states, the
//! details panel frame, the key panel and the two forms. Sizes and colors
//! come from docs/design-system.md through `crate::theme` (cards and rows as
//! on the Hosts screen, the panel as the details panel of UI 1.3).

use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, DragMoveEvent, ExternalPaths,
    Focusable as _, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    ParentElement as _, Pixels, Render, SharedString, StatefulInteractiveElement as _, Styled as _,
    Window, canvas, div, prelude::FluentBuilder as _, px, size,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::input::{Input, Textarea};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenu, PopupMenuItem};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _,
    WindowExt as _, h_flex, v_flex, v_virtual_list,
};
use shellpp_core::service::dto::{KeyView, TagColor};

use super::forms::{GenerateForm, ImportForm, Inspection, KeyFamily};
use super::panel::KeyPanel;
use super::{
    KeychainLayout, KeychainScreen, KeychainSort, PanelContent, Row, SshScan, identity_card_id,
    identity_method, identity_subtitle_id, key_card_id, key_subtitle, key_subtitle_id, key_title,
    key_type, key_type_text, key_unknown_id, ssh_file_id, strings,
};
use crate::hosts::controls::{self, ControlButton, ControlKind, ICON_SIZE};
use crate::hosts::layout as grid;
use crate::panels::widgets;
use crate::shell::details::{self, DetailsHeader, ResizeDetails, ResizePreview};
use crate::state::records_edit::{IdentityView, KeyKind};
use crate::theme::{self, metrics, palette};

/// Horizontal padding of the content (`--space-5`), as on the Hosts screen.
const CONTENT_PADDING: gpui::Rems = metrics::SPACE_5;
/// A section heading row ("Keys", "Identities"), as on the Hosts screen.
const HEADING_HEIGHT: gpui::Rems = metrics::rpx(40.);
/// The tile of a list row (the 36 px card tile does not fit a 44 px row).
const LIST_TILE_SIZE: gpui::Rems = metrics::rpx(28.);
/// The search field of the action bar takes the room left, between these
/// widths (200 px: the owner's polish of run 3).
const SEARCH_MIN_WIDTH: gpui::Rems = metrics::rpx(200.);
const SEARCH_MAX_WIDTH: gpui::Rems = metrics::rpx(320.);
/// Fixed list columns: a key's type, an identity's username and method.
const TYPE_COLUMN: gpui::Rems = metrics::rpx(140.);
const USERNAME_COLUMN: gpui::Rems = metrics::rpx(160.);
const METHOD_COLUMN: gpui::Rems = metrics::rpx(200.);
/// Width of the menus of the screen.
const MENU_WIDTH: gpui::Rems = metrics::rpx(220.);

/// A tile with a white icon on `--tag-graphite`, as host tiles.
fn tile(icon: impl Into<Icon>, tile_size: gpui::Rems) -> impl IntoElement {
    div()
        .flex_none()
        .size(tile_size)
        .rounded(metrics::RADIUS_TILE)
        .bg(palette::tag_color(TagColor::Graphite).hsla())
        .flex()
        .items_center()
        .justify_center()
        .child(
            Icon::new(icon)
                .size(ICON_SIZE)
                .text_color(palette::TEXT_ON_TAG.hsla()),
        )
}

/// A menu item with a stable id; inactive items use `--text-disabled` at
/// full opacity (design-system 6).
fn menu_item(id: &'static str, label: &'static str, enabled: bool, cx: &App) -> AnyElement {
    let color = if enabled {
        cx.theme().foreground
    } else {
        theme::tokens(cx).text_disabled.hsla()
    };
    div()
        .id(id)
        .test_support()
        .flex_1()
        .text_color(color)
        .child(label)
        .into_any_element()
}

/// Width of the Remove confirmation (as Known Hosts').
const REMOVE_DIALOG_WIDTH: gpui::Rems = metrics::rpx(420.);

/// "Remove key?" (D50): the key's label, then the hosts, groups and
/// identities that use it with the warning that they lose it, or that
/// nothing uses it; Cancel and Remove (`--danger`). Remove or Enter
/// confirms; Esc and Cancel close without removing.
pub(super) fn open_remove_confirmation(
    label: &str,
    users: &shellpp_core::service::keychain::KeyUsers,
    window: &mut Window,
    cx: &mut App,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
) {
    let on_confirm = Rc::new(on_confirm);
    let body: SharedString = strings::remove_body(label).into();
    let used = !users.is_empty();
    let lists: Vec<(&'static str, &'static str, SharedString)> = [
        ("kc-remove-hosts", strings::REMOVE_HOSTS, &users.hosts),
        ("kc-remove-groups", strings::REMOVE_GROUPS, &users.groups),
        (
            "kc-remove-identities",
            strings::REMOVE_IDENTITIES,
            &users.identities,
        ),
    ]
    .into_iter()
    .filter(|(_, _, list)| !list.is_empty())
    .map(|(id, heading, list)| {
        let names: Vec<&str> = list.iter().map(|user| user.name.as_str()).collect();
        (id, heading, names.join(", ").into())
    })
    .collect();
    window.open_dialog(cx, move |dialog, _, cx| {
        let confirm = on_confirm.clone();
        let on_ok = on_confirm.clone();
        let theme = cx.theme();
        let mut content = v_flex()
            .id("kc-remove-dialog")
            .test_support()
            .gap(metrics::SPACE_2)
            .text_size(metrics::FONT_SIZE_BODY)
            .text_color(theme.muted_foreground)
            .child(div().child(body.clone()));
        if used {
            content = content.child(
                div()
                    .id("kc-remove-warning")
                    .test_support()
                    .text_color(theme.warning)
                    .child(strings::REMOVE_USED),
            );
            for (id, heading, names) in &lists {
                content = content.child(
                    div()
                        .id(*id)
                        .test_support()
                        .child(
                            div()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.foreground)
                                .child(*heading),
                        )
                        .child(div().child(names.clone())),
                );
            }
        } else {
            content = content.child(div().child(strings::REMOVE_UNUSED));
        }
        dialog
            .w(metrics::to_px(REMOVE_DIALOG_WIDTH, cx))
            .bg(theme.popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .close_button(false)
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(strings::REMOVE_TITLE),
            )
            .child(content)
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
                        ControlButton::new("kc-remove-cancel", ControlKind::Neutral)
                            .label(strings::CANCEL)
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        ControlButton::new("kc-remove-confirm", ControlKind::Danger)
                            .label(strings::REMOVE)
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                confirm(window, cx);
                            }),
                    ),
            )
    });
}

/// The row of a key's type, led by the warning icon `warning_id` when the
/// type is "unknown" (UI 7.2); the caller adds the type text.
fn type_line(view: &KeyView, warning_id: SharedString, cx: &App) -> gpui::Div {
    let warning = cx.theme().warning;
    h_flex()
        .min_w_0()
        .gap(metrics::SPACE_1)
        .items_center()
        .when(key_type(&view.algorithm).is_none(), |this| {
            this.child(
                div().id(warning_id).test_support().flex_none().child(
                    Icon::new(IconName::TriangleAlert)
                        .xsmall()
                        .text_color(warning),
                ),
            )
        })
}

impl KeychainScreen {
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
            .id("kc-search-field")
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
                            .id("kc-search")
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

    fn render_view_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let button = |id: &'static str, icon: Lucide, tooltip: &'static str, mode| {
            ControlButton::new(id, ControlKind::Ghost)
                .icon(icon)
                .tooltip(tooltip)
                .selected(self.layout == mode)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_layout(mode, cx)))
        };
        h_flex()
            .id("kc-view-toggle")
            .test_support()
            .flex_none()
            .gap(metrics::SPACE_1)
            .child(button(
                "kc-view-tiles",
                Lucide::LayoutGrid,
                strings::VIEW_TILES,
                KeychainLayout::Tiles,
            ))
            .child(button(
                "kc-view-list",
                Lucide::List,
                strings::VIEW_LIST,
                KeychainLayout::List,
            ))
    }

    fn render_sort_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = cx.entity().downgrade();
        let current = self.sort;
        ControlButton::new("kc-sort", ControlKind::Ghost)
            .icon(Lucide::ArrowUpDown)
            .tooltip(strings::SORT)
            .dropdown_menu(move |mut menu, _, cx| {
                menu = menu.min_w(metrics::to_px(MENU_WIDTH, cx));
                for sort in KeychainSort::ALL {
                    let (id, label) = match sort {
                        KeychainSort::Name => ("kc-sort-name", strings::SORT_NAME),
                        KeychainSort::Created => ("kc-sort-created", strings::SORT_CREATED),
                        KeychainSort::Modified => ("kc-sort-modified", strings::SORT_MODIFIED),
                    };
                    let screen = screen.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, cx| menu_item(id, label, true, cx))
                            .checked(sort == current)
                            .on_click(move |_, _, cx| {
                                let _ = screen.update(cx, |screen, cx| screen.set_sort(sort, cx));
                            }),
                    );
                }
                menu
            })
    }

    /// The New key split button (▾: Generate key, New identity), then
    /// Certificate (adds a certificate to its key) and FIDO2 (the import of
    /// a security key's file); FIDO2's tooltip says what does not work yet. `compact`: only their icons, so that the search keeps its
    /// room ([`Self::compact_bar`]).
    fn render_bar_left(
        &self,
        compact: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let screen = cx.entity().downgrade();
        let new_key = ControlButton::new("kc-new-key", ControlKind::Neutral)
            .icon(Lucide::Plus)
            .label(strings::NEW_KEY)
            .rounded_sides(true, false)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| this.open_import(window, cx)));
        let new_key_menu = ControlButton::new("kc-new-key-menu", ControlKind::Neutral)
            .icon(IconName::ChevronDown)
            .tooltip(strings::NEW_KEY_MENU)
            .rounded_sides(false, true)
            .dropdown_menu(move |menu, _, cx| {
                let generate = screen.clone();
                menu.min_w(metrics::to_px(MENU_WIDTH, cx))
                    .item(
                        PopupMenuItem::element(|_, cx| {
                            menu_item("kc-menu-generate", strings::GENERATE_KEY, true, cx)
                        })
                        .on_click(move |_, window, cx| {
                            let _ =
                                generate.update(cx, |screen, cx| screen.open_generate(window, cx));
                        }),
                    )
                    // The identity panel (UI 7.4) is not built yet (D30).
                    .item(
                        PopupMenuItem::element(|_, cx| {
                            menu_item("kc-menu-new-identity", strings::NEW_IDENTITY, false, cx)
                        })
                        .disabled(true),
                    )
            });
        let split = h_flex()
            .flex_none()
            .child(new_key)
            .child(
                div()
                    .w(metrics::rpx(1.))
                    .h(metrics::HEIGHT_CONTROL)
                    .bg(theme.border),
            )
            .child(new_key_menu);
        // Certificate and FIDO2 are one group: both as wide as the wider.
        let width = controls::group_width(window, &[strings::CERTIFICATE, strings::FIDO2]);
        let labelled = |button: ControlButton, label: &'static str| {
            if compact {
                button
            } else {
                button.label(label).min_w(width)
            }
        };
        h_flex()
            .id("kc-action-bar-left")
            .test_support()
            .flex_none()
            .gap(metrics::SPACE_2)
            .child(split)
            .child(labelled(
                ControlButton::new("kc-certificate", ControlKind::Neutral)
                    .icon(IconName::FileText)
                    .tooltip(strings::CERTIFICATE_TOOLTIP)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_certificate(window, cx)
                    })),
                strings::CERTIFICATE,
            ))
            .child(labelled(
                ControlButton::new("kc-fido2", ControlKind::Neutral)
                    .icon(Lucide::Usb)
                    .tooltip(strings::FIDO2_TOOLTIP)
                    .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                        this.open_import_security_key(window, cx)
                    })),
                strings::FIDO2,
            ))
    }

    /// The narrowest the right group gets: the search at its minimum, the
    /// view toggle (two buttons 4 px apart) and sort, with their gaps.
    fn bar_right_min() -> f32 {
        metrics::design_px(
            SEARCH_MIN_WIDTH
                + metrics::SPACE_2 * 2.
                + metrics::HEIGHT_CONTROL * 3.
                + metrics::SPACE_1,
        )
    }

    /// Whether the bar cannot hold the labels of Certificate and FIDO2 next
    /// to the search at its minimum: they show only their icons then, before
    /// the search wraps below (an open panel narrows the bar).
    pub(super) fn compact_bar(&self, window: &Window) -> bool {
        let label = |labels: &[&str]| metrics::design_px(controls::group_width(window, labels));
        let gap = metrics::design_px(metrics::SPACE_2);
        let split = label(&[strings::NEW_KEY]) + 1. + metrics::design_px(metrics::HEIGHT_CONTROL);
        let pair = label(&[strings::CERTIFICATE, strings::FIDO2]);
        let left = split + 2. * (gap + pair);
        self.grid_width(window) < left + gap + Self::bar_right_min()
    }

    /// New key, Certificate and FIDO2 on the left; search, view toggle and
    /// sort on the right (7.1). In a narrow area the search narrows to its
    /// minimum, then Certificate and FIDO2 drop their labels; only when that
    /// is not enough the right group wraps below: it never gets narrower than
    /// the search field's minimum and its buttons, so it cannot slide under
    /// the left group.
    fn render_action_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let compact = self.compact_bar(window);
        h_flex()
            .id("kc-action-bar")
            .test_support()
            .w_full()
            .flex_wrap()
            .items_center()
            .gap(metrics::SPACE_2)
            .child(self.render_bar_left(compact, window, cx))
            .child(
                h_flex()
                    .id("kc-action-bar-right")
                    .test_support()
                    .flex_1()
                    .min_w(metrics::rpx(Self::bar_right_min()))
                    .justify_end()
                    .items_center()
                    .gap(metrics::SPACE_2)
                    .child(self.render_search(window, cx))
                    .child(self.render_view_toggle(cx))
                    .child(self.render_sort_menu(cx)),
            )
    }

    // -- Content -------------------------------------------------------------------

    fn total_width(&self, window: &Window) -> f32 {
        self.measured_width.unwrap_or_else(|| {
            (window.viewport_size().width / metrics::zoom_of(window)
                - metrics::at_100(metrics::WIDTH_SIDEBAR))
            .as_f32()
        })
    }

    /// The width of the grid area (the screen without its panel).
    fn grid_width(&self, window: &Window) -> f32 {
        let total = self.total_width(window);
        // A panel over the grid (D100) leaves it the whole width.
        let panel = if self.has_panel() && !details::overlays_in(window) {
            self.fitted_panel_width(total).as_f32()
        } else {
            0.
        };
        (total - panel - metrics::design_px(CONTENT_PADDING) * 2.).max(1.)
    }

    /// The panel on display: the chosen width, narrowed so the grid keeps
    /// 360 px, never below 320 px (design-system 6).
    fn fitted_panel_width(&self, total: f32) -> Pixels {
        let room = px(total) - details::MAIN_AREA_MIN;
        details::clamp_width(self.panel_width)
            .min(room.max(metrics::at_100(metrics::WIDTH_DETAILS_MIN)))
    }

    fn rebuild_rows(&mut self, width: f32) {
        let tiles = self.layout == KeychainLayout::Tiles;
        let columns = if tiles { grid::columns_for(width) } else { 1 };
        let counts = self.shown_keys.len() * 100_003 + self.shown_identities.len();
        let key = (columns, counts, width as i32);
        if key == self.layout_key {
            return;
        }
        self.layout_key = key;
        self.columns = columns;
        self.card_width = if tiles {
            grid::card_width(width, columns)
        } else {
            width
        };
        let mut rows = Vec::new();
        let any_identities = !self.shown_identities.is_empty();
        if !self.shown_keys.is_empty() || any_identities {
            rows.push(Row::KeysHeading);
            if self.shown_keys.is_empty() {
                rows.push(Row::NoKeys);
            } else {
                if !tiles {
                    rows.push(Row::KeyColumns);
                }
                rows.extend((0..self.shown_keys.len()).step_by(columns).map(Row::Keys));
            }
        }
        if any_identities {
            rows.push(Row::IdentitiesHeading);
            if !tiles {
                rows.push(Row::IdentityColumns);
            }
            rows.extend(
                (0..self.shown_identities.len())
                    .step_by(columns)
                    .map(Row::Identities),
            );
        }
        // In design units (D96): the list scales them when it draws.
        let card_row = metrics::at_100(if tiles {
            metrics::HEIGHT_HOST_CARD + metrics::HOST_CARD_GAP
        } else {
            metrics::HEIGHT_ROW
        });
        self.row_sizes = Rc::new(
            rows.iter()
                .map(|row| {
                    let height = match row {
                        Row::KeysHeading | Row::IdentitiesHeading => {
                            metrics::at_100(HEADING_HEIGHT)
                        }
                        Row::KeyColumns | Row::IdentityColumns | Row::NoKeys => {
                            metrics::at_100(metrics::HEIGHT_CONTROL)
                        }
                        Row::Keys(_) | Row::Identities(_) => card_row,
                    };
                    size(px(width), height)
                })
                .collect(),
        );
        self.rows = rows;
    }

    fn render_heading(&self, id: &'static str, label: &'static str, cx: &App) -> AnyElement {
        div()
            .id(id)
            .test_support()
            .h(HEADING_HEIGHT)
            .flex()
            .items_end()
            .pb(metrics::SPACE_2)
            .text_size(metrics::FONT_SIZE_HEADING)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().foreground)
            .child(label)
            .into_any_element()
    }

    /// The list layout's column names of a section, aligned with its rows.
    fn render_columns(&self, keys: bool, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let cell = |label: &'static str| div().min_w_0().truncate().child(label);
        let (id, second, third) = if keys {
            (
                "kc-keys-columns",
                cell(strings::COLUMN_TYPE).flex_none().w(TYPE_COLUMN),
                cell(strings::COLUMN_FINGERPRINT).flex_1(),
            )
        } else {
            (
                "kc-identities-columns",
                cell(strings::COLUMN_USERNAME)
                    .flex_none()
                    .w(USERNAME_COLUMN),
                cell(strings::COLUMN_METHOD).flex_none().w(METHOD_COLUMN),
            )
        };
        h_flex()
            .id(id)
            .test_support()
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
                cell(strings::COLUMN_NAME)
                    .flex_1()
                    .pl(LIST_TILE_SIZE + metrics::SPACE_3),
            )
            .child(second)
            .child(third)
            .into_any_element()
    }

    fn render_key_texts(&self, view: &KeyView, cx: &App) -> AnyElement {
        let theme = cx.theme();
        let id = view.record_id;
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
                    .child(key_title(view)),
            )
            .child(
                type_line(view, key_unknown_id(id), cx).child(
                    div()
                        .id(key_subtitle_id(id))
                        .test_support()
                        .min_w_0()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(key_subtitle(view)),
                ),
            )
            .into_any_element()
    }

    /// The frame of a card or row: selection, hover, pressed and keyboard
    /// focus states when it can be selected (keys), none for read-only
    /// identities.
    /// A card or row; `cursor`: where the arrows are (the keyboard focus
    /// ring shows there while the screen has the focus from the keyboard).
    fn item_frame(
        &self,
        id: SharedString,
        selected: bool,
        cursor: bool,
        interactive: bool,
        window: &Window,
        cx: &App,
    ) -> gpui::Stateful<gpui::Div> {
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let hover = theme.secondary_hover;
        let pressed = theme.secondary_active;
        let keyboard_cursor =
            cursor && self.focus.is_focused(window) && window.last_input_was_keyboard();
        let frame = div()
            .id(id)
            .relative()
            .flex()
            .items_center()
            .gap(metrics::SPACE_3)
            .px(metrics::SPACE_3)
            .cursor_default();
        let (frame, radius) = match self.layout {
            KeychainLayout::Tiles => (
                frame
                    .flex_none()
                    .w(metrics::rpx(self.card_width))
                    .h(metrics::HEIGHT_HOST_CARD)
                    .rounded(metrics::RADIUS_CARD)
                    .bg(theme.secondary)
                    .border_1()
                    .border_color(if selected {
                        theme.primary
                    } else {
                        theme.border
                    })
                    .when(interactive, |this| {
                        this.hover(move |style| style.bg(hover))
                            .active(move |style| style.bg(pressed))
                    }),
                metrics::RADIUS_CARD,
            ),
            KeychainLayout::List => (
                frame
                    .w_full()
                    .h(metrics::HEIGHT_ROW)
                    .border_b_1()
                    .border_color(theme.border)
                    .text_size(metrics::FONT_SIZE_DENSE)
                    .map(|this| {
                        if selected {
                            this.bg(tokens.accent_subtle.hsla())
                        } else if interactive {
                            this.hover(move |style| style.bg(hover))
                                .active(move |style| style.bg(pressed))
                        } else {
                            this
                        }
                    }),
                gpui::Rems::ZERO,
            ),
        };
        frame.when(keyboard_cursor, |this| {
            this.child(controls::focus_ring(radius, metrics::rpx(1.), cx))
        })
    }

    fn render_key(&self, index: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let view = &self.keys[index];
        let id = view.record_id;
        let selected = self.selected == Some(id);
        let cursor = self.cursor_key() == Some(id);
        let theme = cx.theme();
        let frame = self
            .item_frame(key_card_id(id), selected, cursor, true, window, cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                this.focus.focus(window, cx);
                this.open_key(id, window, cx);
            }));
        let frame = gpui_kit::TestSupportExt::test_support(frame);
        match self.layout {
            KeychainLayout::Tiles => frame
                .child(tile(Lucide::Key, metrics::HOST_TILE_SIZE))
                .child(self.render_key_texts(view, cx))
                .into_any_element(),
            KeychainLayout::List => frame
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(metrics::SPACE_3)
                        .items_center()
                        .child(tile(Lucide::Key, LIST_TILE_SIZE))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .font_weight(FontWeight::MEDIUM)
                                .text_color(theme.foreground)
                                .child(key_title(view)),
                        ),
                )
                .child(
                    type_line(view, key_unknown_id(id), cx)
                        .flex_none()
                        .w(TYPE_COLUMN)
                        .child(
                            div()
                                .id(key_subtitle_id(id))
                                .test_support()
                                .min_w_0()
                                .truncate()
                                .text_color(theme.muted_foreground)
                                .child(key_type_text(view)),
                        ),
                )
                .child(
                    widgets::fingerprint_line(view.fingerprint.clone().unwrap_or_default())
                        .flex_1()
                        .text_color(theme.muted_foreground)
                        .into_any_element(),
                )
                .into_any_element(),
        }
    }

    fn render_identity(&self, index: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let view: &IdentityView = &self.identities[index];
        let id = view.record_id;
        let theme = cx.theme();
        let frame = gpui_kit::TestSupportExt::test_support(self.item_frame(
            identity_card_id(id),
            false,
            false,
            false,
            window,
            cx,
        ));
        let title = div()
            .min_w_0()
            .truncate()
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.foreground)
            .child(view.label.clone());
        let method = div()
            .id(identity_subtitle_id(id))
            .test_support()
            .min_w_0()
            .truncate()
            .text_color(theme.muted_foreground)
            .child(identity_method(view));
        match self.layout {
            KeychainLayout::Tiles => frame
                .child(tile(IconName::User, metrics::HOST_TILE_SIZE))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(metrics::rpx(2.))
                        .child(title.text_size(metrics::FONT_SIZE_BODY))
                        .child(method.text_size(metrics::FONT_SIZE_CAPTION)),
                )
                .into_any_element(),
            KeychainLayout::List => frame
                .child(
                    h_flex()
                        .flex_1()
                        .min_w_0()
                        .gap(metrics::SPACE_3)
                        .items_center()
                        .child(tile(IconName::User, LIST_TILE_SIZE))
                        .child(title),
                )
                .child(
                    div()
                        .flex_none()
                        .w(USERNAME_COLUMN)
                        .min_w_0()
                        .truncate()
                        .text_color(theme.muted_foreground)
                        .child(view.username.clone().unwrap_or_default()),
                )
                .child(div().flex_none().w(METHOD_COLUMN).child(method))
                .into_any_element(),
        }
    }

    fn render_row(&self, row: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let Some(kind) = self.rows.get(row).copied() else {
            return div().into_any_element();
        };
        let tiles = self.layout == KeychainLayout::Tiles;
        let cards = |container: gpui::Div| {
            container
                .w_full()
                .when(tiles, |this| {
                    this.h(metrics::HEIGHT_HOST_CARD + metrics::HOST_CARD_GAP)
                        .pb(metrics::HOST_CARD_GAP)
                        .gap(metrics::HOST_CARD_GAP)
                })
                .when(!tiles, |this| this.h(metrics::HEIGHT_ROW))
        };
        let columns = self.row_columns();
        match kind {
            Row::KeysHeading => self.render_heading("kc-heading-keys", strings::KEYS, cx),
            Row::IdentitiesHeading => {
                self.render_heading("kc-heading-identities", strings::IDENTITIES, cx)
            }
            Row::KeyColumns => self.render_columns(true, cx),
            Row::IdentityColumns => self.render_columns(false, cx),
            Row::NoKeys => div()
                .id("kc-no-keys")
                .test_support()
                .h(metrics::HEIGHT_CONTROL)
                .flex()
                .items_center()
                .text_color(cx.theme().muted_foreground)
                .child(strings::NO_KEYS)
                .into_any_element(),
            Row::Keys(start) => {
                let end = (start + columns).min(self.shown_keys.len());
                let mut container = cards(h_flex());
                for position in start..end {
                    container =
                        container.child(self.render_key(self.shown_keys[position], window, cx));
                }
                container.into_any_element()
            }
            Row::Identities(start) => {
                let end = (start + columns).min(self.shown_identities.len());
                let mut container = cards(h_flex());
                for position in start..end {
                    container = container.child(self.render_identity(
                        self.shown_identities[position],
                        window,
                        cx,
                    ));
                }
                container.into_any_element()
            }
        }
    }

    /// No keys and no identities (Generate as the screen's accent button
    /// while no panel is open; Import; Import from ~/.ssh), or none
    /// matching the search.
    fn render_empty(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let none = self.keys.is_empty() && self.identities.is_empty();
        let (id, title, body) = if none {
            ("kc-empty", strings::EMPTY_TITLE, Some(strings::EMPTY_BODY))
        } else {
            ("kc-no-matches", strings::NO_MATCHES, None)
        };
        // One accent button per screen: the open panel holds the primary
        // action then (design-system 1).
        let generate_kind = if self.has_panel() {
            ControlKind::Neutral
        } else {
            ControlKind::Accent
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
                        .max_w(metrics::rpx(420.))
                        .text_center()
                        .text_color(theme.muted_foreground)
                        .child(body),
                )
            })
            .when(none, |this| {
                // One group: all three as wide as the widest.
                let width = controls::group_width(
                    window,
                    &[
                        strings::EMPTY_GENERATE,
                        strings::EMPTY_IMPORT,
                        strings::EMPTY_IMPORT_SSH,
                    ],
                );
                this.child(
                    h_flex()
                        .pt(metrics::SPACE_2)
                        .gap(metrics::SPACE_2)
                        .flex_wrap()
                        .justify_center()
                        .child(
                            ControlButton::new("kc-empty-generate", generate_kind)
                                .icon(Lucide::KeyRound)
                                .label(strings::EMPTY_GENERATE)
                                .min_w(width)
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.open_generate(window, cx)
                                })),
                        )
                        .child(
                            ControlButton::new("kc-empty-import", ControlKind::Neutral)
                                .icon(Lucide::Import)
                                .label(strings::EMPTY_IMPORT)
                                .min_w(width)
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.open_import(window, cx)
                                })),
                        )
                        .child(
                            ControlButton::new("kc-empty-import-ssh", ControlKind::Neutral)
                                .icon(Lucide::FolderOpen)
                                .label(strings::EMPTY_IMPORT_SSH)
                                .path_tooltip(strings::SSH_FOLDER_PATH)
                                .min_w(width)
                                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                    this.open_import_from_ssh(window, cx)
                                })),
                        ),
                )
            })
            .into_any_element()
    }

    /// The details panel frame: the panel's own view, the left border and
    /// the resize handle.
    fn render_panel_frame(
        &self,
        total: f32,
        over: bool,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let content = match self.panel.as_ref()? {
            PanelContent::Key(panel) => panel.clone().into_any_element(),
            PanelContent::Generate(form) => form.clone().into_any_element(),
            PanelContent::Import(form) => form.clone().into_any_element(),
        };
        let theme = cx.theme();
        let handle_hover = theme.drag_border;
        // Too narrow for the panel beside the grid (175 and 200 %, D100): it
        // covers the grid's right, with a shadow.
        let (frame, width) = if over {
            (
                details::over_area(div().id("kc-panel"), cx),
                details::overlay_width(self.panel_width, px(total)),
            )
        } else {
            (
                div().id("kc-panel").relative(),
                self.fitted_panel_width(total),
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
                .child(content)
                .child(
                    div()
                        .id("kc-panel-resize")
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
}

impl Render for KeychainScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // Drawn means shown: stale lists load now (in the background).
        self.load_if_stale(cx);
        let width = self.grid_width(window);
        self.rebuild_rows(width);
        let total = self.total_width(window);
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

        let nothing = self.keys.is_empty() && self.identities.is_empty();
        let content = if nothing && self.is_loading() {
            // No "No keys yet" while the lists are still on their way.
            div().flex_1().into_any_element()
        } else if self.rows.is_empty() {
            self.render_empty(window, cx)
        } else {
            div()
                .size_full()
                .px(CONTENT_PADDING)
                .child(
                    v_virtual_list(
                        cx.entity(),
                        "kc-grid",
                        metrics::scaled_sizes(&self.row_sizes, cx),
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
        let rows = v_flex()
            .relative()
            .flex_1()
            .min_h_0()
            .w_full()
            .child(content)
            .when(!self.rows.is_empty(), |this| {
                this.child(Scrollbar::vertical(&self.scroll))
            });
        let main = v_flex()
            .id("kc-main")
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
                    .id("kc-content")
                    .test_support()
                    .key_context("KeychainGrid")
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
                    .child(rows),
            );
        let panel = self.render_panel_frame(total, details::overlays_in(window), cx);
        h_flex()
            .id("keychain-screen")
            .test_support()
            .relative()
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

// -- Panels ----------------------------------------------------------------------------

/// The scrolling body of a panel below its header.
fn panel_body(id: &'static str) -> widgets::Hit {
    v_flex()
        .id(id)
        .test_support()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .px(metrics::SPACE_4)
        .pb(metrics::SPACE_4)
        .gap(metrics::SPACE_3)
}

/// A monospace text block (fingerprints, keys): wraps anywhere.
fn mono(id: &'static str, text: impl Into<SharedString>, cx: &App) -> impl IntoElement {
    div()
        .id(id)
        .test_support()
        .w_full()
        .min_w_0()
        .font_family(crate::terminal::fonts::JETBRAINS_MONO)
        .text_size(metrics::FONT_SIZE_DENSE)
        .text_color(cx.theme().foreground)
        .whitespace_normal()
        .child(text.into())
}

/// A neutral 32 px button of a panel (Show, Copy), disabled at 0.45.
fn panel_button(
    id: &'static str,
    icon: impl Into<Icon>,
    label: &'static str,
    enabled: bool,
    cx: &App,
) -> widgets::Hit {
    let theme = cx.theme();
    let hover = theme.secondary_hover;
    let pressed = theme.secondary_active;
    let icon: Icon = icon.into();
    widgets::hit(id)
        .flex_none()
        .h(metrics::rpx(32.))
        .px(metrics::SPACE_3)
        .flex()
        .items_center()
        .gap(metrics::SPACE_2)
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .border_color(theme.input)
        .text_size(metrics::FONT_SIZE_BODY)
        .text_color(theme.foreground)
        .cursor_default()
        .map(|this| {
            if enabled {
                this.hover(move |style| style.bg(hover))
                    .active(move |style| style.bg(pressed))
            } else {
                this.opacity(metrics::OPACITY_DISABLED)
            }
        })
        .child(icon.small().text_color(theme.muted_foreground))
        .child(label)
}

/// A labelled read-only value of a panel block.
fn value_row(
    id: &'static str,
    label: &'static str,
    value: impl IntoElement,
    cx: &App,
) -> impl IntoElement {
    v_flex()
        .id(id)
        .test_support()
        .w_full()
        .gap(metrics::SPACE_1)
        .child(widgets::caption(label, cx))
        .child(value)
}

/// The save / ask choice of a passphrase (UI 7.3, 4.8).
fn passphrase_choice<T: 'static>(
    prefix: &'static str,
    save: bool,
    on_choose: impl Fn(&mut T, bool, &mut Context<T>) + Clone + 'static,
    cx: &mut Context<T>,
) -> impl IntoElement {
    let choice = |id: &'static str, label: &'static str, value: bool, cx: &mut Context<T>| {
        let on_choose = on_choose.clone();
        widgets::segment(id, label, save == value, cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| on_choose(this, value, cx)))
            .into_any_element()
    };
    let (store_id, ask_id) = match prefix {
        "kc-gen" => ("kc-gen-passphrase-store", "kc-gen-passphrase-ask"),
        _ => ("kc-import-passphrase-store", "kc-import-passphrase-ask"),
    };
    widgets::segments([
        choice(store_id, strings::PASSPHRASE_STORE, true, cx),
        choice(ask_id, strings::PASSPHRASE_ASK, false, cx),
    ])
}

/// The commit row of a form, pinned below its scrolling body (as the host
/// panel's Connect): Cancel and the accent button (the panel's one primary
/// action, design-system 1).
fn form_footer<T: 'static>(
    cancel_id: &'static str,
    submit_id: &'static str,
    submit_label: &'static str,
    enabled: bool,
    on_cancel: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
    on_submit: impl Fn(&mut T, &mut Window, &mut Context<T>) + 'static,
    cx: &mut Context<T>,
) -> impl IntoElement {
    h_flex()
        .w_full()
        .flex_none()
        .px(metrics::SPACE_4)
        .py(metrics::SPACE_3)
        .border_t_1()
        .border_color(cx.theme().border)
        .gap(metrics::SPACE_2)
        .justify_end()
        .child(
            ControlButton::new(cancel_id, ControlKind::Neutral)
                .label(strings::CANCEL)
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                        on_cancel(this, window, cx)
                    }),
                ),
        )
        .child(
            ControlButton::new(submit_id, ControlKind::Accent)
                .label(submit_label)
                .disabled(!enabled)
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, window, cx| {
                        on_submit(this, window, cx)
                    }),
                ),
        )
}

fn field_error(
    errors: &crate::state::records_edit::FieldErrors,
    field: &str,
) -> Option<&'static str> {
    errors
        .get(field)
        .map(|reason| strings::reason(field, *reason))
}

impl KeyPanel {
    /// The key panel (UI 7.3).
    pub(super) fn render_key_panel(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let view = self.view.clone();
        let panel = cx.entity().downgrade();
        let menu_panel = panel.clone();
        let has_public = self.public_key.is_some();
        let header = DetailsHeader::new(strings::PANEL_TITLE)
            .menu(move |menu: PopupMenu, _, cx| {
                let copy = menu_panel.clone();
                let remove = menu_panel.clone();
                menu.min_w(metrics::to_px(MENU_WIDTH, cx))
                    .item(
                        PopupMenuItem::element(move |_, cx| {
                            menu_item(
                                "kc-menu-copy-public",
                                strings::COPY_PUBLIC_KEY,
                                has_public,
                                cx,
                            )
                        })
                        .disabled(!has_public)
                        .on_click(move |_, window, cx| {
                            let _ = copy.update(cx, |panel, cx| panel.copy_public_key(window, cx));
                        }),
                    )
                    // Not built yet (D30, D33).
                    .item(
                        PopupMenuItem::element(|_, cx| {
                            menu_item("kc-menu-duplicate", strings::MENU_DUPLICATE, false, cx)
                        })
                        .disabled(true),
                    )
                    .item(
                        PopupMenuItem::element(|_, cx| {
                            menu_item("kc-menu-export-file", strings::MENU_EXPORT_FILE, false, cx)
                        })
                        .disabled(true),
                    )
                    .separator()
                    .item(
                        PopupMenuItem::element(|_, cx| {
                            menu_item("kc-menu-remove", strings::MENU_REMOVE, true, cx)
                        })
                        .on_click(move |_, window, cx| {
                            let _ = remove.update(cx, |panel, cx| panel.confirm_remove(window, cx));
                        }),
                    )
            })
            .on_collapse(move |_, _, cx| {
                let _ = panel.update(cx, |panel, cx| panel.close(cx));
            });

        // 1. Vault: only the personal vault for now.
        let vault = widgets::block("kc-key-block-vault", None, cx).child(value_row(
            "kc-key-vault",
            strings::FIELD_VAULT,
            h_flex()
                .gap(metrics::SPACE_2)
                .items_center()
                .child(crate::app::assets::vault_icon().small())
                .child(
                    div()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .child(crate::panels::strings::PERSONAL_VAULT),
                ),
            cx,
        ));

        // 2. Label (required, saved as in UI 1.3).
        let label_error = self
            .label_error
            .map(|reason| strings::reason("label", reason));
        let label = widgets::block("kc-key-block-label", None, cx).child(widgets::field(
            strings::FIELD_LABEL,
            widgets::text_input(
                &self.label,
                "kc-key-label",
                label_error.is_some(),
                window,
                cx,
            ),
            label_error,
            cx,
        ));

        // 3. Private key: hidden by default (the mandatory requirements).
        let unknown = key_type(&view.algorithm).is_none();
        let revealed = self.revealed_text();
        let shown = revealed.is_some();
        let toggle = if shown {
            panel_button("kc-key-hide", IconName::EyeOff, strings::HIDE, true, cx)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.hide(cx)))
        } else {
            panel_button("kc-key-show", IconName::Eye, strings::SHOW, true, cx)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.reveal(cx)))
        };
        let copy_private = panel_button(
            "kc-key-copy-private",
            IconName::Copy,
            strings::COPY_PRIVATE_KEY,
            true,
            cx,
        )
        .on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| this.copy_private_key(window, cx)),
        );
        let passphrase_note = if view.encrypted {
            Some((
                "kc-key-passphrase-note",
                if view.passphrase_saved {
                    strings::PASSPHRASE_SAVED
                } else {
                    strings::PASSPHRASE_ASKED
                },
            ))
        } else if unknown {
            None
        } else {
            // The unobtrusive hint of 7.3.
            Some(("kc-key-no-passphrase", strings::NOT_PROTECTED))
        };
        let private = widgets::block("kc-key-block-private", Some(strings::FIELD_PRIVATE_KEY), cx)
            .child(value_row(
                "kc-key-type",
                strings::FIELD_TYPE,
                type_line(&view, "kc-key-type-unknown".into(), cx).child(
                    div()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .child(key_type_text(&view)),
                ),
                cx,
            ))
            .when(unknown, |this| {
                this.child(
                    div()
                        .id("kc-key-unknown-reason")
                        .test_support()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(theme.muted_foreground)
                        .child(strings::UNKNOWN_REASON),
                )
            })
            .when_some(view.fingerprint.clone(), |this, fingerprint| {
                let copied = fingerprint.clone();
                this.child(value_row(
                    "kc-key-fingerprint-row",
                    strings::FIELD_FINGERPRINT,
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap(metrics::SPACE_2)
                        .items_center()
                        .child(
                            widgets::fingerprint_line(fingerprint)
                                .id("kc-key-fingerprint")
                                .test_support()
                                .flex_1()
                                .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                                .text_size(metrics::FONT_SIZE_DENSE)
                                .text_color(cx.theme().foreground),
                        )
                        .child(
                            ControlButton::new("kc-key-copy-fingerprint", ControlKind::Ghost)
                                .icon(Lucide::Copy)
                                .tooltip(strings::COPY_FINGERPRINT)
                                .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                                    this.copy_fingerprint(&copied, window, cx)
                                })),
                        ),
                    cx,
                ))
            })
            .child(match revealed {
                Some(text) => div()
                    .w_full()
                    .p(metrics::SPACE_3)
                    .rounded(metrics::RADIUS_CONTROL)
                    .bg(theme::tokens(cx).bg_input.hsla())
                    .child(mono("kc-key-private", text, cx))
                    .into_any_element(),
                None => h_flex()
                    .id("kc-key-hidden-note")
                    .test_support()
                    .gap(metrics::SPACE_2)
                    .items_center()
                    .text_size(metrics::FONT_SIZE_BODY)
                    .text_color(theme.muted_foreground)
                    .child(Icon::new(IconName::EyeOff).small())
                    .child(strings::HIDDEN_NOTE)
                    .into_any_element(),
            })
            .when(self.reveal_failed, |this| {
                this.child(widgets::error_text(strings::PRIVATE_KEY_NOT_SHOWN, cx))
            })
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .gap(metrics::SPACE_2)
                    .child(toggle)
                    .child(copy_private),
            )
            .when_some(passphrase_note, |this, (id, note)| {
                this.child(
                    div()
                        .id(id)
                        .test_support()
                        .text_size(metrics::FONT_SIZE_CAPTION)
                        .text_color(theme.muted_foreground)
                        .child(note),
                )
            });

        // 4. Public key (derived from the private key when the record has
        // none).
        let public = widgets::block("kc-key-block-public", Some(strings::FIELD_PUBLIC_KEY), cx)
            .child(match self.public_key.clone() {
                Some(line) => h_flex()
                    .w_full()
                    .gap(metrics::SPACE_2)
                    .items_start()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(mono("kc-key-public", line, cx)),
                    )
                    .child(
                        ControlButton::new("kc-key-copy-public", ControlKind::Ghost)
                            .icon(Lucide::Copy)
                            .tooltip(strings::COPY_PUBLIC_KEY)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                this.copy_public_key(window, cx)
                            })),
                    )
                    .into_any_element(),
                None => widgets::caption(strings::PUBLIC_KEY_MISSING, cx).into_any_element(),
            });

        // 4b. The certificate (stage 4, item 9): principals, validity, CA.
        let certificate = self.certificate.clone().map(|info| {
            let theme = cx.theme();
            let valid = format!(
                "{} \u{2013} {}",
                info.valid_after
                    .as_deref()
                    .map(crate::dates::timestamp_text)
                    .unwrap_or_default(),
                info.valid_before
                    .as_deref()
                    .map(crate::dates::timestamp_text)
                    .unwrap_or_else(|| strings::CERTIFICATE_FOREVER.to_owned())
            );
            let valid_utc = [info.valid_after.as_deref(), info.valid_before.as_deref()]
                .into_iter()
                .flatten()
                .filter_map(crate::dates::timestamp_tooltip)
                .collect::<Vec<_>>();
            let valid_utc = (!valid_utc.is_empty()).then(|| valid_utc.join(" \u{2013} "));
            let principals = if info.principals.is_empty() {
                strings::CERTIFICATE_ANY_PRINCIPAL.to_owned()
            } else {
                info.principals.join(", ")
            };
            let warning = match info.status {
                shellpp_core::keys::certificate::CertificateStatus::Expired => {
                    Some(strings::CERTIFICATE_EXPIRED)
                }
                shellpp_core::keys::certificate::CertificateStatus::NotYetValid => {
                    Some(strings::CERTIFICATE_NOT_YET)
                }
                shellpp_core::keys::certificate::CertificateStatus::Valid => None,
            };
            let danger = theme.danger;
            widgets::block(
                "kc-key-block-certificate",
                Some(strings::CERTIFICATE_TITLE),
                cx,
            )
            .when_some(warning, |this, warning| {
                this.child(
                    div()
                        .id("kc-key-certificate-warning")
                        .test_support()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .text_color(danger)
                        .child(warning),
                )
            })
            .child(value_row(
                "kc-key-certificate-id",
                strings::CERTIFICATE_KEY_ID,
                div().child(info.key_id.clone()),
                cx,
            ))
            .child(value_row(
                "kc-key-certificate-principals",
                strings::CERTIFICATE_PRINCIPALS,
                div().child(principals),
                cx,
            ))
            .child(value_row(
                "kc-key-certificate-valid",
                strings::CERTIFICATE_VALID,
                div()
                    .id("kc-key-certificate-valid-dates")
                    .child(valid)
                    .when_some(valid_utc, |this, utc| {
                        this.tooltip(crate::dates::utc_tooltip(utc))
                    }),
                cx,
            ))
            .child(value_row(
                "kc-key-certificate-ca",
                strings::CERTIFICATE_CA,
                widgets::fingerprint_line(info.ca_fingerprint.clone())
                    .font_family(crate::terminal::fonts::JETBRAINS_MONO)
                    .text_size(metrics::FONT_SIZE_DENSE),
                cx,
            ))
            .child(
                h_flex().child(
                    panel_button(
                        "kc-key-certificate-remove",
                        Lucide::Trash,
                        strings::CERTIFICATE_REMOVE,
                        true,
                        cx,
                    )
                    .on_click(cx.listener(
                        |this, _: &ClickEvent, window, cx| this.remove_certificate(window, cx),
                    )),
                ),
            )
        });

        // 5. Key export: Export to host (stage 4, item 8).
        let export =
            widgets::block("kc-key-block-export", Some(strings::EXPORT_TITLE), cx)
                .child(widgets::caption(strings::EXPORT_HELP, cx))
                .child(
                    h_flex().child(
                        panel_button(
                            "kc-key-export",
                            Lucide::Upload,
                            strings::EXPORT_TO_HOST,
                            self.public_key.is_some(),
                            cx,
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| this.open_export(window, cx),
                        )),
                    ),
                );

        v_flex()
            .id("kc-key-panel")
            .test_support()
            .size_full()
            .child(header)
            .child(
                panel_body("kc-key-panel-body")
                    .child(vault)
                    .child(label)
                    .child(private)
                    .child(public)
                    .children(certificate)
                    .child(export),
            )
    }
}

impl Render for GenerateForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (foreground, muted) = (cx.theme().foreground, cx.theme().muted_foreground);
        let generating = self.is_generating();
        let form = cx.entity().downgrade();
        let header = DetailsHeader::new(strings::GENERATE_TITLE).on_collapse(move |_, _, cx| {
            let _ = form.update(cx, |form, cx| form.cancel(cx));
        });
        let family = self.family();
        let kind = self.kind();
        let family_segment =
            |id: &'static str, label: &'static str, value: KeyFamily, cx: &mut Context<Self>| {
                widgets::segment(id, label, family == value, cx)
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.set_family(value, cx)),
                    )
                    .into_any_element()
            };
        let kind_segment = |id: &'static str,
                            label: &'static str,
                            value: KeyKind,
                            cx: &mut Context<Self>| {
            widgets::segment(id, label, kind == value, cx)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_kind(value, cx)))
                .into_any_element()
        };
        let families = widgets::segments([
            family_segment(
                "kc-gen-type-ed25519",
                strings::ED25519,
                KeyFamily::Ed25519,
                cx,
            ),
            family_segment("kc-gen-type-ecdsa", strings::ECDSA, KeyFamily::Ecdsa, cx),
            family_segment("kc-gen-type-rsa", strings::RSA, KeyFamily::Rsa, cx),
        ]);
        let parameters = match family {
            KeyFamily::Ed25519 => None,
            KeyFamily::Ecdsa => Some(widgets::field(
                strings::FIELD_CURVE,
                widgets::segments([
                    kind_segment("kc-gen-curve-p256", strings::P256, KeyKind::EcdsaP256, cx),
                    kind_segment("kc-gen-curve-p384", strings::P384, KeyKind::EcdsaP384, cx),
                    kind_segment("kc-gen-curve-p521", strings::P521, KeyKind::EcdsaP521, cx),
                ]),
                None,
                cx,
            )),
            KeyFamily::Rsa => Some(widgets::field(
                strings::FIELD_KEY_SIZE,
                widgets::segments([
                    kind_segment("kc-gen-bits-3072", strings::BITS_3072, KeyKind::Rsa3072, cx),
                    kind_segment("kc-gen-bits-4096", strings::BITS_4096, KeyKind::Rsa4096, cx),
                ]),
                None,
                cx,
            )),
        };
        let type_block = widgets::block("kc-gen-block-type", None, cx)
            .child(widgets::field(strings::FIELD_KEY_TYPE, families, None, cx))
            .children(parameters);
        let label_error = field_error(self.errors(), "label");
        let label_block = widgets::block("kc-gen-block-label", None, cx).child(widgets::field(
            strings::FIELD_LABEL,
            widgets::text_input(
                &self.label,
                "kc-gen-label",
                label_error.is_some(),
                window,
                cx,
            ),
            label_error,
            cx,
        ));
        let passphrase_error = field_error(self.errors(), "passphrase");
        let repeat_error = field_error(self.errors(), "passphrase_repeat");
        let has_passphrase = self.has_passphrase(cx);
        let passphrase_block = widgets::block("kc-gen-block-passphrase", None, cx)
            .child(widgets::field(
                strings::FIELD_PASSPHRASE,
                widgets::text_input_with(
                    Input::new(&self.passphrase).mask_toggle(),
                    &self.passphrase,
                    "kc-gen-passphrase",
                    passphrase_error.is_some(),
                    window,
                    cx,
                ),
                passphrase_error,
                cx,
            ))
            // A typo in a new passphrase would lock the key for good: it is
            // typed twice.
            .when(has_passphrase, |this| {
                this.child(widgets::field(
                    strings::FIELD_PASSPHRASE_REPEAT,
                    widgets::text_input_with(
                        Input::new(&self.repeat).mask_toggle(),
                        &self.repeat,
                        "kc-gen-passphrase-repeat",
                        repeat_error.is_some(),
                        window,
                        cx,
                    ),
                    repeat_error,
                    cx,
                ))
            })
            .when(has_passphrase, |this| {
                this.child(passphrase_choice(
                    "kc-gen",
                    self.save_passphrase(),
                    |form: &mut Self, save, cx| form.set_save_passphrase(save, cx),
                    cx,
                ))
            });
        let progress = generating.then(|| {
            h_flex()
                .id("kc-gen-progress")
                .test_support()
                .w_full()
                .gap(metrics::SPACE_2)
                .items_center()
                .flex_wrap()
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(foreground)
                .child(Icon::new(Lucide::LoaderCircle).small().text_color(muted))
                .child(strings::GENERATING)
                .when(family == KeyFamily::Rsa, |this| {
                    this.child(widgets::caption(strings::GENERATING_RSA, cx))
                })
        });
        v_flex()
            .id("kc-generate")
            .test_support()
            .size_full()
            .child(header)
            .child(
                panel_body("kc-generate-body")
                    .child(type_block)
                    .child(label_block)
                    .child(passphrase_block)
                    .children(progress),
            )
            .child(form_footer(
                "kc-gen-cancel",
                "kc-gen-submit",
                if generating {
                    strings::GENERATING
                } else {
                    strings::GENERATE
                },
                !generating,
                |form: &mut Self, _, cx| form.cancel(cx),
                |form: &mut Self, window, cx| form.generate(window, cx),
                cx,
            ))
    }
}

impl Render for ImportForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (danger, warning, muted) = (
            cx.theme().danger,
            cx.theme().warning,
            cx.theme().muted_foreground,
        );
        let tokens = theme::tokens(cx);
        let form = cx.entity().downgrade();
        let (title, hint) = if self.is_security_key() {
            (
                strings::IMPORT_SECURITY_KEY_TITLE,
                strings::IMPORT_SECURITY_KEY_HINT,
            )
        } else {
            (strings::IMPORT_TITLE, strings::IMPORT_HINT)
        };
        let header = DetailsHeader::new(title).on_collapse(move |_, _, cx| {
            let _ = form.update(cx, |form, cx| form.cancel(cx));
        });
        let importing = self.is_importing();

        // "Import from ~/.ssh": the private key files of the folder.
        let ssh = self.ssh_scan().cloned().map(|scan| {
            let body = match scan {
                SshScan::Scanning => widgets::caption(strings::CHECKING, cx).into_any_element(),
                SshScan::Missing => {
                    widgets::caption(strings::SSH_FOLDER_MISSING, cx).into_any_element()
                }
                SshScan::Found(files) if files.is_empty() => {
                    widgets::caption(strings::SSH_FOLDER_EMPTY, cx).into_any_element()
                }
                SshScan::Found(files) => v_flex()
                    .w_full()
                    .gap(metrics::rpx(2.))
                    .children(files.into_iter().map(|file| {
                        let path = file.path.clone();
                        widgets::dropdown_item(
                            ssh_file_id(&file.name),
                            Some(Icon::new(Lucide::Key).small().into_any_element()),
                            file.name.clone(),
                            None,
                            false,
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _: &ClickEvent, window, cx| {
                                this.load_file(path.clone(), window, cx)
                            },
                        ))
                    }))
                    .into_any_element(),
            };
            widgets::block("kc-import-ssh-list", Some(strings::SSH_FOLDER), cx).child(body)
        });

        // The key text: paste, Choose file, or drop a file on the block.
        let text_error = field_error(self.errors(), "private_key");
        let drop_bg = tokens.accent_subtle.hsla();
        let textarea = Textarea::new(&self.text)
            .px(metrics::to_px(metrics::SPACE_3, cx) - gpui_kit::component::Size::Medium.input_px())
            .text_size(metrics::FONT_SIZE_DENSE)
            .font_family(crate::terminal::fonts::JETBRAINS_MONO)
            .bg(tokens.bg_input.hsla())
            .rounded(metrics::RADIUS_CONTROL)
            .when(text_error.is_some(), |this| this.border_color(danger));
        let inspection = match self.inspection().clone() {
            Inspection::Empty => None,
            Inspection::Checking => Some(
                div()
                    .id("kc-import-checking")
                    .test_support()
                    .child(widgets::caption(strings::CHECKING, cx))
                    .into_any_element(),
            ),
            Inspection::Key { format, encrypted } => Some(
                value_row(
                    "kc-import-format",
                    strings::FORMAT,
                    div()
                        .text_size(metrics::FONT_SIZE_BODY)
                        .child(strings::format_text(format, encrypted)),
                    cx,
                )
                .into_any_element(),
            ),
            Inspection::Problem(problem) => Some(
                v_flex()
                    .id("kc-import-unknown")
                    .test_support()
                    .w_full()
                    .gap(metrics::SPACE_1)
                    .child(widgets::caption(strings::FIELD_TYPE, cx))
                    .child(
                        h_flex()
                            .gap(metrics::SPACE_1)
                            .items_center()
                            .child(
                                Icon::new(IconName::TriangleAlert)
                                    .xsmall()
                                    .text_color(warning),
                            )
                            .child(
                                div()
                                    .text_size(metrics::FONT_SIZE_BODY)
                                    .child(strings::TYPE_UNKNOWN),
                            ),
                    )
                    .child(
                        div()
                            .id("kc-import-reason")
                            .test_support()
                            .text_size(metrics::FONT_SIZE_CAPTION)
                            .text_color(muted)
                            .child(strings::problem(&problem)),
                    )
                    .into_any_element(),
            ),
        };
        let key_block =
            widgets::block("kc-import-block-key", Some(strings::FIELD_PRIVATE_KEY), cx)
                .child(widgets::caption(hint, cx))
                .child(
                    div()
                        .id("kc-import-text")
                        .test_support()
                        .relative()
                        .w_full()
                        .child(theme::controls::muted_placeholder(&self.text, cx))
                        .child(textarea),
                )
                .when_some(text_error, |this, error| {
                    this.child(widgets::error_text(error, cx))
                })
                .when_some(self.file_problem(), |this, problem| {
                    this.child(
                        div()
                            .id("kc-import-file-error")
                            .test_support()
                            .child(widgets::error_text(problem.text(), cx)),
                    )
                })
                .child(
                    h_flex().child(
                        panel_button(
                            "kc-import-file",
                            Lucide::FolderOpen,
                            strings::CHOOSE_FILE,
                            true,
                            cx,
                        )
                        .on_click(cx.listener(
                            |this, _: &ClickEvent, window, cx| this.choose_file(window, cx),
                        )),
                    ),
                )
                .children(inspection);

        let label_error = field_error(self.errors(), "label");
        let label_block = widgets::block("kc-import-block-label", None, cx).child(widgets::field(
            strings::FIELD_LABEL,
            widgets::text_input(
                &self.label,
                "kc-import-label",
                label_error.is_some(),
                window,
                cx,
            ),
            label_error,
            cx,
        ));
        let passphrase = self.needs_passphrase().then(|| {
            let error = field_error(self.errors(), "passphrase");
            widgets::block("kc-import-block-passphrase", None, cx)
                .child(widgets::caption(strings::NEEDS_PASSPHRASE, cx))
                .child(widgets::field(
                    strings::FIELD_PASSPHRASE,
                    widgets::text_input_with(
                        Input::new(&self.passphrase).mask_toggle(),
                        &self.passphrase,
                        "kc-import-passphrase",
                        error.is_some(),
                        window,
                        cx,
                    ),
                    error,
                    cx,
                ))
                .child(passphrase_choice(
                    "kc-import",
                    self.save_passphrase(),
                    |form: &mut Self, save, cx| form.set_save_passphrase(save, cx),
                    cx,
                ))
        });
        let can_import = self.can_import(cx);
        v_flex()
            .id("kc-import")
            .test_support()
            .size_full()
            // A file dropped anywhere on the form (UI 7.1).
            .drag_over::<ExternalPaths>(move |style, _, _, _| style.bg(drop_bg))
            .on_drop(cx.listener(|this, paths: &ExternalPaths, window, cx| {
                if let Some(path) = paths.paths().first() {
                    this.load_file(path.clone(), window, cx);
                }
            }))
            .child(header)
            .child(
                panel_body("kc-import-body")
                    .children(ssh)
                    .child(key_block)
                    .child(label_block)
                    .children(passphrase),
            )
            .child(form_footer(
                "kc-import-cancel",
                "kc-import-submit",
                if importing {
                    strings::IMPORTING
                } else {
                    strings::IMPORT
                },
                can_import,
                |form: &mut Self, _, cx| form.cancel(cx),
                |form: &mut Self, window, cx| form.import(window, cx),
                cx,
            ))
    }
}
