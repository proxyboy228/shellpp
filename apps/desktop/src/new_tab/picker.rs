//! The search shared by the new tab page, the Ctrl+K overlay and the pane
//! picker: the search field, the list ([`model`]) with its keyboard
//! selection, and the recent connections read from the core.
//!
//! A view embeds a [`Picker`] and implements [`PickerHost`]; the generic
//! helpers here render the field and the list and handle the keys for it.
//!
//! Look (design-system 2, 5, 6): the field is 36 px, `--bg-input`, 1 px
//! `--border-strong`, radius 6, a search icon and one focus ring. A row is
//! 44 px (`--height-row`), radius 6: a 28 px tile with the white OS icon on
//! the host's color (`--tag-graphite` without one), the name in
//! `--text-primary` and the detail ("vault / group") right-aligned in
//! `--text-secondary`. The selected row has `--accent-subtle`; hover is
//! `--bg-raised-hover` (`--bg-overlay-hover` in the overlay), pressed
//! `--bg-pressed`. A section heading ("Open tabs", "Hosts") is a 28 px row
//! with a 12 px caption in `--text-secondary` [decision].

use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, ClickEvent, Context, Div, Entity, EventEmitter, FocusHandle, Focusable as _,
    IntoElement, Keystroke, Pixels, Render, ScrollStrategy, SharedString, Size, Stateful,
    Subscription, Task, Window, div, px, relative, size,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::base::input::{MoveDown, MoveUp};
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::kbd::Kbd;
use gpui_kit::component::scroll::Scrollbar;
use gpui_kit::component::{
    ActiveTheme as _, Icon, IconName as KitIcon, VirtualListScrollHandle, h_flex, v_flex,
    v_virtual_list,
};
use shellpp_core::service::dto::{RecentConnection, VaultKind};

use super::model::{self, Item, PickerKind, Row, Sources};
use super::{ConfirmInNewWindow, KEY_CONTEXT, NewTabEvent, OpenTab, PickerOrigin, strings};
use crate::hosts::controls::{ICON_SIZE, focus_ring};
use crate::state::{AppState, AppStateEvent};
use crate::theme::{self, metrics, palette};

/// Height of a section heading row of a list [decision].
pub const HEADING_HEIGHT: gpui::Rems = metrics::rpx(28.);
/// The tile of a row (the tile of the Hosts list layout).
const TILE_SIZE: gpui::Rems = metrics::rpx(28.);
/// Width of the column of the page and the pane picker, and of the overlay
/// [decision]: the widest details panel (`--width-details-max`); narrower
/// areas use their own width.
pub const COLUMN_WIDTH: gpui::Rems = metrics::WIDTH_DETAILS_MAX;
/// The vault name until the core has answered.
const DEFAULT_VAULT_NAME: &str = crate::i18n::top_bar::PERSONAL_VAULT;

/// A view that embeds a [`Picker`].
pub(crate) trait PickerHost: EventEmitter<NewTabEvent> + Render + Sized + 'static {
    fn picker(&self) -> &Picker;
    fn picker_mut(&mut self) -> &mut Picker;
}

/// The search: field, list, selection and data.
pub(crate) struct Picker {
    kind: PickerKind,
    /// Element id prefix: `new-tab`, `search-overlay`, `pane-picker`.
    prefix: &'static str,
    state: Entity<AppState>,
    search: Entity<InputState>,
    query: String,
    tabs: Vec<OpenTab>,
    /// The saved workspaces the search finds (UI 4.5).
    workspaces: Vec<crate::state::WorkspaceItem>,
    origin: Option<PickerOrigin>,
    recents: Vec<RecentConnection>,
    vault_name: String,
    items: Vec<Item>,
    sizes: Rc<Vec<Size<Pixels>>>,
    /// Index into `items` (always a row).
    selected: Option<usize>,
    /// The user moved the selection (arrows, a click) since the text last
    /// changed: a list update keeps that row selected; otherwise the first
    /// row is selected again.
    moved: bool,
    scroll: VirtualListScrollHandle,
    /// Increases with every load; an older answer is dropped.
    generation: u64,
    load: Option<Task<()>>,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for Picker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Picker")
            .field("kind", &self.kind)
            .field("items", &self.items.len())
            .field("selected", &self.selected)
            .finish_non_exhaustive()
    }
}

impl Picker {
    /// A picker for the view `V` being built; the search field takes the
    /// focus at once and the recent connections are read in the background.
    pub fn new<V: PickerHost>(
        kind: PickerKind,
        prefix: &'static str,
        state: Entity<AppState>,
        origin: Option<PickerOrigin>,
        window: &mut Window,
        cx: &mut Context<V>,
    ) -> Self {
        super::bind_keys(cx);
        let placeholder = match kind {
            PickerKind::Compact => strings::PANE_SEARCH_PLACEHOLDER,
            PickerKind::Page | PickerKind::Overlay => strings::SEARCH_PLACEHOLDER,
        };
        let search = cx.new(|cx| InputState::new(window, cx).placeholder(placeholder));
        let input = cx.subscribe_in(
            &search,
            window,
            |this: &mut V, input, event: &InputEvent, window, cx| match event {
                InputEvent::Change => {
                    let text = input.read(cx).value().to_string();
                    let picker = this.picker_mut();
                    if picker.query != text {
                        picker.query = text;
                        picker.rebuild(true, cx);
                    }
                    cx.notify();
                }
                InputEvent::PressEnter { .. } => {
                    let alt = window.modifiers().alt;
                    choose(this, alt, cx);
                }
                InputEvent::Focus | InputEvent::Blur => cx.notify(),
            },
        );
        let records = cx.subscribe(
            &state,
            |this: &mut V, _, event: &AppStateEvent, cx| match event {
                AppStateEvent::RecordsChanged | AppStateEvent::LockChanged(_) => {
                    this.picker_mut().rebuild(false, cx);
                    load(this, cx);
                    cx.notify();
                }
                AppStateEvent::SelectionChanged
                | AppStateEvent::LocationChanged(_)
                | AppStateEvent::AutoLockDue => {}
            },
        );
        search.update(cx, |input, cx| input.focus(window, cx));
        let mut picker = Self {
            kind,
            prefix,
            state,
            search,
            query: String::new(),
            tabs: Vec::new(),
            workspaces: Vec::new(),
            origin,
            recents: Vec::new(),
            vault_name: DEFAULT_VAULT_NAME.to_owned(),
            items: Vec::new(),
            sizes: Rc::default(),
            selected: None,
            moved: false,
            scroll: VirtualListScrollHandle::new(),
            generation: 0,
            load: None,
            _subscriptions: vec![input, records],
        };
        picker.rebuild(true, cx);
        picker.load = picker.spawn_load(cx);
        picker
    }

    // -- Reading ---------------------------------------------------------------

    /// The search input (its element id is `{prefix}-search`).
    pub fn search(&self) -> &Entity<InputState> {
        &self.search
    }

    /// The focus handle of the search field.
    pub fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.search.read(cx).focus_handle(cx)
    }

    /// The search text as typed.
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Whether the list shows search results (not Recent connections).
    pub fn is_searching(&self) -> bool {
        !self.query.trim().is_empty()
    }

    /// The entries of the list, in order.
    pub fn items(&self) -> &[Item] {
        &self.items
    }

    /// The rows of the list, in order.
    pub fn rows(&self) -> Vec<&Row> {
        self.items.iter().filter_map(Item::row).collect()
    }

    /// The selected row (Enter chooses it).
    pub fn selected_row(&self) -> Option<&Row> {
        self.selected
            .and_then(|ix| self.items.get(ix))
            .and_then(Item::row)
    }

    /// The element id of a row.
    pub fn row_id(&self, row: &Row) -> SharedString {
        format!("{}-row-{}", self.prefix, row.key).into()
    }

    // -- Writing ---------------------------------------------------------------

    /// The open tabs the search offers.
    pub fn set_open_tabs(&mut self, tabs: Vec<OpenTab>, cx: &App) -> bool {
        if self.tabs == tabs {
            return false;
        }
        self.tabs = tabs;
        self.rebuild(false, cx);
        true
    }

    /// The saved workspaces the search finds (UI 4.5).
    pub fn set_workspaces(
        &mut self,
        workspaces: Vec<crate::state::WorkspaceItem>,
        cx: &App,
    ) -> bool {
        if self.workspaces == workspaces {
            return false;
        }
        self.workspaces = workspaces;
        self.rebuild(false, cx);
        true
    }

    /// The split pane's target (the pane picker's first row).
    pub fn set_origin(&mut self, origin: Option<PickerOrigin>, cx: &App) -> bool {
        if self.origin == origin {
            return false;
        }
        self.origin = origin;
        self.rebuild(true, cx);
        true
    }

    /// Moves the selection to the next (`forward`) or previous row.
    pub fn step(&mut self, forward: bool) {
        self.selected = model::step(&self.items, self.selected, forward);
        self.moved = true;
        if let Some(ix) = self.selected {
            self.scroll.scroll_to_item(ix, ScrollStrategy::Nearest);
        }
    }

    /// Selects the row at `ix` of the list (a click).
    fn select(&mut self, ix: usize) {
        if self.items.get(ix).is_some_and(|item| item.row().is_some()) {
            self.selected = Some(ix);
            self.moved = true;
        }
    }

    /// Rebuilds the list from the sources. `reset` (a new search text)
    /// selects the first row and scrolls to the top; otherwise a row the
    /// user selected stays selected while it is still listed.
    fn rebuild(&mut self, reset: bool, cx: &App) {
        let state = self.state.read(cx);
        let unlocked = state.is_unlocked();
        let hosts = state.hosts();
        let groups = state.groups();
        let recents: &[RecentConnection] = if unlocked { &self.recents } else { &[] };
        let sources = Sources {
            hosts: &hosts,
            groups: &groups,
            tabs: &self.tabs,
            recents,
            origin: self.origin.as_ref(),
            vault_name: &self.vault_name,
            workspaces: &self.workspaces,
        };
        if reset {
            self.moved = false;
        }
        let kept = self
            .moved
            .then(|| self.selected_row().map(|row| row.key.clone()))
            .flatten();
        self.items = model::items(self.kind, &self.query, &sources);
        self.sizes = Rc::new(
            self.items
                .iter()
                .map(|item| size(px(0.), item_height(item)))
                .collect(),
        );
        self.selected = kept
            .and_then(|key| {
                self.items
                    .iter()
                    .position(|item| item.row().is_some_and(|row| row.key == key))
            })
            .or_else(|| model::first_row(&self.items));
        if reset {
            self.scroll.scroll_to_item(0, ScrollStrategy::Top);
        }
    }

    /// Reads the recent connections and the vault name on a background
    /// thread. Nothing while locked.
    fn spawn_load<V: PickerHost>(&mut self, cx: &mut Context<V>) -> Option<Task<()>> {
        self.generation += 1;
        let generation = self.generation;
        let core = {
            let state = self.state.read(cx);
            state.is_unlocked().then(|| state.core().cloned()).flatten()
        };
        let Some(core) = core else {
            self.recents.clear();
            return None;
        };
        let work = cx.background_spawn(async move {
            let recents = core.recent_connections().unwrap_or_else(|error| {
                tracing::warn!(code = error.code(), "recent connections not read");
                Vec::new()
            });
            let vault_name = core.vaults().ok().and_then(|vaults| {
                vaults
                    .into_iter()
                    .find(|vault| vault.kind == VaultKind::Personal)
                    .map(|vault| vault.name)
            });
            (recents, vault_name)
        });
        Some(cx.spawn(async move |this, cx| {
            let (recents, vault_name) = work.await;
            let _ = this.update(cx, |this: &mut V, cx| {
                let picker = this.picker_mut();
                if picker.generation != generation {
                    return;
                }
                picker.recents = recents;
                if let Some(name) = vault_name {
                    picker.vault_name = name;
                }
                picker.rebuild(false, cx);
                cx.notify();
            });
        }))
    }

    // -- Rendering ---------------------------------------------------------------

    /// The search field; `hint` is the shortcut shown at its right end.
    pub fn render_field(
        &self,
        hint: Option<&str>,
        window: &Window,
        cx: &App,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let focused = self.focus_handle(cx).is_focused(window);
        let hint = hint
            .and_then(|key| Keystroke::parse(key).ok())
            .map(|key| shortcut_chip(Kbd::format(&key), cx));
        h_flex()
            .id(SharedString::from(format!("{}-search-field", self.prefix)))
            .test_support()
            .relative()
            .w_full()
            .h(metrics::HEIGHT_CONTROL)
            .flex_none()
            .gap(metrics::SPACE_2)
            .pl(metrics::SPACE_3)
            .pr(metrics::SPACE_2)
            .rounded(metrics::RADIUS_CONTROL)
            .bg(tokens.bg_input.hsla())
            .border_1()
            .border_color(tokens.border_strong.hsla())
            .child(
                Icon::new(KitIcon::Search)
                    .size(ICON_SIZE)
                    .text_color(theme.muted_foreground),
            )
            .child(
                // Our frame draws the border and the one focus ring; the kit
                // input draws none of its own. `pl_0`: the row's 8 px gap is
                // the space after the icon.
                div()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .child(theme::controls::muted_placeholder(&self.search, cx))
                    .child(
                        Input::new(&self.search)
                            .id(SharedString::from(format!("{}-search", self.prefix)))
                            .appearance(false)
                            .pl_0()
                            .text_size(metrics::FONT_SIZE_BODY),
                    ),
            )
            .children(hint)
            .when(focused, |this| {
                this.child(focus_ring(metrics::RADIUS_CONTROL, metrics::rpx(1.), cx))
            })
    }

    /// Total height of the list's entries, in design units.
    pub fn content_height(&self) -> Pixels {
        self.sizes
            .iter()
            .fold(Pixels::ZERO, |total, size| total + size.height)
    }

    /// The list, or its empty state. `height`: a fixed height in design
    /// units (the overlay); `None` fills the remaining space.
    pub fn render_list<V: PickerHost>(
        &self,
        height: Option<Pixels>,
        cx: &mut Context<V>,
    ) -> AnyElement {
        let theme = cx.theme();
        if self.items.is_empty() {
            let text = match (self.is_searching(), self.kind) {
                (false, _) => strings::NO_RECENT,
                (true, PickerKind::Compact) => strings::PANE_NO_MATCHES,
                (true, PickerKind::Page | PickerKind::Overlay) => strings::NO_MATCHES,
            };
            return div()
                .id(SharedString::from(format!("{}-empty", self.prefix)))
                .test_support()
                .w_full()
                .px(metrics::SPACE_3)
                .py(metrics::SPACE_3)
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(theme.muted_foreground)
                .child(text)
                .into_any_element();
        }
        let list = v_virtual_list(
            cx.entity(),
            SharedString::from(format!("{}-list", self.prefix)),
            metrics::scaled_sizes(&self.sizes, cx),
            |this: &mut V, range, _, cx| {
                range
                    .map(|ix| render_item(this, ix, cx))
                    .collect::<Vec<_>>()
            },
        )
        .track_scroll(&self.scroll)
        .size_full();
        v_flex()
            .relative()
            .w_full()
            .map(|this| match height {
                Some(height) => this.h(metrics::rems_of(height)).flex_none(),
                None => this.flex_1().min_h_0(),
            })
            .child(list)
            .child(Scrollbar::vertical(&self.scroll))
            .into_any_element()
    }
}

/// The height of a list entry, in design units (px at 100 %, D96).
fn item_height(item: &Item) -> Pixels {
    match item {
        Item::Heading(_) => metrics::at_100(HEADING_HEIGHT),
        Item::Row(_) => metrics::at_100(metrics::HEIGHT_ROW),
    }
}

/// The shortcut hint at the right end of the field (Ctrl+K): 12 px
/// `--text-secondary` in a 1 px `--border-strong` frame, radius 6.
fn shortcut_chip(text: String, cx: &App) -> Div {
    let theme = cx.theme();
    div()
        .flex_none()
        .h(metrics::rpx(24.))
        .px(metrics::SPACE_2)
        .flex()
        .items_center()
        .rounded(metrics::RADIUS_CONTROL)
        .border_1()
        .border_color(theme::tokens(cx).border_strong.hsla())
        .text_size(metrics::FONT_SIZE_CAPTION)
        .text_color(theme.muted_foreground)
        .child(text)
}

/// One entry of the list.
fn render_item<V: PickerHost>(this: &mut V, ix: usize, cx: &mut Context<V>) -> AnyElement {
    let picker = this.picker();
    match picker.items.get(ix) {
        Some(Item::Heading(label)) => div()
            .w_full()
            .h(HEADING_HEIGHT)
            .px(metrics::SPACE_3)
            .flex()
            .items_center()
            .text_size(metrics::FONT_SIZE_CAPTION)
            .font_weight(metrics::FONT_WEIGHT_MEDIUM)
            .text_color(cx.theme().muted_foreground)
            .child(*label)
            .into_any_element(),
        Some(Item::Row(row)) => render_row(picker, ix, row, cx),
        None => div().into_any_element(),
    }
}

fn render_row<V: PickerHost>(
    picker: &Picker,
    ix: usize,
    row: &Row,
    cx: &mut Context<V>,
) -> AnyElement {
    let theme = cx.theme();
    let tokens = theme::tokens(cx);
    let selected = picker.selected == Some(ix);
    let hover = match picker.kind {
        PickerKind::Overlay => tokens.bg_overlay_hover.hsla(),
        PickerKind::Page | PickerKind::Compact => tokens.bg_raised_hover.hsla(),
    };
    let pressed = tokens.bg_pressed.hsla();
    let tile = div()
        .flex_none()
        .size(TILE_SIZE)
        .rounded(metrics::RADIUS_TILE)
        .bg(palette::tag_color(row.color).hsla())
        .flex()
        .items_center()
        .justify_center()
        .child(
            Icon::new(row.icon)
                .size(ICON_SIZE)
                .text_color(palette::TEXT_ON_TAG.hsla()),
        );
    h_flex()
        .id(picker.row_id(row))
        .test_support()
        .w_full()
        .h(metrics::HEIGHT_ROW)
        .px(metrics::SPACE_3)
        .gap(metrics::SPACE_3)
        .items_center()
        .rounded(metrics::RADIUS_CONTROL)
        .map(|this| {
            if selected {
                this.bg(tokens.accent_subtle.hsla())
            } else {
                this.hover(move |style| style.bg(hover))
                    .active(move |style| style.bg(pressed))
            }
        })
        .child(tile)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(theme.foreground)
                .child(row.title.clone()),
        )
        .children(row.detail.clone().map(|detail| {
            div()
                .min_w_0()
                .max_w(relative(0.5))
                .truncate()
                .text_size(metrics::FONT_SIZE_CAPTION)
                .text_color(theme.muted_foreground)
                .child(detail)
        }))
        .on_click(cx.listener(move |this: &mut V, event: &ClickEvent, _, cx| {
            this.picker_mut().select(ix);
            choose(this, event.modifiers().alt, cx);
        }))
        .into_any_element()
}

/// Emits the event of the selected row (`new_window`: Alt was held).
pub(crate) fn choose<V: PickerHost>(this: &mut V, new_window: bool, cx: &mut Context<V>) {
    let event = this
        .picker()
        .selected_row()
        .map(|row| row.choice.event(new_window));
    if let Some(event) = event {
        cx.emit(event);
    }
    cx.notify();
}

/// Reads the recent connections again (records changed, unlock).
pub(crate) fn load<V: PickerHost>(this: &mut V, cx: &mut Context<V>) {
    let task = this.picker_mut().spawn_load(cx);
    this.picker_mut().load = task;
}

/// The key handling of the search on the element around the field and the
/// list: Up and Down move the selection (taken from the field before it
/// moves its caret), Alt+Enter chooses in a new window.
pub(crate) fn keys<V: PickerHost>(element: Stateful<Div>, cx: &mut Context<V>) -> Stateful<Div> {
    element
        .key_context(KEY_CONTEXT)
        .capture_action(cx.listener(|this: &mut V, _: &MoveUp, _, cx| {
            this.picker_mut().step(false);
            cx.stop_propagation();
            cx.notify();
        }))
        .capture_action(cx.listener(|this: &mut V, _: &MoveDown, _, cx| {
            this.picker_mut().step(true);
            cx.stop_propagation();
            cx.notify();
        }))
        .on_action(
            cx.listener(|this: &mut V, _: &ConfirmInNewWindow, _, cx| choose(this, true, cx)),
        )
}
