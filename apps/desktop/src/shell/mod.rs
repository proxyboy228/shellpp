//! The shell of an unlocked window (UI 1): the top bar, then the content of
//! the active top-bar tab:
//!
//! - Vaults: the sidebar (UI 1.2), the main area with the toasts at its
//!   bottom center, and the details panel on the right (UI 1.3);
//! - SSH and local terminal tabs: their panes ([`PaneGroup`], UI 4.7), each
//!   with its session (`crate::session_ui`), and the terminal sidebar on the
//!   right when it is shown (UI 4.2; `crate::terminal_sidebar`);
//! - SFTP (and the tabs of later stages): the "In development" placeholder
//!   (audit decisions 1 and 6; D30).
//!
//! Session tabs (UI 1.1, 4.4, 4.6, 4.7, 12.5): a session opens from a
//! double click or Enter on a card, Quick Connect, the host panel's Connect
//! and the search bar's quick connect; the Terminal button, New Local
//! Terminal and Ctrl+L (outside a terminal) open a local terminal. Alt held
//! on these opens the session in a new window (UI 2.4). A tab shows the
//! active pane's OS icon (the server icon until the OS is known), its title
//! (or the one set with Rename), the state dot (the worst state of its
//! panes) and the type icon; the close button replaces the OS icon on the
//! active and the hovered tab; a right click opens the tab menu
//! ([`tab_menu`]). Ctrl+1..9, Alt+Left/Right, Ctrl+Shift+T, Ctrl+Shift+K and
//! Ctrl+Shift+W work everywhere, a terminal included; Ctrl+T and Ctrl+K
//! only outside a terminal. Ctrl+Shift+W closes the active pane of a tab
//! with splits, else the tab. When the active tab closes, the left neighbor
//! (else the right one, else Vaults) becomes active (UI 4.6). Tabs that do
//! not fit scroll sideways: the wheel scrolls the strip, and the active tab
//! always scrolls into view (UI 1.1).
//!
//! Windows (UI 1.4; lead decision L3): every window has its own shell; the
//! app state, the session layer and the lock are shared. Window > New
//! Window, Duplicate in a New Window and Alt on a connect action open
//! another window at the size of the current one, offset by `--space-6`
//! ([`windows`]). Restore (UI 6; lead decision L5) is in [`restore`].
//!
//! Other modules talk to the shell through events (`HostsEvent`,
//! `HostsRequest`, `PanelEvent`, `PanelRequest`, `PaneGroupEvent`) or the
//! window-level functions [`show_toast`], [`notify`], [`in_development`] and
//! [`open_session_tab`].

pub mod actions;
pub mod app_menu;
pub mod details;
pub mod notifications;
pub mod placeholder;
pub mod restore;
pub mod sessions;
pub mod sidebar;
pub mod strings;
pub mod tab_menu;
pub mod title_bar;
pub mod toasts;
pub mod windows;

use gpui::{
    Animation, AnimationExt as _, AnyElement, AnyWindowHandle, App, AppContext as _, ClickEvent,
    Context, DragMoveEvent, Entity, FocusHandle, Global, IntoElement, KeyDownEvent,
    ParentElement as _, Pixels, Render, ScrollHandle, SharedString, Styled as _, Subscription,
    WeakEntity, WeakFocusHandle, Window, div, prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::input::{Input, InputEvent, InputState};
use gpui_kit::component::menu::{ContextMenuExt as _, DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{
    ActiveTheme as _, IconName, Selectable as _, WindowExt as _, h_flex, v_flex,
};
use uuid::Uuid;

use crate::hosts::{self, HostsEvent, HostsRequest, HostsScreen};
use crate::i18n;
use crate::new_tab::{NewTabEvent, NewTabPage, OpenTab, SearchOverlay};
use crate::panels::{GroupPanel, HostPanel, PanelEvent, PanelRequest};
use crate::panes::{Direction, Layout, PaneGroup, PaneGroupEvent, PaneStatus, PaneTarget};
use crate::session_ui::{Phase, SessionPane};
use crate::state::AppState;
use crate::terminal_sidebar::{FindInTerminal, SidebarTarget, TerminalSidebar};
use crate::theme::{self, metrics, motion};

pub use actions::{KEY_CONTEXT, ShowSftp, ShowVaults, bind_keys};
pub use details::DetailsHeader;
pub use notifications::Notifications;
pub use sessions::{SessionId, SessionKind, SessionRequest, SessionTab, SessionTabs};
pub use sidebar::Section;
pub use toasts::{Toast, Toasts};

use actions::*;

/// The top-bar tab whose content is on display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TopTab {
    #[default]
    Vaults,
    Sftp,
    Session(SessionId),
}

/// The details panel (UI 1.3).
#[derive(Clone, Debug)]
pub enum Panel {
    Host(Entity<HostPanel>),
    Group(Entity<GroupPanel>),
}

/// The record a details panel showed when the app locked (UI 13.3: unlock
/// brings it back).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PanelTarget {
    Host(Uuid),
    Group(Uuid),
}

/// The shell of each open window, so window-level calls find it.
#[derive(Default)]
struct ShellRegistry(Vec<(AnyWindowHandle, WeakEntity<Shell>)>);

impl Global for ShellRegistry {}

/// The shell of `window` (none while locked).
pub fn shell_for(window: &Window, cx: &App) -> Option<Entity<Shell>> {
    let id = window.window_handle().window_id();
    cx.try_global::<ShellRegistry>()?
        .0
        .iter()
        .rev()
        .find(|(handle, _)| handle.window_id() == id)
        .and_then(|(_, shell)| shell.upgrade())
}

/// Text fields of the window chrome that automatic focus requests of panes
/// must not take the focus from (the Rename field of a tab, UI 4.4): such a
/// request waits until the field has closed, or the field would end its
/// edit on the blur.
#[derive(Default)]
struct HeldFocus(Vec<WeakFocusHandle>);

impl Global for HeldFocus {}

/// Whether the focused element of `window` is a field that panes leave
/// alone ([`HeldFocus`]): their automatic focus requests wait meanwhile.
pub fn pane_focus_held(window: &Window, cx: &App) -> bool {
    cx.try_global::<HeldFocus>().is_some_and(|held| {
        held.0
            .iter()
            .filter_map(WeakFocusHandle::upgrade)
            .any(|handle| handle.is_focused(window))
    })
}

fn hold_focus(handle: &FocusHandle, cx: &mut App) {
    let held = cx.default_global::<HeldFocus>();
    held.0.retain(|weak| weak.upgrade().is_some());
    held.0.push(handle.downgrade());
}

fn release_focus(handle: &FocusHandle, cx: &mut App) {
    if cx.has_global::<HeldFocus>() {
        cx.global_mut::<HeldFocus>()
            .0
            .retain(|weak| weak.upgrade().is_some_and(|held| held != *handle));
    }
}

/// Runs `f` on the window's shell after the current update, when no view is
/// borrowed. Does nothing when the window shows no shell.
fn with_shell(
    window: &mut Window,
    cx: &mut App,
    f: impl FnOnce(&mut Shell, &mut Window, &mut Context<Shell>) + 'static,
) {
    let Some(shell) = shell_for(window, cx) else {
        tracing::debug!("no shell in this window");
        return;
    };
    window.defer(cx, move |window, cx| {
        shell.update(cx, |shell, cx| f(shell, window, cx));
    });
}

/// Shows a toast at the bottom of the window's main area (5 s).
pub fn show_toast(toast: impl Into<Toast>, window: &mut Window, cx: &mut App) {
    let toast = toast.into();
    with_shell(window, cx, move |shell, _, cx| shell.push_toast(toast, cx));
}

/// Adds a notification to the notification center and shows it as a toast
/// (UI 1.5).
pub fn notify(toast: impl Into<Toast>, window: &mut Window, cx: &mut App) {
    let toast = toast.into();
    let message = toast.message().clone();
    let list = Notifications::global(cx);
    window.defer(cx, move |_, cx| {
        list.update(cx, |list, cx| {
            list.push(message, cx);
        });
    });
    show_toast(toast, window, cx);
}

/// The "In development" placeholder of an action whose target is not built
/// yet (audit decision 1): a toast "`feature`: In development".
pub fn in_development(feature: &str, window: &mut Window, cx: &mut App) {
    show_toast(i18n::placeholder::feature(feature), window, cx);
}

/// The window of every open shell, with the shell.
fn all_windows(cx: &App) -> Vec<(AnyWindowHandle, Entity<Shell>)> {
    cx.try_global::<ShellRegistry>()
        .map(|registry| {
            registry
                .0
                .iter()
                .filter_map(|(handle, shell)| Some((*handle, shell.upgrade()?)))
                .collect()
        })
        .unwrap_or_default()
}

/// The shell of every open window.
fn all_shells(cx: &App) -> Vec<Entity<Shell>> {
    cx.try_global::<ShellRegistry>()
        .map(|registry| {
            registry
                .0
                .iter()
                .filter_map(|(_, shell)| shell.upgrade())
                .collect()
        })
        .unwrap_or_default()
}

/// Saves the pending edits of every window's details panel (a window
/// closes, the app quits): the autosave waits 500 ms, and dropping the
/// panel would drop the edit.
pub fn flush_all_panels(cx: &mut App) {
    for shell in all_shells(cx) {
        shell.update(cx, |shell, cx| shell.flush_panel(cx));
    }
}

/// The app quits: every window's sessions end (their connection log
/// entries close), the tabs let go of them, and the session layer shuts
/// down and is dropped, so the core can close its files afterwards (a
/// temporary profile can only be deleted then). Call it before
/// `AppState::shutdown`.
pub fn release_all_sessions(cx: &mut App) {
    for shell in all_shells(cx) {
        shell.update(cx, |shell, cx| {
            for group in shell.groups() {
                group.update(cx, |group, cx| group.release(cx));
            }
        });
    }
    crate::sessions::ui::shutdown(cx);
    if cx.has_global::<crate::sessions::ui::Sessions>() {
        drop(cx.remove_global::<crate::sessions::ui::Sessions>());
    }
}

/// Locks the app (UI 13.3) from outside a shell: Ctrl+Shift+L that no view
/// handled, and a due auto-lock. The open tabs are saved for Restore while
/// the keys are there ([`restore::save_last_session`]); every window's
/// details panel saves its pending edits and closes first, then the core
/// locks once. Does nothing while locked.
pub fn lock_all(state: &Entity<AppState>, cx: &mut App) {
    if !state.read(cx).is_unlocked() {
        return;
    }
    restore::save_last_session(state, None, cx);
    for shell in all_shells(cx) {
        shell.update(cx, |shell, cx| shell.suspend_panel(cx));
    }
    if let Err(error) = state.update(cx, |state, cx| state.lock(cx)) {
        tracing::warn!(code = error.code(), "lock failed");
    }
}

/// Ctrl+Shift+L that no view handled: a kit dialog (Settings, a
/// confirmation) has the focus, so the shell is not on the dispatch path.
/// Locks like the shell's own handler; does nothing while locked (UI 13.3).
fn lock_from_anywhere(_: &LockApp, cx: &mut App) {
    let state = all_shells(cx)
        .first()
        .map(|shell| shell.read(cx).state.clone());
    if let Some(state) = state {
        lock_all(&state, cx);
    }
}

/// Opens a session tab and makes it active.
pub fn open_session_tab(request: SessionRequest, window: &mut Window, cx: &mut App) {
    with_shell(window, cx, move |shell, _, cx| {
        shell.open_session(request, cx);
    });
}

/// What a session tab shows.
enum TabContent {
    /// SSH and local terminal tabs: their panes (UI 4.1, 4.7).
    Panes {
        group: Entity<PaneGroup>,
        /// The active pane's session, kept for [`Shell::pane`] (refreshed
        /// whenever the group changes).
        active: Option<Entity<SessionPane>>,
        _subscriptions: [Subscription; 2],
    },
    /// The new tab page (UI 6) until a target is chosen; created at the
    /// next render (it needs the window).
    NewTab {
        page: Option<(Entity<NewTabPage>, Subscription)>,
    },
    /// Connect → with SFTP (UI 5.1): the dual-pane manager with the host on
    /// the right; created at the next render (it needs the window).
    Sftp {
        host: Uuid,
        screen: Option<Entity<crate::sftp::SftpScreen>>,
    },
    /// The tabs of later stages (serial).
    Placeholder,
}

/// The inline Rename of a tab (UI 4.4): its field and the field's events.
struct Rename {
    tab: SessionId,
    input: Entity<InputState>,
    /// The field's focus, held from the panes while it is open.
    focus: FocusHandle,
    _subscription: Subscription,
}

pub struct Shell {
    state: Entity<AppState>,
    top_tab: TopTab,
    sessions: SessionTabs,
    section: Section,
    sidebar_visible: bool,
    hosts: Entity<HostsScreen>,
    /// The Known Hosts section (UI 10.1), with its own details panel.
    known_hosts: Entity<crate::known_hosts::KnownHostsScreen>,
    sftp: Entity<crate::sftp::SftpScreen>,
    /// The Keychain section (UI 7), with its own details panel.
    keychain: Entity<crate::keychain::KeychainScreen>,
    panel: Option<Panel>,
    details_width: Pixels,
    /// The search bar holds a connectable address (lead decision 3: the
    /// host panel's Connect is neutral meanwhile).
    search_has_address: bool,
    toasts: Entity<Toasts>,
    notifications: Entity<Notifications>,
    focus: FocusHandle,
    /// Keyboard focus stops: Vaults and SFTP tabs, the sidebar items (in
    /// `Section::ALL` order), the session tabs.
    vaults_focus: FocusHandle,
    sftp_focus: FocusHandle,
    nav_focus: [FocusHandle; 6],
    session_focus: Vec<(SessionId, FocusHandle)>,
    /// The sideways scroll of the session tab strip (more tabs than fit).
    tab_scroll: ScrollHandle,
    /// Reveal the active tab once more after the next frame (see
    /// [`Shell::reveal_active_tab`]).
    reveal_after_frame: bool,
    /// The content of every session tab.
    contents: Vec<(SessionId, TabContent)>,
    /// The terminal sidebar of this window (UI 4.2), shown on the right of
    /// a session tab.
    terminal_sidebar: Entity<TerminalSidebar>,
    /// The tab being renamed in place, if any.
    renaming: Option<Rename>,
    /// The Ctrl+K search over this window (UI 6), while open.
    search: Option<(Entity<SearchOverlay>, Subscription)>,
    /// Give the keyboard focus back to the shell at the next render (the
    /// active tab changed to one without a terminal).
    focus_shell: bool,
    /// Put the keyboard focus in the active new tab page's search at the
    /// next render.
    focus_page: bool,
    _subscriptions: Vec<Subscription>,
    /// Subscriptions to the open details panel (replaced with the panel).
    panel_subscriptions: Vec<Subscription>,
    /// The panel closed by the last lock, reopened after unlock.
    panel_before_lock: Option<PanelTarget>,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell")
            .field("top_tab", &self.top_tab)
            .field("section", &self.section)
            .finish_non_exhaustive()
    }
}

impl Shell {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let hosts = cx.new(|cx| HostsScreen::new(state.clone(), window, cx));
        let known_hosts =
            cx.new(|cx| crate::known_hosts::KnownHostsScreen::new(state.clone(), window, cx));
        let keychain = cx.new(|cx| crate::keychain::KeychainScreen::new(state.clone(), window, cx));
        let sftp = cx.new(|cx| crate::sftp::SftpScreen::new(state.clone(), window, cx));
        let toasts = cx.new(|_| Toasts::new());
        let notifications = Notifications::global(cx);
        let subscriptions = vec![
            cx.subscribe_in(&hosts, window, Self::on_hosts_event),
            cx.subscribe_in(&hosts, window, Self::on_hosts_request),
            cx.observe(&notifications, |_, _, cx| cx.notify()),
        ];

        let this = cx.weak_entity();
        let registry = cx.default_global::<ShellRegistry>();
        registry.0.retain(|(_, shell)| shell.upgrade().is_some());
        registry.0.push((window.window_handle(), this));

        // Closing the window while sessions are active asks first (UI 1.4,
        // 4.6). An edit still waiting for the autosave is saved before the
        // window (and the panel with it) goes; the last window saves the
        // open tabs for Restore (UI 6).
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| match this.upgrade() {
            Some(shell) => window_may_close(&shell, window, cx),
            None => true,
        });

        // Session tabs start through the session layer (`crate::sessions`,
        // installed at startup); tests install their own launcher first.
        if !cx.has_global::<crate::session_ui::Launcher>() {
            cx.set_global(crate::session_ui::Launcher(std::rc::Rc::new(
                crate::session_ui::bridge::Bridge,
            )));
        }
        // The terminals follow the `terminal.*` settings (Settings >
        // Terminal replaces the global after each change).
        if let Some(core) = state.read(cx).core().cloned() {
            crate::settings::terminal::TerminalPrefs::refresh(&core, cx);
        }
        // What Restore offers: the tabs of the previous launch, read once
        // at the first unlock (UI 6).
        restore::load_previous_launch(&state, cx);
        let terminal_sidebar = cx.new(|cx| TerminalSidebar::new(state.clone(), window, cx));

        // Shortcuts live in the shell's key context: keep the focus inside
        // the shell from the start (not in the search bar, UI 2.1). The
        // shell replaces the lock screens, so their focus goes with them.
        let focus = cx.focus_handle();
        window.focus(&focus, cx);

        Self {
            state,
            top_tab: TopTab::Vaults,
            sessions: SessionTabs::default(),
            section: Section::Hosts,
            sidebar_visible: true,
            hosts,
            known_hosts,
            sftp,
            keychain,
            panel: None,
            details_width: metrics::WIDTH_DETAILS,
            search_has_address: false,
            toasts,
            notifications,
            focus,
            vaults_focus: cx.focus_handle().tab_stop(true),
            sftp_focus: cx.focus_handle().tab_stop(true),
            nav_focus: std::array::from_fn(|_| cx.focus_handle().tab_stop(true)),
            session_focus: Vec::new(),
            tab_scroll: ScrollHandle::new(),
            reveal_after_frame: false,
            contents: Vec::new(),
            terminal_sidebar,
            renaming: None,
            search: None,
            focus_shell: false,
            focus_page: false,
            _subscriptions: subscriptions,
            panel_subscriptions: Vec::new(),
            panel_before_lock: None,
        }
    }

    pub fn top_tab(&self) -> TopTab {
        self.top_tab
    }

    pub fn section(&self) -> Section {
        self.section
    }

    pub fn sessions(&self) -> &SessionTabs {
        &self.sessions
    }

    pub fn hosts_screen(&self) -> &Entity<HostsScreen> {
        &self.hosts
    }

    pub fn sftp_screen(&self) -> &Entity<crate::sftp::SftpScreen> {
        &self.sftp
    }

    pub fn known_hosts_screen(&self) -> &Entity<crate::known_hosts::KnownHostsScreen> {
        &self.known_hosts
    }

    pub fn keychain_screen(&self) -> &Entity<crate::keychain::KeychainScreen> {
        &self.keychain
    }

    pub fn toasts(&self) -> &Entity<Toasts> {
        &self.toasts
    }

    pub fn panel(&self) -> Option<&Panel> {
        self.panel.as_ref()
    }

    pub fn details_width(&self) -> Pixels {
        self.details_width
    }

    pub fn is_sidebar_visible(&self) -> bool {
        self.sidebar_visible
    }

    /// The shell's focus handle (the app menu dispatches its actions here).
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus
    }

    pub fn set_top_tab(&mut self, tab: TopTab, cx: &mut Context<Self>) {
        if let TopTab::Session(id) = tab
            && self.sessions.get(id).is_none()
        {
            return;
        }
        let previous = self.top_tab;
        self.top_tab = tab;
        self.reveal_active_tab();
        // Into a session tab, or out of one whose terminal had the focus.
        let session = |tab: TopTab| matches!(tab, TopTab::Session(_));
        if previous != tab && (session(tab) || session(previous)) {
            self.focus_active_tab(cx);
        }
        cx.notify();
    }

    /// Scrolls the session tab strip so that the active session tab is
    /// fully in view. Only when the active tab changes or is clicked, so the
    /// wheel can still scroll the strip away from it. GPUI scrolls with the
    /// strip's bounds of the frame before, and the bar changes with the
    /// active tab (the terminal sidebar button), so the reveal runs once
    /// more after the next frame.
    fn reveal_active_tab(&mut self) {
        self.scroll_to_active_tab();
        self.reveal_after_frame = true;
    }

    fn scroll_to_active_tab(&self) {
        if let TopTab::Session(id) = self.top_tab
            && let Some(index) = self.sessions.tabs().iter().position(|tab| tab.id() == id)
        {
            self.tab_scroll.scroll_to_item(index);
        }
    }

    /// After a tab change: the keyboard focus goes into the session tab's
    /// active pane (its terminal, dialog or host picker; UI 4.1, 4.7), else
    /// back to the shell, so that the shortcuts keep working when the
    /// focused terminal is no longer shown.
    fn focus_active_tab(&mut self, cx: &mut Context<Self>) {
        match self.top_tab {
            TopTab::Session(id) => match (self.group(id).cloned(), self.content(id)) {
                (Some(group), _) => group.update(cx, |group, cx| group.request_focus(cx)),
                (None, Some(TabContent::NewTab { .. })) => self.focus_page = true,
                (None, _) => self.focus_shell = true,
            },
            TopTab::Vaults | TopTab::Sftp => self.focus_shell = true,
        }
    }

    /// The panes of session tab `id` (SSH and local terminal tabs).
    pub fn group(&self, id: SessionId) -> Option<&Entity<PaneGroup>> {
        match self.content(id)? {
            TabContent::Panes { group, .. } => Some(group),
            TabContent::NewTab { .. } | TabContent::Sftp { .. } | TabContent::Placeholder => None,
        }
    }

    /// The session of tab `id`'s active pane; `None` while that pane
    /// chooses a host, and for tabs without panes.
    pub fn pane(&self, id: SessionId) -> Option<&Entity<SessionPane>> {
        match self.content(id)? {
            TabContent::Panes { active, .. } => active.as_ref(),
            TabContent::NewTab { .. } | TabContent::Sftp { .. } | TabContent::Placeholder => None,
        }
    }

    fn content(&self, id: SessionId) -> Option<&TabContent> {
        self.contents
            .iter()
            .find(|(tab, _)| *tab == id)
            .map(|(_, content)| content)
    }

    /// Every pane group of this window, in tab order.
    fn groups(&self) -> Vec<Entity<PaneGroup>> {
        self.sessions
            .tabs()
            .iter()
            .filter_map(|tab| self.group(tab.id()).cloned())
            .collect()
    }

    /// The terminal sidebar of this window (UI 4.2).
    pub fn terminal_sidebar(&self) -> &Entity<TerminalSidebar> {
        &self.terminal_sidebar
    }

    /// The state dot of tab `id` (UI 1.1, 4.7): the worst state of its
    /// panes; `None` without a live or dropped session.
    pub fn tab_status(&self, id: SessionId, cx: &App) -> Option<PaneStatus> {
        self.group(id)?.read(cx).worst_status(cx)
    }

    /// Sessions that count for the exit confirmation (UI 4.6): connecting,
    /// connected, reconnecting, or dropped with automatic attempts ahead.
    pub fn active_sessions(&self, cx: &App) -> usize {
        self.groups()
            .iter()
            .flat_map(|group| group.read(cx).sessions().into_iter().cloned())
            .filter(|pane| {
                let pane = pane.read(cx);
                match pane.phase() {
                    Phase::Connecting(_) | Phase::Running | Phase::Reconnecting(_) => true,
                    Phase::Dropped(_) => pane.reconnect_info().is_some(),
                    Phase::Failed(_) | Phase::Ended(_) | Phase::Closed => false,
                }
            })
            .count()
    }

    /// Ends every session of this window (it closes).
    pub fn shutdown_sessions(&mut self, cx: &mut Context<Self>) {
        for group in self.groups() {
            group.update(cx, |group, cx| group.shutdown(cx));
        }
    }

    pub fn set_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.top_tab = TopTab::Vaults;
        self.section = section;
        if section != Section::Hosts {
            self.close_panel(cx);
        }
        cx.notify();
    }

    /// Sets the details panel width (clamped to 320-560 px).
    pub fn set_details_width(&mut self, width: Pixels, cx: &mut Context<Self>) {
        let width = details::clamp_width(width);
        if width != self.details_width {
            self.details_width = width;
            cx.notify();
        }
    }

    pub fn toggle_sidebar(&mut self, cx: &mut Context<Self>) {
        self.sidebar_visible = !self.sidebar_visible;
        cx.notify();
    }

    /// Shows a toast at the bottom of the main area.
    pub fn toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.push_toast(Toast::new(message), cx);
    }

    /// Shows a toast (with an optional action button).
    pub fn push_toast(&mut self, toast: impl Into<Toast>, cx: &mut Context<Self>) {
        let toast = toast.into();
        self.toasts.update(cx, |toasts, cx| {
            toasts.push(toast, cx);
        });
    }

    fn placeholder_toast(&mut self, feature: &str, cx: &mut Context<Self>) {
        self.toast(i18n::placeholder::feature(feature), cx);
    }

    /// Opens a session tab and makes it active. SSH and local terminal tabs
    /// get a [`PaneGroup`] whose first pane connects at the next render;
    /// the tabs of later stages (SFTP, serial) and the new tab page show
    /// the "In development" placeholder.
    pub fn open_session(&mut self, request: SessionRequest, cx: &mut Context<Self>) -> SessionId {
        if let Some(target) = PaneTarget::from_request(&request, self.state.read(cx)) {
            return self.open_tab(TabSeed::Target(target), None, None, cx);
        }
        let content = match (request.kind(), request.host_id()) {
            (SessionKind::NewTab, _) => TabContent::NewTab { page: None },
            (SessionKind::Sftp, Some(host)) => TabContent::Sftp { host, screen: None },
            _ => TabContent::Placeholder,
        };
        let id = self.sessions.open(request);
        self.contents.push((id, content));
        self.after_open(id, cx);
        id
    }

    /// Opens a session tab for `seed` (a target, or the pane tree of
    /// Restore), right after tab `after` when given (Duplicate), with the
    /// title set with Rename when given (Restore).
    fn open_tab(
        &mut self,
        seed: TabSeed,
        after: Option<SessionId>,
        title: Option<String>,
        cx: &mut Context<Self>,
    ) -> SessionId {
        self.place_tab(seed, TabPlace::New(after), title, cx)
    }

    /// Opens `seed` in a new tab (`TabPlace::New`) or in place of the new
    /// tab page of tab `id` (`TabPlace::Replace`; UI 6: the session opens
    /// in the same tab).
    fn place_tab(
        &mut self,
        seed: TabSeed,
        place: TabPlace,
        title: Option<String>,
        cx: &mut Context<Self>,
    ) -> SessionId {
        let state = self.state.clone();
        let first = match &seed {
            TabSeed::Target(target) => Some(target.clone()),
            TabSeed::Snapshot(layout) => layout.leaves().first().map(|target| (*target).clone()),
        };
        let group = cx.new(|cx| match seed {
            TabSeed::Target(target) => PaneGroup::open(state, &target, cx),
            TabSeed::Snapshot(layout) => PaneGroup::restore(state, &layout, cx),
        });
        let mut request = SessionRequest::new(first.as_ref().map_or(SessionKind::Ssh, target_kind));
        if let Some(info) = group.read(cx).info(cx) {
            request = request.title(info.title().clone()).icon(info.icon());
        }
        if let Some(host) = first.as_ref().and_then(PaneTarget::host_id) {
            request = request.host(host);
        }
        let id = match place {
            TabPlace::New(Some(after)) => self.sessions.open_after(after, request),
            TabPlace::New(None) => self.sessions.open(request),
            TabPlace::Replace(id) => {
                self.sessions.replace(id, request);
                self.contents.retain(|(tab, _)| *tab != id);
                id
            }
        };
        if let Some(title) = title {
            self.sessions.rename(id, Some(&title));
        }
        let active = group.read(cx).active_session().cloned();
        let subscriptions = [
            cx.subscribe(&group, move |this, group, event, cx| {
                this.on_group_event(id, &group, event, cx)
            }),
            cx.observe(&group, move |this, group, cx| {
                this.on_group_changed(id, &group, cx)
            }),
        ];
        self.contents.push((
            id,
            TabContent::Panes {
                group,
                active,
                _subscriptions: subscriptions,
            },
        ));
        match place {
            TabPlace::New(_) => self.after_open(id, cx),
            TabPlace::Replace(_) => {
                self.set_top_tab(TopTab::Session(id), cx);
                self.focus_active_tab(cx);
            }
        }
        id
    }

    /// A new tab becomes active, with its focus stop.
    fn after_open(&mut self, id: SessionId, cx: &mut Context<Self>) {
        self.session_focus
            .push((id, cx.focus_handle().tab_stop(true)));
        self.top_tab = TopTab::Session(id);
        self.reveal_active_tab();
        self.focus_active_tab(cx);
        cx.notify();
    }

    /// Opens a session to `target` in a new tab (Duplicate, a new window);
    /// `after` puts it right after that tab.
    pub fn open_target(
        &mut self,
        target: PaneTarget,
        after: Option<SessionId>,
        cx: &mut Context<Self>,
    ) -> SessionId {
        self.open_tab(TabSeed::Target(target), after, None, cx)
    }

    /// Opens a tab with the panes of a Restore snapshot and the title set
    /// with Rename, if any.
    pub fn open_snapshot(
        &mut self,
        layout: Layout<PaneTarget>,
        title: Option<String>,
        cx: &mut Context<Self>,
    ) -> SessionId {
        self.open_tab(TabSeed::Snapshot(layout), None, title, cx)
    }

    fn on_group_event(
        &mut self,
        id: SessionId,
        group: &Entity<PaneGroup>,
        event: &PaneGroupEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            // The last pane closed (UI 4.6, 4.7): the neighbor rule applies.
            PaneGroupEvent::Empty => self.close_session(id, cx),
            PaneGroupEvent::Changed => self.on_group_changed(id, group, cx),
            // A quick connect (UI 2.1) offers to save the host.
            PaneGroupEvent::Connected(PaneTarget::Quick(target)) => {
                self.offer_to_save(target.clone(), cx)
            }
            PaneGroupEvent::Connected(_) => {}
        }
    }

    /// The group changed (active pane, title, icon, state): the tab follows
    /// its active pane (UI 4.7).
    fn on_group_changed(
        &mut self,
        id: SessionId,
        group: &Entity<PaneGroup>,
        cx: &mut Context<Self>,
    ) {
        let (info, active, host) = {
            let group = group.read(cx);
            (
                group.info(cx),
                group.active_session().cloned(),
                group.active_target(cx).map(|target| target.host_id()),
            )
        };
        if let Some((_, TabContent::Panes { active: slot, .. })) =
            self.contents.iter_mut().find(|(tab, _)| *tab == id)
        {
            *slot = active;
        }
        if let Some(info) = info {
            self.sessions.retitle(id, info.title().clone(), info.icon());
        }
        // While the active pane chooses a host, the tab keeps the host of
        // the pane it shows.
        if let Some(host) = host {
            self.sessions.set_host(id, host);
        }
        cx.notify();
    }

    /// The unobtrusive offer to save the host after a successful quick
    /// connect (UI 2.1): "Save" creates it like the Hosts screen's "Save as
    /// host" and opens its panel.
    fn offer_to_save(&mut self, target: hosts::QuickTarget, cx: &mut Context<Self>) {
        let screen = self.hosts.downgrade();
        self.push_toast(
            Toast::new(hosts::strings::SAVE_HOST_OFFER).action_with_id(
                "toast-save-host",
                hosts::strings::SAVE_HOST_ACTION,
                move |_, cx| {
                    let _ = screen.update(cx, |screen, cx| screen.save_quick_target(&target, cx));
                },
            ),
            cx,
        );
    }

    /// Closes a session tab and ends its sessions; when it was active, the
    /// neighbor of UI 4.6 becomes active (left, else right, else Vaults).
    pub fn close_session(&mut self, id: SessionId, cx: &mut Context<Self>) {
        let was_active = self.top_tab == TopTab::Session(id);
        if let Some(index) = self.contents.iter().position(|(tab, _)| *tab == id) {
            let (_, content) = self.contents.remove(index);
            if let TabContent::Panes { group, .. } = content {
                // Ends the sessions and lets go of them, so that nothing
                // keeps the core open if the group outlives its tab.
                group.update(cx, |group, cx| group.release(cx));
            }
        }
        if self
            .renaming
            .as_ref()
            .is_some_and(|rename| rename.tab == id)
        {
            self.take_rename(cx);
        }
        let next = self.sessions.close(id);
        self.session_focus.retain(|(session, _)| *session != id);
        if was_active {
            self.top_tab = next.map_or(TopTab::Vaults, TopTab::Session);
            self.reveal_active_tab();
            self.focus_active_tab(cx);
        }
        cx.notify();
    }

    /// Ctrl+Shift+W (UI 4.7, 12.5): the active pane of a tab with splits,
    /// else the active tab.
    pub fn close_active(&mut self, cx: &mut Context<Self>) {
        let TopTab::Session(id) = self.top_tab else {
            return;
        };
        match self.group(id).cloned() {
            Some(group) if group.read(cx).has_splits() => {
                group.update(cx, |group, cx| {
                    group.close_active_pane(cx);
                });
            }
            _ => self.close_session(id, cx),
        }
    }

    /// The top-bar tabs in order: Vaults, SFTP, then the session tabs.
    fn tab_order(&self) -> Vec<TopTab> {
        [TopTab::Vaults, TopTab::Sftp]
            .into_iter()
            .chain(
                self.sessions
                    .tabs()
                    .iter()
                    .map(|tab| TopTab::Session(tab.id())),
            )
            .collect()
    }

    /// Moves to the next (`step` 1) or previous (`step` -1) tab, wrapping.
    pub fn cycle_tab(&mut self, step: isize, cx: &mut Context<Self>) {
        let order = self.tab_order();
        let current = order
            .iter()
            .position(|tab| *tab == self.top_tab)
            .unwrap_or(0) as isize;
        let next = (current + step).rem_euclid(order.len() as isize) as usize;
        self.set_top_tab(order[next], cx);
    }

    /// Ctrl+N: tab N (1 = Vaults, 2 = SFTP, 3.. = session tabs).
    pub fn activate_tab(&mut self, number: usize, cx: &mut Context<Self>) {
        if let Some(tab) = number
            .checked_sub(1)
            .and_then(|index| self.tab_order().get(index).copied())
        {
            self.set_top_tab(tab, cx);
        }
    }

    /// Exit (UI 1.4): asks first while sessions are active (UI 4.6).
    /// Pending panel edits are saved first.
    pub fn request_exit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let sessions = all_shells(cx)
            .iter()
            .filter(|shell| shell.entity_id() != cx.entity_id())
            .map(|shell| shell.read(cx).active_sessions(cx))
            .sum::<usize>()
            + self.active_sessions(cx);
        if sessions == 0 {
            self.flush_panel(cx);
            self.shutdown_sessions(cx);
            let me = cx.entity_id();
            for shell in all_shells(cx) {
                if shell.entity_id() != me {
                    shell.update(cx, |shell, cx| shell.shutdown_sessions(cx));
                }
            }
            cx.quit();
        } else {
            confirm_exit(sessions, window, cx);
        }
    }

    /// Opens the host panel for `record_id`, or an empty one for a new host
    /// (focus in Address, UI 3).
    pub fn open_host_panel(
        &mut self,
        record_id: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.top_tab = TopTab::Vaults;
        if let (Some(Panel::Host(panel)), Some(id)) = (&self.panel, record_id)
            && panel.read(cx).record_id() == Some(id)
        {
            cx.notify();
            return;
        }
        self.flush_panel(cx);
        let state = self.state.clone();
        let panel = cx.new(|cx| HostPanel::new(state, record_id, window, cx));
        let search_has_address = self.search_has_address;
        panel.update(cx, |panel, cx| {
            panel.set_search_has_address(search_has_address, cx);
        });
        self.panel_subscriptions = vec![
            cx.subscribe_in(&panel, window, Self::on_panel_event),
            cx.subscribe_in(&panel, window, Self::on_panel_request),
        ];
        self.panel = Some(Panel::Host(panel));
        cx.notify();
    }

    /// Opens the group panel (UI 3.4).
    pub fn open_group_panel(
        &mut self,
        record_id: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.top_tab = TopTab::Vaults;
        self.flush_panel(cx);
        let state = self.state.clone();
        let panel = cx.new(|cx| GroupPanel::new(state, record_id, window, cx));
        self.panel_subscriptions = vec![
            cx.subscribe_in(&panel, window, Self::on_panel_event),
            cx.subscribe_in(&panel, window, Self::on_panel_request),
        ];
        self.panel = Some(Panel::Group(panel));
        cx.notify();
    }

    /// Locks the app now (Lock, Ctrl+Shift+L; UI 13.3). Pending panel edits
    /// of every window are saved before the keys go. Does nothing while
    /// locked.
    pub fn lock_app(&mut self, cx: &mut Context<Self>) {
        if !self.state.read(cx).is_unlocked() {
            return;
        }
        // The open tabs are saved for Restore while the keys are there
        // (this shell's tabs from here, the others' through their
        // entities).
        restore::save_from(self, cx);
        // This shell is being updated: its own panel here, the others'
        // through their entities.
        self.suspend_panel(cx);
        let me = cx.entity_id();
        for shell in all_shells(cx) {
            if shell.entity_id() != me {
                shell.update(cx, |shell, cx| shell.suspend_panel(cx));
            }
        }
        if let Err(error) = self.state.update(cx, |state, cx| state.lock(cx)) {
            tracing::warn!(code = error.code(), "lock failed");
        }
    }

    /// On lock: closes the Ctrl+K search (it lists hosts) and the details
    /// panel (its record data goes with the keys; pending edits are saved
    /// first while they still can be) and remembers the panel's record for
    /// [`Shell::resume_panel`]. Without an open panel it keeps what an
    /// earlier call remembered.
    pub fn suspend_panel(&mut self, cx: &mut Context<Self>) {
        self.close_search(cx);
        let target = match &self.panel {
            Some(Panel::Host(panel)) => panel.read(cx).record_id().map(PanelTarget::Host),
            Some(Panel::Group(panel)) => panel.read(cx).record_id().map(PanelTarget::Group),
            None => return,
        };
        self.panel_before_lock = target;
        self.close_panel(cx);
    }

    /// After unlock: reopens the panel [`Shell::suspend_panel`] closed when
    /// its record still exists; the top-bar tab stays as it was (UI 13.3)
    /// and gets the focus back (a session tab: its terminal or dialog).
    pub fn resume_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_active_tab(cx);
        let tab = self.top_tab;
        match self.panel_before_lock.take() {
            Some(PanelTarget::Host(id)) if self.state.read(cx).host(id).is_some() => {
                self.open_host_panel(Some(id), window, cx);
            }
            Some(PanelTarget::Group(id)) if self.state.read(cx).group(id).is_some() => {
                self.open_group_panel(Some(id), window, cx);
            }
            _ => return,
        }
        self.top_tab = tab;
        cx.notify();
    }

    /// Closes the details panel; pending edits are saved first.
    pub fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.flush_panel(cx);
        self.panel = None;
        self.panel_subscriptions.clear();
        cx.notify();
    }

    /// Saves the details panel's pending edit now (the autosave waits
    /// 500 ms): before it closes or changes, before the window closes and
    /// before the app quits.
    pub(crate) fn flush_panel(&mut self, cx: &mut Context<Self>) {
        match &self.panel {
            Some(Panel::Host(panel)) => panel.update(cx, |panel, cx| panel.flush(cx)),
            Some(Panel::Group(panel)) => panel.update(cx, |panel, cx| panel.flush(cx)),
            None => {}
        }
    }

    fn on_hosts_event(
        &mut self,
        _: &Entity<HostsScreen>,
        event: &HostsEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            HostsEvent::OpenHost(id) => self.open_host_panel(Some(*id), window, cx),
            HostsEvent::NewHost => self.open_host_panel(None, window, cx),
            HostsEvent::EditGroup(id) => self.open_group_panel(Some(*id), window, cx),
            HostsEvent::NewGroup => self.open_group_panel(None, window, cx),
            HostsEvent::Toast(message) => self.toast(message.clone(), cx),
        }
    }

    fn on_hosts_request(
        &mut self,
        _: &Entity<HostsScreen>,
        request: &HostsRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            HostsRequest::OpenSession(request) => {
                let kind = match request.kind {
                    hosts::SessionKind::Ssh => SessionKind::Ssh,
                    hosts::SessionKind::Sftp => SessionKind::Sftp,
                    hosts::SessionKind::Terminal => SessionKind::Local,
                    hosts::SessionKind::Serial => SessionKind::Serial,
                };
                let mut session = SessionRequest::new(kind).title(request.title.clone());
                if let Some(host) = request.host {
                    session = session.host(host);
                }
                // The search bar's quick connect (UI 2.1).
                if let Some(target) = request.target.clone() {
                    session = session.quick(target);
                }
                // Alt held (UI 2.4): in a new window.
                if request.new_window {
                    windows::open_in_new_window(
                        self.state.clone(),
                        window,
                        cx,
                        move |shell, _, cx| {
                            shell.open_session(session, cx);
                        },
                    );
                } else {
                    self.open_session(session, cx);
                }
            }
            HostsRequest::EditHost(id) => {
                self.open_host_panel(Some(*id), window, cx);
                if let Some(Panel::Host(panel)) = &self.panel {
                    panel.update(cx, |panel, cx| panel.focus_address(window, cx));
                }
            }
            HostsRequest::ToastWithAction { message, action } => {
                let run = action.run.clone();
                self.push_toast(
                    Toast::new(message.clone()).action_with_id(
                        action.id.clone(),
                        action.label.clone(),
                        move |window, cx| run(window, cx),
                    ),
                    cx,
                );
            }
            HostsRequest::SearchAddressChanged(has_address) => {
                self.search_has_address = *has_address;
                if let Some(Panel::Host(panel)) = &self.panel {
                    let has_address = *has_address;
                    panel.update(cx, |panel, cx| {
                        panel.set_search_has_address(has_address, cx)
                    });
                }
            }
        }
    }

    fn on_panel_event(
        &mut self,
        _: &Entity<impl Render>,
        event: &PanelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PanelEvent::Close => self.close_panel(cx),
            PanelEvent::Toast(message) => self.toast(message.clone(), cx),
        }
    }

    fn on_panel_request(
        &mut self,
        _: &Entity<impl Render>,
        request: &PanelRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match request {
            PanelRequest::Connect(id) => {
                let host = self.state.read(cx).host(*id).cloned();
                let mut session = SessionRequest::new(SessionKind::Ssh).host(*id);
                if let Some(host) = host {
                    session = session.title(host.title().to_owned());
                }
                // Alt + Connect (UI 2.4): in a new window. The click (or the
                // menu item) is still being handled, so the window's
                // modifiers are the ones it came with.
                if window.modifiers().alt {
                    windows::open_in_new_window(
                        self.state.clone(),
                        window,
                        cx,
                        move |shell, _, cx| {
                            shell.open_session(session, cx);
                        },
                    );
                } else {
                    self.open_session(session, cx);
                }
            }
            PanelRequest::OpenHost(id) => self.open_host_panel(Some(*id), window, cx),
            PanelRequest::OpenGroup(id) => self.open_group_panel(Some(*id), window, cx),
            PanelRequest::Removed(ids) => {
                let state = self.state.clone();
                let ids = ids.clone();
                self.push_toast(
                    Toast::new(i18n::panels::REMOVED_ONE).action_with_id(
                        "toast-undo",
                        i18n::notifications::UNDO,
                        move |_, cx| {
                            let ids = ids.clone();
                            let restored =
                                state.update(cx, |state, cx| state.restore_records(ids, cx));
                            if let Err(error) = restored {
                                tracing::warn!(code = error.code(), "undo of remove failed");
                            }
                        },
                    ),
                    cx,
                );
            }
        }
    }

    fn render_title_bar(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.focus.clone();
        let menu = title_bar::icon_button("app-menu", IconName::Menu, cx)
            .tooltip(i18n::top_bar::APP_MENU)
            .dropdown_menu(move |menu, window, cx| {
                app_menu::build(menu, focus.clone(), window, cx)
            });
        let vaults_active = self.top_tab == TopTab::Vaults;
        let vaults = title_bar::vaults_tab(
            "tab-vaults",
            &self.vaults_focus,
            Lucide::Vault,
            i18n::top_bar::VAULTS,
            vaults_active,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_top_tab(TopTab::Vaults, cx)));
        let vault_switcher = title_bar::vault_switcher_button(cx).dropdown_menu(|menu, _, _| {
            // Team vaults arrive with sync (later stage); Personal is the only one.
            menu.item(PopupMenuItem::new(i18n::top_bar::PERSONAL_VAULT).checked(true))
        });
        let sftp = title_bar::top_tab(
            "tab-sftp",
            &self.sftp_focus,
            Lucide::FolderOpen,
            i18n::top_bar::SFTP,
            self.top_tab == TopTab::Sftp,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_top_tab(TopTab::Sftp, cx)));
        // The tab being renamed shows its field in place of the title.
        let mut rename_field = self.renaming.as_ref().and_then(|rename| {
            let tab = self.sessions.get(rename.tab)?;
            Some((rename.tab, self.render_rename(tab, rename, cx)))
        });
        let session_tabs: Vec<AnyElement> = self
            .sessions
            .tabs()
            .iter()
            .map(|tab| {
                let id = tab.id();
                let close = title_bar::session_tab_close(tab, cx).on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| {
                        cx.stop_propagation();
                        this.close_session(id, cx);
                        this.focus_active_tab(cx);
                    },
                ));
                let focus = self
                    .session_focus
                    .iter()
                    .find(|(session, _)| *session == id)
                    .map(|(_, focus)| focus.clone())
                    .unwrap_or_else(|| self.focus.clone());
                let rename = rename_field
                    .take_if(|(tab, _)| *tab == id)
                    .map(|(_, field)| field);
                let shell = cx.weak_entity();
                title_bar::session_tab(
                    tab,
                    self.tab_status(id, cx),
                    rename,
                    &focus,
                    self.top_tab == TopTab::Session(id),
                    close,
                    cx,
                )
                .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                    this.set_top_tab(TopTab::Session(id), cx);
                    // A click on the active tab gives its terminal the
                    // focus back.
                    this.focus_active_tab(cx);
                }))
                // Right click: the tab menu (UI 4.4).
                .context_menu(move |menu, window, cx| {
                    let Some(shell) = shell.upgrade() else {
                        return menu;
                    };
                    let facts = shell.read(cx).tab_facts(id, cx);
                    let handler: tab_menu::TabCommandHandler = {
                        let shell = shell.downgrade();
                        std::rc::Rc::new(move |command, window, cx| {
                            let _ = shell.update(cx, |shell, cx| {
                                shell.run_tab_command(id, command, window, cx)
                            });
                        })
                    };
                    tab_menu::build(menu, tab_menu::entries(facts), handler, window, cx)
                })
                .into_any_element()
            })
            .collect();
        // "+": the new tab page (UI 1.1, 6).
        let new_tab = title_bar::icon_button("tab-new", IconName::Plus, cx)
            .tooltip(i18n::top_bar::NEW_TAB)
            .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                this.new_tab(window, cx);
            }));

        let unread = self.notifications.read(cx).unread();
        let list = self.notifications.clone();
        let mark_read = self.notifications.clone();
        let notifications = Popover::new("notifications-popover")
            .anchor(gpui::Anchor::TopRight)
            .trigger(title_bar::notifications_button(unread, cx))
            .on_open_change(move |open, _, cx| {
                if *open {
                    mark_read.update(cx, |list, cx| list.mark_all_read(cx));
                }
            })
            .w(px(320.))
            .p(metrics::SPACE_2)
            .rounded(metrics::RADIUS_OVERLAY)
            .shadow(theme::shadow_overlay(cx))
            .content(move |_, _, cx| render_notification_list(&list, cx));
        // The terminal sidebar button, on session tabs only (UI 1.1, 4.2).
        let sidebar_shown = self.terminal_sidebar.read(cx).is_visible();
        let terminal_sidebar = matches!(self.top_tab, TopTab::Session(_)).then(|| {
            title_bar::icon_button("top-bar-terminal-sidebar", IconName::PanelRight, cx)
                .tooltip(i18n::top_bar::TERMINAL_SIDEBAR)
                .selected(sidebar_shown)
                .on_click(cx.listener(|this, _: &ClickEvent, window, cx| {
                    this.toggle_terminal_sidebar(window, cx)
                }))
        });

        title_bar::bare_title_bar(cx).child(
            h_flex()
                .id("top-bar")
                .test_support()
                // Fills the kit's bar without sizing it: the bar keeps an
                // automatic minimum width (its content's min-content), so a
                // long tab strip in the flow would widen the bar past the
                // window instead of scrolling. The bar's own padding (12 px
                // in full screen) does not apply to an absolute child.
                .absolute()
                .inset_0()
                .when(window.is_fullscreen(), |bar| bar.left(metrics::SPACE_3))
                .gap(metrics::SPACE_2)
                .justify_between()
                .child(
                    h_flex()
                        .h_full()
                        .flex_1()
                        .min_w_0()
                        .gap(metrics::SPACE_1)
                        .items_center()
                        // Windows: interactive children occlude the drag area.
                        .child(div().occlude().child(menu))
                        .child(title_bar::vaults_pill(
                            vaults,
                            div().occlude().child(vault_switcher),
                            vaults_active,
                            cx,
                        ))
                        .child(sftp)
                        .child(
                            // Tabs that do not fit scroll sideways (a vertical
                            // wheel too). The strip occludes the drag area as
                            // a whole; its tabs let the wheel through to it.
                            h_flex()
                                .id("session-tabs")
                                .test_support()
                                .occlude()
                                .min_w_0()
                                .gap(metrics::SPACE_1)
                                .items_center()
                                .overflow_x_scroll()
                                .track_scroll(&self.tab_scroll)
                                .children(session_tabs),
                        )
                        .child(div().occlude().child(new_tab)),
                )
                .child(
                    h_flex()
                        .h_full()
                        .flex_none()
                        .gap(metrics::SPACE_1)
                        .items_center()
                        .child(div().occlude().child(notifications))
                        .children(terminal_sidebar.map(|button| div().occlude().child(button))),
                ),
        )
    }

    fn render_toast_host(&self) -> impl IntoElement {
        // Bottom center of the content area, 16 px above its bottom edge
        // (audit decision 7); never over the details panel.
        div()
            .absolute()
            .left_0()
            .right_0()
            .bottom(metrics::SPACE_4)
            .flex()
            .justify_center()
            .child(self.toasts.clone())
    }

    fn render_main(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.section {
            Section::Hosts => self.hosts.clone().into_any_element(),
            Section::KnownHosts => self.known_hosts.clone().into_any_element(),
            Section::Keychain => self.keychain.clone().into_any_element(),
            _ => placeholder::placeholder("section-placeholder", self.section.label(), cx)
                .into_any_element(),
        };
        div()
            .id("main-area")
            .test_support()
            .relative()
            .flex_1()
            .min_w_0()
            .h_full()
            .bg(cx.theme().colors.list)
            .child(content)
            .child(self.render_toast_host())
    }

    fn render_panel(&self, window: &Window, cx: &mut Context<Self>) -> Option<impl IntoElement> {
        let (panel, entity_id) = match self.panel.as_ref()? {
            Panel::Host(panel) => (panel.clone().into_any_element(), panel.entity_id()),
            Panel::Group(panel) => (panel.clone().into_any_element(), panel.entity_id()),
        };
        let theme = cx.theme();
        let duration = motion::normal(cx);
        let body = div().size_full().child(panel);
        let body = if duration.is_zero() {
            body.into_any_element()
        } else {
            body.with_animation(
                ("panel-enter", entity_id.as_u64()),
                Animation::new(duration).with_easing(motion::ease_out),
                |body, delta| body.opacity(delta),
            )
            .into_any_element()
        };
        let handle_hover = theme.drag_border;
        // The main area keeps its minimum width (lead decision 4), next to
        // the sidebar as it is drawn (full or the strip of icons).
        let sidebar = if self.sidebar_visible {
            let viewport = window.viewport_size().width;
            sidebar::width(sidebar::is_collapsed(viewport, Some(self.details_width)))
        } else {
            px(0.)
        };
        let width =
            details::fitted_width(self.details_width, window.viewport_size().width, sidebar);
        Some(
            div()
                .id("right-panel")
                .test_support()
                .relative()
                .w(width)
                .flex_none()
                .h_full()
                .border_l_1()
                .border_color(theme.border)
                .bg(theme.colors.list)
                .on_drag_move(cx.listener(
                    |this, event: &DragMoveEvent<details::ResizeDetails>, _, cx| {
                        let width =
                            details::width_for_drag(event.bounds.right(), event.event.position.x);
                        this.set_details_width(width, cx);
                    },
                ))
                .child(body)
                .child(
                    div()
                        .id("panel-resize")
                        .test_support()
                        .absolute()
                        .top_0()
                        .bottom_0()
                        .left(px(-3.))
                        .w(px(6.))
                        .cursor_col_resize()
                        .hover(move |style| style.bg(handle_hover.opacity(0.5)))
                        .on_drag(details::ResizeDetails, |_, _, _, cx| {
                            cx.new(|_| details::ResizePreview)
                        }),
                ),
        )
    }

    /// A session tab: its panes (SSH and local terminals, UI 4.1, 4.7) or
    /// the "In development" placeholder (the new tab page and the tabs of
    /// later stages), with the terminal sidebar on the right when it is
    /// shown (UI 4.2: the terminal narrows instead of being covered).
    fn render_session(
        &self,
        id: SessionId,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let page = match self.content(id) {
            Some(TabContent::NewTab {
                page: Some((page, _)),
            }) => Some(page.clone().into_any_element()),
            Some(TabContent::Sftp {
                screen: Some(screen),
                ..
            }) => Some(screen.clone().into_any_element()),
            _ => None,
        };
        let content = match (self.group(id), page) {
            (Some(group), _) => group.clone().into_any_element(),
            (None, Some(page)) => page,
            (None, None) => {
                let title = self
                    .sessions
                    .get(id)
                    .map(|tab| tab.title().clone())
                    .unwrap_or_default();
                placeholder::placeholder("session-placeholder", title, cx).into_any_element()
            }
        };
        let sidebar = self.terminal_sidebar.read(cx).is_visible();
        h_flex()
            .size_full()
            .child(
                div()
                    .id("session-area")
                    .test_support()
                    .relative()
                    .flex_1()
                    .min_w_0()
                    .h_full()
                    .bg(cx.theme().colors.list)
                    .child(content)
                    .child(self.render_toast_host()),
            )
            .when(sidebar, |area| area.child(self.terminal_sidebar.clone()))
    }

    // -- New tab, search, new window --------------------------------------------

    /// "+", Ctrl+T and Ctrl+Shift+T (UI 6): a "New Tab" tab with the new tab
    /// page.
    pub fn new_tab(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> SessionId {
        self.open_session(SessionRequest::new(SessionKind::NewTab), cx)
    }

    /// Ctrl+K and Ctrl+Shift+K (UI 6): the search over open tabs and hosts,
    /// as an overlay over this window; Esc closes it. Again while open:
    /// nothing.
    pub fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.is_some() {
            return;
        }
        let tabs = self.open_tabs(None, window, cx);
        let state = self.state.clone();
        let overlay = cx.new(|cx| SearchOverlay::new(state, tabs, window, cx));
        let subscription = cx.subscribe_in(&overlay, window, |this, _, event, window, cx| {
            this.on_new_tab_event(None, event, window, cx)
        });
        self.search = Some((overlay, subscription));
        cx.notify();
    }

    /// The search overlay, while open (tests).
    pub fn search_overlay(&self) -> Option<&Entity<SearchOverlay>> {
        self.search.as_ref().map(|(overlay, _)| overlay)
    }

    /// The new tab page of tab `id`, once rendered.
    pub fn new_tab_page(&self, id: SessionId) -> Option<&Entity<NewTabPage>> {
        match self.content(id)? {
            TabContent::NewTab {
                page: Some((page, _)),
            } => Some(page),
            _ => None,
        }
    }

    /// The open session tabs of every window for the search (UI 6), this
    /// window's first; `except` is left out (the page's own tab).
    fn open_tabs(&self, except: Option<SessionId>, window: &Window, cx: &App) -> Vec<OpenTab> {
        let own = window.window_handle();
        let mut tabs = self.own_open_tabs(own, except);
        for (handle, shell) in all_windows(cx) {
            if handle.window_id() != own.window_id() {
                tabs.extend(shell.read(cx).own_open_tabs(handle, None));
            }
        }
        tabs
    }

    /// This window's session tabs (not the new tab pages) as the search
    /// offers them.
    fn own_open_tabs(&self, handle: AnyWindowHandle, except: Option<SessionId>) -> Vec<OpenTab> {
        self.sessions
            .tabs()
            .iter()
            .filter(|tab| Some(tab.id()) != except && tab.kind() != SessionKind::NewTab)
            .map(|tab| {
                OpenTab::new(handle, tab.id(), tab.title().clone())
                    .icon(tab.icon())
                    .host(tab.host())
            })
            .collect()
    }

    /// Creates the SFTP session tabs that wait for the window: Local on the
    /// left, their host connecting on the right (UI 5.1).
    fn prepare_sftp_tabs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let waiting: Vec<(SessionId, Uuid)> = self
            .contents
            .iter()
            .filter_map(|(id, content)| match content {
                TabContent::Sftp { host, screen: None } => Some((*id, *host)),
                _ => None,
            })
            .collect();
        for (id, host) in waiting {
            let state = self.state.clone();
            let Some(item) = state.read(cx).host(host).cloned() else {
                continue;
            };
            let title: SharedString = item
                .label
                .clone()
                .unwrap_or_else(|| item.address.clone())
                .into();
            let screen = cx.new(|cx| crate::sftp::SftpScreen::new(state, window, cx));
            let record = shellpp_core::service::dto::RecordRef {
                vault_id: item.vault_id,
                record_id: item.record_id,
            };
            screen.update(cx, |screen, cx| screen.open_host(record, title, window, cx));
            if let Some((_, TabContent::Sftp { screen: slot, .. })) =
                self.contents.iter_mut().find(|(tab, _)| *tab == id)
            {
                *slot = Some(screen);
            }
        }
    }

    /// Creates the new tab pages that wait for the window, and hands the
    /// active page and the search the open tabs and the Restore flag.
    fn prepare_new_tab_pages(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let waiting: Vec<SessionId> = self
            .contents
            .iter()
            .filter(|(_, content)| matches!(content, TabContent::NewTab { page: None }))
            .map(|(id, _)| *id)
            .collect();
        for id in waiting {
            let state = self.state.clone();
            let page = cx.new(|cx| NewTabPage::new(state, window, cx));
            let subscription = cx.subscribe_in(&page, window, move |this, _, event, window, cx| {
                this.on_new_tab_event(Some(id), event, window, cx)
            });
            if let Some((_, TabContent::NewTab { page: slot })) =
                self.contents.iter_mut().find(|(tab, _)| *tab == id)
            {
                *slot = Some((page, subscription));
            }
        }
        let restore = restore::can_restore(cx);
        if let TopTab::Session(id) = self.top_tab
            && let Some(page) = self.new_tab_page(id).cloned()
        {
            let tabs = self.open_tabs(Some(id), window, cx);
            page.update(cx, |page, cx| {
                page.set_open_tabs(tabs, cx);
                page.set_restore_available(restore, cx);
            });
            if std::mem::take(&mut self.focus_page) {
                window.defer(cx, move |window, cx| {
                    page.update(cx, |page, cx| page.focus_search(window, cx));
                });
            }
        }
        if let Some((overlay, _)) = &self.search {
            let overlay = overlay.clone();
            let tabs = self.open_tabs(None, window, cx);
            overlay.update(cx, |overlay, cx| overlay.set_open_tabs(tabs, cx));
        }
    }

    /// A choice on the new tab page of tab `page` (UI 6: the session opens
    /// in that tab), or in the search overlay (`page` is `None`: a new
    /// tab). `new_window`: in a new window instead (Alt, UI 2.4).
    fn on_new_tab_event(
        &mut self,
        page: Option<SessionId>,
        event: &NewTabEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (target, new_window) = match event {
            NewTabEvent::OpenHost {
                host_id,
                new_window,
            } => {
                let request = SessionRequest::new(SessionKind::Ssh).host(*host_id);
                (
                    PaneTarget::from_request(&request, self.state.read(cx)),
                    *new_window,
                )
            }
            NewTabEvent::QuickConnect { target, new_window } => {
                (Some(PaneTarget::Quick(target.clone())), *new_window)
            }
            NewTabEvent::OpenLocal {
                shell_id,
                new_window,
            } => (
                Some(PaneTarget::Local {
                    shell_id: shell_id.clone(),
                }),
                *new_window,
            ),
            NewTabEvent::ActivateTab {
                window: handle,
                tab,
            } => {
                self.close_search(cx);
                self.activate_open_tab(*handle, *tab, window, cx);
                return;
            }
            NewTabEvent::Restore => {
                self.close_search(cx);
                let opened = restore::restore_previous_launch(self, window, cx);
                // The page served its purpose: the restored tabs replace it
                // [decision].
                if let Some(page) = page.filter(|_| opened > 0) {
                    self.close_session(page, cx);
                }
                return;
            }
            NewTabEvent::Dismissed => {
                if page.is_none() {
                    self.close_search(cx);
                }
                return;
            }
        };
        self.close_search(cx);
        let Some(target) = target else {
            return;
        };
        if new_window {
            windows::open_in_new_window(self.state.clone(), window, cx, move |shell, _, cx| {
                shell.open_target(target, None, cx);
            });
            return;
        }
        match page {
            Some(page) => {
                self.place_tab(TabSeed::Target(target), TabPlace::Replace(page), None, cx);
            }
            None => {
                self.open_target(target, None, cx);
            }
        }
    }

    /// Closes the search overlay.
    fn close_search(&mut self, cx: &mut Context<Self>) {
        if self.search.take().is_some() {
            cx.notify();
        }
    }

    /// Activates tab `tab` of window `handle`: this window's directly, another
    /// window's after this update, with that window brought to the front
    /// (UI 6).
    fn activate_open_tab(
        &mut self,
        handle: AnyWindowHandle,
        tab: SessionId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if handle.window_id() == window.window_handle().window_id() {
            self.set_top_tab(TopTab::Session(tab), cx);
            self.focus_active_tab(cx);
            return;
        }
        cx.defer(move |cx| windows::activate_tab_in(handle, tab, cx));
    }

    // -- Terminal sidebar ------------------------------------------------------------

    /// The top bar button (UI 4.2): shows or hides the terminal sidebar of
    /// this window; only on a session tab.
    pub fn toggle_terminal_sidebar(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.top_tab, TopTab::Session(_)) {
            return;
        }
        let visible = !self.terminal_sidebar.read(cx).is_visible();
        self.terminal_sidebar
            .update(cx, |sidebar, cx| sidebar.set_visible(visible, cx));
        self.sync_sidebar_target(window, cx);
        if !visible {
            // The focus may have been in the sidebar: back to the pane.
            self.focus_active_tab(cx);
        }
        cx.notify();
    }

    /// Ctrl+Shift+F (UI 4.2, 12.5): the sidebar opens on Tools with the
    /// search field focused; only on a session tab.
    pub fn find_in_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !matches!(self.top_tab, TopTab::Session(_)) {
            return;
        }
        self.sync_sidebar_target(window, cx);
        self.terminal_sidebar
            .update(cx, |sidebar, cx| sidebar.find_in_terminal(window, cx));
        cx.notify();
    }

    /// What the terminal sidebar acts on: the terminal of the active tab's
    /// active pane, its saved host and the tab; `None` on other tabs and
    /// while the active pane chooses a host.
    fn sidebar_target(&self, cx: &App) -> Option<SidebarTarget> {
        let TopTab::Session(id) = self.top_tab else {
            return None;
        };
        let group = self.group(id)?.read(cx);
        let terminal = group.active_terminal(cx)?;
        let host = group.active_target(cx).and_then(|target| target.host_id());
        Some(SidebarTarget::new(&terminal).host(host).session(Some(id)))
    }

    /// Hands the sidebar its target when it changed (a tab or pane became
    /// active, a session started its terminal).
    fn sync_sidebar_target(&self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.sidebar_target(cx);
        if self.terminal_sidebar.read(cx).target() == target.as_ref() {
            return;
        }
        let sidebar = self.terminal_sidebar.clone();
        window.defer(cx, move |window, cx| {
            sidebar.update(cx, |sidebar, cx| sidebar.set_target(target, window, cx));
        });
    }

    // -- Tab menu (UI 4.4) ---------------------------------------------------------

    /// What the menu of tab `id` offers.
    fn tab_facts(&self, id: SessionId, cx: &App) -> tab_menu::TabFacts {
        let Some(group) = self.group(id) else {
            return tab_menu::TabFacts::default();
        };
        let group = group.read(cx);
        let active = group.active_pane();
        // UI 4.4: Duplicate and the splits stay disabled while the active
        // pane has no session target (a pane still choosing a host).
        let has_target = group.active_target(cx).is_some();
        tab_menu::TabFacts {
            has_target,
            can_split_down: has_target && group.can_split(active, Direction::Column),
            can_split_right: has_target && group.can_split(active, Direction::Row),
            has_splits: group.has_splits(),
        }
    }

    /// Runs a command of tab `id`'s menu.
    pub fn run_tab_command(
        &mut self,
        id: SessionId,
        command: tab_menu::TabCommand,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        use tab_menu::TabCommand;
        let group = self.group(id).cloned();
        match command {
            TabCommand::Duplicate => {
                if let Some(target) = group.and_then(|group| group.read(cx).active_target(cx)) {
                    self.open_target(target, Some(id), cx);
                }
            }
            TabCommand::DuplicateInNewWindow => {
                if let Some(target) = group.and_then(|group| group.read(cx).active_target(cx)) {
                    windows::open_in_new_window(
                        self.state.clone(),
                        window,
                        cx,
                        move |shell, _, cx| {
                            shell.open_target(target, None, cx);
                        },
                    );
                }
            }
            TabCommand::Rename => self.start_rename(id, window, cx),
            TabCommand::SplitHorizontally | TabCommand::SplitVertically => {
                let direction = if command == TabCommand::SplitHorizontally {
                    Direction::Column
                } else {
                    Direction::Row
                };
                if let Some(group) = group {
                    self.set_top_tab(TopTab::Session(id), cx);
                    group.update(cx, |group, cx| {
                        group.split(direction, cx);
                    });
                }
            }
            // Stage 9 (D30): shown disabled.
            TabCommand::BroadcastInput => {}
            TabCommand::Close => {
                self.close_session(id, cx);
                self.focus_active_tab(cx);
            }
        }
    }

    // -- Rename (UI 4.4) -------------------------------------------------------------

    /// Starts editing tab `id`'s title in place: the field holds the title
    /// on display, selected. Enter or leaving the field saves, Esc cancels,
    /// and an empty title brings back the host's.
    pub fn start_rename(&mut self, id: SessionId, window: &mut Window, cx: &mut Context<Self>) {
        let Some(title) = self.sessions.get(id).map(|tab| tab.title().clone()) else {
            return;
        };
        self.take_rename(cx);
        // Into the tab without a focus request for its pane: the field takes
        // the focus, and a pane taking it back would end the edit.
        self.top_tab = TopTab::Session(id);
        self.reveal_active_tab();
        self.focus_page = false;
        // At most the length Restore keeps, without control characters
        // (a pasted tab): sessions::MAX_TITLE_CHARS.
        let input =
            cx.new(|cx| InputState::new(window, cx).validate(|text, _| sessions::title_ok(text)));
        input.update(cx, |input, cx| {
            input.set_value(title, window, cx);
            input.select_all(window, cx);
            input.focus(window, cx);
        });
        let focus = gpui::Focusable::focus_handle(input.read(cx), cx);
        hold_focus(&focus, cx);
        let subscription = cx.subscribe_in(
            &input,
            window,
            move |this: &mut Self, _, event: &InputEvent, _, cx| match event {
                InputEvent::PressEnter { .. } => this.finish_rename(true, true, cx),
                // A click elsewhere saves and leaves the focus where it went.
                InputEvent::Blur => this.finish_rename(true, false, cx),
                InputEvent::Change | InputEvent::Focus => {}
            },
        );
        self.renaming = Some(Rename {
            tab: id,
            input,
            focus,
            _subscription: subscription,
        });
        cx.notify();
    }

    /// Ends the inline Rename: `save` keeps the field's text as the tab's
    /// title when it was changed (empty: the host's title again), else
    /// nothing changes; `refocus` moves the focus back into the tab's pane
    /// (Enter and Esc, not a click elsewhere).
    pub fn finish_rename(&mut self, save: bool, refocus: bool, cx: &mut Context<Self>) {
        let Some(rename) = self.take_rename(cx) else {
            return;
        };
        if save {
            let text = rename.input.read(cx).value().to_string();
            // The title on display, unchanged, stays the host's: the tab
            // keeps following the active pane (UI 4.7).
            let changed = self
                .sessions
                .get(rename.tab)
                .is_none_or(|tab| tab.title().as_ref() != text.trim());
            if changed {
                self.sessions.rename(rename.tab, Some(&text));
            }
        }
        if refocus {
            self.focus_active_tab(cx);
        }
        cx.notify();
    }

    /// Closes the Rename field without saving and lets the panes take the
    /// focus again.
    fn take_rename(&mut self, cx: &mut Context<Self>) -> Option<Rename> {
        let rename = self.renaming.take()?;
        release_focus(&rename.focus, cx);
        Some(rename)
    }

    /// The tab being renamed (tests).
    pub fn renaming(&self) -> Option<(SessionId, &Entity<InputState>)> {
        self.renaming
            .as_ref()
            .map(|rename| (rename.tab, &rename.input))
    }

    /// The Rename field in place of the tab title: Esc cancels.
    fn render_rename(
        &self,
        tab: &SessionTab,
        rename: &Rename,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(tab.rename_element_id())
            .test_support()
            .w_full()
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                if event.keystroke.key == "escape" {
                    cx.stop_propagation();
                    this.finish_rename(false, true, cx);
                }
            }))
            .child(
                Input::new(&rename.input)
                    .appearance(false)
                    .px_0()
                    .text_size(metrics::FONT_SIZE_BODY),
            )
            .into_any_element()
    }

    // -- Restore (UI 6) ----------------------------------------------------------------

    /// This window's tabs for Restore (lead decision L5): their pane trees
    /// with each session's target and the titles set with Rename. Tabs
    /// without a session (the new tab page, SFTP, serial) are left out.
    pub fn snapshot(&self, cx: &App) -> restore::WindowSnapshot {
        self.sessions
            .tabs()
            .iter()
            .filter_map(|tab| {
                let layout = self.group(tab.id())?.read(cx).snapshot(cx)?;
                Some((layout, tab.custom_title().map(|title| title.to_string())))
            })
            .collect()
    }
}

/// `user@host:port` as known before connecting (the port only when it is
/// not 22, IPv6 in brackets with a port), for the connection screen.
pub fn display_address(username: Option<&str>, host: &str, port: Option<u16>) -> String {
    let host = match port {
        Some(port) if port != 22 && host.contains(':') => format!("[{host}]:{port}"),
        Some(port) if port != 22 => format!("{host}:{port}"),
        _ => host.to_owned(),
    };
    match username.filter(|user| !user.is_empty()) {
        Some(user) => format!("{user}@{host}"),
        None => host,
    }
}

/// Where [`Shell::place_tab`] puts a session.
#[derive(Clone, Copy)]
enum TabPlace {
    /// A new tab, right after this one when given.
    New(Option<SessionId>),
    /// In place of this tab's new tab page.
    Replace(SessionId),
}

/// What a new session tab opens with.
enum TabSeed {
    /// One pane connecting to a target.
    Target(PaneTarget),
    /// The panes of a Restore snapshot.
    Snapshot(Layout<PaneTarget>),
}

/// The kind of a tab whose (first) pane connects to `target`.
fn target_kind(target: &PaneTarget) -> SessionKind {
    if target.is_local() {
        SessionKind::Local
    } else {
        SessionKind::Ssh
    }
}

/// Whether the window of `shell` may close now: with active sessions it
/// asks first (UI 1.4, 4.6: [`confirm_close_window`] closes it on Close
/// Window); otherwise the pending edit is saved, the last window saves the
/// tabs for Restore, and it may go.
fn window_may_close(shell: &Entity<Shell>, window: &mut Window, cx: &mut App) -> bool {
    let sessions = shell.read(cx).active_sessions(cx);
    if sessions == 0 {
        windows::before_close(shell, cx);
        return true;
    }
    confirm_close_window(sessions, window, cx);
    false
}

/// The close button of the window: the same as the system close
/// ([`window_may_close`]), for the buttons the top bar draws itself (Linux
/// client-side decorations call it instead of closing at once).
pub fn request_close_window(window: &mut Window, cx: &mut App) {
    let may_close = match shell_for(window, cx) {
        Some(shell) => window_may_close(&shell, window, cx),
        None => true,
    };
    if may_close {
        window.remove_window();
    }
}

/// Closing a window while its sessions are active (UI 1.4, 4.6): Close
/// Window ends them (and, for the last window, saves the tabs for Restore
/// first); Cancel keeps the window.
fn confirm_close_window(sessions: usize, window: &mut Window, cx: &mut App) {
    window.open_alert_dialog(cx, move |alert, _, cx| {
        let theme = cx.theme();
        alert
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .child(strings::CLOSE_WINDOW_TITLE),
            )
            .description(strings::close_window_body(sessions))
            .show_cancel(true)
            .cancel_text(strings::CLOSE_WINDOW_CANCEL)
            .ok_text(strings::CLOSE_WINDOW_OK)
            .ok_variant(ButtonVariant::Danger)
            .bg(theme.popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .shadow(theme::shadow_modal(cx))
            .on_ok(move |_, window, cx| {
                if let Some(shell) = shell_for(window, cx) {
                    windows::before_close(&shell, cx);
                }
                window.remove_window();
                true
            })
    });
}

/// The exit confirmation (UI 1.4, 4.6): Exit ends the sessions of every
/// window and quits (the open tabs are saved for Restore on the way out,
/// `crate::app::run`).
fn confirm_exit(sessions: usize, window: &mut Window, cx: &mut App) {
    window.open_alert_dialog(cx, move |alert, _, cx| {
        let theme = cx.theme();
        alert
            .title(
                div()
                    .text_size(metrics::FONT_SIZE_TITLE)
                    .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                    .child(i18n::exit::TITLE),
            )
            .description(i18n::exit::body(sessions))
            .show_cancel(true)
            .cancel_text(i18n::exit::CANCEL)
            .ok_text(i18n::exit::EXIT)
            .ok_variant(ButtonVariant::Danger)
            .bg(theme.popover)
            .rounded(metrics::RADIUS_OVERLAY)
            .shadow(theme::shadow_modal(cx))
            .on_ok(move |_, window, cx| {
                // Pending panel edits are saved before the windows go; the
                // sessions of every window end.
                if let Some(shell) = shell_for(window, cx) {
                    shell.update(cx, |shell, cx| shell.flush_panel(cx));
                }
                for shell in all_shells(cx) {
                    shell.update(cx, |shell, cx| shell.shutdown_sessions(cx));
                }
                cx.quit();
                true
            })
    });
}

fn render_notification_list(list: &Entity<Notifications>, cx: &App) -> AnyElement {
    let theme = cx.theme();
    let entries = list.read(cx).entries();
    v_flex()
        .id("notifications-list")
        .test_support()
        .gap(metrics::SPACE_1)
        .child(
            div()
                .px(metrics::SPACE_2)
                .py(metrics::SPACE_1)
                .text_size(metrics::FONT_SIZE_BODY)
                .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
                .text_color(theme.popover_foreground)
                .child(i18n::notifications::TITLE),
        )
        .when(entries.is_empty(), |list| {
            list.child(
                div()
                    .id("notifications-empty")
                    .test_support()
                    .px(metrics::SPACE_2)
                    .py(metrics::SPACE_2)
                    .text_size(metrics::FONT_SIZE_BODY)
                    .text_color(theme.muted_foreground)
                    .child(i18n::notifications::EMPTY),
            )
        })
        .children(entries.iter().map(|entry| {
            div()
                .id(entry.element_id())
                .test_support()
                .px(metrics::SPACE_2)
                .py(metrics::SPACE_2)
                .rounded(metrics::RADIUS_CONTROL)
                .text_size(metrics::FONT_SIZE_BODY)
                .text_color(theme.popover_foreground)
                .child(entry.message().clone())
        }))
        .into_any_element()
}

impl Render for Shell {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The terminal sidebar acts on the active pane's session (UI 4.2).
        self.sync_sidebar_target(window, cx);
        // A private key shown in the Keychain is hidden again once the
        // window shows anything else (UI 7.3).
        let keychain_shown = self.top_tab == TopTab::Vaults && self.section == Section::Keychain;
        self.keychain
            .update(cx, |screen, cx| screen.set_shown(keychain_shown, cx));
        // New tab pages need the window; the active one and the search
        // get the open tabs and the Restore flag.
        self.prepare_new_tab_pages(window, cx);
        self.prepare_sftp_tabs(window, cx);
        // Sessions need the window to start: the panes of tabs in the
        // background connect all the same.
        for group in self.groups() {
            group.update(cx, |group, cx| group.start_pending(window, cx));
        }
        if self.focus_shell && !window.has_active_dialog(cx) {
            self.focus_shell = false;
            let focus = self.focus.clone();
            window.defer(cx, move |window, cx| window.focus(&focus, cx));
        }
        if std::mem::take(&mut self.reveal_after_frame) {
            // This frame lays the bar out anew; the next one scrolls with it.
            let shell = cx.weak_entity();
            window.on_next_frame(move |_, cx| {
                let _ = shell.update(cx, |shell, cx| {
                    shell.scroll_to_active_tab();
                    cx.notify();
                });
            });
        }
        let body = match self.top_tab {
            TopTab::Vaults => h_flex()
                .size_full()
                .when(self.sidebar_visible, |body| {
                    body.child(sidebar::sidebar(
                        self.section,
                        &self.nav_focus,
                        // The strip of icons below a 600 px main area.
                        sidebar::is_collapsed(
                            window.viewport_size().width,
                            self.panel.as_ref().map(|_| self.details_width).or_else(|| {
                                // Known Hosts and the Keychain draw their own
                                // details panel.
                                match self.section {
                                    Section::KnownHosts => {
                                        self.known_hosts.read(cx).open_panel_width()
                                    }
                                    Section::Keychain => self.keychain.read(cx).open_panel_width(),
                                    _ => None,
                                }
                            }),
                        ),
                        {
                            let shell = cx.weak_entity();
                            move |section, _, cx| {
                                let _ =
                                    shell.update(cx, |shell, cx| shell.set_section(section, cx));
                            }
                        },
                        cx,
                    ))
                })
                .child(self.render_main(cx))
                .children(self.render_panel(window, cx))
                .into_any_element(),
            TopTab::Sftp => div()
                .id("sftp-area")
                .test_support()
                .relative()
                .size_full()
                .bg(cx.theme().colors.list)
                .child(self.sftp.clone())
                .child(self.render_toast_host())
                .into_any_element(),
            TopTab::Session(id) => self.render_session(id, window, cx).into_any_element(),
        };
        v_flex()
            .id("shell")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            // A click in the shell while the focus is elsewhere (or nowhere)
            // brings it back, so the shell's shortcuts keep working; the
            // clicked control may then take the focus itself.
            .capture_any_mouse_down(cx.listener(|this, _, window, cx| {
                if !this.focus.contains_focused(window, cx) {
                    window.focus(&this.focus, cx);
                }
            }))
            .on_action(
                cx.listener(|this, _: &ShowVaults, _, cx| this.set_top_tab(TopTab::Vaults, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowSftp, _, cx| this.set_top_tab(TopTab::Sftp, cx)))
            .on_action(
                cx.listener(|this, action: &ActivateTab, _, cx| this.activate_tab(action.0, cx)),
            )
            .on_action(cx.listener(|this, _: &OpenSettings, window, cx| {
                crate::settings::open(this.state.clone(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &NewLocalTerminal, _, cx| {
                this.open_session(SessionRequest::new(SessionKind::Local), cx);
            }))
            .on_action(cx.listener(|this, _: &NewSerialConnection, _, cx| {
                this.open_session(SessionRequest::new(SessionKind::Serial), cx);
            }))
            // "+", Ctrl+T outside a terminal, Ctrl+Shift+T (UI 6).
            .on_action(cx.listener(|this, _: &NewTab, window, cx| {
                this.new_tab(window, cx);
            }))
            // Ctrl+K outside a terminal, Ctrl+Shift+K (UI 6).
            .on_action(cx.listener(|this, _: &OpenSearch, window, cx| this.open_search(window, cx)))
            .on_action(cx.listener(|this, _: &ZoomIn, _, cx| {
                this.placeholder_toast(i18n::menu::INTERFACE_ZOOM, cx)
            }))
            .on_action(cx.listener(|this, _: &ZoomOut, _, cx| {
                this.placeholder_toast(i18n::menu::INTERFACE_ZOOM, cx)
            }))
            .on_action(cx.listener(|this, _: &ActualSize, _, cx| {
                this.placeholder_toast(i18n::menu::INTERFACE_ZOOM, cx)
            }))
            .on_action(cx.listener(|_, _: &ToggleFullScreen, window, _| window.toggle_fullscreen()))
            .on_action(cx.listener(|this, _: &ToggleSidebar, _, cx| this.toggle_sidebar(cx)))
            .on_action(cx.listener(|this, _: &NewWindow, window, cx| {
                windows::open_in_new_window(this.state.clone(), window, cx, |_, _, _| {});
            }))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| this.cycle_tab(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, _, cx| this.cycle_tab(-1, cx)))
            // The focus goes to the next active pane or tab.
            .on_action(cx.listener(|this, _: &CloseTab, _, cx| this.close_active(cx)))
            .on_action(cx.listener(|_, _: &OpenDocumentation, _, cx| {
                cx.open_url(&format!("{}#readme", env!("CARGO_PKG_REPOSITORY")));
            }))
            .on_action(cx.listener(|_, _: &ReportIssue, _, cx| {
                cx.open_url(&format!("{}/issues", env!("CARGO_PKG_REPOSITORY")));
            }))
            .on_action(cx.listener(|this, _: &ImportData, window, cx| {
                crate::data_transfer::open_import(this.state.clone(), window, cx);
            }))
            .on_action(cx.listener(|this, _: &ExportData, window, cx| {
                crate::data_transfer::open_export(this.state.clone(), window, cx);
            }))
            .on_action(cx.listener(|this, _: &CheckForUpdates, _, cx| {
                this.placeholder_toast(i18n::menu::CHECK_FOR_UPDATES, cx)
            }))
            .on_action(
                cx.listener(|this, _: &About, _, cx| this.toast(i18n::menu::about_text(), cx)),
            )
            .on_action(cx.listener(|this, _: &LockApp, _, cx| this.lock_app(cx)))
            .on_action(cx.listener(|this, _: &Exit, window, cx| this.request_exit(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleTerminalSidebar, window, cx| {
                this.toggle_terminal_sidebar(window, cx)
            }))
            .on_action(
                cx.listener(|this, _: &FindInTerminal, window, cx| {
                    this.find_in_terminal(window, cx)
                }),
            )
            .child(self.render_title_bar(window, cx))
            .child(div().flex_1().min_h_0().child(body))
            .children(self.search.as_ref().map(|(overlay, _)| overlay.clone()))
    }
}

#[cfg(test)]
mod tabs_tests;
#[cfg(test)]
mod tests;
