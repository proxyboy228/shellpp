//! The Hosts screen (UI 2): the search bar with quick connect (2.1), the
//! action bar (2.2), groups as folders with breadcrumbs, the adaptive card
//! grid or the list (2.3), and the card actions: clicks, keys, the context
//! menu, multiple selection and drag and drop (2.3, 2.4).
//!
//! The screen talks to the shell through two event types:
//! - [`HostsEvent`]: open the host or group panel, show a plain toast;
//! - [`HostsRequest`]: open a session tab (a placeholder until terminals
//!   exist), open the host panel focused on Address (E), a toast with an
//!   action button ("Save host?", Undo), and whether the search text is a
//!   recognized address (then the host panel's Connect is not the accent
//!   button: one accent-filled button per screen, lead decision 3).
//!
//! Module map: [`address`] (quick-connect parser), [`layout`] (columns,
//! keyboard navigation, subtitle, tile icon), [`menu`] (context menu model
//! and builder), [`controls`] (buttons and chips per design-system 6),
//! `view` (rendering), `dialogs` (removal confirmation, placeholders),
//! [`strings`] (English texts). The filter, sort and layout preferences are
//! in [`crate::state::hosts_view`].

pub mod address;
pub mod controls;
mod dialogs;
pub mod layout;
pub mod menu;
pub mod strings;
mod view;

#[cfg(test)]
mod tests;

use std::collections::HashSet;
use std::rc::Rc;

use gpui::{
    App, AppContext as _, ClickEvent, ClipboardItem, Context, Entity, EventEmitter, FocusHandle,
    Focusable, KeyDownEvent, Pixels, ScrollHandle, SharedString, Size, Subscription, Window,
};
use gpui_kit::component::VirtualListScrollHandle;
use gpui_kit::component::input::{InputEvent, InputState};
use serde_json::{Map, Value, json};
use shellpp_core::service::dto::MovePosition;
use uuid::Uuid;

use crate::state::hosts_view::{
    self, HostsFilter, HostsLayout, HostsListing, HostsSort, group_tree, subtree_of,
};
use crate::state::{AppState, AppStateEvent, GroupItem, HostItem, SelectMode};

pub use address::{QuickTarget, parse_address};
pub use menu::{MenuCommand, MenuEntry};

/// What the Hosts screen asks the shell to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostsEvent {
    /// Open the host panel (single click on a card, arrows).
    OpenHost(Uuid),
    /// Open an empty host panel (New host, Create host).
    NewHost,
    /// Open the group panel of a group (Edit in the group menu).
    EditGroup(Uuid),
    /// Open an empty group panel (New host ▸ New Group).
    NewGroup,
    /// Show a toast.
    Toast(SharedString),
}

/// The kind of a session tab the screen opens.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionKind {
    /// Terminal session over SSH (Connect, Quick Connect, double click).
    Ssh,
    /// SFTP session tab (Connect ▸ via SFTP).
    Sftp,
    /// Local terminal (Terminal button).
    Terminal,
    /// Serial port (Serial button).
    Serial,
}

/// A session tab to open. Until terminals exist the shell shows an "In
/// development" placeholder in the tab (lead decision 6).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionRequest {
    pub kind: SessionKind,
    /// The saved host, if any.
    pub host: Option<Uuid>,
    /// The quick-connect target of the search bar (no saved host).
    pub target: Option<QuickTarget>,
    /// Tab title: the host title, `user@host`, "Terminal" or "Serial".
    pub title: SharedString,
    /// Alt was held: open in a new window (UI 2.4).
    pub new_window: bool,
}

/// The action button of a toast.
#[derive(Clone)]
pub struct ToastAction {
    /// Element id of the button (`toast-save-host`, `toast-undo`).
    pub id: SharedString,
    pub label: SharedString,
    pub run: ToastHandler,
}

/// What a toast action button runs.
pub type ToastHandler = Rc<dyn Fn(&mut Window, &mut App)>;

impl std::fmt::Debug for ToastAction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ToastAction")
            .field("id", &self.id)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// Requests of the Hosts screen beyond [`HostsEvent`]. A separate event
/// type, so the shell adopts them without breaking its `HostsEvent` match.
#[derive(Clone, Debug)]
pub enum HostsRequest {
    /// Open a session tab.
    OpenSession(SessionRequest),
    /// Open the host panel with the focus in Address (E, Edit Host Details;
    /// lead decision 8).
    EditHost(Uuid),
    /// A toast with one action button, shown for 5 seconds ("Save host?"
    /// after a quick connect, Undo after Remove).
    ToastWithAction {
        message: SharedString,
        action: ToastAction,
    },
    /// The search text became (or stopped being) a connectable address.
    /// While true, the search bar's Connect is the accent button and the
    /// host panel's Connect must be neutral (lead decision 3).
    SearchAddressChanged(bool),
}

/// What Enter or Connect in the search bar would do (UI 2.1).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SearchTarget {
    /// Not an address: Connect is disabled.
    #[default]
    None,
    /// The text exactly matches the label or address of a saved host.
    Saved(Uuid),
    /// A quick-connect address, connected without saving.
    Quick(QuickTarget),
}

/// The value carried while cards are dragged (UI 2.3).
#[derive(Clone, Debug)]
pub struct DraggedCards {
    /// The dragged records: the selection when the dragged card is part of
    /// it, otherwise the card alone.
    pub ids: Vec<Uuid>,
    /// The card under the pointer when the drag started.
    pub origin: Uuid,
    /// Groups among `ids`.
    pub groups: Vec<Uuid>,
    pub label: SharedString,
}

/// Stable element id of a host card (grid and list layouts).
pub fn host_card_id(id: Uuid) -> SharedString {
    format!("host-card-{id}").into()
}

/// Stable element id of a group card (grid and list layouts).
pub fn group_card_id(id: Uuid) -> SharedString {
    format!("group-card-{id}").into()
}

/// Stable element id of a tag filter chip.
pub fn tag_chip_id(tag: &str) -> SharedString {
    format!("tag-chip-{}", tag.to_lowercase()).into()
}

/// One row of the virtualized content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    GroupsHeading,
    /// Groups `start..start + columns` of the listing (one in the list layout).
    Groups(usize),
    HostsHeading,
    /// Hosts `start..start + columns` of the listing (one in the list layout).
    Hosts(usize),
}

pub struct HostsScreen {
    state: Entity<AppState>,
    search: Entity<InputState>,
    filter: HostsFilter,
    layout: HostsLayout,
    sort: HostsSort,
    /// Snapshot of the state's lists, refreshed on `RecordsChanged`.
    hosts: Rc<Vec<HostItem>>,
    groups: Rc<Vec<GroupItem>>,
    listing: HostsListing,
    /// Every tag in use (tag filter chips).
    tags: Vec<String>,
    search_target: SearchTarget,
    rows: Vec<Row>,
    row_sizes: Rc<Vec<Size<Pixels>>>,
    /// (columns, rows, width) the rows were built for.
    layout_key: (usize, usize, i32),
    columns: usize,
    card_width: f32,
    /// Width of the content area, measured after layout (the details panel
    /// narrows it).
    measured_width: Option<f32>,
    scroll: VirtualListScrollHandle,
    /// Sideways scroll of the tag filter strip (its edge fades read it).
    tag_scroll: ScrollHandle,
    /// Keyboard focus of the cards (Enter, E, arrows).
    focus: FocusHandle,
    /// The card of the last single click: a second click counts as a double
    /// click only on the same card.
    last_click: Option<Uuid>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for HostsScreen {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HostsScreen")
            .field("hosts", &self.listing.hosts.len())
            .field("groups", &self.listing.groups.len())
            .finish_non_exhaustive()
    }
}

impl EventEmitter<HostsEvent> for HostsScreen {}
impl EventEmitter<HostsRequest> for HostsScreen {}

impl Focusable for HostsScreen {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl HostsScreen {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        // No autofocus: the search bar is focused only by the user (2.1).
        let search =
            cx.new(|cx| InputState::new(window, cx).placeholder(strings::SEARCH_PLACEHOLDER));
        let search_subscription = cx.subscribe_in(
            &search,
            window,
            |this: &mut Self, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    this.filter.set_text(text);
                    this.refresh(cx);
                    this.scroll_to_top();
                }
                InputEvent::PressEnter { .. } => this.connect_from_search(window, cx),
                InputEvent::Focus | InputEvent::Blur => cx.notify(),
            },
        );
        let state_subscription =
            cx.subscribe(&state, |this: &mut Self, _, event, cx| match event {
                AppStateEvent::RecordsChanged => this.refresh(cx),
                AppStateEvent::LocationChanged(_) => {
                    this.refresh(cx);
                    this.scroll_to_top();
                }
                AppStateEvent::SelectionChanged => cx.notify(),
                AppStateEvent::LockChanged(_) => {}
            });
        let (layout, sort) = {
            let state = state.read(cx);
            (state.hosts_layout(), state.hosts_sort())
        };
        let mut this = Self {
            state,
            search,
            filter: HostsFilter::default(),
            layout,
            sort,
            hosts: Rc::default(),
            groups: Rc::default(),
            listing: HostsListing::default(),
            tags: Vec::new(),
            search_target: SearchTarget::None,
            rows: Vec::new(),
            row_sizes: Rc::default(),
            layout_key: (0, 0, 0),
            columns: 1,
            card_width: 0.,
            measured_width: None,
            scroll: VirtualListScrollHandle::new(),
            tag_scroll: ScrollHandle::new(),
            // The cards are one tab stop; the arrows move inside them.
            focus: cx.focus_handle().tab_stop(true),
            last_click: None,
            _subscriptions: vec![search_subscription, state_subscription],
        };
        this.refresh(cx);
        this
    }

    // -- Reading (tests and the shell) ----------------------------------------

    /// The search input (tests type into it through its element id `search`).
    pub fn search(&self) -> &Entity<InputState> {
        &self.search
    }

    /// Whether the search field has the keyboard focus (its one focus ring).
    pub fn search_focused(&self, window: &Window, cx: &App) -> bool {
        self.search
            .read(cx)
            .focus_handle(cx)
            .contains_focused(window, cx)
    }

    /// What Enter / Connect in the search bar would do.
    pub fn search_target(&self) -> &SearchTarget {
        &self.search_target
    }

    /// Whether the search bar's Connect is enabled (and the accent button).
    pub fn connect_enabled(&self) -> bool {
        self.search_target != SearchTarget::None
    }

    /// Ids of the host cards on screen, in screen order.
    pub fn visible_host_ids(&self) -> Vec<Uuid> {
        self.listing
            .hosts
            .iter()
            .map(|&ix| self.hosts[ix].record_id)
            .collect()
    }

    /// Ids of the group cards on screen, in screen order.
    pub fn visible_group_ids(&self) -> Vec<Uuid> {
        self.listing
            .groups
            .iter()
            .map(|&ix| self.groups[ix].record_id)
            .collect()
    }

    /// Every card on screen: groups first, then hosts (selection order).
    fn visible_ids(&self) -> Vec<Uuid> {
        let mut ids = self.visible_group_ids();
        ids.extend(self.visible_host_ids());
        ids
    }

    pub fn layout(&self) -> HostsLayout {
        self.layout
    }

    pub fn sort(&self) -> HostsSort {
        self.sort
    }

    pub fn filter(&self) -> &HostsFilter {
        &self.filter
    }

    /// Every tag in use (the tag filter chips).
    pub fn tags(&self) -> &[String] {
        &self.tags
    }

    /// Card columns of the last layout (1 in the list layout).
    pub fn columns(&self) -> usize {
        self.columns
    }

    /// The width of one card column of the last layout, in pixels.
    pub fn card_width(&self) -> f32 {
        self.card_width
    }

    /// The context menu a right click on `card` would show (the model).
    pub fn context_menu_for(&self, card: Uuid, cx: &App) -> Vec<MenuEntry> {
        let targets = self.menu_targets(card, cx);
        let move_targets = self.move_targets(&targets, cx);
        if self.is_group(card) {
            menu::group_menu(targets.len(), &move_targets)
        } else {
            menu::host_menu(targets.len(), &move_targets)
        }
    }

    // -- Changing the view ------------------------------------------------------

    /// Tiles or list (remembered in the settings).
    pub fn set_layout(&mut self, layout: HostsLayout, cx: &mut Context<Self>) {
        if self.layout == layout {
            return;
        }
        self.layout = layout;
        if let Err(error) = self
            .state
            .update(cx, |state, _| state.set_hosts_layout(layout))
        {
            tracing::warn!(code = error.code(), "hosts layout not saved");
        }
        self.layout_key = (0, 0, 0);
        cx.notify();
    }

    /// Manual, Name, Date created or Date modified (remembered).
    pub fn set_sort(&mut self, sort: HostsSort, cx: &mut Context<Self>) {
        if self.sort == sort {
            return;
        }
        self.sort = sort;
        if let Err(error) = self.state.update(cx, |state, _| state.set_hosts_sort(sort)) {
            tracing::warn!(code = error.code(), "hosts sort not saved");
        }
        self.refresh(cx);
    }

    /// Turns a tag filter chip on or off.
    pub fn toggle_tag(&mut self, tag: &str, cx: &mut Context<Self>) {
        self.filter.toggle_tag(tag);
        self.refresh(cx);
        self.scroll_to_top();
    }

    /// New results start at the top.
    fn scroll_to_top(&self) {
        self.scroll.scroll_to_item(0, gpui::ScrollStrategy::Top);
    }

    /// Re-reads the lists from the state and applies location, filter and
    /// sort; recomputes the search target.
    fn refresh(&mut self, cx: &mut Context<Self>) {
        let state = self.state.read(cx);
        self.hosts = state.hosts();
        self.groups = state.groups();
        let location = state.location();
        self.tags = hosts_view::all_tags(&self.hosts);
        self.filter.retain_tags(&self.tags);
        self.listing =
            hosts_view::listing(&self.hosts, &self.groups, location, &self.filter, self.sort);
        self.update_search_target(cx);
        // Force a rebuild of the rows on the next render.
        self.layout_key = (0, 0, 0);
        cx.notify();
    }

    fn update_search_target(&mut self, cx: &mut Context<Self>) {
        let text = self.filter.text().trim();
        let target = if text.is_empty() {
            SearchTarget::None
        } else if let Some(host) = exact_match(&self.hosts, text) {
            SearchTarget::Saved(host)
        } else if let Some(target) = parse_address(text) {
            SearchTarget::Quick(target)
        } else {
            SearchTarget::None
        };
        let was_enabled = self.connect_enabled();
        self.search_target = target;
        if was_enabled != self.connect_enabled() {
            cx.emit(HostsRequest::SearchAddressChanged(self.connect_enabled()));
        }
    }

    fn is_group(&self, id: Uuid) -> bool {
        self.groups.iter().any(|group| group.record_id == id)
    }

    fn host(&self, id: Uuid) -> Option<&HostItem> {
        self.hosts.iter().find(|host| host.record_id == id)
    }

    fn group(&self, id: Uuid) -> Option<&GroupItem> {
        self.groups.iter().find(|group| group.record_id == id)
    }

    fn title_of(&self, id: Uuid) -> Option<String> {
        self.host(id)
            .map(|host| host.title().to_owned())
            .or_else(|| self.group(id).map(|group| group.name.clone()))
    }

    // -- Clicks and keys ------------------------------------------------------------

    fn select_mode(event: &ClickEvent) -> SelectMode {
        let modifiers = event.modifiers();
        if modifiers.shift {
            SelectMode::Range
        } else if modifiers.secondary() {
            SelectMode::Toggle
        } else {
            SelectMode::Replace
        }
    }

    /// Single click: select and open the panel; Ctrl/Shift: change the
    /// selection; double click: connect (UI 2.4).
    fn on_host_click(
        &mut self,
        id: Uuid,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        let mode = Self::select_mode(event);
        if event.click_count() >= 2 && mode == SelectMode::Replace {
            // The double click connects the card of its first click: that
            // click opened the details panel, which narrows the grid, so the
            // second click may land on a card that moved under the pointer.
            // After a click on a group card (which opened the group) there is
            // no first card, and nothing connects.
            if let Some(first) = self.last_click.take() {
                self.connect_hosts(&[first], SessionKind::Ssh, event.modifiers().alt, cx);
            }
            return;
        }
        self.last_click = Some(id);
        let order = self.visible_ids();
        self.state
            .update(cx, |state, cx| state.select(id, mode, &order, cx));
        if mode == SelectMode::Replace {
            cx.emit(HostsEvent::OpenHost(id));
        }
    }

    /// Click on a group card: open it like a folder; Ctrl/Shift select it.
    fn on_group_click(
        &mut self,
        id: Uuid,
        event: &ClickEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus.focus(window, cx);
        let mode = Self::select_mode(event);
        if mode != SelectMode::Replace {
            let order = self.visible_ids();
            self.state
                .update(cx, |state, cx| state.select(id, mode, &order, cx));
            return;
        }
        if event.click_count() >= 2 {
            // The first click already opened the group.
            return;
        }
        self.last_click = None;
        self.open_group(id, cx);
    }

    fn open_group(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.state
            .update(cx, |state, cx| state.set_location(Some(id), cx));
    }

    /// Enter, E and the arrows on the focused cards (UI 2.4).
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;
        if keystroke.modifiers.modified() {
            return;
        }
        let selection = self.state.read(cx).selection().clone();
        match keystroke.key.as_str() {
            "enter" => {
                if selection.is_empty() {
                    return;
                }
                cx.stop_propagation();
                if let Some(group) = selection.single().filter(|id| self.is_group(*id)) {
                    self.open_group(group, cx);
                } else {
                    let hosts: Vec<Uuid> = selection
                        .ids()
                        .iter()
                        .copied()
                        .filter(|id| !self.is_group(*id))
                        .collect();
                    self.connect_hosts(&hosts, SessionKind::Ssh, false, cx);
                }
            }
            "e" => {
                if let Some(id) = selection.single().filter(|id| !self.is_group(*id)) {
                    cx.stop_propagation();
                    cx.emit(HostsRequest::EditHost(id));
                }
            }
            key => {
                if let Some(arrow) = layout::Arrow::from_key(key) {
                    cx.stop_propagation();
                    self.move_selection(arrow, window, cx);
                }
            }
        }
    }

    /// Arrow keys: move the selection across the groups and the hosts.
    fn move_selection(
        &mut self,
        arrow: layout::Arrow,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let order = self.visible_ids();
        let groups = self.listing.groups.len();
        let current = {
            let selection = self.state.read(cx).selection();
            selection
                .anchor()
                .or_else(|| selection.ids().first().copied())
                .and_then(|id| order.iter().position(|item| *item == id))
        };
        let columns = match self.layout {
            HostsLayout::Tiles => self.columns,
            HostsLayout::List => 1,
        };
        let Some(target) = sections_target(groups, order.len() - groups, current, columns, arrow)
        else {
            return;
        };
        let id = order[target];
        self.state.update(cx, |state, cx| {
            state.select(id, SelectMode::Replace, &order, cx)
        });
        if let Some(row) = self.row_of(target) {
            self.scroll
                .scroll_to_item(row, gpui::ScrollStrategy::Nearest);
        }
        if target >= groups {
            cx.emit(HostsEvent::OpenHost(id));
        }
        cx.notify();
    }

    /// The virtual-list row that shows item `index` of `visible_ids`.
    fn row_of(&self, index: usize) -> Option<usize> {
        let groups = self.listing.groups.len();
        self.rows.iter().position(|row| match *row {
            Row::Groups(start) => index < groups && (start..start + self.columns).contains(&index),
            Row::Hosts(start) => {
                index >= groups && (start..start + self.columns).contains(&(index - groups))
            }
            Row::GroupsHeading | Row::HostsHeading => false,
        })
    }

    // -- Search and connect -------------------------------------------------------

    /// Enter or Connect in the search bar (UI 2.1).
    fn connect_from_search(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        match self.search_target.clone() {
            SearchTarget::None => {}
            SearchTarget::Saved(id) => self.connect_hosts(&[id], SessionKind::Ssh, false, cx),
            SearchTarget::Quick(target) => {
                cx.emit(HostsRequest::OpenSession(SessionRequest {
                    kind: SessionKind::Ssh,
                    host: None,
                    title: target.title().into(),
                    target: Some(target.clone()),
                    new_window: false,
                }));
                // The offer to save the host after a quick connect (2.1).
                let screen = cx.entity().downgrade();
                cx.emit(HostsRequest::ToastWithAction {
                    message: strings::SAVE_HOST_OFFER.into(),
                    action: ToastAction {
                        id: "toast-save-host".into(),
                        label: strings::SAVE_HOST_ACTION.into(),
                        run: Rc::new(move |_, cx| {
                            let target = target.clone();
                            let _ = screen
                                .update(cx, |screen, cx| screen.save_quick_target(&target, cx));
                        }),
                    },
                });
            }
        }
    }

    /// "Save host?" accepted: creates the host from the quick-connect
    /// target, selects it and opens its panel.
    pub fn save_quick_target(&mut self, target: &QuickTarget, cx: &mut Context<Self>) {
        let mut data = Map::new();
        data.insert("address".to_owned(), Value::from(target.host.clone()));
        let mut ssh = Map::new();
        if let Some(user) = &target.username {
            ssh.insert("username".to_owned(), Value::from(user.clone()));
        }
        if let Some(port) = target.port {
            ssh.insert("port".to_owned(), json!(port));
        }
        if !ssh.is_empty() {
            data.insert("ssh".to_owned(), Value::Object(ssh));
        }
        let result = self
            .state
            .update(cx, |state, cx| state.save_host(None, data, cx));
        match result {
            Ok(host) => {
                let id = host.record_id;
                let order = self.visible_ids();
                self.state.update(cx, |state, cx| {
                    state.select(id, SelectMode::Replace, &order, cx)
                });
                cx.emit(HostsEvent::OpenHost(id));
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "quick-connect host not saved");
                cx.emit(HostsEvent::Toast(strings::FAILED.into()));
            }
        }
    }

    /// Opens one session tab per host.
    fn connect_hosts(
        &mut self,
        ids: &[Uuid],
        kind: SessionKind,
        new_window: bool,
        cx: &mut Context<Self>,
    ) {
        for &id in ids {
            let Some(host) = self.host(id) else {
                continue;
            };
            let title: SharedString = host.title().to_owned().into();
            cx.emit(HostsRequest::OpenSession(SessionRequest {
                kind,
                host: Some(id),
                target: None,
                title,
                new_window,
            }));
        }
    }

    /// Opens a local terminal or serial session tab (placeholders).
    fn open_local(&mut self, kind: SessionKind, cx: &mut Context<Self>) {
        let title = match kind {
            SessionKind::Serial => strings::SERIAL,
            _ => strings::TERMINAL,
        };
        cx.emit(HostsRequest::OpenSession(SessionRequest {
            kind,
            host: None,
            target: None,
            title: title.into(),
            new_window: false,
        }));
    }

    // -- Context menu commands -------------------------------------------------------

    /// The records a menu opened on `card` acts on: the selection when the
    /// card is part of it, otherwise the card alone.
    fn menu_targets(&self, card: Uuid, cx: &App) -> Vec<Uuid> {
        let selection = self.state.read(cx).selection();
        if selection.contains(&card) && selection.len() > 1 {
            selection.ids().to_vec()
        } else {
            vec![card]
        }
    }

    /// Right click: a card outside the selection becomes the selection.
    fn prepare_menu(&mut self, card: Uuid, cx: &mut Context<Self>) -> Vec<Uuid> {
        let selected = self.state.read(cx).selection().contains(&card);
        if !selected {
            let order = self.visible_ids();
            self.state.update(cx, |state, cx| {
                state.select(card, SelectMode::Replace, &order, cx)
            });
        }
        self.menu_targets(card, cx)
    }

    /// The Move to ▸ targets for `records`: the top level and every group in
    /// tree order; inactive where the records already are and inside a
    /// moved group.
    fn move_targets(&self, records: &[Uuid], cx: &App) -> Vec<menu::MoveTarget> {
        let mut blocked: HashSet<Uuid> = HashSet::new();
        for id in records {
            if self.is_group(*id) {
                blocked.extend(subtree_of(&self.groups, *id));
            }
        }
        let parents: HashSet<Option<Uuid>> = records
            .iter()
            .map(|id| {
                self.host(*id)
                    .map(|host| host.group_id)
                    .or_else(|| self.group(*id).map(|group| group.parent_id))
                    .unwrap_or(self.state.read(cx).location())
            })
            .collect();
        let all_in = |parent: Option<Uuid>| parents.len() == 1 && parents.contains(&parent);
        let mut targets = vec![menu::MoveTarget {
            group: None,
            name: strings::MENU_TOP_LEVEL.to_owned(),
            depth: 0,
            enabled: !all_in(None),
        }];
        targets.extend(group_tree(&self.groups).into_iter().map(|(ix, depth)| {
            let group = &self.groups[ix];
            let id = group.record_id;
            menu::MoveTarget {
                group: Some(id),
                name: group.name.clone(),
                depth,
                enabled: !blocked.contains(&id) && !all_in(Some(id)),
            }
        }));
        targets
    }

    /// Runs a context menu command on `records`.
    pub fn run_command(
        &mut self,
        command: MenuCommand,
        records: Vec<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let hosts: Vec<Uuid> = records
            .iter()
            .copied()
            .filter(|id| !self.is_group(*id))
            .collect();
        let alt = window.modifiers().alt;
        match command {
            MenuCommand::QuickConnect | MenuCommand::ConnectSsh => {
                self.connect_hosts(&hosts, SessionKind::Ssh, alt, cx)
            }
            MenuCommand::ConnectSftp => self.connect_hosts(&hosts, SessionKind::Sftp, alt, cx),
            MenuCommand::EditHost => {
                if let [id] = hosts.as_slice() {
                    cx.emit(HostsRequest::EditHost(*id));
                }
            }
            MenuCommand::Collaborate | MenuCommand::MoveToVault | MenuCommand::CopyToVault => {
                dialogs::open_placeholder("vault-dialog", strings::MOVE_OR_COPY_TITLE, window, cx);
            }
            MenuCommand::MoveTo(group) => self.move_records(records, group, cx),
            MenuCommand::Duplicate => {
                if let [id] = hosts.as_slice() {
                    self.duplicate(*id, cx);
                }
            }
            MenuCommand::CopyLink => {
                if let [id] = hosts.as_slice() {
                    cx.write_to_clipboard(ClipboardItem::new_string(format!(
                        "shellpp://host/{id}"
                    )));
                    cx.emit(HostsEvent::Toast(strings::LINK_COPIED.into()));
                }
            }
            MenuCommand::Remove => self.confirm_remove(records, window, cx),
            MenuCommand::OpenGroup => {
                if let [id] = records.as_slice() {
                    self.open_group(*id, cx);
                }
            }
            MenuCommand::EditGroup => {
                if let [id] = records.as_slice() {
                    cx.emit(HostsEvent::EditGroup(*id));
                }
            }
        }
    }

    fn duplicate(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let result = self
            .state
            .update(cx, |state, cx| state.duplicate_record(id, cx));
        match result {
            Ok(view) => {
                let copy = view.record_id();
                let order = self.visible_ids();
                self.state.update(cx, |state, cx| {
                    state.select(copy, SelectMode::Replace, &order, cx)
                });
                cx.emit(HostsEvent::OpenHost(copy));
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "duplicate failed");
                cx.emit(HostsEvent::Toast(strings::FAILED.into()));
            }
        }
    }

    /// Moves records into a group (`None`: the top level), at the end.
    fn move_records(&mut self, records: Vec<Uuid>, group: Option<Uuid>, cx: &mut Context<Self>) {
        if records.is_empty() {
            return;
        }
        let result = self
            .state
            .update(cx, |state, cx| state.move_records(records, group, cx));
        match result {
            Ok(()) => {
                let name = group.and_then(|id| self.group(id).map(|group| group.name.clone()));
                cx.emit(HostsEvent::Toast(strings::moved_to(name.as_deref()).into()));
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "move failed");
                cx.emit(HostsEvent::Toast(strings::FAILED.into()));
            }
        }
    }

    /// Remove: a confirmation dialog first (UI 2.4).
    fn confirm_remove(&mut self, records: Vec<Uuid>, window: &mut Window, cx: &mut Context<Self>) {
        if records.is_empty() {
            return;
        }
        let groups = records.iter().filter(|id| self.is_group(**id)).count();
        let non_empty = records
            .iter()
            .filter_map(|id| self.group(*id))
            .filter(|group| group.child_count > 0)
            .count();
        let name = match records.as_slice() {
            [id] => self.title_of(*id),
            _ => None,
        };
        let title = strings::remove_title(records.len(), groups);
        let body = strings::remove_body(name.as_deref(), records.len(), non_empty);
        let screen = cx.entity().downgrade();
        dialogs::open_remove_confirmation(title, body, window, cx, move |_, cx| {
            let records = records.clone();
            let name = name.clone();
            let _ = screen.update(cx, |screen, cx| screen.remove_now(records, name, cx));
        });
    }

    /// Removes the records and offers Undo for 5 seconds.
    pub fn remove_now(&mut self, records: Vec<Uuid>, name: Option<String>, cx: &mut Context<Self>) {
        let count = records.len();
        let result = self
            .state
            .update(cx, |state, cx| state.delete_records(records, cx));
        match result {
            Ok(result) => {
                let deleted = result.deleted;
                let state = self.state.clone();
                cx.emit(HostsRequest::ToastWithAction {
                    message: strings::removed(name.as_deref(), count).into(),
                    action: ToastAction {
                        id: "toast-undo".into(),
                        label: strings::UNDO.into(),
                        run: Rc::new(move |_, cx| {
                            let deleted = deleted.clone();
                            if let Err(error) =
                                state.update(cx, |state, cx| state.restore_records(deleted, cx))
                            {
                                tracing::warn!(code = error.code(), "undo of remove failed");
                            }
                        }),
                    },
                });
            }
            Err(error) => {
                tracing::warn!(code = error.code(), "remove failed");
                cx.emit(HostsEvent::Toast(strings::FAILED.into()));
            }
        }
    }

    // -- Drag and drop ---------------------------------------------------------------

    /// The value of a drag that starts on `card`.
    fn drag_value(&self, card: Uuid, cx: &App) -> DraggedCards {
        let ids = self.menu_targets(card, cx);
        let groups: Vec<Uuid> = ids
            .iter()
            .copied()
            .filter(|id| self.is_group(*id))
            .collect();
        let label: SharedString = match ids.as_slice() {
            [id] => self.title_of(*id).unwrap_or_default().into(),
            _ => format!("{} items", ids.len()).into(),
        };
        DraggedCards {
            ids,
            origin: card,
            groups,
            label,
        }
    }

    /// Whether a drop onto group `target` is allowed (never into itself or
    /// below a dragged group).
    fn can_drop_into(&self, dragged: &DraggedCards, target: Option<Uuid>) -> bool {
        let Some(target) = target else {
            return true;
        };
        !dragged
            .groups
            .iter()
            .any(|group| subtree_of(&self.groups, *group).contains(&target))
    }

    /// Whether dragging reorders cards: Manual sort, a folder (no filter),
    /// hosts only.
    fn can_reorder(&self, dragged: &DraggedCards, target: Uuid) -> bool {
        self.sort == HostsSort::Manual
            && !self.filter.is_active()
            && dragged.groups.is_empty()
            && !dragged.ids.contains(&target)
    }

    /// Drop on a group card or a breadcrumb: move into it (any sort mode).
    fn drop_into(&mut self, dragged: &DraggedCards, target: Option<Uuid>, cx: &mut Context<Self>) {
        if !self.can_drop_into(dragged, target) {
            return;
        }
        let ids: Vec<Uuid> = dragged
            .ids
            .iter()
            .copied()
            .filter(|id| Some(*id) != target)
            .collect();
        self.move_records(ids, target, cx);
    }

    /// Drop on a host card in Manual mode: the dragged hosts go before the
    /// target when they came from below it, else after it.
    fn drop_on_host(&mut self, dragged: &DraggedCards, target: Uuid, cx: &mut Context<Self>) {
        if !self.can_reorder(dragged, target) {
            return;
        }
        let order = self.visible_host_ids();
        let from = order.iter().position(|id| *id == dragged.origin);
        let to = order.iter().position(|id| *id == target);
        let position = match (from, to) {
            (Some(from), Some(to)) if from > to => MovePosition::Before { record_id: target },
            _ => MovePosition::After { record_id: target },
        };
        let location = self.state.read(cx).location();
        let ids = dragged.ids.clone();
        let result = self.state.update(cx, |state, cx| {
            state.move_records_to(ids, location, position, cx)
        });
        if let Err(error) = result {
            tracing::warn!(code = error.code(), "reorder failed");
            cx.emit(HostsEvent::Toast(strings::FAILED.into()));
        }
    }
}

/// The first host whose label or address equals `text` (case-insensitive).
fn exact_match(hosts: &[HostItem], text: &str) -> Option<Uuid> {
    let exact = hosts
        .iter()
        .find(|host| host.label.as_deref() == Some(text) || host.address == text);
    exact
        .or_else(|| {
            hosts.iter().find(|host| {
                host.label
                    .as_deref()
                    .is_some_and(|label| label.eq_ignore_ascii_case(text))
                    || host.address.eq_ignore_ascii_case(text)
            })
        })
        .map(|host| host.record_id)
}

/// Arrow navigation over two sections (groups, then hosts), each laid out
/// in rows of `columns`. Up from the first host row goes to the last group
/// row; down from the last group row goes to the first host row.
fn sections_target(
    groups: usize,
    hosts: usize,
    current: Option<usize>,
    columns: usize,
    arrow: layout::Arrow,
) -> Option<usize> {
    use layout::Arrow;
    let total = groups + hosts;
    if total == 0 {
        return None;
    }
    let Some(current) = current.filter(|current| *current < total) else {
        return Some(0);
    };
    let columns = columns.max(1);
    let (start, len) = if current < groups {
        (0, groups)
    } else {
        (groups, hosts)
    };
    let local = current - start;
    let next = layout::arrow_target(Some(local), len, columns, arrow).unwrap_or(local);
    if next != local {
        return Some(start + next);
    }
    let column = local % columns;
    let in_groups = start == 0 && groups > 0;
    match arrow {
        Arrow::Down if in_groups && hosts > 0 => Some(groups + column.min(hosts - 1)),
        Arrow::Right if in_groups && hosts > 0 && local + 1 == len => Some(groups),
        Arrow::Up if !in_groups && groups > 0 && local < columns => {
            let last_row = (groups - 1) / columns * columns;
            Some((last_row + column).min(groups - 1))
        }
        Arrow::Left if !in_groups && groups > 0 && local == 0 => Some(groups - 1),
        _ => Some(current),
    }
}
