//! The terminal sidebar (UI 4.2; lead decision L6): one per window, on the
//! right of the session area; the terminal narrows instead of being
//! covered. The top bar button toggles it while a session tab is active
//! (UI 1.1; [`ToggleTerminalSidebar`]), and Ctrl+Shift+F opens it on Tools
//! with the search field focused ([`FindInTerminal`]). It acts on the
//! session of the active pane ([`SidebarTarget`], set by the window shell
//! whenever the active tab or pane changes).
//!
//! Layout [decision]: 320 px wide (`--width-details-min`), `--bg-surface`
//! with a 1 px `--border-subtle` edge on the left like the details panel;
//! a row of four icon tabs at the top (36 px squares; the selected one has
//! the `--accent-subtle` fill and `--text-primary` icon, the others
//! `--text-secondary`; the name is in the tooltip); below it the tab's
//! content, which scrolls on its own.
//!
//! - Tools ([`tools`]): search in the output and the scrollback.
//!   Autocomplete is not shown (later stage).
//! - Snippets: the vault's snippets, inserted or run in the active pane
//!   ([`snippets`]).
//! - History ([`history`]): the command history of the host (the history
//!   package owns that view; this container only hosts it).
//! - Appearance ([`appearance`]): the Font page (family, size, line
//!   spacing; global settings) and the theme list (applies to the session,
//!   saved for a saved host).
//!
//! The window shell embeds the sidebar ([`TerminalSidebar`]), binds
//! Ctrl+Shift+F ([`bind_keys`]) and calls [`TerminalSidebar::set_target`],
//! [`TerminalSidebar::set_visible`] and [`TerminalSidebar::find_in_terminal`].

pub mod appearance;
pub mod history;
pub mod snippets;
pub mod strings;
pub mod tools;

#[cfg(test)]
mod tests;

use gpui::prelude::*;
use gpui::{
    App, ClickEvent, Context, Entity, EntityId, FocusHandle, IntoElement, KeyBinding, Render,
    SharedString, WeakEntity, Window, div,
};
use gpui_kit::TestSupportExt as _;
use gpui_kit::assets::IconName;
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::component::{ActiveTheme as _, Icon, h_flex, v_flex};
use uuid::Uuid;

use crate::state::AppState;
use crate::terminal::TerminalView;
use crate::theme::focus::FocusRingExt as _;
use crate::theme::{self, metrics};

/// The top bar button (UI 1.1, 4.2); the window shell handles it.
pub use crate::shell::actions::ToggleTerminalSidebar;

gpui_kit::actions!(
    terminal_sidebar,
    [
        /// Ctrl+Shift+F (UI 12.5): the sidebar opens on Tools with the
        /// search field focused. The window shell handles it.
        FindInTerminal,
    ]
);

/// Width of the sidebar [decision]: `--width-details-min`.
pub const WIDTH: gpui::Rems = metrics::WIDTH_DETAILS_MIN;
/// The key of [`FindInTerminal`] (UI 12.5; `secondary` is Cmd on macOS).
pub const FIND_KEYS: &str = "secondary-shift-f";
/// Key context of the sidebar.
pub const KEY_CONTEXT: &str = "TerminalSidebar";

/// Binds Ctrl+Shift+F to [`FindInTerminal`] in the shell's key context: it
/// works in the terminal too (the terminal passes Ctrl+Shift shortcuts on,
/// UI 12.5).
pub fn bind_keys(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        FIND_KEYS,
        FindInTerminal,
        Some(crate::shell::actions::KEY_CONTEXT),
    )]);
}

/// The tabs of the sidebar, in their order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SidebarTab {
    #[default]
    Tools,
    Snippets,
    History,
    Appearance,
}

impl SidebarTab {
    pub const ALL: [SidebarTab; 4] = [
        SidebarTab::Tools,
        SidebarTab::Snippets,
        SidebarTab::History,
        SidebarTab::Appearance,
    ];

    pub fn label(self) -> &'static str {
        match self {
            SidebarTab::Tools => strings::TOOLS,
            SidebarTab::Snippets => strings::SNIPPETS,
            SidebarTab::History => strings::HISTORY,
            SidebarTab::Appearance => strings::APPEARANCE,
        }
    }

    pub fn icon(self) -> IconName {
        match self {
            SidebarTab::Tools => IconName::Wrench,
            SidebarTab::Snippets => IconName::Braces,
            SidebarTab::History => IconName::RotateCcwClock,
            SidebarTab::Appearance => IconName::Palette,
        }
    }

    /// Stable element id of the tab button (tests).
    pub fn element_id(self) -> &'static str {
        match self {
            SidebarTab::Tools => "terminal-sidebar-tab-tools",
            SidebarTab::Snippets => "terminal-sidebar-tab-snippets",
            SidebarTab::History => "terminal-sidebar-tab-history",
            SidebarTab::Appearance => "terminal-sidebar-tab-appearance",
        }
    }
}

/// What the sidebar acts on: the terminal of the active pane, the saved
/// host of its session (`None` for quick connect and local terminals) and
/// the session's id.
#[derive(Clone)]
pub struct SidebarTarget {
    terminal: WeakEntity<TerminalView>,
    host_id: Option<Uuid>,
    session_id: Option<u64>,
}

impl std::fmt::Debug for SidebarTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SidebarTarget")
            .field("terminal", &self.terminal.entity_id())
            .field("saved_host", &self.host_id.is_some())
            .field("session_id", &self.session_id)
            .finish()
    }
}

impl PartialEq for SidebarTarget {
    fn eq(&self, other: &Self) -> bool {
        self.terminal.entity_id() == other.terminal.entity_id()
            && self.host_id == other.host_id
            && self.session_id == other.session_id
    }
}

impl SidebarTarget {
    pub fn new(terminal: &Entity<TerminalView>) -> Self {
        Self {
            terminal: terminal.downgrade(),
            host_id: None,
            session_id: None,
        }
    }

    /// The saved host of the session (history, saving the theme).
    pub fn host(mut self, host_id: Option<Uuid>) -> Self {
        self.host_id = host_id;
        self
    }

    /// The session's id.
    pub fn session(mut self, session_id: Option<u64>) -> Self {
        self.session_id = session_id;
        self
    }

    /// The terminal, while its pane is open.
    pub fn terminal(&self) -> Option<Entity<TerminalView>> {
        self.terminal.upgrade()
    }

    pub fn terminal_id(&self) -> EntityId {
        self.terminal.entity_id()
    }

    pub fn host_id(&self) -> Option<Uuid> {
        self.host_id
    }

    pub fn session_id(&self) -> Option<u64> {
        self.session_id
    }
}

/// The terminal sidebar of one window.
pub struct TerminalSidebar {
    tab: SidebarTab,
    target: Option<SidebarTarget>,
    visible: bool,
    tools: Entity<tools::ToolsTab>,
    appearance: Entity<appearance::AppearanceTab>,
    history: Entity<history::HistoryTab>,
    snippets: Entity<snippets::SnippetsTab>,
    /// Focus handles of the tab buttons, in [`SidebarTab::ALL`] order.
    tab_focus: [FocusHandle; 4],
}

impl std::fmt::Debug for TerminalSidebar {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TerminalSidebar")
            .field("tab", &self.tab)
            .field("visible", &self.visible)
            .field("target", &self.target)
            .finish_non_exhaustive()
    }
}

impl TerminalSidebar {
    /// A hidden sidebar on Tools without a target.
    pub fn new(state: Entity<AppState>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tools = cx.new(|cx| tools::ToolsTab::new(window, cx));
        let appearance = cx.new(|cx| appearance::AppearanceTab::new(state.clone(), window, cx));
        let snippets = cx.new(|cx| snippets::SnippetsTab::new(state.clone(), window, cx));
        let history = cx.new(|cx| history::HistoryTab::new(state, window, cx));
        Self {
            tab: SidebarTab::default(),
            target: None,
            visible: false,
            tools,
            appearance,
            history,
            snippets,
            tab_focus: std::array::from_fn(|_| cx.focus_handle()),
        }
    }

    /// The tab on display.
    pub fn tab(&self) -> SidebarTab {
        self.tab
    }

    /// Shows `tab`.
    pub fn show_tab(&mut self, tab: SidebarTab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            cx.notify();
        }
    }

    /// What the sidebar acts on.
    pub fn target(&self) -> Option<&SidebarTarget> {
        self.target.as_ref()
    }

    /// The active pane changed (another tab, another pane, a pane that
    /// shows the host picker: `None`). The search moves to the new
    /// terminal; the tabs show the new host.
    pub fn set_target(
        &mut self,
        target: Option<SidebarTarget>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.target == target {
            return;
        }
        self.target = target.clone();
        let visible = self.visible;
        self.tools.update(cx, |tools, cx| {
            tools.set_target(target.clone(), visible, cx)
        });
        self.appearance.update(cx, |appearance, cx| {
            appearance.set_target(target.clone(), cx)
        });
        self.snippets
            .update(cx, |snippets, cx| snippets.set_target(target.clone(), cx));
        self.history
            .update(cx, |history, cx| history.set_target(target, window, cx));
        cx.notify();
    }

    /// Whether the window shows the sidebar.
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// The window shell shows or hides the sidebar. While it is hidden the
    /// terminal shows no search highlights; they come back with it.
    pub fn set_visible(&mut self, visible: bool, cx: &mut Context<Self>) {
        if self.visible == visible {
            return;
        }
        self.visible = visible;
        self.tools
            .update(cx, |tools, cx| tools.set_active(visible, cx));
        cx.notify();
    }

    /// Ctrl+Shift+F: Tools, the search field focused with its text
    /// selected. The caller shows the sidebar.
    pub fn find_in_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.set_visible(true, cx);
        self.show_tab(SidebarTab::Tools, cx);
        self.tools
            .update(cx, |tools, cx| tools.focus_search(window, cx));
    }

    pub fn tools(&self) -> &Entity<tools::ToolsTab> {
        &self.tools
    }

    pub fn appearance(&self) -> &Entity<appearance::AppearanceTab> {
        &self.appearance
    }

    pub fn snippets(&self) -> &Entity<snippets::SnippetsTab> {
        &self.snippets
    }

    pub fn history(&self) -> &Entity<history::HistoryTab> {
        &self.history
    }

    fn render_tabs(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let tokens = theme::tokens(cx);
        let surface = theme.colors.list;
        let hover = theme.secondary_hover;
        let pressed = theme.secondary_active;
        let buttons = SidebarTab::ALL
            .into_iter()
            .zip(&self.tab_focus)
            .map(|(tab, focus)| {
                let selected = tab == self.tab;
                let label = tab.label();
                let tooltip_id = SharedString::from(format!("{}-tooltip", tab.element_id()));
                div()
                    .id(tab.element_id())
                    .test_support()
                    .flex_none()
                    .size(metrics::HEIGHT_CONTROL)
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded(metrics::RADIUS_CONTROL)
                    .track_focus(focus)
                    .tab_index(0)
                    .focus_ring_visible(surface, cx)
                    .tooltip(move |window, cx| {
                        let id = tooltip_id.clone();
                        Tooltip::element(move |_, _| {
                            div().id(id.clone()).test_support().child(label)
                        })
                        .build(window, cx)
                    })
                    .map(|this| {
                        if selected {
                            this.bg(tokens.accent_subtle.hsla())
                                .text_color(theme.foreground)
                        } else {
                            this.text_color(theme.muted_foreground)
                                .hover(move |style| style.bg(hover))
                                .active(move |style| style.bg(pressed))
                        }
                    })
                    .child(Icon::new(tab.icon()).size(gpui::px(16.)))
                    .on_click(
                        cx.listener(move |this, _: &ClickEvent, _, cx| this.show_tab(tab, cx)),
                    )
            });
        h_flex()
            .id("terminal-sidebar-tabs")
            .test_support()
            .flex_none()
            .h(metrics::HEIGHT_CONTROL + metrics::SPACE_4)
            .px(metrics::SPACE_3)
            .gap(metrics::SPACE_1)
            .items_center()
            .border_b_1()
            .border_color(theme.border)
            .children(buttons)
    }
}

/// The heading of a sidebar tab (16 px, 600).
pub(crate) fn heading(text: &'static str, cx: &App) -> gpui::Div {
    div()
        .text_size(metrics::FONT_SIZE_HEADING)
        .font_weight(metrics::FONT_WEIGHT_SEMIBOLD)
        .text_color(cx.theme().foreground)
        .child(text)
}

impl Render for TerminalSidebar {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = cx.theme();
        let content = match self.tab {
            SidebarTab::Tools => self.tools.clone().into_any_element(),
            SidebarTab::Snippets => self.snippets.clone().into_any_element(),
            SidebarTab::History => self.history.clone().into_any_element(),
            SidebarTab::Appearance => self.appearance.clone().into_any_element(),
        };
        v_flex()
            .id("terminal-sidebar")
            .test_support()
            .key_context(KEY_CONTEXT)
            .flex_none()
            .w(WIDTH)
            .h_full()
            .bg(theme.colors.list)
            .border_l_1()
            .border_color(theme.border)
            .child(self.render_tabs(cx))
            .child(
                div()
                    .id("terminal-sidebar-content")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(content),
            )
    }
}
