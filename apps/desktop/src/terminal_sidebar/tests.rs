//! Headless tests of the terminal sidebar (UI 4.2): a temporary profile,
//! the in-memory secret store, a terminal fed with bytes (no shell, no
//! network) and the sidebar next to it.

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, Context, Entity, Focusable as _, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, TestAppContext, Window, WindowBounds,
    WindowOptions, div, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::*;
use crate::settings::terminal::TerminalPrefs;
use crate::state::AppState;
use crate::terminal::{SearchSummary, TerminalOptions, TerminalView};
use crate::testing::TestProfile;
use crate::theme::{self, Appearance};

/// A window with a terminal and the sidebar on its right.
struct Root {
    terminal: Entity<TerminalView>,
    other: Entity<TerminalView>,
    sidebar: Entity<TerminalSidebar>,
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .key_context(crate::shell::actions::KEY_CONTEXT)
            .flex()
            .size_full()
            .child(div().flex_1().h_full().child(self.terminal.clone()))
            .child(self.sidebar.clone())
    }
}

struct Ui {
    state: Entity<AppState>,
    window: AnyWindowHandle,
    root: Entity<Root>,
}

impl Ui {
    fn open(cx: &mut TestAppContext) -> Self {
        cx.executor().allow_parking();
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::install(Appearance::Dark, cx);
            crate::terminal::init(cx);
            crate::shell::actions::bind_keys(cx);
            bind_keys(cx);
        });
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
        cx.update(|cx| {
            let core = state.read(cx).core().cloned().expect("core");
            TerminalPrefs::refresh(&core, cx);
        });
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(1100.), px(700.)),
            ))),
            ..Default::default()
        };
        let sidebar_state = state.clone();
        let (window, root) = cx.update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Root {
                    terminal: cx
                        .new(|cx| TerminalView::new(TerminalOptions::default(), window, cx)),
                    other: cx.new(|cx| TerminalView::new(TerminalOptions::default(), window, cx)),
                    sidebar: cx.new(|cx| TerminalSidebar::new(sidebar_state, window, cx)),
                })
            })
            .expect("test window")
        });
        let ui = Self {
            state,
            window,
            root,
        };
        ui.settle(cx);
        ui
    }

    fn settle(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(self.window, |_, window, cx| window.render_frame(cx))
            .unwrap();
    }

    fn terminal(&self, cx: &mut TestAppContext) -> Entity<TerminalView> {
        cx.update(|cx| self.root.read(cx).terminal.clone())
    }

    fn other(&self, cx: &mut TestAppContext) -> Entity<TerminalView> {
        cx.update(|cx| self.root.read(cx).other.clone())
    }

    fn sidebar(&self, cx: &mut TestAppContext) -> Entity<TerminalSidebar> {
        cx.update(|cx| self.root.read(cx).sidebar.clone())
    }

    /// Output of the terminal, parsed.
    fn feed(&self, terminal: &Entity<TerminalView>, bytes: &[u8], cx: &mut TestAppContext) {
        let handle = cx.update(|cx| terminal.read(cx).handle());
        handle.feed_blocking(bytes).unwrap();
        handle.flush_blocking();
        self.settle(cx);
    }

    fn show(&self, target: Option<SidebarTarget>, cx: &mut TestAppContext) {
        let sidebar = self.sidebar(cx);
        cx.update_window(self.window, |_, window, cx| {
            sidebar.update(cx, |sidebar, cx| {
                sidebar.set_target(target, window, cx);
                sidebar.set_visible(true, cx);
            })
        })
        .unwrap();
        self.settle(cx);
    }

    fn visible(&self, id: &str, cx: &mut TestAppContext) -> bool {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.window, move |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id.clone()).is_some_and(|el| el.visible())
        })
        .unwrap()
    }

    fn click(&self, id: &str, cx: &mut TestAppContext) {
        let id = SharedString::from(id.to_owned());
        cx.update_window(self.window, move |_, window, cx| {
            window.render_frame(cx);
            window.click(id.clone(), cx);
        })
        .unwrap();
        self.settle(cx);
    }

    fn press(&self, key: &str, cx: &mut TestAppContext) {
        let key = key.to_owned();
        cx.update_window(self.window, move |_, window, cx| window.press(&key, cx))
            .unwrap();
        self.settle(cx);
    }

    fn type_text(&self, text: &str, cx: &mut TestAppContext) {
        let text = text.to_owned();
        cx.update_window(self.window, move |_, window, cx| window.input(&text, cx))
            .unwrap();
        self.settle(cx);
    }

    fn summary(&self, cx: &mut TestAppContext) -> Option<SearchSummary> {
        let sidebar = self.sidebar(cx);
        cx.update(|cx| sidebar.read(cx).tools().read(cx).summary())
    }

    /// A saved host in the personal vault.
    fn saved_host(&self, cx: &mut TestAppContext) -> Uuid {
        let data: Map<String, Value> = json!({"address": "192.0.2.10", "label": "Lab"})
            .as_object()
            .cloned()
            .unwrap();
        self.state
            .update(cx, |state, cx| state.save_host(None, data, cx))
            .expect("host saved")
            .record_id
    }

    fn finish(self, cx: &mut TestAppContext) {
        self.state.update(cx, |state, _| state.shutdown());
    }
}

/// The four icon tabs switch the content; Snippets shows the stage 5 note;
/// without a session the search field is disabled with a reason.
#[gpui::test]
fn tabs_and_the_snippets_placeholder(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    ui.show(None, cx);
    for tab in SidebarTab::ALL {
        assert!(ui.visible(tab.element_id(), cx), "{tab:?}");
    }
    assert!(ui.visible("terminal-sidebar-tools", cx), "Tools first");
    assert!(ui.visible("terminal-search", cx));
    let sidebar = ui.sidebar(cx);
    let tools = cx.update(|cx| sidebar.read(cx).tools().clone());
    assert_eq!(cx.update(|cx| tools.read(cx).summary()), None);

    ui.click("terminal-sidebar-tab-snippets", cx);
    assert!(ui.visible("terminal-sidebar-snippets-empty", cx));
    assert_eq!(
        strings::SNIPPETS_LATER,
        "Snippets will be available in stage 5."
    );
    ui.click("terminal-sidebar-tab-history", cx);
    assert!(ui.visible("terminal-sidebar-history", cx));
    ui.click("terminal-sidebar-tab-appearance", cx);
    assert!(ui.visible("terminal-sidebar-appearance", cx));
    assert_eq!(
        cx.update(|cx| sidebar.read(cx).tab()),
        SidebarTab::Appearance
    );
    // No autocomplete in Tools (later stage).
    ui.click("terminal-sidebar-tab-tools", cx);
    assert!(!ui.visible("terminal-autocomplete", cx));
    ui.finish(cx);
}

/// UI 4.2 Tools: typing searches the output and the scrollback, Enter and
/// Shift+Enter step with wrap-around, the counter reads "N of M"; hiding
/// the sidebar removes the highlights; another pane takes the search over.
#[gpui::test]
fn search_from_the_tools_tab(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let terminal = ui.terminal(cx);
    let mut text = String::new();
    for line in 0..120 {
        if line % 40 == 1 {
            text.push_str(&format!("{line}: Warning disk\r\n"));
        } else {
            text.push_str(&format!("{line}: ok\r\n"));
        }
    }
    ui.feed(&terminal, text.as_bytes(), cx);
    ui.show(Some(SidebarTarget::new(&terminal)), cx);

    // Ctrl+Shift+F would call this: the field gets the focus.
    let sidebar = ui.sidebar(cx);
    cx.update_window(ui.window, |_, window, cx| {
        sidebar.update(cx, |sidebar, cx| sidebar.find_in_terminal(window, cx))
    })
    .unwrap();
    ui.settle(cx);
    ui.type_text("warning", cx);
    assert_eq!(
        ui.summary(cx),
        Some(SearchSummary {
            count: 3,
            current: Some(3),
            truncated: false
        })
    );
    assert!(ui.visible("terminal-search-count", cx));
    let highlighted = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            terminal
                .read(cx)
                .search_state()
                .map(|search| search.count())
        })
    };
    assert_eq!(highlighted(cx), Some(3));

    // Enter: the next match wraps to the first; Shift+Enter goes back.
    ui.press("enter", cx);
    assert_eq!(ui.summary(cx).and_then(|summary| summary.current), Some(1));
    ui.press("shift-enter", cx);
    assert_eq!(ui.summary(cx).and_then(|summary| summary.current), Some(3));
    ui.click("terminal-search-previous", cx);
    assert_eq!(ui.summary(cx).and_then(|summary| summary.current), Some(2));
    ui.click("terminal-search-next", cx);
    assert_eq!(ui.summary(cx).and_then(|summary| summary.current), Some(3));

    // No match.
    ui.click("terminal-search", cx);
    ui.press("ctrl-a", cx);
    ui.type_text("absent", cx);
    assert_eq!(ui.summary(cx).map(|summary| summary.count), Some(0));
    ui.press("ctrl-a", cx);
    ui.type_text("disk", cx);
    assert_eq!(ui.summary(cx).map(|summary| summary.count), Some(3));

    // Hidden sidebar: no highlights; shown again: they come back.
    sidebar.update(cx, |sidebar, cx| sidebar.set_visible(false, cx));
    ui.settle(cx);
    assert_eq!(highlighted(cx), None);
    sidebar.update(cx, |sidebar, cx| sidebar.set_visible(true, cx));
    ui.settle(cx);
    assert_eq!(highlighted(cx), Some(3));

    // Another pane becomes active: the search moves there.
    let other = ui.other(cx);
    ui.feed(&other, b"disk one\r\nnothing\r\n", cx);
    ui.show(Some(SidebarTarget::new(&other)), cx);
    assert_eq!(highlighted(cx), None, "the old pane lost its highlights");
    assert_eq!(
        cx.update(|cx| other.read(cx).search_summary().map(|summary| summary.count)),
        Some(1)
    );
    assert_eq!(ui.summary(cx).map(|summary| summary.count), Some(1));

    // Esc returns the focus to the terminal.
    ui.click("terminal-search", cx);
    ui.press("escape", cx);
    let focused = cx
        .update_window(ui.window, |_, window, cx| {
            other.read(cx).focus_handle(cx).is_focused(window)
        })
        .unwrap();
    assert!(focused, "Esc focuses the terminal");
    ui.finish(cx);
}

/// UI 4.2 Appearance: a theme click applies at once and is saved for a
/// saved host; quick connect and local terminals keep it for the session.
#[gpui::test]
fn appearance_applies_and_saves_a_theme(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let host_id = ui.saved_host(cx);
    let terminal = ui.terminal(cx);
    ui.show(
        Some(
            SidebarTarget::new(&terminal)
                .host(Some(host_id))
                .session(Some(1)),
        ),
        cx,
    );
    ui.click("terminal-sidebar-tab-appearance", cx);
    let name = |terminal: &Entity<TerminalView>, cx: &mut TestAppContext| {
        cx.update(|cx| terminal.read(cx).theme().name.clone())
    };
    assert_eq!(name(&terminal, cx), "Graphite");
    let sidebar = ui.sidebar(cx);
    let current = |cx: &mut TestAppContext| {
        cx.update(|cx| {
            sidebar
                .read(cx)
                .appearance()
                .read(cx)
                .current_theme(cx)
                .map(|slug| slug.to_string())
        })
    };
    assert_eq!(current(cx).as_deref(), Some("graphite"));

    ui.click("terminal-theme-paper", cx);
    assert_eq!(name(&terminal, cx), "Paper", "applied at once");
    assert_eq!(current(cx).as_deref(), Some("paper"));
    let saved = cx.update(|cx| {
        ui.state
            .read(cx)
            .host(host_id)
            .map(|host| host.data["terminal_theme"].clone())
    });
    assert_eq!(saved, Some(json!({"builtin": "paper"})));

    // A local terminal (no host): the session only.
    let other = ui.other(cx);
    ui.show(Some(SidebarTarget::new(&other)), cx);
    ui.click("terminal-theme-paper", cx);
    assert_eq!(name(&other, cx), "Paper");
    let hosts = cx.update(|cx| ui.state.read(cx).hosts().len());
    assert_eq!(hosts, 1, "nothing else was saved");
    // Back to the default theme: the session follows the default again.
    ui.click("terminal-theme-graphite", cx);
    assert_eq!(name(&other, cx), "Graphite");
    ui.finish(cx);
}

/// UI 4.2 Appearance > Font: size and line spacing are the global
/// settings; every terminal follows at once.
#[gpui::test]
fn the_font_page_changes_size_and_spacing(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let terminal = ui.terminal(cx);
    ui.show(Some(SidebarTarget::new(&terminal)), cx);
    ui.click("terminal-sidebar-tab-appearance", cx);
    ui.click("terminal-appearance-font", cx);
    assert!(ui.visible("terminal-sidebar-font", cx));
    assert!(ui.visible("terminal-font-JetBrains Mono", cx));
    let stored = |key: &str, cx: &mut TestAppContext| {
        cx.update(|cx| ui.state.read(cx).core().unwrap().settings_get().unwrap()[key].clone())
    };
    ui.click("terminal-font-size-plus", cx);
    assert_eq!(stored("terminal.font_size", cx), json!(15));
    ui.click("terminal-line-height-plus", cx);
    assert_eq!(stored("terminal.line_height", cx), json!(1.3));
    let settings = cx.update(|cx| terminal.read(cx).settings().clone());
    assert_eq!(settings.font_size, 15.);
    assert!((settings.line_height - 1.3).abs() < 0.001);
    // A typed value out of range is refused.
    ui.click("terminal-line-height", cx);
    ui.press("ctrl-a", cx);
    ui.type_text("3", cx);
    assert!(ui.visible("terminal-line-height-error", cx));
    assert_eq!(stored("terminal.line_height", cx), json!(1.3));
    ui.press("ctrl-a", cx);
    ui.type_text("1.5", cx);
    assert_eq!(stored("terminal.line_height", cx), json!(1.5));
    // Esc goes back to the theme list.
    ui.press("escape", cx);
    assert!(ui.visible("terminal-sidebar-appearance", cx));
    ui.finish(cx);
}
