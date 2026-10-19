//! The shell of an unlocked window (UI 1): the top bar, then the content of
//! the active top-bar tab:
//!
//! - Vaults: the sidebar (UI 1.2), the main area with the toasts at its
//!   bottom center, and the details panel on the right (UI 1.3);
//! - SFTP and session tabs: the "In development" placeholder (the SFTP
//!   manager and terminals are later stages; audit decisions 1 and 6).
//!
//! Other modules talk to the shell through events (`HostsEvent`,
//! `HostsRequest`, `PanelEvent`, `PanelRequest`) or the window-level
//! functions [`show_toast`], [`notify`], [`in_development`] and
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
    SharedString, Styled as _, Subscription, WeakEntity, Window, WindowId, div, prelude::*, px,
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
    _subscriptions: Vec<Subscription>,
    /// Subscriptions to the open details panel (replaced with the panel).
    panel_subscriptions: Vec<Subscription>,
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

        // Closing the window while sessions are open asks first (UI 4.6).
        let this = cx.weak_entity();
        window.on_window_should_close(cx, move |window, cx| {
            let Some(shell) = this.upgrade() else {
                return true;
            };
            if shell.read(cx).sessions.is_empty() {
                return true;
            }
            let sessions = shell.read(cx).sessions.len();
            confirm_exit(sessions, false, window, cx);
            false
        });

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
            _subscriptions: subscriptions,
            panel_subscriptions: Vec::new(),
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
        self.top_tab = tab;
        cx.notify();
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

    /// Opens a session tab and makes it active. Until terminals exist the
    /// tab shows the "In development" placeholder.
    pub fn open_session(&mut self, request: SessionRequest, cx: &mut Context<Self>) -> SessionId {
        let id = self.sessions.open(request);
        self.session_focus
            .push((id, cx.focus_handle().tab_stop(true)));
        self.top_tab = TopTab::Session(id);
        cx.notify();
        id
    }

    /// Closes a session tab; when it was active, the neighbor of UI 4.6
    /// becomes active (left, else right, else Vaults).
    pub fn close_session(&mut self, id: SessionId, cx: &mut Context<Self>) {
        let was_active = self.top_tab == TopTab::Session(id);
        let next = self.sessions.close(id);
        self.session_focus.retain(|(session, _)| *session != id);
        if was_active {
            self.top_tab = next.map_or(TopTab::Vaults, TopTab::Session);
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

    /// Exit (UI 1.4): asks first while session tabs are open (UI 4.6).
    pub fn request_exit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.sessions.is_empty() {
            cx.quit();
        } else {
            confirm_exit(self.sessions.len(), true, window, cx);
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

    /// Closes the details panel; pending edits are saved first.
    pub fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.flush_panel(cx);
        self.panel = None;
        self.panel_subscriptions.clear();
        cx.notify();
    }

    fn flush_panel(&mut self, cx: &mut Context<Self>) {
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

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                    move |this, _: &ClickEvent, window, cx| {
                        cx.stop_propagation();
                        this.close_session(id, cx);
                        window.focus(&this.focus, cx);
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
                        this.set_top_tab(TopTab::Session(id), cx)
                    }))
                    .into_any_element()
            })
            .collect();
        let new_tab = title_bar::icon_button("tab-new", IconName::Plus, cx)
            .tooltip(i18n::top_bar::NEW_TAB)
            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                this.open_session(SessionRequest::new(SessionKind::NewTab), cx);
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
        let terminal_sidebar = matches!(self.top_tab, TopTab::Session(_)).then(|| {
            title_bar::icon_button("terminal-sidebar", IconName::PanelRight, cx)
                .tooltip(i18n::top_bar::TERMINAL_SIDEBAR)
                .on_click(cx.listener(|this, _: &ClickEvent, _, cx| {
                    this.placeholder_toast(i18n::top_bar::TERMINAL_SIDEBAR, cx)
                }))
        });

        title_bar::bare_title_bar(cx).child(
            h_flex()
                .id("top-bar")
                .test_support()
                .h_full()
                .flex_1()
                .min_w_0()
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
                            h_flex()
                                .id("session-tabs")
                                .test_support()
                                .min_w_0()
                                .gap(metrics::SPACE_1)
                                .items_center()
                                .overflow_hidden()
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

    fn render_panel(&self, cx: &mut Context<Self>) -> Option<impl IntoElement> {
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
        Some(
            div()
                .id("right-panel")
                .test_support()
                .relative()
                .w(self.details_width)
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

    fn render_session(&self, id: SessionId, cx: &mut Context<Self>) -> impl IntoElement {
        let title = self
            .sessions
            .get(id)
            .map(|tab| tab.title().clone())
            .unwrap_or_default();
        div()
            .id("session-area")
            .test_support()
            .relative()
            .size_full()
            .bg(cx.theme().colors.list)
            .child(placeholder::placeholder("session-placeholder", title, cx))
            .child(self.render_toast_host())
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
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.top_tab {
            TopTab::Vaults => h_flex()
                .size_full()
                .when(self.sidebar_visible, |body| {
                    body.child(sidebar::sidebar(
                        self.section,
                        &self.nav_focus,
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
                .children(self.render_panel(cx))
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
            .on_action(cx.listener(|this, _: &OpenSettings, _, cx| {
                this.placeholder_toast(i18n::menu::SETTINGS, cx)
            }))
            .on_action(cx.listener(|this, _: &NewLocalTerminal, _, cx| {
                this.open_session(SessionRequest::new(SessionKind::Local), cx);
            }))
            .on_action(cx.listener(|this, _: &NewSerialConnection, _, cx| {
                this.open_session(SessionRequest::new(SessionKind::Serial), cx);
            }))
            .on_action(cx.listener(|this, _: &NewTab, _, cx| {
                this.open_session(SessionRequest::new(SessionKind::NewTab), cx);
            }))
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
            .on_action(cx.listener(|this, _: &CloseTab, window, cx| {
                if let TopTab::Session(id) = this.top_tab {
                    this.close_session(id, cx);
                    window.focus(&this.focus, cx);
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
            .on_action(cx.listener(|this, _: &LockApp, _, cx| {
                if let Err(error) = this.state.update(cx, |state, cx| state.lock(cx)) {
                    tracing::warn!(code = error.code(), "lock failed");
                }
            }))
            .on_action(cx.listener(|this, _: &Exit, window, cx| this.request_exit(window, cx)))
            .on_action(cx.listener(|this, _: &ToggleTerminalSidebar, _, cx| {
                this.placeholder_toast(i18n::top_bar::TERMINAL_SIDEBAR, cx)
            }))
            .child(self.render_title_bar(cx))
            .child(div().flex_1().min_h_0().child(body))
    }
}

#[cfg(test)]
mod tests;
