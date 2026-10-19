//! The shell of an unlocked window: the top bar, the Vaults tab (sidebar,
//! main area with the toasts at its bottom, right panel) and the SFTP tab
//! (placeholder in this stage).
//!
//! Skeleton status: working minimal versions; the shell work fills in the
//! rest of UI 1.1-1.5 (see the TODOs in the submodules).

pub mod placeholder;
pub mod sidebar;
pub mod title_bar;
pub mod toasts;

use gpui::{
    App, AppContext as _, ClickEvent, Context, Entity, FocusHandle, IntoElement, KeyBinding,
    ParentElement as _, Render, SharedString, Styled as _, Subscription, Window, div, prelude::*,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName as Lucide;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::menu::{DropdownMenu as _, PopupMenuItem};
use gpui_kit::component::sidebar::{Sidebar, SidebarMenu, SidebarMenuItem};
use gpui_kit::component::{ActiveTheme as _, Icon, IconName, Sizable as _, h_flex, v_flex};
use uuid::Uuid;

use crate::hosts::{HostsEvent, HostsScreen};
use crate::i18n;
use crate::panels::{GroupPanel, HostPanel, PanelEvent};
use crate::state::AppState;
use crate::theme::metrics;

pub use sidebar::Section;
pub use toasts::Toasts;

gpui_kit::actions!(shell, [ShowVaults, ShowSftp]);

/// Key context of the shell.
pub const KEY_CONTEXT: &str = "Shell";

/// Shortcuts of UI 1.4 handled by the shell.
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new("secondary-1", ShowVaults, Some(KEY_CONTEXT)),
        KeyBinding::new("secondary-2", ShowSftp, Some(KEY_CONTEXT)),
    ]);
}

/// The persistent tabs of the top bar. Session tabs arrive with terminals.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum TopTab {
    #[default]
    Vaults,
    Sftp,
}

/// The right panel (UI 1.3).
#[derive(Clone, Debug)]
pub enum Panel {
    Host(Entity<HostPanel>),
    Group(Entity<GroupPanel>),
}

pub struct Shell {
    state: Entity<AppState>,
    top_tab: TopTab,
    section: Section,
    hosts: Entity<HostsScreen>,
    panel: Option<Panel>,
    toasts: Entity<Toasts>,
    focus: FocusHandle,
    _subscriptions: Vec<Subscription>,
}

impl std::fmt::Debug for Shell {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shell").finish_non_exhaustive()
    }
}

impl Shell {
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let hosts = cx.new(|cx| HostsScreen::new(state.clone(), window, cx));
        let toasts = cx.new(|_| Toasts::new());
        let subscription = cx.subscribe_in(&hosts, window, Self::on_hosts_event);
        Self {
            state,
            top_tab: TopTab::Vaults,
            section: Section::Hosts,
            hosts,
            panel: None,
            toasts,
            focus: cx.focus_handle(),
            _subscriptions: vec![subscription],
        }
    }

    pub fn top_tab(&self) -> TopTab {
        self.top_tab
    }

    pub fn section(&self) -> Section {
        self.section
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

    pub fn set_top_tab(&mut self, tab: TopTab, cx: &mut Context<Self>) {
        self.top_tab = tab;
        cx.notify();
    }

    pub fn set_section(&mut self, section: Section, cx: &mut Context<Self>) {
        self.top_tab = TopTab::Vaults;
        self.section = section;
        if section != Section::Hosts {
            self.panel = None;
        }
        cx.notify();
    }

    /// Shows a toast at the bottom of the main area.
    pub fn toast(&mut self, message: impl Into<SharedString>, cx: &mut Context<Self>) {
        let message = message.into();
        self.toasts.update(cx, |toasts, cx| {
            toasts.push(message, cx);
        });
    }

    /// Opens the host panel for `record_id`, or an empty one for a new host
    /// (focus in Address, UI 3).
    pub fn open_host_panel(
        &mut self,
        record_id: Option<Uuid>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let (Some(Panel::Host(panel)), Some(id)) = (&self.panel, record_id)
            && panel.read(cx).record_id() == Some(id)
        {
            return;
        }
        self.flush_panel(cx);
        let state = self.state.clone();
        let panel = cx.new(|cx| HostPanel::new(state, record_id, window, cx));
        let subscription = cx.subscribe_in(&panel, window, Self::on_panel_event);
        self._subscriptions.push(subscription);
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
        self.flush_panel(cx);
        let state = self.state.clone();
        let panel = cx.new(|cx| GroupPanel::new(state, record_id, window, cx));
        let subscription = cx.subscribe_in(&panel, window, Self::on_panel_event);
        self._subscriptions.push(subscription);
        self.panel = Some(Panel::Group(panel));
        cx.notify();
    }

    /// Closes the right panel; pending edits are saved first.
    pub fn close_panel(&mut self, cx: &mut Context<Self>) {
        self.flush_panel(cx);
        self.panel = None;
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

    fn render_title_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.state.clone();
        let menu = Button::new("app-menu")
            .ghost()
            .small()
            .icon(IconName::Menu)
            .dropdown_menu(move |menu, _, _| {
                let lock_state = state.clone();
                menu.item(
                    PopupMenuItem::new(i18n::top_bar::MENU_LOCK).on_click(move |_, _, cx| {
                        let _ = lock_state.update(cx, |state, cx| state.lock(cx));
                    }),
                )
                .separator()
                .item(PopupMenuItem::new(i18n::top_bar::MENU_EXIT).on_click(|_, _, cx| cx.quit()))
            });
        let vaults = title_bar::top_tab(
            "tab-vaults",
            Lucide::Vault,
            i18n::top_bar::VAULTS,
            self.top_tab == TopTab::Vaults,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_top_tab(TopTab::Vaults, cx)));
        let sftp = title_bar::top_tab(
            "tab-sftp",
            Lucide::FolderOpen,
            i18n::top_bar::SFTP,
            self.top_tab == TopTab::Sftp,
            cx,
        )
        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.set_top_tab(TopTab::Sftp, cx)));
        let new_tab = title_bar::icon_button("tab-new", Lucide::Plus, cx).on_click(
            cx.listener(|this, _: &ClickEvent, _, cx| this.toast(i18n::placeholder::TITLE, cx)),
        );
        let notifications = title_bar::notifications_button(cx);

        title_bar::bare_title_bar(cx).child(
            h_flex()
                .h_full()
                .flex_1()
                .justify_between()
                .child(
                    h_flex()
                        .h_full()
                        .gap(metrics::SPACE_1)
                        // Windows: interactive children must occlude the drag area.
                        .child(div().occlude().child(menu))
                        .child(div().occlude().child(vaults))
                        .child(div().occlude().child(sftp))
                        .child(div().occlude().child(new_tab)),
                )
                .child(
                    h_flex()
                        .h_full()
                        .pr(metrics::SPACE_2)
                        .child(div().occlude().child(notifications)),
                ),
        )
    }

    fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let items = Section::ALL.into_iter().map(|section| {
            SidebarMenuItem::new(section.label())
                .icon(Icon::new(section.icon()))
                .active(section == self.section)
                .on_click(
                    cx.listener(move |this, _: &ClickEvent, _, cx| this.set_section(section, cx)),
                )
        });
        Sidebar::new("sidebar")
            .w(metrics::WIDTH_SIDEBAR)
            .collapsible(false)
            .child(SidebarMenu::new().children(items))
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
            // Toasts at the bottom of the main area: never over the panel.
            .child(
                div()
                    .absolute()
                    .left_0()
                    .right_0()
                    .bottom(metrics::SPACE_5)
                    .flex()
                    .justify_center()
                    .child(self.toasts.clone()),
            )
    }

    fn render_panel(&self, cx: &App) -> Option<impl IntoElement> {
        let panel = match self.panel.as_ref()? {
            Panel::Host(panel) => panel.clone().into_any_element(),
            Panel::Group(panel) => panel.clone().into_any_element(),
        };
        Some(
            div()
                .id("right-panel")
                .test_support()
                .w(metrics::WIDTH_DETAILS)
                .flex_none()
                .h_full()
                .border_l_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().sidebar)
                .child(panel),
        )
    }
}

impl Render for Shell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match self.top_tab {
            TopTab::Vaults => h_flex()
                .size_full()
                .child(self.render_sidebar(cx))
                .child(self.render_main(cx))
                .children(self.render_panel(cx))
                .into_any_element(),
            TopTab::Sftp => div()
                .size_full()
                .bg(cx.theme().colors.list)
                .child(placeholder::placeholder(
                    "sftp-placeholder",
                    i18n::top_bar::SFTP,
                    cx,
                ))
                .into_any_element(),
        };
        v_flex()
            .id("shell")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .on_action(
                cx.listener(|this, _: &ShowVaults, _, cx| this.set_top_tab(TopTab::Vaults, cx)),
            )
            .on_action(cx.listener(|this, _: &ShowSftp, _, cx| this.set_top_tab(TopTab::Sftp, cx)))
            .child(self.render_title_bar(cx))
            .child(div().flex_1().min_h_0().child(body))
    }
}
