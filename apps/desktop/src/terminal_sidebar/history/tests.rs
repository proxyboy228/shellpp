//! Headless tests of the History tab (UI 4.2; lead decision L7): a
//! temporary profile with the in-memory secret store, a terminal fed with
//! bytes (no shell, no network), entries written through the core.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use gpui::{
    AnyWindowHandle, Bounds, Context, Entity, Focusable as _, IntoElement, Render, TestAppContext,
    Window, WindowBounds, WindowOptions, div, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Map, Value, json};
use shellpp_core::service::dto::{HistoryCandidate, SettingsSetInput};
use shellpp_core::service::host_api::{HistoryDecision, HistoryRecordInput};

use super::*;
use crate::settings::terminal::TerminalPrefs;
use crate::terminal::{HistorySink, TerminalOptions, TerminalSink, TerminalSize};
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

/// Command lines the terminal's capture passed on.
#[derive(Default)]
struct Captured(Mutex<Vec<String>>);

impl HistorySink for Captured {
    fn record(&self, candidate: HistoryCandidate) {
        self.0.lock().unwrap().push(candidate.command);
    }
}

/// A terminal with the History tab on its right (the sidebar's width).
struct Root {
    terminal: Entity<TerminalView>,
    tab: Entity<HistoryTab>,
}

impl Render for Root {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .size_full()
            .child(div().flex_1().h_full().child(self.terminal.clone()))
            .child(
                div()
                    .id("history-scroll")
                    .w(super::super::WIDTH)
                    .h_full()
                    .overflow_y_scroll()
                    .child(self.tab.clone()),
            )
    }
}

struct Ui {
    state: Entity<AppState>,
    window: AnyWindowHandle,
    root: Entity<Root>,
    sent: Arc<Sent>,
    captured: Arc<Captured>,
}

impl Ui {
    fn open(cx: &mut TestAppContext) -> Self {
        cx.executor().allow_parking();
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            crate::terminal::init(cx);
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
                    tab: cx.new(|cx| HistoryTab::new(tab_state, window, cx)),
                })
            })
            .expect("test window")
        });
        let sent = Arc::new(Sent::default());
        let captured = Arc::new(Captured::default());
        let handle = cx.update(|cx| root.read(cx).terminal.read(cx).handle());
        handle.set_sink(Some(sent.clone()));
        handle.set_history_sink(Some(captured.clone()));
        let ui = Self {
            state,
            window,
            root,
            sent,
            captured,
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

    fn tab(&self, cx: &mut TestAppContext) -> Entity<HistoryTab> {
        cx.update(|cx| self.root.read(cx).tab.clone())
    }

    fn terminal(&self, cx: &mut TestAppContext) -> Entity<TerminalView> {
        cx.update(|cx| self.root.read(cx).terminal.clone())
    }

    fn core(&self, cx: &mut TestAppContext) -> Arc<shellpp_core::Core> {
        cx.update(|cx| self.state.read(cx).core().cloned().expect("core"))
    }

    /// A saved host in the personal vault.
    fn saved_host(&self, address: &str, cx: &mut TestAppContext) -> RecordRef {
        let data: Map<String, Value> = json!({ "address": address }).as_object().cloned().unwrap();
        let host = self
            .state
            .update(cx, |state, cx| state.save_host(None, data, cx))
            .expect("host saved");
        RecordRef {
            vault_id: host.vault_id,
            record_id: host.record_id,
        }
    }

    /// A command recorded for `host` through the core (a new session each
    /// time, so repeats count).
    fn record(&self, host: RecordRef, command: &str, cx: &mut TestAppContext) {
        let decision = self
            .core(cx)
            .history_record(HistoryRecordInput {
                host: Some(host),
                session_id: Uuid::now_v7(),
                candidate: HistoryCandidate {
                    command: command.to_owned(),
                    prompt_text: "alice@web:~$".to_owned(),
                    typed_printable: true,
                    alt_screen: false,
                },
                injected: false,
                record_history: true,
            })
            .unwrap();
        assert_eq!(decision, HistoryDecision::Recorded, "{command}");
    }

    fn show(&self, host: Option<RecordRef>, cx: &mut TestAppContext) {
        let terminal = self.terminal(cx);
        let tab = self.tab(cx);
        let target = SidebarTarget::new(&terminal)
            .host(host.map(|host| host.record_id))
            .session(Some(1));
        cx.update_window(self.window, |_, window, cx| {
            tab.update(cx, |tab, cx| tab.set_target(Some(target), window, cx))
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

    fn commands(&self, cx: &mut TestAppContext) -> Vec<String> {
        let tab = self.tab(cx);
        cx.update(|cx| tab.read(cx).commands())
    }

    fn stored(&self, host: RecordRef, cx: &mut TestAppContext) -> Vec<String> {
        self.core(cx)
            .history_list(HistoryListInput {
                host: Some(host),
                text: None,
                limit: None,
            })
            .unwrap()
            .into_iter()
            .map(|entry| entry.command)
            .collect()
    }

    /// Output of the shell, parsed.
    fn output(&self, text: &str, cx: &mut TestAppContext) {
        let terminal = self.terminal(cx);
        let handle = cx.update(|cx| terminal.read(cx).handle());
        handle.feed_blocking(text.as_bytes()).unwrap();
        handle.flush_blocking();
        self.settle(cx);
    }

    fn sent(&self) -> String {
        String::from_utf8(self.sent.0.lock().unwrap().clone()).unwrap()
    }

    /// Confirms the open dialog (Enter = OK) once it has opened.
    fn confirm_dialog(&self, cx: &mut TestAppContext) {
        cx.run_until_parked();
        cx.executor().advance_clock(Duration::from_millis(500));
        cx.run_until_parked();
        cx.update_window(self.window, |_, window, cx| {
            window.render_frame(cx);
            assert!(window.has_active_dialog(cx), "the clear asks first");
            window.press("enter", cx);
        })
        .unwrap();
        self.settle(cx);
    }

    fn finish(self, cx: &mut TestAppContext) {
        self.state.update(cx, |state, _| state.shutdown());
    }
}

/// The list shows the host's commands newest first, searches, inserts a
/// command at the prompt without running it (injected: a line that holds
/// it is never recorded again) and deletes single entries.
#[gpui::test]
fn list_search_insert_and_delete(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let web = ui.saved_host("web.example.com", cx);
    let db = ui.saved_host("db.example.com", cx);
    for command in ["uptime", "git status", "git log --oneline"] {
        ui.record(web, command, cx);
    }
    ui.record(db, "psql", cx);
    ui.show(Some(web), cx);
    assert_eq!(
        ui.commands(cx),
        ["git log --oneline", "git status", "uptime"],
        "newest first, this host only"
    );
    assert!(ui.visible("history-entry-0", cx));
    assert!(!ui.visible("history-recording-off", cx));

    // Search: plain text, case-insensitive.
    let tab = ui.tab(cx);
    cx.update_window(ui.window, |_, window, cx| {
        tab.update(cx, |tab, cx| tab.set_search("GIT", window, cx))
    })
    .unwrap();
    ui.settle(cx);
    assert_eq!(ui.commands(cx), ["git log --oneline", "git status"]);
    cx.update_window(ui.window, |_, window, cx| {
        tab.update(cx, |tab, cx| tab.set_search("nothing like it", window, cx))
    })
    .unwrap();
    ui.settle(cx);
    assert!(ui.commands(cx).is_empty());
    assert!(ui.visible("history-empty", cx));
    cx.update_window(ui.window, |_, window, cx| {
        tab.update(cx, |tab, cx| tab.set_search("", window, cx))
    })
    .unwrap();
    ui.settle(cx);

    // Insert: the command goes to the session without Enter; the terminal
    // gets the focus.
    ui.output("alice@web:~$ ", cx);
    ui.click("history-entry-1", cx);
    assert_eq!(ui.sent(), "git status");
    let terminal = ui.terminal(cx);
    cx.update_window(ui.window, |_, window, cx| {
        assert!(terminal.read(cx).focus_handle(cx).is_focused(window));
    })
    .unwrap();
    // The shell echoes it, the user presses Enter: not recorded again.
    ui.output("git status", cx);
    let handle = cx.update(|cx| terminal.read(cx).handle());
    handle.send_typed(b"\r".to_vec());
    ui.output("\r\nOn branch main\r\nalice@web:~$ ", cx);
    assert!(ui.captured.0.lock().unwrap().is_empty());

    // Delete one entry.
    ui.click("history-entry-0-delete", cx);
    assert_eq!(ui.commands(cx), ["git status", "uptime"]);
    assert_eq!(ui.stored(web, cx), ["git status", "uptime"]);
    assert_eq!(ui.stored(db, cx), ["psql"]);
    ui.finish(cx);
}

/// "Clear history" asks first, then removes every entry of the host.
#[gpui::test]
fn clear_history_asks_first(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let web = ui.saved_host("web.example.com", cx);
    let db = ui.saved_host("db.example.com", cx);
    ui.record(web, "uptime", cx);
    ui.record(web, "df -h", cx);
    ui.record(db, "psql", cx);
    ui.show(Some(web), cx);
    assert_eq!(ui.commands(cx).len(), 2);
    ui.click("history-clear", cx);
    ui.confirm_dialog(cx);
    assert!(ui.commands(cx).is_empty());
    assert!(ui.stored(web, cx).is_empty());
    assert_eq!(ui.stored(db, cx), ["psql"], "other hosts keep theirs");
    assert!(ui.visible("history-empty", cx));
    ui.finish(cx);
}

/// The hint offers the three most frequent commands in editable fields;
/// "Save" is disabled until stage 5; "Close" hides it for the host for
/// good, also in a new tab.
#[gpui::test]
fn the_hint_card_and_its_close(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let web = ui.saved_host("web.example.com", cx);
    for (command, times) in [("df -h", 3), ("uptime", 2), ("free -m", 2), ("ls", 1)] {
        for _ in 0..times {
            ui.record(web, command, cx);
        }
    }
    ui.show(Some(web), cx);
    let tab = ui.tab(cx);
    assert_eq!(
        cx.update(|cx| tab.read(cx).hint_commands().to_vec()),
        ["df -h", "free -m", "uptime"]
    );
    assert!(ui.visible("history-hint", cx));
    for index in 0..3 {
        assert!(ui.visible(&format!("history-hint-command-{index}"), cx));
    }
    assert!(ui.visible("history-hint-save", cx));
    // Save does nothing yet.
    ui.click("history-hint-save", cx);
    assert!(ui.visible("history-hint", cx));

    ui.click("history-hint-close", cx);
    assert!(!ui.visible("history-hint", cx));
    assert!(ui.core(cx).history_hint(web).unwrap().dismissed);
    // A new tab (another window, a restart) does not show it again.
    let state = ui.state.clone();
    let fresh = cx
        .update_window(ui.window, |_, window, cx| {
            cx.new(|cx| HistoryTab::new(state, window, cx))
        })
        .unwrap();
    let terminal = ui.terminal(cx);
    cx.update_window(ui.window, |_, window, cx| {
        fresh.update(cx, |fresh, cx| {
            fresh.set_target(
                Some(SidebarTarget::new(&terminal).host(Some(web.record_id))),
                window,
                cx,
            )
        })
    })
    .unwrap();
    ui.settle(cx);
    assert!(cx.update(|cx| fresh.read(cx).hint_commands().is_empty()));
    // Every entry is listed (3 + 2 + 2 + 1).
    assert_eq!(cx.update(|cx| fresh.read(cx).commands().len()), 8);
    ui.finish(cx);
}

/// Quick connects and local terminals keep no history; without a session
/// the tab says how to get one; "Recording is off" follows the setting.
#[gpui::test]
fn unavailable_and_recording_off(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    assert!(ui.visible("history-unavailable", cx), "no session");
    ui.show(None, cx);
    assert!(ui.visible("history-unavailable", cx), "not a saved host");
    assert!(!ui.visible("history-search", cx));

    let web = ui.saved_host("web.example.com", cx);
    ui.record(web, "uptime", cx);
    ui.show(Some(web), cx);
    assert!(ui.visible("history-search", cx));
    assert!(!ui.visible("history-recording-off", cx));
    let mut values = std::collections::BTreeMap::new();
    values.insert("terminal.record_history".to_owned(), json!(false));
    ui.core(cx)
        .settings_set(SettingsSetInput {
            values,
            reset: Vec::new(),
        })
        .unwrap();
    let tab = ui.tab(cx);
    tab.update(cx, |tab, cx| tab.reload(cx));
    ui.settle(cx);
    assert!(ui.visible("history-recording-off", cx));
    // Stored entries stay usable.
    assert_eq!(ui.commands(cx), ["uptime"]);

    // Lock: the tab holds no commands.
    ui.state.update(cx, |state, cx| state.lock(cx)).unwrap();
    ui.settle(cx);
    assert!(ui.commands(cx).is_empty());
    assert!(cx.update(|cx| tab.read(cx).hint_commands().is_empty()));
    ui.finish(cx);
}
