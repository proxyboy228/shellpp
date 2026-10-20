//! The panes of a session tab (UI 4.7, lead decision L1): a [`PaneGroup`]
//! holds a tree of panes ([`layout::Layout`]), each with its own session
//! ([`SessionPane`]) or, right after a split, the host selection
//! ([`picker`]).
//!
//! - **Split**: Ctrl+Shift+D (a new pane to the right) and Ctrl+Shift+E (a
//!   new pane below), or [`PaneGroup::split`] from the tab menu. The active
//!   pane is cut in half and the new pane, which shows the compact picker
//!   with the split pane's target first, becomes active.
//! - **Dividers** can be dragged; every pane keeps at least [`PANE_MIN`]
//!   (and 5 % of its split).
//! - **Active pane**: a click in a pane or Ctrl+Alt+arrows (the nearest
//!   pane on screen in that direction; nothing at the edge of the tab).
//!   With more than one pane, the active one has the 2 px `--accent` bar
//!   along its top edge (`--size-tab-indicator`, like the active tab).
//! - **Closing**: exit status 0 or "Close" of a pane closes that pane and
//!   its sibling takes its space; a non-zero exit and a drop keep their
//!   overlay in the pane (UI 4.6). Ctrl+Shift+W (the shell) closes the
//!   active pane ([`PaneGroup::close_active_pane`]) when there is more than
//!   one, else the tab. When the last pane closes, the group emits [`PaneGroupEvent::Empty`] and the
//!   shell closes the tab by the neighbor rule of 4.6.
//! - **The tab**: its title and icon are the active pane's
//!   ([`PaneGroup::info`]); its state dot is the worst state of its panes
//!   ([`PaneGroup::worst_status`]: drop, then connecting, then connected).
//! - **Restore** (lead decision L5): [`PaneGroup::snapshot`] gives the tree
//!   with each pane's target ([`PaneTarget`]) and shares, and
//!   [`PaneGroup::restore`] opens it again.
//!
//! The group is a view of its own; the window shell holds one per session
//! tab and listens to its [`PaneGroupEvent`]s.

pub mod layout;
mod picker;
mod target;

#[cfg(test)]
mod tests;

use std::cell::Cell;
use std::rc::Rc;

use gpui::prelude::*;
use gpui::{
    AnyElement, App, Bounds, Context, DragMoveEvent, Entity, EventEmitter, FocusHandle, Focusable,
    IntoElement, KeyBinding, Pixels, SharedString, Subscription, Window, canvas, div, px, relative,
    size,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::{ActiveTheme as _, WindowExt as _};

pub use layout::{Direction, Layout, PaneId, Side, SplitPath};
pub use picker::Choice;
pub use target::PaneTarget;

use crate::session_ui::{Phase, SessionPane, SessionPaneEvent, TabInfo};
use crate::state::AppState;
use crate::terminal::TerminalView;
use crate::theme::{self, metrics};

/// Key context of a pane group.
pub const KEY_CONTEXT: &str = "PaneGroup";

/// The smallest width and height of a pane [decision]: 128 px, four times
/// `--space-6` (the design system names no pane size).
pub const PANE_MIN: Pixels = px(128.);

/// The width of a divider's drag area [decision]: `--space-2`, centered on
/// the 1 px `--border-subtle` line between the panes.
const DIVIDER_HIT: Pixels = metrics::SPACE_2;

/// At most this many panes in a tab, and splits nested at most this deep
/// on disk (docs/data-model.md, section 12: the limits of a saved tab).
pub const MAX_PANES: usize = shellpp_core::settings::SESSION_LAST_MAX_LEAVES;
pub const MAX_DEPTH: usize = shellpp_core::settings::SESSION_LAST_MAX_DEPTH;

gpui_kit::actions!(
    panes,
    [
        /// Ctrl+Shift+D: a new pane to the right ("Split vertically").
        SplitRight,
        /// Ctrl+Shift+E: a new pane below ("Split horizontally").
        SplitDown,
        /// Ctrl+Alt+Left.
        FocusLeft,
        /// Ctrl+Alt+Right.
        FocusRight,
        /// Ctrl+Alt+Up.
        FocusUp,
        /// Ctrl+Alt+Down.
        FocusDown,
    ]
);

/// The keys of UI 12.5 for splits (`secondary` is Cmd on macOS). They are
/// bound in the group's context, so they work in its terminals (the
/// terminal passes Ctrl+Shift and Ctrl+Alt+arrow keys to the app).
pub fn bind_keys(cx: &mut App) {
    let context = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("secondary-shift-d", SplitRight, context),
        KeyBinding::new("secondary-shift-e", SplitDown, context),
        KeyBinding::new("secondary-alt-left", FocusLeft, context),
        KeyBinding::new("secondary-alt-right", FocusRight, context),
        KeyBinding::new("secondary-alt-up", FocusUp, context),
        KeyBinding::new("secondary-alt-down", FocusDown, context),
    ]);
}

/// What a pane group tells the window shell.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneGroupEvent {
    /// The last pane closed: the tab closes (UI 4.6). Every session of the
    /// group has ended.
    Empty,
    /// The active pane, its title or icon, or the worst state changed: the
    /// tab redraws, and the terminal sidebar follows the active pane.
    Changed,
    /// A pane's session runs for the first time (once per pane). A quick
    /// connect then offers to save the host (UI 2.1).
    Connected(PaneTarget),
}

/// The state a pane shows on its tab's dot (design-system 3.3), worst
/// first: the tab shows the worst of its panes (UI 4.7).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum PaneStatus {
    /// Dropped, or a failed attempt (`--danger`).
    Dropped,
    /// Connecting or reconnecting (`--warning`).
    Connecting,
    /// Ended with a non-zero exit code, the output kept (inactive,
    /// `--text-muted`; UI 1.1).
    Ended,
    /// Connected (`--success`).
    Connected,
}

impl PaneStatus {
    /// The status of a session phase; `None` once the session is closed.
    pub fn of(phase: &Phase) -> Option<Self> {
        match phase {
            Phase::Connecting(_) | Phase::Reconnecting(_) => Some(PaneStatus::Connecting),
            Phase::Running => Some(PaneStatus::Connected),
            Phase::Dropped(_) | Phase::Failed(_) => Some(PaneStatus::Dropped),
            Phase::Ended(_) => Some(PaneStatus::Ended),
            Phase::Closed => None,
        }
    }
}

/// What [`PaneGroup::close_pane`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneClosed {
    /// The pane is gone; others remain.
    Closed,
    /// It was the last pane: the tab closes ([`PaneGroupEvent::Empty`] was
    /// emitted).
    TabEmpty,
    /// No such pane.
    NotFound,
}

/// The content of one pane.
enum Content {
    Session(Entity<SessionPane>),
    /// The host selection of a new pane; created at the next render (it
    /// needs the window).
    Picker {
        /// The split pane's target and title (the picker's first row).
        origin: Option<(PaneTarget, SharedString)>,
        picker: Option<picker::Picker>,
    },
}

struct Pane {
    id: PaneId,
    content: Content,
    _subscriptions: Vec<Subscription>,
}

/// The drag of a divider: the split it belongs to.
#[derive(Clone, Debug)]
struct DividerDrag {
    path: SplitPath,
}

/// What a divider shows under the pointer while dragged: nothing (the
/// panes move).
struct DragGhost;

impl Render for DragGhost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
    }
}

/// The panes of one session tab.
pub struct PaneGroup {
    state: Entity<AppState>,
    layout: Layout<PaneId>,
    panes: Vec<Pane>,
    active: PaneId,
    last_id: PaneId,
    /// The last active pane with a session: the tab keeps its title while a
    /// new pane chooses a host.
    last_session: Option<PaneId>,
    /// The group's bounds in its last frame (divider drags, new panes'
    /// first terminal size).
    bounds: Rc<Cell<Option<Bounds<Pixels>>>>,
    /// Sessions that start at the next render (the terminal needs the
    /// window).
    unstarted: Vec<Entity<SessionPane>>,
    /// Move the keyboard focus into the active pane at the next render.
    focus_request: bool,
    /// What the tab showed last ([`PaneGroupEvent::Changed`] only on a
    /// change).
    summary: Option<(Option<PaneStatus>, SharedString, IconName, PaneId)>,
    /// Every pane closed (the group is spent).
    empty: bool,
    focus: FocusHandle,
}

impl std::fmt::Debug for PaneGroup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PaneGroup")
            .field("panes", &self.panes.len())
            .field("active", &self.active)
            .finish_non_exhaustive()
    }
}

impl EventEmitter<PaneGroupEvent> for PaneGroup {}

impl Focusable for PaneGroup {
    /// The active pane's focus (its terminal or dialog, or the picker).
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self.pane(self.active).map(|pane| &pane.content) {
            Some(Content::Session(session)) => session.focus_handle(cx),
            Some(Content::Picker {
                picker: Some(picker),
                ..
            }) => picker.focus_handle(cx),
            _ => self.focus.clone(),
        }
    }
}

impl PaneGroup {
    /// A group with one pane showing `first` (a session pane the shell
    /// created; it starts at the group's next render unless it already
    /// runs).
    pub fn new(
        state: Entity<AppState>,
        first: Entity<SessionPane>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut group = Self::empty(state, cx);
        let id = group.next_id();
        group.layout = Layout::leaf(id);
        group.active = id;
        group.install_session(id, first, cx);
        group.last_session = Some(id);
        group
    }

    /// A group with one pane connecting to `target`.
    pub fn open(state: Entity<AppState>, target: &PaneTarget, cx: &mut Context<Self>) -> Self {
        let (info, request) = target.session(state.read(cx));
        let first = cx.new(|cx| SessionPane::new(info, request, cx));
        Self::new(state, first, cx)
    }

    /// The panes of a Restore snapshot, each connecting to its target with
    /// the saved shares; the first pane is active.
    pub fn restore(
        state: Entity<AppState>,
        snapshot: &Layout<PaneTarget>,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut group = Self::empty(state, cx);
        let mut opened = Vec::new();
        let mut layout = snapshot
            .filter_map(&mut |target: &PaneTarget| {
                let id = group.next_id();
                opened.push((id, target.clone()));
                Some(id)
            })
            .expect("a snapshot has panes");
        layout.sanitize();
        group.layout = layout;
        group.active = group.layout.leaves().first().map_or(0, |id| **id);
        group.last_session = Some(group.active);
        for (id, target) in opened {
            let (info, request) = target.session(group.state.read(cx));
            let session = cx.new(|cx| SessionPane::new(info, request, cx));
            group.install_session(id, session, cx);
        }
        group
    }

    fn empty(state: Entity<AppState>, cx: &mut Context<Self>) -> Self {
        Self {
            state,
            layout: Layout::leaf(0),
            panes: Vec::new(),
            active: 0,
            last_id: 0,
            last_session: None,
            bounds: Rc::new(Cell::new(None)),
            unstarted: Vec::new(),
            focus_request: true,
            summary: None,
            empty: false,
            focus: cx.focus_handle(),
        }
    }

    fn next_id(&mut self) -> PaneId {
        self.last_id += 1;
        self.last_id
    }

    fn pane(&self, id: PaneId) -> Option<&Pane> {
        self.panes.iter().find(|pane| pane.id == id)
    }

    /// Puts `session` into pane `id` (new or replacing its picker) and
    /// follows its events and state.
    fn install_session(
        &mut self,
        id: PaneId,
        session: Entity<SessionPane>,
        cx: &mut Context<Self>,
    ) {
        let subscriptions = vec![
            cx.subscribe(&session, move |this, session, event, cx| {
                this.on_session_event(id, &session, event, cx)
            }),
            cx.observe(&session, |this, _, cx| this.refresh_summary(cx)),
        ];
        if !session.read(cx).is_started() {
            self.unstarted.push(session.clone());
        }
        let active = id == self.active;
        session.update(cx, |session, cx| session.set_active(active, cx));
        let pane = Pane {
            id,
            content: Content::Session(session),
            _subscriptions: subscriptions,
        };
        match self.panes.iter_mut().find(|pane| pane.id == id) {
            Some(slot) => *slot = pane,
            None => self.panes.push(pane),
        }
    }

    /// Tells every session whether its pane is the active one (only that
    /// one takes the keyboard focus).
    fn mark_active(&self, cx: &mut Context<Self>) {
        for pane in &self.panes {
            if let Content::Session(session) = &pane.content {
                let active = pane.id == self.active;
                session.update(cx, |session, cx| session.set_active(active, cx));
            }
        }
    }

    // Reading -----------------------------------------------------------------

    /// The pane tree (pane ids and shares).
    pub fn layout(&self) -> &Layout<PaneId> {
        &self.layout
    }

    /// The panes, left to right and top to bottom (none once the last one
    /// closed).
    pub fn pane_ids(&self) -> Vec<PaneId> {
        if self.empty {
            return Vec::new();
        }
        self.layout.leaves().into_iter().copied().collect()
    }

    pub fn pane_count(&self) -> usize {
        self.panes.len()
    }

    /// More than one pane (the tab menu's Broadcast input, UI 4.4).
    pub fn has_splits(&self) -> bool {
        self.panes.len() > 1
    }

    pub fn active_pane(&self) -> PaneId {
        self.active
    }

    /// The session of pane `id`; `None` for a pane that shows the picker.
    pub fn session(&self, id: PaneId) -> Option<&Entity<SessionPane>> {
        match &self.pane(id)?.content {
            Content::Session(session) => Some(session),
            Content::Picker { .. } => None,
        }
    }

    /// The session of the active pane (the tab's Duplicate, the terminal
    /// sidebar); `None` while it shows the picker.
    pub fn active_session(&self) -> Option<&Entity<SessionPane>> {
        self.session(self.active)
    }

    /// The terminal of the active pane, once its session started.
    pub fn active_terminal(&self, cx: &App) -> Option<Entity<TerminalView>> {
        self.active_session()?.read(cx).terminal().cloned()
    }

    /// Every session of the group (exit confirmation, shutdown).
    pub fn sessions(&self) -> Vec<&Entity<SessionPane>> {
        self.panes
            .iter()
            .filter_map(|pane| match &pane.content {
                Content::Session(session) => Some(session),
                Content::Picker { .. } => None,
            })
            .collect()
    }

    /// Whether pane `id` shows the host selection.
    pub fn shows_picker(&self, id: PaneId) -> bool {
        matches!(
            self.pane(id).map(|pane| &pane.content),
            Some(Content::Picker { .. })
        )
    }

    /// The target of pane `id`'s session.
    pub fn target(&self, id: PaneId, cx: &App) -> Option<PaneTarget> {
        let session = self.session(id)?;
        Some(PaneTarget::from_open(session.read(cx).request()))
    }

    /// The target of the active pane's session (Duplicate).
    pub fn active_target(&self, cx: &App) -> Option<PaneTarget> {
        self.target(self.active, cx)
    }

    /// What the tab shows (UI 4.7): the active pane's title and icon; while
    /// the active pane chooses a host, the pane that was active before.
    pub fn info(&self, cx: &App) -> Option<TabInfo> {
        let shown = self
            .session(self.active)
            .or_else(|| self.last_session.and_then(|id| self.session(id)))
            .or_else(|| self.sessions().into_iter().next())?;
        Some(shown.read(cx).info().clone())
    }

    /// The worst state of the panes for the tab's dot (UI 4.7): a drop,
    /// then connecting, then an ended session, then connected; `None` when
    /// no pane has a session (only pickers).
    pub fn worst_status(&self, cx: &App) -> Option<PaneStatus> {
        self.sessions()
            .into_iter()
            .filter_map(|session| PaneStatus::of(session.read(cx).phase()))
            .min()
    }

    /// Whether pane `id` can be split in `direction`: the tab holds fewer
    /// than [`MAX_PANES`], the split keeps the saved form within
    /// [`MAX_DEPTH`], and (once laid out) both halves keep [`PANE_MIN`].
    pub fn can_split(&self, id: PaneId, direction: Direction) -> bool {
        if self.pane(id).is_none() || self.panes.len() >= MAX_PANES {
            return false;
        }
        let mut tree = self.layout.clone();
        if !tree.split(&id, direction, PaneId::MAX) || tree.wire_depth() > MAX_DEPTH {
            return false;
        }
        let Some(bounds) = self.bounds.get() else {
            return true;
        };
        let Some((_, rect)) = self
            .layout
            .rects()
            .into_iter()
            .find(|(pane, _)| **pane == id)
        else {
            return false;
        };
        let extent = match direction {
            Direction::Row => bounds.size.width * rect.w,
            Direction::Column => bounds.size.height * rect.h,
        };
        extent >= PANE_MIN * 2.
    }

    /// The element id of pane `id` (`pane-<id>`).
    pub fn element_id(id: PaneId) -> SharedString {
        format!("pane-{id}").into()
    }

    // Changing ----------------------------------------------------------------

    /// Splits the active pane (UI 4.7): it keeps the first half, and the
    /// new pane, in the second half (right or below), shows the compact
    /// picker with the split pane's target first and becomes active.
    /// `None` when the pane cannot be split ([`Self::can_split`]).
    pub fn split(&mut self, direction: Direction, cx: &mut Context<Self>) -> Option<PaneId> {
        let from = self.active;
        if !self.can_split(from, direction) {
            return None;
        }
        let origin = match &self.pane(from).map(|pane| &pane.content) {
            Some(Content::Session(session)) => {
                let session = session.read(cx);
                Some((
                    PaneTarget::from_open(session.request()),
                    session.info().title().clone(),
                ))
            }
            Some(Content::Picker { origin, .. }) => origin.clone(),
            None => None,
        };
        let id = self.next_id();
        self.layout.split(&from, direction, id);
        self.panes.push(Pane {
            id,
            content: Content::Picker {
                origin,
                picker: None,
            },
            _subscriptions: Vec::new(),
        });
        self.set_active(id, cx);
        Some(id)
    }

    /// Ctrl+Shift+D / Ctrl+Shift+E: [`Self::split`], except from a pane
    /// still choosing a host, as the tab menu (UI 4.4: the splits are
    /// disabled while the active pane has no session target).
    fn split_from_keys(&mut self, direction: Direction, cx: &mut Context<Self>) -> Option<PaneId> {
        if self.shows_picker(self.active) {
            return None;
        }
        self.split(direction, cx)
    }

    /// Opens a session to `target` in pane `id` (the picker's choice); the
    /// session starts at the next render. Does nothing for a pane that
    /// already has a session.
    pub fn open_in(&mut self, id: PaneId, target: &PaneTarget, cx: &mut Context<Self>) {
        if !self.shows_picker(id) {
            return;
        }
        let (info, request) = target.session(self.state.read(cx));
        let session = cx.new(|cx| SessionPane::new(info, request, cx));
        self.install_session(id, session, cx);
        if id == self.active {
            self.last_session = Some(id);
            self.focus_request = true;
        }
        self.refresh_summary(cx);
        cx.notify();
    }

    /// Makes pane `id` active and moves the keyboard focus into it.
    pub fn activate(&mut self, id: PaneId, cx: &mut Context<Self>) {
        if self.pane(id).is_some() {
            self.set_active(id, cx);
        }
    }

    fn set_active(&mut self, id: PaneId, cx: &mut Context<Self>) {
        self.active = id;
        if self.session(id).is_some() {
            self.last_session = Some(id);
        }
        self.mark_active(cx);
        self.focus_request = true;
        self.refresh_summary(cx);
        cx.notify();
    }

    /// Ctrl+Alt+arrow: the nearest pane on `side` of the active one on
    /// screen becomes active. Returns false at the edge of the tab.
    pub fn focus_side(&mut self, side: Side, cx: &mut Context<Self>) -> bool {
        match self.layout.neighbor(&self.active, side) {
            Some(next) => {
                self.set_active(next, cx);
                true
            }
            None => false,
        }
    }

    /// Sets the share of the split at `path`, kept so that every pane has
    /// at least [`PANE_MIN`] (once the group is laid out) and 5 %.
    pub fn set_share(&mut self, path: &SplitPath, share: f32, cx: &mut Context<Self>) -> bool {
        let share = match (self.bounds.get(), self.split_extent(path)) {
            (Some(_), Some(extent)) => {
                match self.layout.share_bounds(path, extent, f32::from(PANE_MIN)) {
                    Some((low, high)) => share.clamp(low, high),
                    None => return false,
                }
            }
            _ => share,
        };
        let changed = self.layout.set_ratio(path, share);
        if changed {
            cx.notify();
        }
        changed
    }

    /// The length of the split at `path` along its axis, in pixels of the
    /// last frame.
    fn split_extent(&self, path: &SplitPath) -> Option<f32> {
        let bounds = self.bounds.get()?;
        let divider = self
            .layout
            .dividers()
            .into_iter()
            .find(|divider| divider.path == *path)?;
        Some(match divider.direction {
            Direction::Row => f32::from(bounds.size.width) * divider.area.w,
            Direction::Column => f32::from(bounds.size.height) * divider.area.h,
        })
    }

    /// Closes pane `id`: its session ends, its sibling takes its space and,
    /// when it was active, the sibling's pane next to it becomes active.
    /// Closing the last pane emits [`PaneGroupEvent::Empty`].
    pub fn close_pane(&mut self, id: PaneId, cx: &mut Context<Self>) -> PaneClosed {
        let Some(index) = self.panes.iter().position(|pane| pane.id == id) else {
            return PaneClosed::NotFound;
        };
        let pane = self.panes.remove(index);
        if let Content::Session(session) = &pane.content {
            self.unstarted.retain(|unstarted| unstarted != session);
            session.update(cx, |session, cx| session.release(cx));
        }
        if self.last_session == Some(id) {
            self.last_session = None;
        }
        match self.layout.remove(&id) {
            layout::Removed::Removed { next } => {
                if self.active == id {
                    self.set_active(next, cx);
                } else {
                    self.refresh_summary(cx);
                    cx.notify();
                }
                PaneClosed::Closed
            }
            layout::Removed::Last | layout::Removed::NotFound => {
                self.empty = true;
                cx.emit(PaneGroupEvent::Empty);
                cx.notify();
                PaneClosed::TabEmpty
            }
        }
    }

    /// Ctrl+Shift+W on a tab with splits; the shell closes the tab when
    /// this was the last pane.
    pub fn close_active_pane(&mut self, cx: &mut Context<Self>) -> PaneClosed {
        self.close_pane(self.active, cx)
    }

    /// Asks for the keyboard focus in the active pane at the next render
    /// (the tab became active).
    pub fn request_focus(&mut self, cx: &mut Context<Self>) {
        self.focus_request = true;
        cx.notify();
    }

    /// Ends every session (the window closes). Safe to call more than once.
    pub fn shutdown(&mut self, cx: &mut Context<Self>) {
        for session in self.sessions().into_iter().cloned().collect::<Vec<_>>() {
            session.update(cx, |session, cx| session.shutdown(cx));
        }
    }

    /// The tab closes or the app quits: every session ends and is let go
    /// ([`SessionPane::release`]).
    pub fn release(&mut self, cx: &mut Context<Self>) {
        self.unstarted.clear();
        for session in self.sessions().into_iter().cloned().collect::<Vec<_>>() {
            session.update(cx, |session, cx| session.release(cx));
        }
    }

    /// The tab for Restore (lead decision L5): the pane tree with each
    /// session's target and the shares; panes still choosing a host are
    /// left out (their sibling takes the space). `None` without sessions.
    pub fn snapshot(&self, cx: &App) -> Option<Layout<PaneTarget>> {
        self.layout.filter_map(&mut |id| self.target(*id, cx))
    }

    /// Starts the sessions that wait for a window (new panes, a restored
    /// tab), at the size their pane had in the last frame. The group does
    /// it when drawn; the window shell also calls it for tabs in the
    /// background, so that they connect all the same.
    pub fn start_pending(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.unstarted.is_empty() {
            return;
        }
        let bounds = self.bounds.get();
        let rects = self.layout.rects();
        for session in std::mem::take(&mut self.unstarted) {
            let area = bounds.and_then(|bounds| {
                let id = self.panes.iter().find_map(|pane| match &pane.content {
                    Content::Session(own) if *own == session => Some(pane.id),
                    _ => None,
                })?;
                let (_, rect) = rects.iter().find(|(pane, _)| **pane == id)?;
                Some(size(
                    bounds.size.width * rect.w,
                    bounds.size.height * rect.h,
                ))
            });
            window.defer(cx, move |window, cx| {
                session.update(cx, |session, cx| {
                    if let Some(area) = area {
                        session.set_area_hint(area);
                    }
                    session.start(window, cx);
                });
            });
        }
    }

    // Events ------------------------------------------------------------------

    fn on_session_event(
        &mut self,
        id: PaneId,
        session: &Entity<SessionPane>,
        event: &SessionPaneEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            // Exit status 0 or Close (UI 4.6, 4.7): this pane closes.
            SessionPaneEvent::Close => {
                self.close_pane(id, cx);
            }
            SessionPaneEvent::Retitled => self.refresh_summary(cx),
            SessionPaneEvent::Connected => {
                let target = PaneTarget::from_open(session.read(cx).request());
                cx.emit(PaneGroupEvent::Connected(target));
            }
        }
    }

    fn on_choice(&mut self, id: PaneId, choice: Choice, cx: &mut Context<Self>) {
        match choice {
            Choice::Open(target) => self.open_in(id, &target, cx),
            Choice::Dismiss => {
                self.close_pane(id, cx);
            }
        }
    }

    /// Emits [`PaneGroupEvent::Changed`] when what the tab shows changed.
    fn refresh_summary(&mut self, cx: &mut Context<Self>) {
        if self.empty {
            return;
        }
        let info = self.info(cx);
        let summary = (
            self.worst_status(cx),
            info.as_ref()
                .map(|info| info.title().clone())
                .unwrap_or_default(),
            info.map_or(IconName::Server, |info| info.icon()),
            self.active,
        );
        if self.summary.as_ref() != Some(&summary) {
            self.summary = Some(summary);
            cx.emit(PaneGroupEvent::Changed);
        }
    }

    // Drawing -----------------------------------------------------------------

    fn render_pane(&self, id: PaneId, rect: layout::Rect, cx: &mut Context<Self>) -> AnyElement {
        let Some(pane) = self.pane(id) else {
            return div().into_any_element();
        };
        let tokens = theme::tokens(cx);
        let content = match &pane.content {
            Content::Session(session) => session.clone().into_any_element(),
            Content::Picker {
                picker: Some(picker),
                ..
            } => picker.view().into_any_element(),
            Content::Picker { picker: None, .. } => div().into_any_element(),
        };
        let marked = self.panes.len() > 1 && id == self.active;
        div()
            .id(Self::element_id(id))
            .test_support()
            .absolute()
            .left(relative(rect.x))
            .top(relative(rect.y))
            .w(relative(rect.w))
            .h(relative(rect.h))
            .overflow_hidden()
            .border_color(tokens.border_subtle.hsla())
            .when(rect.x > 0., |pane| pane.border_l_1())
            .when(rect.y > 0., |pane| pane.border_t_1())
            // A click anywhere in the pane makes it active first.
            .capture_any_mouse_down(cx.listener(move |this, _, _, cx| {
                if this.active != id {
                    this.set_active(id, cx);
                }
            }))
            .child(content)
            .when(marked, |pane| {
                pane.child(
                    div()
                        .id(SharedString::from(format!("pane-{id}-active")))
                        .test_support()
                        .absolute()
                        .top_0()
                        .left_0()
                        .right_0()
                        .h(metrics::SIZE_TAB_INDICATOR)
                        .bg(tokens.accent.hsla()),
                )
            })
            .into_any_element()
    }

    fn render_divider(&self, divider: &layout::Divider, cx: &mut Context<Self>) -> AnyElement {
        let hover = cx.theme().drag_border.opacity(0.5);
        let area = divider.area;
        let id = SharedString::from(format!("pane-divider-{}", divider.path.key()));
        let drag = DividerDrag {
            path: divider.path.clone(),
        };
        // A zero-size anchor on the line, with the drag area centered on it.
        let (anchor, handle) = match divider.direction {
            Direction::Row => (
                div()
                    .absolute()
                    .left(relative(area.x + area.w * divider.ratio))
                    .top(relative(area.y))
                    .w(px(0.))
                    .h(relative(area.h)),
                div()
                    .id(id)
                    .test_support()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(-DIVIDER_HIT / 2.)
                    .w(DIVIDER_HIT)
                    .cursor_col_resize(),
            ),
            Direction::Column => (
                div()
                    .absolute()
                    .top(relative(area.y + area.h * divider.ratio))
                    .left(relative(area.x))
                    .h(px(0.))
                    .w(relative(area.w)),
                div()
                    .id(id)
                    .test_support()
                    .absolute()
                    .left_0()
                    .right_0()
                    .top(-DIVIDER_HIT / 2.)
                    .h(DIVIDER_HIT)
                    .cursor_row_resize(),
            ),
        };
        anchor
            .child(
                handle
                    .occlude()
                    .hover(move |style| style.bg(hover))
                    .on_drag(drag, |_, _, _, cx| cx.new(|_| DragGhost)),
            )
            .into_any_element()
    }

    /// A divider moves with the pointer: the share is where the pointer is
    /// within the split's area, kept inside the pane minimum.
    fn on_divider_move(&mut self, event: &DragMoveEvent<DividerDrag>, cx: &mut Context<Self>) {
        let path = event.drag(cx).path.clone();
        let bounds = event.bounds;
        self.bounds.set(Some(bounds));
        let Some(divider) = self
            .layout
            .dividers()
            .into_iter()
            .find(|divider| divider.path == path)
        else {
            return;
        };
        let position = event.event.position;
        let (start, extent, pointer) = match divider.direction {
            Direction::Row => (
                f32::from(bounds.origin.x) + f32::from(bounds.size.width) * divider.area.x,
                f32::from(bounds.size.width) * divider.area.w,
                f32::from(position.x),
            ),
            Direction::Column => (
                f32::from(bounds.origin.y) + f32::from(bounds.size.height) * divider.area.y,
                f32::from(bounds.size.height) * divider.area.h,
                f32::from(position.y),
            ),
        };
        if extent <= 0. {
            return;
        }
        self.set_share(&path, (pointer - start) / extent, cx);
    }
}

impl Render for PaneGroup {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // New pickers need the window.
        let state = self.state.clone();
        for index in 0..self.panes.len() {
            let id = self.panes[index].id;
            if let Content::Picker {
                origin,
                picker: picker @ None,
            } = &mut self.panes[index].content
            {
                let origin = origin.clone();
                *picker = Some(picker::Picker::new(
                    &state,
                    origin,
                    window,
                    cx,
                    move |group, choice, _, cx| group.on_choice(id, choice, cx),
                ));
            }
        }
        // New sessions start now, at the size their pane had last.
        self.start_pending(window, cx);
        // Not from under a dialog, nor from the Rename field of the tab
        // (the request waits until they are closed).
        if self.focus_request
            && !window.has_active_dialog(cx)
            && !crate::shell::pane_focus_held(window, cx)
        {
            self.focus_request = false;
            match self.pane(self.active).map(|pane| &pane.content) {
                Some(Content::Session(session)) => {
                    session.update(cx, |session, cx| session.request_focus(cx));
                }
                Some(Content::Picker {
                    picker: Some(picker),
                    ..
                }) => {
                    let focus = picker.focus_handle(cx);
                    window.defer(cx, move |window, cx| window.focus(&focus, cx));
                }
                _ => {}
            }
        }

        let measure = {
            let bounds = self.bounds.clone();
            canvas(move |frame, _, _| bounds.set(Some(frame)), |_, _, _, _| {})
                .absolute()
                .size_full()
        };
        let rects = self.layout.rects();
        let panes: Vec<AnyElement> = rects
            .iter()
            .map(|(id, rect)| self.render_pane(**id, *rect, cx))
            .collect();
        let dividers: Vec<AnyElement> = self
            .layout
            .dividers()
            .iter()
            .map(|divider| self.render_divider(divider, cx))
            .collect();
        div()
            .id("pane-group")
            .test_support()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .relative()
            .size_full()
            .bg(cx.theme().colors.list)
            .on_action(cx.listener(|this, _: &SplitRight, _, cx| {
                this.split_from_keys(Direction::Row, cx);
            }))
            .on_action(cx.listener(|this, _: &SplitDown, _, cx| {
                this.split_from_keys(Direction::Column, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusLeft, _, cx| {
                this.focus_side(Side::Left, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusRight, _, cx| {
                this.focus_side(Side::Right, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusUp, _, cx| {
                this.focus_side(Side::Up, cx);
            }))
            .on_action(cx.listener(|this, _: &FocusDown, _, cx| {
                this.focus_side(Side::Down, cx);
            }))
            .on_drag_move(
                cx.listener(|this, event: &DragMoveEvent<DividerDrag>, _, cx| {
                    this.on_divider_move(event, cx)
                }),
            )
            .child(measure)
            .children(panes)
            .children(dividers)
    }
}
