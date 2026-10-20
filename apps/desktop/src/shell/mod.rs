//! The shell of an unlocked window (UI 1): the top bar, then the content of
//! the active top-bar tab:
//!
//! - Vaults: the sidebar (UI 1.2), the main area with the toasts at its
//!   bottom center, and the details panel on the right (UI 1.3);
//! - SSH and local terminal tabs: their [`SessionPane`] (connection screen,
//!   dialogs, terminal, end and drop overlays; `crate::session_ui`);
//! - SFTP (and the tabs of later stages): the "In development" placeholder
//!   (audit decisions 1 and 6; D30).
//!
//! Session tabs (UI 1.1, 4.6, 12.5; stage 2a lead decision 9): a session
//! opens from a double click or Enter on a card, Quick Connect, the host
//! panel's Connect and the search bar's quick connect; the Terminal button,
//! New Local Terminal and Ctrl+L (outside a terminal) open a local terminal.
//! A tab shows the host's OS icon (the server icon until the OS is known)
//! and its name, with a close button. Ctrl+1..9, Alt+Left/Right and
//! Ctrl+Shift+W work everywhere, a terminal included. When the active tab
//! closes, the left neighbor (else the right one, else Vaults) becomes
//! active (UI 4.6). Tabs that do not fit scroll sideways: the wheel scrolls
//! the strip, and the active tab always scrolls into view (UI 1.1). The "+"
//! button and the terminal sidebar button lead to stage 2b and are shown
//! disabled (D30).
//!
//! Other modules talk to the shell through events (`HostsEvent`,
//! `HostsRequest`, `PanelEvent`, `PanelRequest`, `SessionPaneEvent`) or the
//! window-level functions [`show_toast`], [`notify`], [`in_development`] and
//! [`open_session_tab`].

pub mod actions;
pub mod app_menu;
pub mod details;
pub mod notifications;
pub mod placeholder;
pub mod sessions;
pub mod sidebar;
pub mod title_bar;
pub mod toasts;

use gpui::{
    Animation, AnimationExt as _, AnyElement, App, AppContext as _, ClickEvent, Context,
    DragMoveEvent, Entity, FocusHandle, Global, IntoElement, ParentElement as _, Pixels, Render,
    ScrollHandle, SharedString, Styled as _, Subscription, WeakEntity, Window, WindowId, div,
    prelude::*, px,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::ButtonVariant;
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::popover::Popover;
use gpui_kit::component::{ActiveTheme as _, IconName, WindowExt as _, h_flex, v_flex};
use uuid::Uuid;

use crate::hosts::{self, HostsEvent, HostsRequest, HostsScreen};
use crate::i18n;
use crate::panels::{GroupPanel, HostPanel, PanelEvent, PanelRequest};
use crate::session_ui::{Phase, SessionPane, SessionPaneEvent, TabInfo};
use crate::sessions::OpenRequest;
use crate::state::AppState;
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
struct ShellRegistry(Vec<(WindowId, WeakEntity<Shell>)>);

impl Global for ShellRegistry {}

/// The shell of `window` (none while locked).
pub fn shell_for(window: &Window, cx: &App) -> Option<Entity<Shell>> {
    let id = window.window_handle().window_id();
    cx.try_global::<ShellRegistry>()?
        .0
        .iter()
        .rev()
        .find(|(window_id, _)| *window_id == id)
        .and_then(|(_, shell)| shell.upgrade())
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
            for (_, pane, _) in &shell.panes {
                pane.update(cx, |pane, cx| pane.release(cx));
            }
        });
    }
    crate::sessions::ui::shutdown(cx);
    if cx.has_global::<crate::sessions::ui::Sessions>() {
        drop(cx.remove_global::<crate::sessions::ui::Sessions>());
    }
}

/// Locks the app (UI 13.3) from outside a shell: Ctrl+Shift+L that no view
/// handled, and a due auto-lock. Every window's details panel saves its
/// pending edits and closes first, then the core locks once. Does nothing
/// while locked.
pub fn lock_all(state: &Entity<AppState>, cx: &mut App) {
    if !state.read(cx).is_unlocked() {
        return;
    }
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

pub struct Shell {
    state: Entity<AppState>,
    top_tab: TopTab,
    sessions: SessionTabs,
    section: Section,
    sidebar_visible: bool,
    hosts: Entity<HostsScreen>,
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
    /// The content of SSH and local terminal tabs, with the subscription to
    /// each pane's events.
    panes: Vec<(SessionId, Entity<SessionPane>, Subscription)>,
    /// Panes whose session starts at the next render (the terminal needs
    /// the window).
    unstarted: Vec<Entity<SessionPane>>,
    /// Give the keyboard focus back to the shell at the next render (the
    /// active tab changed to one without a terminal).
    focus_shell: bool,
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
        registry.0.push((window.window_handle().window_id(), this));

        // Closing the window while sessions are active asks first (UI 4.6).
        // An edit still waiting for the autosave is saved before the window
        // (and the panel with it) goes.
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let Some(shell) = this.upgrade() else {
                return true;
            };
            let sessions = shell.read(cx).active_sessions(cx);
            if sessions == 0 {
                shell.update(cx, |shell, cx| {
                    shell.flush_panel(cx);
                    shell.shutdown_sessions(cx);
                });
                return true;
            }
            confirm_exit(sessions, false, window, cx);
            false
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
            panes: Vec::new(),
            unstarted: Vec::new(),
            focus_shell: false,
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
    /// terminal or dialog (UI 4.1), else back to the shell, so that the
    /// shortcuts keep working when the focused terminal is no longer shown.
    fn focus_active_tab(&mut self, cx: &mut Context<Self>) {
        match self.top_tab {
            TopTab::Session(id) => match self.pane(id).cloned() {
                Some(pane) => pane.update(cx, |pane, cx| pane.request_focus(cx)),
                None => self.focus_shell = true,
            },
            TopTab::Vaults | TopTab::Sftp => self.focus_shell = true,
        }
    }

    /// The content of session tab `id` (SSH and local terminal tabs).
    pub fn pane(&self, id: SessionId) -> Option<&Entity<SessionPane>> {
        self.panes
            .iter()
            .find(|(session, _, _)| *session == id)
            .map(|(_, pane, _)| pane)
    }

    /// Sessions that count for the exit confirmation (UI 4.6): connecting,
    /// connected, reconnecting, or dropped with automatic attempts ahead.
    pub fn active_sessions(&self, cx: &App) -> usize {
        self.panes
            .iter()
            .filter(|(_, pane, _)| {
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
        for (_, pane, _) in &self.panes {
            pane.update(cx, |pane, cx| pane.shutdown(cx));
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
    /// get a [`SessionPane`] whose session starts at the next render; the
    /// tabs of later stages (SFTP, serial, the new tab page) show the "In
    /// development" placeholder.
    pub fn open_session(&mut self, request: SessionRequest, cx: &mut Context<Self>) -> SessionId {
        let pane = self.pane_for(&request, cx);
        let request = match &pane {
            Some((info, _)) => request.title(info.title().clone()).icon(info.icon()),
            None => request,
        };
        // A quick connect (UI 2.1) offers to save the host once it connects.
        let quick = request.quick_target().cloned();
        let id = self.sessions.open(request);
        if let Some((info, open)) = pane {
            let pane = cx.new(|cx| SessionPane::new(info, open, cx));
            let subscription = cx.subscribe(&pane, move |this, pane, event, cx| {
                this.on_pane_event(id, &pane, event, quick.as_ref(), cx)
            });
            self.unstarted.push(pane.clone());
            self.panes.push((id, pane, subscription));
        }
        self.session_focus
            .push((id, cx.focus_handle().tab_stop(true)));
        self.top_tab = TopTab::Session(id);
        self.reveal_active_tab();
        self.focus_active_tab(cx);
        cx.notify();
        id
    }

    /// What an SSH or local terminal tab shows first and what it opens;
    /// `None` for the tabs of later stages.
    fn pane_for(&self, request: &SessionRequest, cx: &App) -> Option<(TabInfo, OpenRequest)> {
        use shellpp_core::service::dto::SessionTarget;
        match request.kind() {
            SessionKind::Ssh => {
                let state = self.state.read(cx);
                if let Some(id) = request.host_id() {
                    let Some(host) = state.host(id) else {
                        // Gone meanwhile: the session layer reports it.
                        let title = request
                            .requested_title()
                            .cloned()
                            .unwrap_or_else(|| SessionKind::Ssh.default_title().into());
                        let target = SessionTarget::Host {
                            vault_id: state.vault_id()?,
                            record_id: id,
                        };
                        let icon = crate::session_ui::bridge::initial_icon(None, false);
                        return Some((TabInfo::remote(title, "", icon), OpenRequest::Ssh(target)));
                    };
                    let info = TabInfo::remote(
                        host.title().to_owned(),
                        display_address(host.username.as_deref(), &host.address, host.port),
                        crate::session_ui::bridge::initial_icon(host.os_family.as_deref(), false),
                    );
                    let target = SessionTarget::Host {
                        vault_id: host.vault_id,
                        record_id: host.record_id,
                    };
                    return Some((info, OpenRequest::Ssh(target)));
                }
                let quick = request.quick_target()?;
                let title = request
                    .requested_title()
                    .cloned()
                    .unwrap_or_else(|| quick.title().into());
                let info = TabInfo::remote(
                    title,
                    display_address(quick.username.as_deref(), &quick.host, quick.port),
                    crate::session_ui::bridge::initial_icon(None, false),
                );
                let target = SessionTarget::Quick {
                    address: quick.host.clone(),
                    port: quick.port,
                    username: quick.username.clone(),
                };
                Some((info, OpenRequest::Ssh(target)))
            }
            SessionKind::Local => {
                let title = request
                    .requested_title()
                    .cloned()
                    .unwrap_or_else(|| SessionKind::Local.default_title().into());
                let info = TabInfo::local(title, SessionKind::Local.default_icon());
                Some((info, OpenRequest::Local { shell_id: None }))
            }
            SessionKind::Sftp | SessionKind::Serial | SessionKind::NewTab => None,
        }
    }

    fn on_pane_event(
        &mut self,
        id: SessionId,
        pane: &Entity<SessionPane>,
        event: &SessionPaneEvent,
        quick: Option<&hosts::QuickTarget>,
        cx: &mut Context<Self>,
    ) {
        match event {
            // Exit status 0 or Close (UI 4.6): the neighbor rule applies.
            SessionPaneEvent::Close => self.close_session(id, cx),
            SessionPaneEvent::Connected => {
                if let Some(target) = quick {
                    self.offer_to_save(target.clone(), cx);
                }
            }
            SessionPaneEvent::Retitled => {
                let info = pane.read(cx).info();
                let (title, icon) = (info.title().clone(), info.icon());
                if self.sessions.retitle(id, title, icon) {
                    cx.notify();
                }
            }
        }
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

    /// Closes a session tab and ends its session; when it was active, the
    /// neighbor of UI 4.6 becomes active (left, else right, else Vaults).
    pub fn close_session(&mut self, id: SessionId, cx: &mut Context<Self>) {
        let was_active = self.top_tab == TopTab::Session(id);
        if let Some(index) = self.panes.iter().position(|(session, _, _)| *session == id) {
            let (_, pane, _subscription) = self.panes.remove(index);
            self.unstarted.retain(|unstarted| *unstarted != pane);
            // Ends the session and lets go of it, so that nothing keeps the
            // core open if the pane outlives its tab.
            pane.update(cx, |pane, cx| pane.release(cx));
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
            confirm_exit(sessions, true, window, cx);
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

    /// On lock: closes the details panel (its record data goes with the
    /// keys; pending edits are saved first while they still can be) and
    /// remembers its record for [`Shell::resume_panel`]. Without an open
    /// panel it keeps what an earlier call remembered.
    pub fn suspend_panel(&mut self, cx: &mut Context<Self>) {
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
                if request.new_window {
                    self.placeholder_toast(i18n::menu::NEW_WINDOW, cx);
                }
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
                self.open_session(session, cx);
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
                self.open_session(session, cx);
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
        let vaults = title_bar::top_tab(
            "tab-vaults",
            &self.vaults_focus,
            Lucide::Vault,
            i18n::top_bar::VAULTS,
            self.top_tab == TopTab::Vaults,
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
                title_bar::session_tab(tab, &focus, self.top_tab == TopTab::Session(id), close, cx)
                    .on_click(cx.listener(move |this, _: &ClickEvent, _, cx| {
                        this.set_top_tab(TopTab::Session(id), cx);
                        // A click on the active tab gives its terminal the
                        // focus back.
                        this.focus_active_tab(cx);
                    }))
                    .into_any_element()
            })
            .collect();
        // The new tab page is stage 2b: "+" is shown disabled (D30).
        let new_tab = title_bar::disabled(
            title_bar::icon_button("tab-new", IconName::Plus, cx).tooltip(i18n::top_bar::NEW_TAB),
            cx,
        );

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
        // The terminal sidebar is stage 2b: shown disabled on session tabs (D30).
        let terminal_sidebar = matches!(self.top_tab, TopTab::Session(_)).then(|| {
            title_bar::disabled(
                title_bar::icon_button("terminal-sidebar", IconName::PanelRight, cx)
                    .tooltip(i18n::top_bar::TERMINAL_SIDEBAR),
                cx,
            )
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
                        .child(
                            h_flex()
                                .items_center()
                                .child(vaults)
                                .child(div().occlude().child(vault_switcher)),
                        )
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
        let content = if self.section.is_built() {
            self.hosts.clone().into_any_element()
        } else {
            placeholder::placeholder("section-placeholder", self.section.label(), cx)
                .into_any_element()
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

    /// A session tab: its pane (SSH and local terminals, UI 4.1) or the "In
    /// development" placeholder (the tabs of later stages).
    fn render_session(&self, id: SessionId, cx: &mut Context<Self>) -> impl IntoElement {
        let content = match self.pane(id) {
            Some(pane) => pane.clone().into_any_element(),
            None => {
                let title = self
                    .sessions
                    .get(id)
                    .map(|tab| tab.title().clone())
                    .unwrap_or_default();
                placeholder::placeholder("session-placeholder", title, cx).into_any_element()
            }
        };
        div()
            .id("session-area")
            .test_support()
            .relative()
            .size_full()
            .bg(cx.theme().colors.list)
            .child(content)
            .child(self.render_toast_host())
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

/// The exit confirmation (UI 1.4, 4.6): `quit` true quits the app, false
/// closes the window.
fn confirm_exit(sessions: usize, quit: bool, window: &mut Window, cx: &mut App) {
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
                // Pending panel edits are saved before the window goes; its
                // sessions (every window's on Exit) end.
                if let Some(shell) = shell_for(window, cx) {
                    shell.update(cx, |shell, cx| shell.flush_panel(cx));
                }
                let ending = if quit {
                    all_shells(cx)
                } else {
                    shell_for(window, cx).into_iter().collect()
                };
                for shell in ending {
                    shell.update(cx, |shell, cx| shell.shutdown_sessions(cx));
                }
                if quit {
                    cx.quit();
                } else {
                    window.remove_window();
                }
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
        // New session tabs start now: their terminal needs the window. A
        // tab in the background connects all the same.
        for pane in std::mem::take(&mut self.unstarted) {
            window.defer(cx, move |window, cx| {
                pane.update(cx, |pane, cx| pane.start(window, cx));
            });
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
                            self.panel.as_ref().map(|_| self.details_width),
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
                .child(placeholder::placeholder(
                    "sftp-placeholder",
                    i18n::top_bar::SFTP,
                    cx,
                ))
                .child(self.render_toast_host())
                .into_any_element(),
            TopTab::Session(id) => self.render_session(id, cx).into_any_element(),
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
            // The new tab page and the terminal sidebar are stage 2b: their
            // entry points are disabled and the actions do nothing (D30).
            .on_action(cx.listener(|_, _: &NewTab, _, _| {}))
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
            .on_action(cx.listener(|this, _: &NewWindow, _, cx| {
                this.placeholder_toast(i18n::menu::NEW_WINDOW, cx)
            }))
            .on_action(cx.listener(|this, _: &NextTab, _, cx| this.cycle_tab(1, cx)))
            .on_action(cx.listener(|this, _: &PreviousTab, _, cx| this.cycle_tab(-1, cx)))
            // The focus goes to the next active tab (`close_session`).
            .on_action(cx.listener(|this, _: &CloseTab, _, cx| {
                if let TopTab::Session(id) = this.top_tab {
                    this.close_session(id, cx);
                }
            }))
            .on_action(cx.listener(|_, _: &OpenDocumentation, _, cx| {
                cx.open_url(&format!("{}#readme", env!("CARGO_PKG_REPOSITORY")));
            }))
            .on_action(cx.listener(|_, _: &ReportIssue, _, cx| {
                cx.open_url(&format!("{}/issues", env!("CARGO_PKG_REPOSITORY")));
            }))
            .on_action(cx.listener(|this, _: &CheckForUpdates, _, cx| {
                this.placeholder_toast(i18n::menu::CHECK_FOR_UPDATES, cx)
            }))
            .on_action(
                cx.listener(|this, _: &About, _, cx| this.toast(i18n::menu::about_text(), cx)),
            )
            .on_action(cx.listener(|this, _: &LockApp, _, cx| this.lock_app(cx)))
            .on_action(cx.listener(|this, _: &Exit, window, cx| this.request_exit(window, cx)))
            .on_action(cx.listener(|_, _: &ToggleTerminalSidebar, _, _| {}))
            .child(self.render_title_bar(window, cx))
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests;
