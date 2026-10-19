//! Rendering of the Hosts screen: search bar, action bar, breadcrumbs, the
//! card grid or list, the empty states and the drag preview.

use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, Bounds, ClickEvent, Context, FontWeight,
    InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton, ParentElement, Render,
    ScrollHandle, SharedString, StatefulInteractiveElement, Styled, Window, canvas, div, fill,
    linear_color_stop, linear_gradient, point, prelude::FluentBuilder as _, px, size,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, h_flex,
    v_flex, v_virtual_list,
};
use shellpp_core::service::dto::TagColor;
use uuid::Uuid;

use super::controls::{self, ControlButton, ControlKind, ICON_SIZE};
use super::menu::{self, CommandHandler};
use super::{
    DraggedCards, HostsEvent, HostsScreen, Row, SessionKind, group_card_id, host_card_id, layout,
    strings, tag_chip_id,
};
use crate::state::hosts_view::{HostsLayout, HostsSort};
use crate::theme::{self, metrics, palette};

/// Height of a section heading row ("Groups", "Hosts").
const HEADING_HEIGHT: f32 = 40.;
/// Horizontal padding of the content (`--space-5`).
const CONTENT_PADDING: gpui::Pixels = metrics::SPACE_5;
/// Tile of a list row (the 36 px card tile does not fit a 44 px row).
const LIST_TILE_SIZE: gpui::Pixels = px(28.);
/// Height of the Connect button inside the 36 px search field.
const INNER_BUTTON_HEIGHT: gpui::Pixels = px(28.);

/// The floating label under the pointer while cards are dragged.
pub(super) struct DragPreview {
    label: SharedString,
}

impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        div()
            .px(metrics::SPACE_3)
            .py(metrics::SPACE_1)
            .rounded(metrics::RADIUS_CONTROL)
            .bg(theme.popover)
            .border_1()
            .border_color(theme::tokens(cx).border_strong.hsla())
            .text_color(theme.popover_foreground)
            .text_size(metrics::FONT_SIZE_BODY)
            .child(self.label.clone())
    }
}

/// The icon tile: the user's color or `--tag-graphite`, a white monochrome
/// icon, never a letter (owner's audit item 3).
fn tile(color: TagColor, icon: Lucide, tile_size: gpui::Pixels) -> impl IntoElement {
    div()
        .flex_none()
        .size(tile_size)
        .rounded(metrics::RADIUS_TILE)
        .bg(palette::tag_color(color).hsla())
        .flex()
        .items_center()
        .justify_center()
        .child(
            Icon::new(icon)
                .with_size(ICON_SIZE)
                .text_color(palette::TEXT_ON_TAG.hsla()),
        )
}

impl HostsScreen {
    // -- Search bar (UI 2.1) --------------------------------------------------------

    fn render_search(&self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let focused = self.search_focused(window, cx);
        let enabled = self.connect_enabled();
        let connect = ControlButton::new(
            "quick-connect",
            if enabled {
                ControlKind::Accent
            } else {
                ControlKind::Neutral
            },
        )
        .label(strings::CONNECT)
        .height(INNER_BUTTON_HEIGHT)
        .disabled(!enabled)
        .on_click(
            cx.listener(|this, _: &ClickEvent, window, cx| this.connect_from_search(window, cx)),
        );
        h_flex()
            .id("search-field")
            .test_support()
            .relative()
            .w_full()
            .h(metrics::HEIGHT_CONTROL)
            .flex_none()
            .gap(metrics::SPACE_2)
            .pl(metrics::SPACE_3)
            .pr(metrics::SPACE_1)
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
                // Our frame draws the border and the single focus ring; the
                // kit input draws none of its own (no double outline).
                // The kit's default input text is `text_sm` (12.25 px with
                // our 14 px rem); the field text is body size (14 px).
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .child(theme::controls::muted_placeholder(&self.search, cx))
                    .child(
                        // `pl_0`: the kit pads a single-line input by 10 px,
                        // which would put 18 px between the icon and the
                        // text instead of the row's 8 px gap.
                        Input::new(&self.search)
                            .id("search")
                            .appearance(false)
                            .cleanable(true)
                            .pl_0()
                            .text_size(metrics::FONT_SIZE_BODY),
                    ),
            )
            .child(connect)
            .when(focused, |this| {
                this.child(controls::focus_ring(metrics::RADIUS_CONTROL, px(1.), cx))
            })
    }

    // -- Action bar (UI 2.2) -----------------------------------------------------------

    fn render_action_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let screen = cx.entity().downgrade();
        let new_host = ControlButton::new("new-host", ControlKind::Neutral)
            .icon(Lucide::Plus)
            .label(strings::NEW_HOST)
            .rounded_sides(true, false)
            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(HostsEvent::NewHost)));
        let new_host_menu = ControlButton::new("new-host-menu", ControlKind::Neutral)
            .icon(IconName::ChevronDown)
            .tooltip(strings::NEW_HOST_MENU)
            .rounded_sides(false, true)
            .dropdown_menu(move |menu, _, _| {
                let new_group = screen.clone();
                let import = screen.clone();
                menu.min_w(px(200.))
                    .item(
                        PopupMenuItem::element(|_, cx| {
                            menu_text("new-group", strings::NEW_GROUP, cx)
                        })
                        .on_click(move |_, _, cx| {
                            let _ = new_group.update(cx, |_, cx| cx.emit(HostsEvent::NewGroup));
                        }),
                    )
                    .item(
                        PopupMenuItem::element(|_, cx| menu_text("import", strings::IMPORT, cx))
                            .on_click(move |_, window, cx| {
                                let _ = import.update(cx, |_, cx| {
                                    super::dialogs::open_placeholder(
                                        "import-dialog",
                                        strings::IMPORT_TITLE,
                                        window,
                                        cx,
                                    )
                                });
                            }),
                    )
            });
        let split = h_flex()
            .flex_none()
            .child(new_host)
            .child(div().w(px(1.)).h(metrics::HEIGHT_CONTROL).bg(theme.border))
            .child(new_host_menu);
        let terminal = ControlButton::new("terminal", ControlKind::Neutral)
            .icon(Lucide::SquareTerminal)
            .label(strings::TERMINAL)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.open_local(SessionKind::Terminal, cx)
            }));
        let serial = ControlButton::new("serial", ControlKind::Neutral)
            .icon(Lucide::Usb)
            .label(strings::SERIAL)
            .on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| this.open_local(SessionKind::Serial, cx)),
            );

        h_flex()
            .id("action-bar")
            .test_support()
            .w_full()
            .gap(metrics::SPACE_2)
            .child(split)
            .child(terminal)
            .child(serial)
            .child(div().flex_1().min_w(metrics::SPACE_2))
            .child(self.render_view_toggle(cx))
            .children(self.render_tag_chips(cx))
            .child(self.render_sort_menu(cx))
    }

    fn render_view_toggle(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let button = |id: &'static str, icon: Lucide, tooltip: &'static str, mode: HostsLayout| {
            ControlButton::new(id, ControlKind::Ghost)
                .icon(icon)
                .tooltip(tooltip)
                .selected(self.layout == mode)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.set_layout(mode, cx)))
        };
        h_flex()
            .id("view-toggle")
            .test_support()
            .flex_none()
            .gap(metrics::SPACE_1)
            .child(button(
                "view-tiles",
                Lucide::LayoutGrid,
                strings::VIEW_TILES,
                HostsLayout::Tiles,
            ))
            .child(button(
                "view-list",
                Lucide::List,
                strings::VIEW_LIST,
                HostsLayout::List,
            ))
    }

    /// The tag filter (UI 2.2, design-system 3.4): one chip per tag, the
    /// active ones filled; the strip scrolls sideways when it is too long.
    fn render_tag_chips(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        if self.tags.is_empty() {
            return None;
        }
        let chips = self.tags.iter().map(|tag| {
            let active = self.filter.has_tag(tag);
            let tag = tag.clone();
            controls::tag_chip(tag_chip_id(&tag), tag.clone(), active, cx)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_tag(&tag, cx)))
        });
        let strip = h_flex()
            .id("tag-filter")
            .test_support()
            .flex_1()
            .min_w_0()
            .overflow_x_scroll()
            .track_scroll(&self.tag_scroll)
            .gap(metrics::SPACE_1)
            .child(
                Icon::new(Lucide::Tag)
                    .with_size(ICON_SIZE)
                    .text_color(cx.theme().muted_foreground)
                    .flex_none(),
            )
            .children(chips);
        // No fixed cap: the strip takes the room the action bar has left
        // and only scrolls (with edge fades) when that room runs out.
        Some(
            h_flex()
                .relative()
                .min_w_0()
                .child(strip)
                .child(tag_strip_fades(self.tag_scroll.clone(), cx)),
        )
    }

    fn render_sort_menu(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = cx.entity().downgrade();
        let current = self.sort;
        ControlButton::new("sort", ControlKind::Ghost)
            .icon(Lucide::ArrowUpDown)
            .tooltip(strings::SORT)
            .dropdown_menu(move |mut menu, _, _| {
                menu = menu.min_w(px(200.));
                for sort in HostsSort::ALL {
                    let (id, label) = match sort {
                        HostsSort::Manual => ("sort-manual", strings::SORT_MANUAL),
                        HostsSort::Name => ("sort-name", strings::SORT_NAME),
                        HostsSort::Created => ("sort-created", strings::SORT_CREATED),
                        HostsSort::Modified => ("sort-modified", strings::SORT_MODIFIED),
                    };
                    let screen = screen.clone();
                    menu = menu.item(
                        PopupMenuItem::element(move |_, cx| menu_text(id, label, cx))
                            .checked(sort == current)
                            .on_click(move |_, _, cx| {
                                let _ = screen.update(cx, |screen, cx| screen.set_sort(sort, cx));
                            }),
                    );
                }
                menu
            })
    }

    // -- Breadcrumbs (UI 2.3) ---------------------------------------------------------

    /// "Hosts › A › B", only inside a group; every level but the current one
    /// opens on click and takes dropped cards.
    fn render_breadcrumbs(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let path: Vec<(Uuid, String)> = self
            .state
            .read(cx)
            .location_path()
            .into_iter()
            .map(|group| (group.record_id, group.name.clone()))
            .collect();
        if path.is_empty() {
            return None;
        }
        let theme = cx.theme();
        let secondary = theme.muted_foreground;
        let primary = theme.foreground;
        let drop_bg = theme.drop_target;
        let last = path.len() - 1;
        let crumb = |id: SharedString,
                     target: Option<Uuid>,
                     label: SharedString,
                     cx: &mut Context<Self>| {
            div()
                .id(id)
                .test_support()
                .px(metrics::SPACE_1)
                .rounded(metrics::RADIUS_CONTROL)
                .cursor_pointer()
                .text_color(secondary)
                .hover(move |style| style.text_color(primary))
                .drag_over::<DraggedCards>(move |style, _, _, _| style.bg(drop_bg))
                .on_drop(cx.listener(move |this, dragged: &DraggedCards, _, cx| {
                    this.drop_into(dragged, target, cx)
                }))
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.state
                        .update(cx, |state, cx| state.set_location(target, cx))
                }))
                .child(label)
        };
        // Each crumb has 4 px of padding for its hover and drop background;
        // the row starts 4 px earlier so the first label sits on the 24 px
        // content edge, aligned with the search bar and the cards.
        let mut crumbs = h_flex()
            .id("breadcrumbs")
            .test_support()
            .pl(CONTENT_PADDING - metrics::SPACE_1)
            .pr(CONTENT_PADDING)
            .pb(metrics::SPACE_2)
            .gap(metrics::SPACE_1)
            .text_size(metrics::FONT_SIZE_BODY)
            .child(crumb("crumb-root".into(), None, strings::HOSTS.into(), cx));
        for (index, (id, name)) in path.into_iter().enumerate() {
            crumbs = crumbs.child(
                div()
                    .text_color(secondary)
                    .child(strings::BREADCRUMB_SEPARATOR),
            );
            let element_id: SharedString = format!("crumb-{id}").into();
            crumbs = crumbs.child(if index == last {
                div()
                    .id(element_id)
                    .test_support()
                    .px(metrics::SPACE_1)
                    .text_color(primary)
                    .font_weight(FontWeight::MEDIUM)
                    .child(name)
                    .into_any_element()
            } else {
                crumb(element_id, Some(id), name.into(), cx).into_any_element()
            });
        }
        Some(crumbs)
    }

    // -- Content (UI 2.3) ----------------------------------------------------------------

    /// Rebuilds the virtual-list rows for a content width.
    fn rebuild_rows(&mut self, width: f32) {
        let columns = match self.layout {
            HostsLayout::Tiles => layout::columns_for(width),
            HostsLayout::List => 1,
        };
        let key = (
            columns,
            self.listing.groups.len() * 100_000 + self.listing.hosts.len(),
            width as i32,
        );
        if key == self.layout_key {
            return;
        }
        self.layout_key = key;
        self.columns = columns;
        self.card_width = match self.layout {
            HostsLayout::Tiles => layout::card_width(width, columns),
            HostsLayout::List => width,
        };
        let mut rows = Vec::new();
        if !self.listing.groups.is_empty() {
            rows.push(Row::GroupsHeading);
            rows.extend(
                (0..self.listing.groups.len())
                    .step_by(columns)
                    .map(Row::Groups),
            );
        }
        if !self.listing.hosts.is_empty() {
            rows.push(Row::HostsHeading);
            rows.extend(
                (0..self.listing.hosts.len())
                    .step_by(columns)
                    .map(Row::Hosts),
            );
        }
        let item_row = match self.layout {
            HostsLayout::Tiles => {
                metrics::HEIGHT_HOST_CARD.as_f32() + metrics::HOST_CARD_GAP.as_f32()
            }
            HostsLayout::List => metrics::HEIGHT_ROW.as_f32(),
        };
        self.row_sizes = Rc::new(
            rows.iter()
                .map(|row| {
                    let height = match row {
                        Row::GroupsHeading | Row::HostsHeading => HEADING_HEIGHT,
                        Row::Groups(_) | Row::Hosts(_) => item_row,
                    };
                    size(px(width), px(height))
                })
                .collect(),
        );
        self.rows = rows;
    }

    fn render_heading(&self, id: &'static str, label: &'static str, cx: &App) -> AnyElement {
        div()
            .id(id)
            .test_support()
            .h(px(HEADING_HEIGHT))
            .flex()
            .items_end()
            .pb(metrics::SPACE_2)
            .text_size(metrics::FONT_SIZE_HEADING)
            .font_weight(FontWeight::SEMIBOLD)
            .text_color(cx.theme().foreground)
            .child(label)
            .into_any_element()
    }

    /// The frame of a grid card or a list row, with selection, hover,
    /// pressed and keyboard-focus states.
    fn item_frame(
        &self,
        id: SharedString,
        record: Uuid,
        window: &Window,
        cx: &App,
    ) -> impl StatefulInteractiveElement + ParentElement + Styled + IntoElement + 'static {
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let selection = self.state.read(cx).selection();
        let selected = selection.contains(&record);
        let keyboard_cursor = selection.anchor() == Some(record)
            && self.focus.is_focused(window)
            && window.last_input_was_keyboard();
        let hover = theme.secondary_hover;
        let pressed = theme.secondary_active;
        let frame = div()
            .id(id)
            .test_support()
            .relative()
            .flex()
            .items_center()
            .cursor_default();
        let frame = match self.layout {
            HostsLayout::Tiles => frame
                .flex_none()
                .w(px(self.card_width))
                .h(metrics::HEIGHT_HOST_CARD)
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
                .hover(move |style| style.bg(hover))
                .active(move |style| style.bg(pressed)),
            HostsLayout::List => frame
                .w_full()
                .h(metrics::HEIGHT_ROW)
                .gap(metrics::SPACE_3)
                .px(metrics::SPACE_3)
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
                }),
        };
        let radius = match self.layout {
            HostsLayout::Tiles => metrics::RADIUS_CARD,
            HostsLayout::List => gpui::Pixels::ZERO,
        };
        frame.when(keyboard_cursor, |this| {
            this.child(controls::focus_ring(radius, px(1.), cx))
        })
    }

    fn render_texts(&self, title: String, subtitle: String, cx: &App) -> AnyElement {
        let theme = cx.theme();
        match self.layout {
            HostsLayout::Tiles => v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.))
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
                .into_any_element(),
            HostsLayout::List => h_flex()
                .flex_1()
                .min_w_0()
                .gap(metrics::SPACE_3)
                .text_size(metrics::FONT_SIZE_DENSE)
                .child(
                    div()
                        .w_2_5()
                        .min_w_0()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.foreground)
                        .truncate()
                        .child(title),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(theme.muted_foreground)
                        .truncate()
                        .child(subtitle),
                )
                .into_any_element(),
        }
    }

    fn tile_size(&self) -> gpui::Pixels {
        match self.layout {
            HostsLayout::Tiles => metrics::HOST_TILE_SIZE,
            HostsLayout::List => LIST_TILE_SIZE,
        }
    }

    fn menu_handler(&self, records: Vec<uuid::Uuid>, cx: &Context<Self>) -> CommandHandler {
        let screen = cx.entity().downgrade();
        Rc::new(move |command, window, cx| {
            let records = records.clone();
            let _ = screen.update(cx, |screen, cx| {
                screen.run_command(command, records, window, cx)
            });
        })
    }

    fn render_host_card(&self, ix: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let host = &self.hosts[ix];
        let id = host.record_id;
        // Inside a folder the group is known; filtered results come from
        // several folders, so they name the group (tags otherwise).
        let group_name = if self.filter.is_active() {
            host.group_id
                .and_then(|group| self.group(group))
                .map(|group| group.name.clone())
        } else {
            None
        };
        let subtitle = layout::subtitle(host, group_name.as_deref());
        let title = host.title().to_owned();
        let screen = cx.entity().downgrade();
        let drag = self.drag_value(id, cx);
        let drop_bg = cx.theme().drop_target;
        let reorder_screen = cx.entity().downgrade();
        self.item_frame(host_card_id(id), id, window, cx)
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.on_host_click(id, event, window, cx)
            }))
            .on_drag(drag, |dragged: &DraggedCards, _, _, cx| {
                cx.new(|_| DragPreview {
                    label: dragged.label.clone(),
                })
            })
            .can_drop(move |value, _, cx| {
                let Some(dragged) = value.downcast_ref::<DraggedCards>() else {
                    return false;
                };
                reorder_screen
                    .upgrade()
                    .is_some_and(|screen| screen.read(cx).can_reorder(dragged, id))
            })
            .drag_over::<DraggedCards>(move |style, _, _, _| style.bg(drop_bg))
            .on_drop(cx.listener(move |this, dragged: &DraggedCards, _, cx| {
                this.drop_on_host(dragged, id, cx)
            }))
            .child(tile(
                host.color,
                layout::os_icon(host.os_family.as_deref()),
                self.tile_size(),
            ))
            .child(self.render_texts(title, subtitle, cx))
            .context_menu(move |menu, window, cx| {
                let Some(screen) = screen.upgrade() else {
                    return menu;
                };
                let (entries, handler) = screen.update(cx, |screen, cx| {
                    let records = screen.prepare_menu(id, cx);
                    let targets = screen.move_targets(&records, cx);
                    (
                        menu::host_menu(records.len(), &targets),
                        screen.menu_handler(records, cx),
                    )
                });
                menu::build_popup(menu, entries, handler, window, cx)
            })
            .into_any_element()
    }

    fn render_group_card(&self, ix: usize, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let group = &self.groups[ix];
        let id = group.record_id;
        let screen = cx.entity().downgrade();
        let drop_screen = cx.entity().downgrade();
        let drag = self.drag_value(id, cx);
        let drop_bg = cx.theme().drop_target;
        self.item_frame(group_card_id(id), id, window, cx)
            .on_click(cx.listener(move |this, event: &ClickEvent, window, cx| {
                this.on_group_click(id, event, window, cx)
            }))
            .on_drag(drag, |dragged: &DraggedCards, _, _, cx| {
                cx.new(|_| DragPreview {
                    label: dragged.label.clone(),
                })
            })
            .can_drop(move |value, _, cx| {
                let Some(dragged) = value.downcast_ref::<DraggedCards>() else {
                    return false;
                };
                !dragged.ids.contains(&id)
                    && drop_screen
                        .upgrade()
                        .is_some_and(|screen| screen.read(cx).can_drop_into(dragged, Some(id)))
            })
            .drag_over::<DraggedCards>(move |style, _, _, _| style.bg(drop_bg))
            .on_drop(cx.listener(move |this, dragged: &DraggedCards, _, cx| {
                this.drop_into(dragged, Some(id), cx)
            }))
            .child(tile(group.color, Lucide::Folder, self.tile_size()))
            .child(self.render_texts(
                group.name.clone(),
                strings::host_count(group.host_count as usize),
                cx,
            ))
            .context_menu(move |menu, window, cx| {
                let Some(screen) = screen.upgrade() else {
                    return menu;
                };
                let (entries, handler) = screen.update(cx, |screen, cx| {
                    let records = screen.prepare_menu(id, cx);
                    let targets = screen.move_targets(&records, cx);
                    (
                        menu::group_menu(records.len(), &targets),
                        screen.menu_handler(records, cx),
                    )
                });
                menu::build_popup(menu, entries, handler, window, cx)
            })
            .into_any_element()
    }

    fn render_row(&self, row: Row, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let columns = self.columns;
        let tiles = self.layout == HostsLayout::Tiles;
        let container = || {
            h_flex()
                .w_full()
                .when(tiles, |this| {
                    this.h(metrics::HEIGHT_HOST_CARD + metrics::HOST_CARD_GAP)
                        .pb(metrics::HOST_CARD_GAP)
                        .gap(metrics::HOST_CARD_GAP)
                })
                .when(!tiles, |this| this.h(metrics::HEIGHT_ROW))
        };
        match row {
            Row::GroupsHeading => self.render_heading("heading-groups", strings::GROUPS, cx),
            Row::HostsHeading => self.render_heading("heading-hosts", strings::HOSTS, cx),
            Row::Groups(start) => {
                let end = (start + columns).min(self.listing.groups.len());
                let items: Vec<usize> = self.listing.groups[start..end].to_vec();
                let mut row = container();
                for item in items {
                    row = row.child(self.render_group_card(item, window, cx));
                }
                row.into_any_element()
            }
            Row::Hosts(start) => {
                let end = (start + columns).min(self.listing.hosts.len());
                let items: Vec<usize> = self.listing.hosts[start..end].to_vec();
                let mut row = container();
                for item in items {
                    row = row.child(self.render_host_card(item, window, cx));
                }
                row.into_any_element()
            }
        }
    }

    // -- Empty states (UI 2.3) ---------------------------------------------------------

    fn render_empty(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let vault_empty = self.hosts.is_empty() && self.groups.is_empty();
        let (id, title, body): (&'static str, &'static str, Option<&'static str>) =
            if self.filter.is_active() {
                ("hosts-no-matches", strings::NO_MATCHES, None)
            } else if vault_empty {
                (
                    "hosts-empty",
                    strings::EMPTY_TITLE,
                    Some(strings::EMPTY_BODY),
                )
            } else {
                ("group-empty", strings::EMPTY_GROUP, None)
            };
        let actions = (vault_empty && !self.filter.is_active()).then(|| {
            h_flex()
                .gap(metrics::SPACE_2)
                .pt(metrics::SPACE_2)
                .child(
                    ControlButton::new("create-host", ControlKind::Neutral)
                        .icon(Lucide::Plus)
                        .label(strings::CREATE_HOST)
                        .on_click(
                            cx.listener(|_, _: &ClickEvent, _, cx| cx.emit(HostsEvent::NewHost)),
                        ),
                )
                .child(
                    ControlButton::new("import-ssh-config", ControlKind::Neutral)
                        .label(strings::IMPORT_SSH_CONFIG)
                        .on_click(cx.listener(|_, _: &ClickEvent, window, cx| {
                            super::dialogs::open_placeholder(
                                "import-dialog",
                                strings::IMPORT_TITLE,
                                window,
                                cx,
                            )
                        })),
                )
        });
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
            .children(actions)
            .into_any_element()
    }

    /// The content width for the columns: measured after the previous
    /// layout; the first frame estimates it from the window.
    fn content_width(&self, window: &Window) -> f32 {
        let outer = self
            .measured_width
            .unwrap_or_else(|| (window.viewport_size().width - metrics::WIDTH_SIDEBAR).as_f32());
        (outer - CONTENT_PADDING.as_f32() * 2.).max(metrics::HOST_CARD_MIN_WIDTH.as_f32())
    }
}

/// Width of the fade over a clipped edge of the tag filter strip.
const TAG_FADE_WIDTH: gpui::Pixels = metrics::SPACE_5;

/// Fades the edges of the tag filter strip where chips are cut off
/// (`--bg-surface` over the chips), so a clipped chip reads as "scroll for
/// more" instead of a broken border. The strip prepaints before this
/// overlay, so its scroll handle already holds this frame's overflow.
fn tag_strip_fades(handle: ScrollHandle, cx: &App) -> impl IntoElement {
    let surface = cx.theme().colors.list;
    canvas(
        move |_, _, _| {
            let scrolled = -handle.offset().x;
            let hidden_right = handle.max_offset().x - scrolled;
            (scrolled > px(0.5), hidden_right > px(0.5))
        },
        move |bounds, (left, right), window, _| {
            let clear = surface.opacity(0.);
            let height = bounds.size.height;
            if left {
                let area = Bounds::new(bounds.origin, size(TAG_FADE_WIDTH, height));
                let fade = linear_gradient(
                    270.,
                    linear_color_stop(clear, 0.),
                    linear_color_stop(surface, 1.),
                );
                window.paint_quad(fill(area, fade));
            }
            if right {
                let origin = point(bounds.right() - TAG_FADE_WIDTH, bounds.origin.y);
                let area = Bounds::new(origin, size(TAG_FADE_WIDTH, height));
                let fade = linear_gradient(
                    90.,
                    linear_color_stop(clear, 0.),
                    linear_color_stop(surface, 1.),
                );
                window.paint_quad(fill(area, fade));
            }
        },
    )
    .absolute()
    .top_0()
    .left_0()
    .size_full()
}

/// A plain text menu item with a stable id.
fn menu_text(id: &'static str, label: &'static str, cx: &App) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .flex_1()
        .text_size(metrics::FONT_SIZE_BODY)
        .text_color(cx.theme().foreground)
        .child(label)
        .into_any_element()
}

impl Render for HostsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = self.content_width(window);
        self.rebuild_rows(width);
        let screen = cx.entity().downgrade();
        // Measures the content area (the details panel narrows it); a new
        // width relays the grid out on the next frame.
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

        let content = if self.rows.is_empty() {
            self.render_empty(cx)
        } else {
            div()
                .size_full()
                .px(CONTENT_PADDING)
                .child(
                    v_virtual_list(
                        cx.entity(),
                        "host-grid",
                        self.row_sizes.clone(),
                        |this: &mut Self, range, window, cx| {
                            range
                                .map(|ix| {
                                    let row = this.rows[ix];
                                    this.render_row(row, window, cx)
                                })
                                .collect::<Vec<_>>()
                        },
                    )
                    .track_scroll(&self.scroll)
                    .size_full(),
                )
                .into_any_element()
        };

        v_flex()
            .id("hosts-screen")
            .test_support()
            .size_full()
            .bg(cx.theme().colors.list)
            .text_color(cx.theme().foreground)
            .child(
                v_flex()
                    .flex_none()
                    .gap(metrics::SPACE_3)
                    .px(CONTENT_PADDING)
                    .pt(metrics::SPACE_4)
                    .pb(metrics::SPACE_3)
                    .child(self.render_search(window, cx))
                    .child(self.render_action_bar(cx)),
            )
            .children(self.render_breadcrumbs(cx))
            .child(
                div()
                    .id("hosts-content")
                    .test_support()
                    .key_context("HostsGrid")
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
                        cx.listener(|this, _, window, cx| {
                            this.focus.focus(window, cx);
                        }),
                    )
                    .child(measure)
                    .child(content)
                    .when(!self.rows.is_empty(), |this| {
                        this.child(Scrollbar::vertical(&self.scroll))
                    }),
            )
    }
}
