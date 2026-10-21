//! Headless tests of the terminal sidebar's Snippets tab (UI 4.2; D66): a
//! temporary profile, a terminal fed with bytes (no shell, no network),
//! snippets saved through the core.

use std::sync::{Arc, Mutex};

use gpui::{
    AnyWindowHandle, Bounds, Context, Entity, IntoElement, Render, TestAppContext, Window,
    WindowBounds, WindowOptions, div, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Value, json};
use shellpp_core::service::snippets::RecordInput;

use super::*;
use crate::settings::terminal::TerminalPrefs;
use crate::terminal::{TerminalOptions, TerminalSink, TerminalSize};
use crate::testing::TestProfile;

/// What reached the session.
#[derive(Default)]
struct Sent(Mutex<Vec<u8>>);

impl TerminalSink for Sent {
    fn send(&self, bytes: Vec<u8>) {
        self.0.lock().unwrap().extend_from_slice(&bytes);
    }

    fn resize(&self, _size: TerminalSize) {}
}

struct Root {
    terminal: Entity<TerminalView>,
    tab: Entity<SnippetsTab>,
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .child(div().flex_1().h_full().child(self.terminal.clone()))
            .child(
                div()
                    .w(super::super::WIDTH)
                    .h_full()
                    .child(self.tab.clone()),
            )
    }
}

struct Ui {
    state: Entity<AppState>,
    window: AnyWindowHandle,
    root: Entity<Root>,
    sent: Arc<Sent>,
}

impl Ui {
    fn open(cx: &mut TestAppContext) -> Self {
        cx.executor().allow_parking();
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            crate::terminal::init(cx);
            cx.set_reduce_motion(true);
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
        let tab_state = state.clone();
        let (window, root) = cx.update(|cx| {
            gpui_kit::open_window(options, cx, |window, cx| {
                cx.new(|cx| Root {
                    terminal: cx
                        .new(|cx| TerminalView::new(TerminalOptions::default(), window, cx)),
                    tab: cx.new(|cx| SnippetsTab::new(tab_state, window, cx)),
                })
            })
            .expect("test window")
        });
        let sent = Arc::new(Sent::default());
        let handle = cx.update(|cx| root.read(cx).terminal.read(cx).handle());
        handle.set_sink(Some(sent.clone()));
        let ui = Self {
            state,
            window,
            root,
            sent,
        };
        ui.settle(cx);
        ui
    }

    fn settle(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.update_window(self.window, |_, window, cx| window.render_frame(cx))
            .unwrap();
        cx.run_until_parked();
    }

    fn tab(&self, cx: &mut TestAppContext) -> Entity<SnippetsTab> {
        cx.update(|cx| self.root.read(cx).tab.clone())
    }

    fn save(&self, data: Value, cx: &mut TestAppContext) -> Uuid {
        let (core, vault_id) = cx.update(|cx| {
            let state = self.state.read(cx);
            (state.core().cloned().unwrap(), state.vault_id().unwrap())
        });
        let view = core
            .snippet_save(RecordInput {
                vault_id,
                record_id: None,
                data: data.as_object().cloned().unwrap(),
            })
            .unwrap();
        let tab = self.tab(cx);
        tab.update(cx, |tab, cx| tab.reload(cx));
        self.settle(cx);
        view.record.record_id
    }

    fn show(&self, cx: &mut TestAppContext) {
        let terminal = cx.update(|cx| self.root.read(cx).terminal.clone());
        let tab = self.tab(cx);
        let target = SidebarTarget::new(&terminal).session(Some(1));
        tab.update(cx, |tab, cx| tab.set_target(Some(target), cx));
        self.settle(cx);
    }

    fn visible(&self, id: impl Into<SharedString>, cx: &mut TestAppContext) -> bool {
        let id = id.into();
        cx.update_window(self.window, move |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id.clone()).is_some_and(|el| el.visible())
        })
        .unwrap()
    }

    fn click(&self, id: impl Into<SharedString>, cx: &mut TestAppContext) {
        let id = id.into();
        cx.update_window(self.window, move |_, window, cx| {
            window.render_frame(cx);
            window.click(id.clone(), cx);
        })
        .unwrap();
        self.settle(cx);
    }

    fn output(&self, text: &str, cx: &mut TestAppContext) {
        let terminal = cx.update(|cx| self.root.read(cx).terminal.clone());
        let handle = cx.update(|cx| terminal.read(cx).handle());
        handle.feed_blocking(text.as_bytes()).unwrap();
        handle.flush_blocking();
        self.settle(cx);
    }

    /// What reached the session since the last call.
    fn take_sent(&self) -> String {
        String::from_utf8(std::mem::take(&mut *self.sent.0.lock().unwrap())).unwrap()
    }

    fn finish(self, cx: &mut TestAppContext) {
        self.state.update(cx, |state, _| state.shutdown());
    }
}

#[gpui::test]
fn snippets_are_inserted_or_run_in_the_pane(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    // No pane yet: nothing to insert into.
    assert!(ui.visible("sidebar-snippets-no-session", cx));
    ui.show(cx);
    assert!(ui.visible("terminal-sidebar-snippets-empty", cx));
    let uptime = ui.save(json!({"label": "Load", "script": "uptime"}), cx);
    let multi = ui.save(
        json!({"label": "List app", "script": "cd /srv/app\nls -la\n"}),
        cx,
    );
    assert!(ui.visible(row_id(uptime), cx));
    ui.output("alice@web:~$ ", cx);

    // A click puts it at the prompt without Enter; Run sends Enter too.
    ui.click(row_id(uptime), cx);
    assert_eq!(ui.take_sent(), "uptime");
    ui.click(run_id(uptime), cx);
    assert_eq!(ui.take_sent(), "uptime\r");

    // Several lines without bracketed paste: Insert would run them, so it
    // is unavailable (D68: its tooltip says why) and sends nothing; Run
    // sends each line.
    let tab = ui.tab(cx);
    assert!(cx.update(|cx| tab.read(cx).can_insert(uptime, cx)));
    assert!(!cx.update(|cx| tab.read(cx).can_insert(multi, cx)));
    ui.click(row_id(multi), cx);
    assert_eq!(ui.take_sent(), "");
    ui.click(run_id(multi), cx);
    assert_eq!(ui.take_sent(), "cd /srv/app\rls -la\r");
    // With bracketed paste the shell holds the lines until Enter, and the
    // insert is there again.
    ui.output("\x1b[?2004h", cx);
    assert!(cx.update(|cx| tab.read(cx).can_insert(multi, cx)));
    ui.click(row_id(multi), cx);
    assert_eq!(ui.take_sent(), "\x1b[200~cd /srv/app\rls -la\x1b[201~");

    // A search by label or script.
    cx.update_window(ui.window, |_, window, cx| {
        window.click("sidebar-snippets-search", cx);
        window.input("srv", cx);
    })
    .unwrap();
    ui.settle(cx);
    assert_eq!(cx.update(|cx| tab.read(cx).shown_ids()), [multi]);
    ui.finish(cx);
}

#[gpui::test]
fn variables_are_asked_before_insert(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    ui.show(cx);
    let disk = ui.save(json!({"label": "Disk", "script": "df -h {{path}}"}), cx);
    ui.output("alice@web:~$ ", cx);
    ui.click(run_id(disk), cx);
    assert!(ui.visible("snippet-variables-form", cx));
    assert_eq!(ui.take_sent(), "");
    cx.update_window(ui.window, |_, window, cx| {
        window.click("snippet-variable-path", cx);
        window.input("/var", cx);
    })
    .unwrap();
    ui.settle(cx);
    ui.click("snippet-variables-run", cx);
    assert_eq!(ui.take_sent(), "df -h /var\r");
    assert!(!ui.visible("snippet-variables-form", cx));
    ui.finish(cx);
}
