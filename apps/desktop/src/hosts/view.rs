//! Rendering of the Hosts screen: search bar, action bar (with the tag
//! filter), breadcrumbs, the card grid or the list (a header with resizable
//! columns over the rows, [`super::columns`]), the empty states and the drag
//! preview.
//!
//! Narrow windows (owner's step 1.4, stage 2a step 0.1): the action bar is
//! planned from the widths of its parts, measured with the window's text
//! system ([`layout::plan_action_bar`]): the tag chips collapse into "+N"
//! first, down to one chip; then Terminal and Serial, then New host lose
//! their labels, and at last the right group wraps to a second row. "+N"
//! lists the hidden tags in its tooltip. Breadcrumbs truncate, the empty
//! state's buttons wrap, and cards narrow below 240 px only when the content
//! area itself is narrower.

use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, ClickEvent, Context, DispatchPhase, DragMoveEvent,
    Focusable as _, FontWeight, InteractiveElement as _, IntoElement, KeyDownEvent, MouseButton,
    MouseUpEvent, ParentElement, Pixels, Render, SharedString, StatefulInteractiveElement, Styled,
    Window, canvas, div, prelude::FluentBuilder as _, px, size,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::input::Input;
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Selectable as _, Sizable as _, h_flex,
    v_flex, v_virtual_list,
};
use shellpp_core::service::dto::TagColor;
use uuid::Uuid;

use super::columns::{self, Column, ListColumns, ResizeColumn};
use super::controls::{
    self, ControlButton, ControlKind, ICON_SIZE, LABEL_BUTTON_EXTRA, text_width,
};
use super::layout::{BarLabels, BarMeasure, BarPlan};
use super::menu::{self, CommandHandler};
use super::{
    DraggedCards, HostsEvent, HostsScreen, Row, SearchTarget, SessionKind, group_card_id,
    host_card_id, layout, list_cell_id, strings, tag_chip_id, tag_filter,
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
/// The kit input's right padding (medium size): the search hint ends before it.
const SEARCH_INPUT_PAD_RIGHT: f32 = 10.;

/// The width of Terminal and Serial with their labels: one group, as wide
/// as the wider of the two (design-system 6 [decision]).
fn local_group_width(window: &Window) -> Pixels {
    controls::group_width(window, &[strings::TERMINAL, strings::SERIAL])
}

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
        // A hint that fits the measured text area (lead decision 4).
        if let Some(area) = self.search_text_width {
            let available = area - SEARCH_INPUT_PAD_RIGHT;
            let hint = layout::search_placeholder(available, |text| {
                text_width(window, text, metrics::FONT_SIZE_BODY, FontWeight::NORMAL)
            });
            if self.search.read(cx).presentation().placeholder().as_ref() != hint {
                self.search
                    .update(cx, |search, cx| search.set_placeholder(hint, window, cx));
            }
        }
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
        let screen = cx.entity().downgrade();
        let measure_text = canvas(
            move |bounds, _, cx| {
                let width = bounds.size.width.as_f32();
                let _ = screen.update(cx, |screen, cx| {
                    if screen
                        .search_text_width
                        .is_none_or(|measured| (measured - width).abs() >= 0.5)
                    {
                        screen.search_text_width = Some(width);
                        cx.notify();
                    }
                });
            },
            |_, _, _, _| {},
        )
        .absolute()
        .size_full();
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
                    .child(measure_text)
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

    /// The display spelling of a tag that is on (`key` is lower case).
    pub(super) fn tag_name(&self, key: &str) -> String {
        self.tag_options
            .iter()
            .find(|option| option.key() == key)
            .map(|option| option.name.clone())
            .unwrap_or_else(|| key.to_owned())
    }

    /// The widths the action bar is planned from.
    fn measure_action_bar(&self, available: f32, window: &Window) -> BarMeasure {
        let label = |text: &str| {
            text_width(window, text, metrics::FONT_SIZE_BODY, FontWeight::MEDIUM)
                + LABEL_BUTTON_EXTRA
        };
        let icon = metrics::HEIGHT_CONTROL.as_f32();
        let gap = metrics::SPACE_2.as_f32();
        // The split button: New host, the 1 px divider, the arrow.
        let split = |new_host: f32| new_host + 1. + icon;
        let new_host = label(strings::NEW_HOST);
        // Terminal and Serial are one group: both as wide as the wider.
        let local = local_group_width(window).as_f32();
        let left = [
            split(new_host) + gap + local + gap + local,
            split(new_host) + gap + icon + gap + icon,
            split(icon) + gap + icon + gap + icon,
        ];
        // View toggle (two buttons 4 px apart), tag button, sort.
        let right = (icon * 2. + metrics::SPACE_1.as_f32()) + gap + icon + gap + icon;
        let chips = self
            .filter
            .tags()
            .iter()
            .map(|key| {
                tag_filter::selected_chip_width(text_width(
                    window,
                    &self.tag_name(key),
                    metrics::FONT_SIZE_CAPTION,
                    FontWeight::NORMAL,
                ))
            })
            .collect();
        BarMeasure {
            available,
            left,
            right,
            chips,
            gap,
            chip_gap: metrics::SPACE_1.as_f32(),
        }
    }

    fn render_action_bar(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let measure = self.measure_action_bar(self.inner_width(window), window);
        let plan = layout::plan_action_bar(&measure, |hidden| {
            tag_filter::more_chip_width(text_width(
                window,
                &strings::more_tags(hidden),
                metrics::FONT_SIZE_CAPTION,
                FontWeight::NORMAL,
            ))
        });
        self.bar_plan = Some(plan);

        let left = self
            .render_bar_left(plan, local_group_width(window), cx)
            .into_any_element();
        let right = h_flex()
            .id("action-bar-right")
            .test_support()
            .flex_none()
            .items_center()
            .gap(metrics::SPACE_2)
            .child(self.render_view_toggle(cx))
            .child(self.render_tag_filter(cx))
            .children(self.render_tag_chips(plan, cx))
            .child(self.render_sort_menu(cx));
        if plan.wrapped {
            v_flex()
                .id("action-bar")
                .test_support()
                .w_full()
                .gap(metrics::SPACE_2)
                .child(left)
                .child(h_flex().w_full().justify_end().child(right))
                .into_any_element()
        } else {
            h_flex()
                .id("action-bar")
                .test_support()
                .w_full()
                .child(left)
                .child(div().flex_1().min_w(metrics::SPACE_2))
                .child(right)
                .into_any_element()
        }
    }

    /// Split New host (▾: New Group, Import), Terminal, Serial; labels as
    /// the plan allows (icon buttons carry their names as tooltips).
    /// Terminal and Serial with labels are `local_width` wide each.
    fn render_bar_left(
        &self,
        plan: BarPlan,
        local_width: Pixels,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let theme = cx.theme();
        let screen = cx.entity().downgrade();
        let new_host = ControlButton::new("new-host", ControlKind::Neutral)
            .icon(Lucide::Plus)
            .map(|button| match plan.labels {
                BarLabels::Icons => button.tooltip(strings::NEW_HOST),
                BarLabels::Full | BarLabels::LocalIcons => button.label(strings::NEW_HOST),
            })
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
                                let _ = import.update(cx, |screen, cx| {
                                    crate::import_sources::open(screen.state.clone(), window, cx);
                                });
                            }),
                    )
            });
        let split = h_flex()
            .flex_none()
            .child(new_host)
            .child(div().w(px(1.)).h(metrics::HEIGHT_CONTROL).bg(theme.border))
            .child(new_host_menu);
        let local_labels = plan.labels == BarLabels::Full;
        let terminal = ControlButton::new("terminal", ControlKind::Neutral)
            .icon(Lucide::SquareTerminal)
            .map(|button| {
                if local_labels {
                    button.label(strings::TERMINAL).min_w(local_width)
                } else {
                    button.tooltip(strings::TERMINAL)
                }
            })
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.open_local(SessionKind::Terminal, cx)
            }));
        let serial = ControlButton::new("serial", ControlKind::Neutral)
            .icon(Lucide::Usb)
            .map(|button| {
                if local_labels {
                    button.label(strings::SERIAL).min_w(local_width)
                } else {
                    button.tooltip(strings::SERIAL)
                }
            })
            .on_click(
                cx.listener(|this, _: &ClickEvent, _, cx| this.open_local(SessionKind::Serial, cx)),
            );
        h_flex()
            .id("action-bar-left")
            .test_support()
            .flex_none()
            .gap(metrics::SPACE_2)
            .child(split)
            .child(terminal)
            .child(serial)
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

    /// The tag filter (UI 2.2, lead decision 5): the tag button opens the
    /// dropdown (`tag_filter::TagFilterMenu`); it shows the active state
    /// while any tag is on.
    fn render_tag_filter(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let screen = cx.entity().downgrade();
        let menu = self.tag_menu.clone();
        let focus = self.tag_menu.read(cx).focus_handle(cx);
        let active = !self.filter.tags().is_empty();
        Popover::new("tag-filter-popover")
            .anchor(gpui::Anchor::TopRight)
            .trigger(
                ControlButton::new("tag-filter", ControlKind::Ghost)
                    .icon(Lucide::Tag)
                    .tooltip(strings::TAG_FILTER)
                    .selected(active),
            )
            .open(self.tag_menu_open)
            .on_open_change(move |open, window, cx| {
                let open = *open;
                let _ = screen.update(cx, |screen, cx| screen.set_tag_menu_open(open, window, cx));
            })
            .track_focus(&focus)
            .p(metrics::SPACE_2)
            .rounded(metrics::RADIUS_OVERLAY)
            .shadow(theme::shadow_overlay(cx))
            .content(move |_, _, _| menu.clone())
    }

    /// The chips of the tags that are on, then "+N" for the ones that did
    /// not fit (it opens the dropdown). A chip's click turns its tag off.
    fn render_tag_chips(&self, plan: BarPlan, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let selected = self.filter.tags();
        if selected.is_empty() {
            return None;
        }
        let shown = plan.chips.min(selected.len());
        let chips = selected[..shown].iter().map(|key| {
            let key = key.clone();
            tag_filter::selected_chip(tag_chip_id(&key), self.tag_name(&key), cx)
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.toggle_tag(&key, cx)))
        });
        let hidden: Vec<String> = selected[shown..]
            .iter()
            .map(|key| self.tag_name(key))
            .collect();
        let more = (!hidden.is_empty()).then(|| {
            tag_filter::more_chip(hidden, cx).on_click(cx.listener(
                |this, _: &ClickEvent, window, cx| this.set_tag_menu_open(true, window, cx),
            ))
        });
        Some(
            h_flex()
                .id("tag-chips")
                .test_support()
                .flex_none()
                .items_center()
                .gap(metrics::SPACE_1)
                .children(chips)
                .children(more),
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
                .min_w_0()
                .truncate()
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
            .w_full()
            .min_w_0()
            .overflow_hidden()
            .pl(CONTENT_PADDING - metrics::SPACE_1)
            .pr(CONTENT_PADDING)
            .pb(metrics::SPACE_2)
            .gap(metrics::SPACE_1)
            .text_size(metrics::FONT_SIZE_BODY)
            .child(crumb("crumb-root".into(), None, strings::HOSTS.into(), cx));
        for (index, (id, name)) in path.into_iter().enumerate() {
            crumbs = crumbs.child(
                div()
                    .flex_none()
                    .text_color(secondary)
                    .child(strings::BREADCRUMB_SEPARATOR),
            );
            let element_id: SharedString = format!("crumb-{id}").into();
            crumbs = crumbs.child(if index == last {
                div()
                    .id(element_id)
                    .test_support()
                    .min_w_0()
                    .truncate()
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

    /// The title and the subtitle of a grid card.
    fn render_texts(&self, title: String, subtitle: String, cx: &App) -> AnyElement {
        let theme = cx.theme();
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
                    .child(title),
            )
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_CAPTION)
                    .text_color(theme.muted_foreground)
                    .truncate()
                    .child(subtitle),
            )
            .into_any_element()
    }

    /// The cells of a list row (owner's step 0.4), in the widths of the
    /// header: the tile and the title (Name), then Address, User and the
    /// last column, in `--text-secondary`.
    #[allow(clippy::too_many_arguments)]
    fn render_cells(
        &self,
        record: Uuid,
        tile: AnyElement,
        title: String,
        address: String,
        user: String,
        last: AnyElement,
        cx: &App,
    ) -> Vec<AnyElement> {
        let theme = cx.theme();
        let [name_width, address_width, user_width, _] = self.fitted_columns;
        let secondary = |column: &str, width: f32, text: String| {
            div()
                .id(list_cell_id(record, column))
                .test_support()
                .flex_none()
                .w(px(width))
                .min_w_0()
                .truncate()
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element()
        };
        vec![
            h_flex()
                .id(list_cell_id(record, "name"))
                .test_support()
                .flex_none()
                .w(px(name_width))
                .min_w_0()
                .gap(metrics::SPACE_3)
                .items_center()
                .child(tile)
                .child(
                    div()
                        .min_w_0()
                        .truncate()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(theme.foreground)
                        .child(title),
                )
                .into_any_element(),
            secondary("address", address_width, address),
            secondary("user", user_width, user),
            h_flex()
                .id(list_cell_id(record, "tags"))
                .test_support()
                .flex_1()
                .min_w_0()
                .items_center()
                .text_color(theme.muted_foreground)
                .child(last)
                .into_any_element(),
        ]
    }

    /// The last list column of a host: its tags, or its group (with the
    /// folder icon) when it has no tags.
    fn host_last_cell(&self, host: &crate::state::HostItem, cx: &App) -> AnyElement {
        let group = host
            .group_id
            .and_then(|group| self.group(group))
            .map(|group| group.name.as_str());
        match layout::last_column(host, group) {
            layout::LastColumn::Tags(tags) => {
                div().min_w_0().truncate().child(tags).into_any_element()
            }
            layout::LastColumn::Group(name) => h_flex()
                .min_w_0()
                .gap(metrics::SPACE_1)
                .child(
                    Icon::new(Lucide::Folder)
                        .with_size(ICON_SIZE)
                        .text_color(cx.theme().muted_foreground),
                )
                .child(div().min_w_0().truncate().child(name))
                .into_any_element(),
            layout::LastColumn::Empty => div().into_any_element(),
        }
    }

    /// The header row of the list layout: the column names and, between
    /// them, the dividers that resize the column on their left. It is
    /// aligned with the rows (same padding and widths) and does not scroll.
    fn render_list_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let fitted = self.fitted_columns;
        let line = theme.border;
        let handle_hover = theme.drag_border;
        let pad = metrics::SPACE_3.as_f32();
        let cell = |id: &'static str, label: &'static str, width: Option<f32>| {
            div()
                .id(id)
                .test_support()
                .min_w_0()
                .truncate()
                .map(|this| match width {
                    Some(width) => this.flex_none().w(px(width)),
                    None => this.flex_1(),
                })
                .child(label)
        };
        let dividers = Column::ALL.map(|column| {
            let x = pad + ListColumns::divider_x(&fitted, column);
            div()
                .id(column.divider_id())
                .test_support()
                .absolute()
                .top_0()
                .bottom_0()
                .left(px(x - columns::DIVIDER_HIT / 2.))
                .w(px(columns::DIVIDER_HIT))
                .flex()
                .justify_center()
                .items_center()
                .cursor_col_resize()
                .hover(move |style| style.bg(handle_hover.opacity(0.5)))
                .on_drag(ResizeColumn(column), |_, _, _, cx| {
                    cx.new(|_| crate::shell::details::ResizePreview)
                })
                .child(div().w(px(1.)).h(metrics::SPACE_4).bg(line))
        });
        let screen = cx.entity().downgrade();
        // While a divider is dragged: the button going up anywhere ends it.
        let release = self.column_resizing.then(|| {
            canvas(
                |_, _, _| {},
                move |_, _, window, _| {
                    let screen = screen.clone();
                    window.on_mouse_event(move |_: &MouseUpEvent, phase, _, cx| {
                        if phase == DispatchPhase::Bubble {
                            let _ = screen.update(cx, |screen, cx| screen.finish_column_resize(cx));
                        }
                    });
                },
            )
            .absolute()
            .size_0()
        });
        h_flex()
            .id("hosts-list-header")
            .test_support()
            .relative()
            .flex_none()
            .w_full()
            .h(metrics::HEIGHT_CONTROL)
            .px(metrics::SPACE_3)
            .gap(metrics::SPACE_3)
            .items_center()
            .border_b_1()
            .border_color(line)
            .text_size(metrics::FONT_SIZE_CAPTION)
            .font_weight(FontWeight::MEDIUM)
            .text_color(theme.muted_foreground)
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<ResizeColumn>, _, cx| {
                    let ResizeColumn(column) = *event.drag(cx);
                    let x = event.event.position.x - event.bounds.left() - metrics::SPACE_3;
                    this.drag_column(column, x.as_f32(), cx);
                }),
            )
            .child(
                // The label sits over the titles, after the tiles.
                cell("list-header-name", strings::COLUMN_NAME, Some(fitted[0]))
                    .pl(LIST_TILE_SIZE + metrics::SPACE_3),
            )
            .child(cell(
                "list-header-address",
                strings::COLUMN_ADDRESS,
                Some(fitted[1]),
            ))
            .child(cell(
                "list-header-user",
                strings::COLUMN_USER,
                Some(fitted[2]),
            ))
            .child(cell("list-header-tags", strings::COLUMN_TAGS, None))
            .children(dividers)
            .children(release)
    }

    /// The width of a list row's content (inside its 12 px padding).
    pub(super) fn list_row_inner_width(&self) -> f32 {
        (self.card_width - 2. * metrics::SPACE_3.as_f32()).max(0.)
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
        let title = host.title().to_owned();
        let host_tile = tile(
            host.color,
            layout::os_icon(host.os_family.as_deref()),
            self.tile_size(),
        )
        .into_any_element();
        let body = match self.layout {
            HostsLayout::Tiles => vec![
                host_tile,
                self.render_texts(title, layout::subtitle(host, group_name.as_deref()), cx),
            ],
            HostsLayout::List => self.render_cells(
                id,
                host_tile,
                title,
                host.address.clone(),
                host.username.clone().unwrap_or_default(),
                self.host_last_cell(host, cx),
                cx,
            ),
        };
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
            .children(body)
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
        let group_tile = tile(group.color, Lucide::Folder, self.tile_size()).into_any_element();
        let count = strings::host_count(group.host_count as usize);
        let body = match self.layout {
            HostsLayout::Tiles => {
                vec![group_tile, self.render_texts(group.name.clone(), count, cx)]
            }
            // A group has no address, user or tags: its host count stands
            // in the Address column.
            HostsLayout::List => self.render_cells(
                id,
                group_tile,
                group.name.clone(),
                count,
                String::new(),
                div().into_any_element(),
                cx,
            ),
        };
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
            .children(body)
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

    /// The empty states (UI 2.3): no hosts at all ("Create host", the
    /// screen's accent button, and Import; owner's step 0.3), an empty
    /// group, no search results, and no results for a search that is an
    /// address: Enter connects to it, "Save as host" saves it (owner's step
    /// 0.2; the search bar's Connect is the accent button meanwhile).
    /// While `panel_open` (a host or group panel), the panel holds the
    /// screen's one accent button and "Create host" is neutral.
    fn render_empty(
        &self,
        panel_open: bool,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let theme = cx.theme();
        let vault_empty = self.hosts.is_empty() && self.groups.is_empty();
        let quick = match &self.search_target {
            SearchTarget::Quick(target) if self.filter.is_active() => Some(target.clone()),
            _ => None,
        };
        let (id, title, body): (&'static str, SharedString, Option<&'static str>) =
            if let Some(target) = &quick {
                (
                    "hosts-quick-connect",
                    strings::press_enter_to_connect(&target.display()).into(),
                    None,
                )
            } else if self.filter.is_active() {
                ("hosts-no-matches", strings::NO_MATCHES.into(), None)
            } else if vault_empty {
                (
                    "hosts-empty",
                    strings::EMPTY_TITLE.into(),
                    Some(strings::EMPTY_BODY),
                )
            } else {
                ("group-empty", strings::EMPTY_GROUP.into(), None)
            };
        let buttons = || {
            h_flex()
                .flex_wrap()
                .justify_center()
                .gap(metrics::SPACE_2)
                .pt(metrics::SPACE_2)
        };
        let actions = if let Some(target) = quick {
            Some(
                buttons().child(
                    ControlButton::new("save-as-host", ControlKind::Neutral)
                        .icon(Lucide::Plus)
                        .label(strings::SAVE_AS_HOST)
                        .on_click(cx.listener(move |this, _: &ClickEvent, window, cx| {
                            this.save_search_address(&target, window, cx)
                        })),
                ),
            )
        } else if vault_empty && !self.filter.is_active() {
            // One group: both as wide as the wider (design-system 6).
            let width =
                controls::group_width(window, &[strings::CREATE_HOST, strings::IMPORT_SSH_CONFIG]);
            Some(
                buttons()
                    .child(
                        ControlButton::new("create-host", create_host_kind(panel_open))
                            .icon(Lucide::Plus)
                            .label(strings::CREATE_HOST)
                            .min_w(width)
                            .on_click(
                                cx.listener(|_, _: &ClickEvent, _, cx| {
                                    cx.emit(HostsEvent::NewHost)
                                }),
                            ),
                    )
                    .child(
                        ControlButton::new("import-ssh-config", ControlKind::Neutral)
                            .icon(Lucide::Import)
                            .label(strings::IMPORT_SSH_CONFIG)
                            .path_tooltip(strings::SSH_CONFIG_PATH)
                            .min_w(width)
                            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                                crate::import_sources::open(this.state.clone(), window, cx);
                            })),
                    ),
            )
        } else {
            None
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
                // A long address wraps instead of leaving the screen.
                div()
                    .id("hosts-empty-title")
                    .test_support()
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
            .children(actions)
            .into_any_element()
    }

    /// The width inside the 24 px side padding (search bar, action bar,
    /// cards): measured after the previous layout; the first frame
    /// estimates it from the window.
    fn inner_width(&self, window: &Window) -> f32 {
        let outer = self
            .measured_width
            .unwrap_or_else(|| (window.viewport_size().width - metrics::WIDTH_SIDEBAR).as_f32());
        (outer - CONTENT_PADDING.as_f32() * 2.).max(0.)
    }

    /// The content width for the columns. Below 240 px (a narrow window with
    /// a wide details panel) the one column narrows with the area instead of
    /// being clipped.
    fn content_width(&self, window: &Window) -> f32 {
        self.inner_width(window).max(1.)
    }

    /// The search hint on display and whether it fits its measured area.
    #[cfg(test)]
    pub(super) fn search_hint_fits(&self, window: &Window, cx: &App) -> (SharedString, bool) {
        let hint = self.search.read(cx).presentation().placeholder().clone();
        let fits = self.search_text_width.is_some_and(|area| {
            text_width(window, &hint, metrics::FONT_SIZE_BODY, FontWeight::NORMAL)
                <= area - SEARCH_INPUT_PAD_RIGHT + 0.5
        });
        (hint, fits)
    }
}

/// A plain text menu item with a stable id, in the kit's menu item text
/// size (the size of every menu of the app).
fn menu_text(id: &'static str, label: &'static str, cx: &App) -> AnyElement {
    div()
        .id(id)
        .test_support()
        .flex_1()
        .text_color(cx.theme().foreground)
        .child(label)
        .into_any_element()
}

/// Whether the window's details panel shows a host or group panel (UI 1.3).
/// The shell owns the panel; without a shell (the screen alone) there is
/// none.
fn details_panel_open(window: &Window, cx: &App) -> bool {
    crate::shell::shell_for(window, cx).is_some_and(|shell| shell.read(cx).panel().is_some())
}

/// The style of the empty vault's "Create host" (owner's step 0.3): the
/// screen's primary button, unless an open details panel holds the
/// primary action (`docs/design-system.md` 1: one accent button per screen).
fn create_host_kind(panel_open: bool) -> ControlKind {
    if panel_open {
        ControlKind::Neutral
    } else {
        ControlKind::Accent
    }
}

impl Render for HostsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let width = self.content_width(window);
        self.rebuild_rows(width);
        self.reveal_pending();
        self.fitted_columns = self.list_columns.fit(self.list_row_inner_width());
        let action_bar = self.render_action_bar(window, cx);
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
            self.render_empty(details_panel_open(window, cx), window, cx)
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
        // The list layout's column header stays above the scrolling rows.
        let header = (self.layout == HostsLayout::List && !self.rows.is_empty()).then(|| {
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
            .when(!self.rows.is_empty(), |this| {
                this.child(Scrollbar::vertical(&self.scroll))
            });

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
                    .child(action_bar),
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
                    .children(header)
                    .child(rows),
            )
    }
}
