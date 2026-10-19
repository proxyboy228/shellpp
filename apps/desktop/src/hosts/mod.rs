//! The Hosts screen (UI 2): search bar, action bar, groups as folders with
//! breadcrumbs, and a virtualized grid of host cards.
//!
//! Skeleton status: a working minimal screen. TODO(hosts) items for the
//! Hosts screen work: quick-connect address recognition and the Connect
//! button state (2.1), the split "New host" button with New Group and Import,
//! Terminal and Serial buttons, view/tag/sort controls (2.2), card context
//! menu, Enter and E keys (2.4), empty-state copy and multi-select actions
//! (2.3), OS icons on tiles.

pub mod layout;

use std::rc::Rc;

use gpui::{
    AnyElement, AppContext as _, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, IntoElement, ParentElement, Pixels, Render, SharedString, Size,
    StatefulInteractiveElement, Styled as _, Subscription, Window, div, prelude::*, px, size,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable as _, VirtualListScrollHandle,
    h_flex, v_flex, v_virtual_list,
};
use uuid::Uuid;

use crate::i18n;
use crate::state::{AppState, AppStateEvent, GroupItem, HostItem, SelectMode};
use crate::theme::{self, metrics, palette};

/// What the Hosts screen asks the shell to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostsEvent {
    /// Open the host panel (single click on a card).
    OpenHost(Uuid),
    /// Open an empty host panel (New host).
    NewHost,
    /// Open the group panel of a group.
    EditGroup(Uuid),
    /// Open an empty group panel (New group).
    NewGroup,
    /// Show a toast.
    Toast(SharedString),
}

/// Stable element id of a host card.
pub fn host_card_id(id: Uuid) -> SharedString {
    format!("host-card-{id}").into()
}

/// Stable element id of a group card.
pub fn group_card_id(id: Uuid) -> SharedString {
    format!("group-card-{id}").into()
}

/// One row of the virtualized list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    GroupsHeading,
    /// Groups `start..start + columns` of the visible groups.
    Groups(usize),
    HostsHeading,
    /// Hosts `start..start + columns` of the visible hosts.
    Hosts(usize),
}

const HEADING_HEIGHT: f32 = 40.;
/// Horizontal padding of the content (`--space-5`).
const CONTENT_PADDING: Pixels = metrics::SPACE_5;

pub struct HostsScreen {
    state: Entity<AppState>,
    search: Entity<InputState>,
    query: String,
    /// Snapshot of the state's lists, refreshed on `RecordsChanged`.
    hosts: Rc<Vec<HostItem>>,
    groups: Rc<Vec<GroupItem>>,
    /// Indices into `groups` / `hosts` of what is on screen, in order.
    visible_groups: Vec<usize>,
    visible_hosts: Vec<usize>,
    rows: Vec<Row>,
    row_sizes: Rc<Vec<Size<Pixels>>>,
    /// (columns, rows, width) the rows were built for.
    layout_key: (usize, usize, i32),
    columns: usize,
    scroll: VirtualListScrollHandle,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for HostsScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostsScreen").finish_non_exhaustive()
    }
}

impl EventEmitter<HostsEvent> for HostsScreen {}

impl Focusable for HostsScreen {
    fn focus_handle(&self, _: &gpui::App) -> FocusHandle {
        self.focus.clone()
    }
}

impl HostsScreen {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // No autofocus: the search bar is focused only by the user (2.1).
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(i18n::hosts::SEARCH_PLACEHOLDER));
        let search_subscription = cx.subscribe_in(
            &search,
            window,
            |this: &mut Self, input, event: &InputEvent, _, cx| {
                if let InputEvent::Change = event {
                    this.query = input.read(cx).value().to_string();
                    this.refresh(cx);
                }
            },
        );
        let state_subscription =
            cx.subscribe(&state, |this: &mut Self, _, event, cx| match event {
                AppStateEvent::RecordsChanged | AppStateEvent::LocationChanged(_) => {
                    this.refresh(cx)
                }
                AppStateEvent::SelectionChanged => cx.notify(),
                AppStateEvent::LockChanged(_) => {}
            });
        let mut this = Self {
            state,
            search,
            query: String::new(),
            hosts: Rc::default(),
            groups: Rc::default(),
            visible_groups: Vec::new(),
            visible_hosts: Vec::new(),
            rows: Vec::new(),
            row_sizes: Rc::default(),
            layout_key: (0, 0, 0),
            columns: 1,
            scroll: VirtualListScrollHandle::new(),
            focus: cx.focus_handle(),
            _subscriptions: vec![search_subscription, state_subscription],
        };
        this.refresh(cx);
        this
    }

    /// The search input (tests type into it through its element id `search`).
    pub fn search(&self) -> &Entity<InputState> {
        &self.search
    }

    /// Ids of the host cards on screen, in screen order.
    pub fn visible_host_ids(&self) -> Vec<Uuid> {
        self.visible_hosts
            .iter()
            .map(|&ix| self.hosts[ix].record_id)
            .collect()
    }

    /// Ids of the group cards on screen, in screen order.
    pub fn visible_group_ids(&self) -> Vec<Uuid> {
        self.visible_groups
            .iter()
            .map(|&ix| self.groups[ix].record_id)
            .collect()
    }

    /// Re-reads the lists from the state and applies location and search.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        self.hosts = state.hosts();
        self.groups = state.groups();
        let location = state.location();
        let searching = !self.query.trim().is_empty();
        self.visible_groups = if searching {
            Vec::new()
        } else {
            (0..self.groups.len())
                .filter(|&ix| self.groups[ix].parent_id == location)
                .collect()
        };
        self.visible_hosts = (0..self.hosts.len())
            .filter(|&ix| {
                let host = &self.hosts[ix];
                if searching {
                    layout::host_matches(host, &self.query)
                } else {
                    host.group_id == location
                }
            })
            .collect();
        // Force a rebuild of the rows on the next render.
        self.layout_key = (0, 0, 0);
        cx.notify();
    }

    fn rebuild_rows(&mut self, width: f32) {
        let columns = layout::columns_for(width);
        let key = (
            columns,
            self.visible_groups.len() * 100_000 + self.visible_hosts.len(),
            width as i32,
        );
        if key == self.layout_key {
            return;
        }
        self.layout_key = key;
        self.columns = columns;
        let mut rows = Vec::new();
        if !self.visible_groups.is_empty() {
            rows.push(Row::GroupsHeading);
            rows.extend(
                (0..self.visible_groups.len())
                    .step_by(columns)
                    .map(Row::Groups),
            );
        }
        if !self.visible_hosts.is_empty() {
            rows.push(Row::HostsHeading);
            rows.extend(
                (0..self.visible_hosts.len())
                    .step_by(columns)
                    .map(Row::Hosts),
            );
        }
        let card_row = metrics::HEIGHT_HOST_CARD.as_f32() + metrics::HOST_CARD_GAP.as_f32();
        self.row_sizes = Rc::new(
            rows.iter()
                .map(|row| {
                    let height = match row {
                        Row::GroupsHeading | Row::HostsHeading => HEADING_HEIGHT,
                        Row::Groups(_) | Row::Hosts(_) => card_row,
                    };
                    size(px(width), px(height))
                })
                .collect(),
        );
        self.rows = rows;
    }

    fn on_host_click(&mut self, id: Uuid, event: &ClickEvent, cx: &mut Context<Self>) {
        let modifiers = event.modifiers();
        let mode = if modifiers.shift {
            SelectMode::Range
        } else if modifiers.secondary() {
            SelectMode::Toggle
        } else {
            SelectMode::Replace
        };
        let order = self.visible_host_ids();
        self.state
            .update(cx, |state, cx| state.select(id, mode, &order, cx));
        if event.click_count() >= 2 {
            // TODO(hosts): connect (UI 2.4) once terminal sessions exist.
            cx.emit(HostsEvent::Toast(i18n::placeholder::TITLE.into()));
        } else if mode == SelectMode::Replace {
            cx.emit(HostsEvent::OpenHost(id));
        }
    }

    fn on_group_click(&mut self, id: Uuid, cx: &mut Context<Self>) {
        // Groups work as folders (2.3).
        self.state
            .update(cx, |state, cx| state.set_location(Some(id), cx));
    }

    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap(metrics::SPACE_3)
            .px(CONTENT_PADDING)
            .pt(metrics::SPACE_4)
            .pb(metrics::SPACE_2)
            .child(
                h_flex()
                    .w_full()
                    .gap(metrics::SPACE_2)
                    .child(
                        Input::new(&self.search)
                            .id("search")
                            .flex_1()
                            .prefix(Icon::new(IconName::Search).small())
                            .cleanable(true),
                    )
                    .child(
                        // TODO(hosts): enabled and accent-filled only for a
                        // recognized address (owner's audit item 2).
                        Button::new("quick-connect")
                            .label(i18n::hosts::CONNECT)
                            .disabled(true),
                    ),
            )
            .child(
                h_flex()
                    .gap(metrics::SPACE_2)
                    .child(
                        Button::new("new-host")
                            .outline()
                            .icon(Icon::new(Lucide::Plus))
                            .label(i18n::hosts::NEW_HOST)
                            .on_click(
                                cx.listener(|_, _: &ClickEvent, _, cx| {
                                    cx.emit(HostsEvent::NewHost)
                                }),
                            ),
                    )
                    .child(
                        Button::new("new-group")
                            .ghost()
                            .label(i18n::hosts::NEW_GROUP)
                            .on_click(cx.listener(|_, _: &ClickEvent, _, cx| {
                                cx.emit(HostsEvent::NewGroup)
                            })),
                    ),
            )
    }

    /// Breadcrumbs, only inside a group (owner's audit item 7).
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
        let last = path.len() - 1;
        let mut crumbs = h_flex()
            .id("breadcrumbs")
            .test_support()
            .px(CONTENT_PADDING)
            .pb(metrics::SPACE_2)
            .gap(metrics::SPACE_1)
            .text_size(metrics::FONT_SIZE_BODY)
            .child(
                div()
                    .id("crumb-root")
                    .test_support()
                    .cursor_pointer()
                    .text_color(secondary)
                    .child(i18n::hosts::HOSTS)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                        this.state
                            .update(cx, |state, cx| state.set_location(None, cx))
                    })),
            );
        for (index, (id, name)) in path.into_iter().enumerate() {
            crumbs = crumbs.child(div().text_color(secondary).child("/"));
            let crumb = div()
                .id(SharedString::from(format!("crumb-{id}")))
                .test_support()
                .text_color(if index == last { primary } else { secondary })
                .child(name);
            crumbs = crumbs.child(if index == last {
                crumb.into_any_element()
            } else {
                crumb
                    .cursor_pointer()
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.state
                            .update(cx, |state, cx| state.set_location(Some(id), cx))
                    }))
                    .into_any_element()
            });
        }
        Some(crumbs)
    }

    fn render_heading(&self, label: &'static str, cx: &Context<Self>) -> AnyElement {
        div()
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

    fn tile(color: shellpp_core::service::dto::TagColor, icon: Lucide) -> impl IntoElement {
        // White monochrome icon on the tile color; never a letter (audit item 3).
        div()
            .flex_none()
            .size(metrics::HOST_TILE_SIZE)
            .rounded(metrics::RADIUS_TILE)
            .bg(palette::tag_color(color).hsla())
            .flex()
            .items_center()
            .justify_center()
            .child(
                Icon::new(icon)
                    .text_color(palette::TEXT_ON_TAG.hsla())
                    .small(),
            )
    }

    fn card_frame(
        id: SharedString,
        selected: bool,
        cx: &Context<Self>,
    ) -> impl StatefulInteractiveElement + ParentElement + IntoElement {
        let theme = cx.theme();
        let hover = theme.secondary_hover;
        h_flex()
            .id(id)
            .test_support()
            .flex_1()
            .min_w_0()
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
            .cursor_pointer()
            .hover(move |style| style.bg(hover))
    }

    fn render_texts(title: String, subtitle: String, cx: &Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        v_flex()
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
    }

    fn render_host_card(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let host = &self.hosts[ix];
        let id = host.record_id;
        let selected = self.state.read(cx).selection().contains(&id);
        let subtitle = match &host.username {
            Some(user) => format!("SSH, {user}"),
            None => "SSH".to_owned(),
        };
        Self::card_frame(host_card_id(id), selected, cx)
            .on_click(
                cx.listener(move |this, event: &ClickEvent, _, cx| {
                    this.on_host_click(id, event, cx)
                }),
            )
            // TODO(hosts): OS icon from `os_family` when detected.
            .child(Self::tile(host.color, Lucide::Server))
            .child(Self::render_texts(host.title().to_owned(), subtitle, cx))
            .into_any_element()
    }

    fn render_group_card(&self, ix: usize, cx: &mut Context<Self>) -> AnyElement {
        let group = &self.groups[ix];
        let id = group.record_id;
        let selected = self.state.read(cx).selection().contains(&id);
        Self::card_frame(group_card_id(id), selected, cx)
            .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| this.on_group_click(id, cx)))
            .child(Self::tile(group.color, Lucide::Folder))
            .child(Self::render_texts(
                group.name.clone(),
                i18n::hosts::host_count(group.host_count as usize),
                cx,
            ))
            .into_any_element()
    }

    fn render_row(&self, row: Row, cx: &mut Context<Self>) -> AnyElement {
        let columns = self.columns;
        let cards =
            |items: &[usize], start: usize, this: &Self, cx: &mut Context<Self>, host: bool| {
                let end = (start + columns).min(items.len());
                let mut row = h_flex()
                    .w_full()
                    .h(metrics::HEIGHT_HOST_CARD + metrics::HOST_CARD_GAP)
                    .pb(metrics::HOST_CARD_GAP)
                    .gap(metrics::HOST_CARD_GAP);
                for &item in &items[start..end] {
                    let card = if host {
                        this.render_host_card(item, cx)
                    } else {
                        this.render_group_card(item, cx)
                    };
                    row = row.child(card);
                }
                // Keeps card widths equal on a partly filled last row.
                for _ in end..start + columns {
                    row = row.child(div().flex_1().min_w_0());
                }
                row.into_any_element()
            };
        match row {
            Row::GroupsHeading => self.render_heading(i18n::hosts::GROUPS, cx),
            Row::HostsHeading => self.render_heading(i18n::hosts::HOSTS, cx),
            Row::Groups(start) => cards(&self.visible_groups.clone(), start, self, cx, false),
            Row::Hosts(start) => cards(&self.visible_hosts.clone(), start, self, cx, true),
        }
    }

    fn render_empty(&self, cx: &Context<Self>) -> AnyElement {
        let theme = cx.theme();
        let (title, body) = if self.query.trim().is_empty() {
            (i18n::hosts::EMPTY_TITLE, i18n::hosts::EMPTY_BODY)
        } else {
            (i18n::hosts::NO_MATCHES, "")
        };
        v_flex()
            .id("hosts-empty")
            .test_support()
            .flex_1()
            .items_center()
            .justify_center()
            .gap(metrics::SPACE_2)
            .child(
                div()
                    .text_size(metrics::FONT_SIZE_HEADING)
                    .text_color(theme.foreground)
                    .child(title),
            )
            .child(div().text_color(theme.muted_foreground).child(body))
            .into_any_element()
    }
}

impl Render for HostsScreen {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // TODO(hosts): measure the real content width instead of deriving it
        // from the window (the right panel narrows the area too).
        let width = (window.viewport_size().width - metrics::WIDTH_SIDEBAR - CONTENT_PADDING * 2.)
            .as_f32()
            .max(metrics::HOST_CARD_MIN_WIDTH.as_f32());
        self.rebuild_rows(width);

        let content = if self.rows.is_empty() {
            self.render_empty(cx)
        } else {
            div()
                .relative()
                .flex_1()
                .min_h_0()
                .child(
                    div().size_full().px(CONTENT_PADDING).child(
                        v_virtual_list(
                            cx.entity(),
                            "host-grid",
                            self.row_sizes.clone(),
                            |this: &mut Self, range, _window, cx| {
                                range
                                    .map(|ix| {
                                        let row = this.rows[ix];
                                        this.render_row(row, cx)
                                    })
                                    .collect::<Vec<_>>()
                            },
                        )
                        .track_scroll(&self.scroll)
                        .size_full(),
                    ),
                )
                .child(Scrollbar::vertical(&self.scroll))
                .into_any_element()
        };

        v_flex()
            .id("hosts-screen")
            .test_support()
            .track_focus(&self.focus)
            .size_full()
            .bg(cx.theme().colors.list)
            .text_color(theme::tokens(cx).text_primary.hsla())
            .child(self.render_toolbar(cx))
            .children(self.render_breadcrumbs(cx))
            .child(content)
    }
}
