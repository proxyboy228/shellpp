//! Headless tests of the Snippets screen (GPUI `TestAppContext`, no real
//! window) over a temporary profile. Nothing connects: a run without the
//! session layer marks its hosts as unable to run, and the results of a
//! finished run are set by hand where a test needs them.

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, TestAppContext, Window,
    WindowBounds, WindowOptions, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Map, Value, json};
use shellpp_ssh::batch::TargetOutcome;
use shellpp_ssh::client::ExecOutput;

use super::*;
use crate::state::AppState;
use crate::testing::TestProfile;

struct Screen {
    state: Entity<AppState>,
    screen: Entity<SnippetsScreen>,
    handle: AnyWindowHandle,
}

impl Screen {
    fn open(cx: &mut TestAppContext, hosts: &[Value]) -> Self {
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            cx.set_reduce_motion(true);
        });
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
        state.update(cx, |state, cx| {
            for host in hosts {
                let data: Map<String, Value> = host.as_object().cloned().unwrap();
                state.save_host(None, data, cx).unwrap();
            }
        });
        cx.run_until_parked();
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(1280.), px(800.)),
            ))),
            ..Default::default()
        };
        let screen_state = state.clone();
        let (handle, screen) = cx
            .update(|cx| {
                gpui_kit::open_window(options, cx, |window, cx| {
                    cx.new(|cx| SnippetsScreen::new(screen_state, window, cx))
                })
            })
            .unwrap();
        let this = Self {
            state,
            screen,
            handle,
        };
        this.frame(cx, |_, _| {});
        this.frame(cx, |_, _| {});
        this
    }

    fn frame<R>(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        let result = cx
            .update_window(self.handle, |_, window, cx| {
                window.render_frame(cx);
                f(window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        result
    }

    fn visible(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        self.frame(cx, |window, _| {
            window.try_find(id).is_some_and(|found| found.visible())
        })
    }

    fn click(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) {
        let id = id.into();
        self.frame(cx, |window, cx| window.click(id, cx));
    }

    fn type_into(&self, cx: &mut TestAppContext, id: &'static str, text: &str) {
        self.frame(cx, |window, cx| {
            window.click(id, cx);
            window.press("ctrl-a", cx);
            window.press("backspace", cx);
            window.input(text, cx);
        });
    }

    fn act(
        &self,
        cx: &mut TestAppContext,
        f: impl FnOnce(&mut SnippetsScreen, &mut Window, &mut Context<SnippetsScreen>),
    ) {
        let screen = self.screen.clone();
        self.frame(cx, move |window, cx| {
            screen.update(cx, |screen, cx| f(screen, window, cx))
        });
    }

    fn form(&self, cx: &mut TestAppContext) -> Entity<SnippetForm> {
        self.screen
            .read_with(cx, |screen, _| screen.form().cloned())
            .expect("the form is open")
    }

    fn core(&self, cx: &mut TestAppContext) -> std::sync::Arc<shellpp_core::Core> {
        self.state
            .read_with(cx, |state, _| state.core().cloned().unwrap())
    }

    fn vault(&self, cx: &mut TestAppContext) -> Uuid {
        self.state
            .read_with(cx, |state, _| state.vault_id().unwrap())
    }

    fn snippets(&self, cx: &mut TestAppContext) -> Vec<SnippetView> {
        let vault = self.vault(cx);
        self.core(cx).snippets_list(vault).unwrap()
    }

    fn host_id(&self, cx: &mut TestAppContext, address: &str) -> Uuid {
        self.state.read_with(cx, |state, _| {
            state
                .hosts()
                .iter()
                .find(|host| host.address == address)
                .unwrap()
                .record_id
        })
    }

    fn save(&self, cx: &mut TestAppContext, data: Value) -> SnippetView {
        let vault = self.vault(cx);
        let view = self
            .core(cx)
            .snippet_save(RecordInput {
                vault_id: vault,
                record_id: None,
                data: data.as_object().cloned().unwrap(),
            })
            .unwrap();
        self.screen.update(cx, |screen, cx| screen.reload(cx));
        self.frame(cx, |_, _| {});
        view
    }

    fn package(&self, cx: &mut TestAppContext, name: &str) -> Uuid {
        let vault = self.vault(cx);
        let view = self
            .core(cx)
            .snippet_package_save(RecordInput {
                vault_id: vault,
                record_id: None,
                data: json!({"name": name}).as_object().cloned().unwrap(),
            })
            .unwrap();
        self.screen.update(cx, |screen, cx| screen.reload(cx));
        view.record.record_id
    }

    fn finish(self, cx: &mut TestAppContext) {
        let _ = cx.update_window(self.handle, |_, window, _| window.remove_window());
        cx.run_until_parked();
        self.state.update(cx, |state, _| state.shutdown());
    }
}

#[test]
fn titles_tags_and_output_are_read_safely() {
    let mut view = SnippetView {
        record: shellpp_core::service::dto::RecordRef {
            vault_id: Uuid::nil(),
            record_id: Uuid::nil(),
        },
        label: None,
        script: "\n  systemctl restart nginx\nsystemctl status nginx".into(),
        package_id: None,
        tags: vec!["web".into()],
        target_host_ids: Vec::new(),
        variables: Vec::new(),
    };
    assert_eq!(card_title(&view), "systemctl restart nginx");
    assert!(matches_search(&view, "WEB") && matches_search(&view, "status"));
    assert!(!matches_search(&view, "apache"));
    view.label = Some("Restart nginx".into());
    assert_eq!(card_title(&view), "Restart nginx");
    view.script = "  ".into();
    view.label = None;
    assert_eq!(card_title(&view), strings::UNTITLED);
    assert_eq!(form::parse_tags(" web, ,deploy ,"), ["web", "deploy"]);
    // Escape sequences and bidirectional overrides do not reach the screen.
    assert_eq!(
        run::output_text(b"ok\x1b[31m\tred\n\xe2\x80\xaeevil"),
        "ok·[31m\tred\n·evil"
    );
}

#[gpui_kit::test]
fn a_snippet_is_created_with_its_package_tags_and_targets(cx: &mut TestAppContext) {
    let ui = Screen::open(
        cx,
        &[
            json!({"address": "web.example.com", "label": "Web"}),
            json!({"address": "db.example.com", "label": "DB"}),
        ],
    );
    assert!(ui.visible(cx, "snippet-empty"));
    let deploy = ui.package(cx, "Deploy");
    ui.click(cx, "snippet-new");
    assert!(ui.visible(cx, "snippet-panel"));
    // Run waits for a script and a target.
    let form = ui.form(cx);
    assert!(!form.read_with(cx, |form, cx| form.can_run(cx)));
    ui.click(cx, "snippet-create");
    assert_eq!(
        form.read_with(cx, |form, _| form.errors().get("script").copied()),
        Some(strings::ERROR_SCRIPT)
    );
    ui.type_into(cx, "snippet-label", "Restart nginx");
    // Tags are chips (D74): a comma adds "web", Enter adds "deploy".
    ui.type_into(cx, "snippet-tags", "web, deploy");
    ui.frame(cx, |window, cx| window.press("enter", cx));
    assert_eq!(
        form.read_with(cx, |form, _| form.tags().to_vec()),
        ["web", "deploy"]
    );
    ui.type_into(cx, "snippet-script", "sudo systemctl restart {{service}}");
    assert!(ui.visible(cx, "snippet-variables"));
    ui.click(cx, "snippet-package");
    ui.click(cx, format!("snippet-package-pick-{deploy}"));
    ui.click(cx, "snippet-add-targets");
    let web = ui.host_id(cx, "web.example.com");
    let db = ui.host_id(cx, "db.example.com");
    // The picker opens under the button (below the fold of this window).
    assert!(form.read_with(cx, |form, _| form.targets().is_empty()));
    form.update(cx, |form, cx| {
        form.toggle_target(web, cx);
        form.toggle_target(db, cx);
        form.set_picker_open(false, cx);
    });
    assert!(form.read_with(cx, |form, cx| form.can_run(cx)));
    assert!(ui.visible(cx, form::target_id(db)));
    ui.click(cx, "snippet-create");
    let saved = ui.snippets(cx);
    assert_eq!(saved.len(), 1);
    let snippet = &saved[0];
    assert_eq!(snippet.label.as_deref(), Some("Restart nginx"));
    assert_eq!(snippet.tags, ["web", "deploy"]);
    assert_eq!(snippet.package_id, Some(deploy));
    assert_eq!(snippet.target_host_ids, [web, db]);
    assert_eq!(snippet.variables, ["service"]);
    // It lives in its package: the top level shows the package only.
    let id = snippet.record.record_id;
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.shown_ids()),
        Vec::<Uuid>::new()
    );
    assert!(ui.visible(cx, package_id(deploy)));
    ui.click(cx, package_id(deploy));
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.open_package()),
        Some(deploy)
    );
    assert!(ui.visible(cx, card_id(id)));
    assert!(ui.visible(cx, "snippet-breadcrumb"));
    ui.click(cx, "snippet-back");
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.open_package()),
        None
    );
    ui.finish(cx);
}

#[gpui_kit::test]
fn a_run_asks_values_confirms_and_shows_results(cx: &mut TestAppContext) {
    let ui = Screen::open(
        cx,
        &[
            json!({"address": "web.example.com", "label": "Web"}),
            json!({"address": "db.example.com", "label": "DB"}),
        ],
    );
    let web = ui.host_id(cx, "web.example.com");
    let db = ui.host_id(cx, "db.example.com");
    let snippet = ui.save(
        cx,
        json!({"label": "Disk", "script": "df -h {{path}}", "target_host_ids": [web, db]}),
    );
    let id = snippet.record.record_id;
    // A double click runs: the values first.
    ui.frame(cx, |window, cx| window.double_click(card_id(id), cx));
    assert!(ui.visible(cx, "snippet-variables-form"));
    ui.type_into(cx, "snippet-variable-path", "/var");
    ui.click(cx, "snippet-variables-run");
    // Two hosts: the confirmation lists them.
    assert!(ui.visible(cx, "snippet-confirm"));
    ui.click(cx, "snippet-confirm-run");
    let results = ui
        .screen
        .read_with(cx, |screen, _| screen.results().cloned())
        .expect("the results are open");
    assert!(ui.visible(cx, "snippet-results"));
    // No session layer here: every host says why it could not run.
    results.read_with(cx, |results, _| {
        assert_eq!(results.rows().len(), 2);
        assert!(results.finished());
        assert_eq!(results.rows()[0].title, "Web");
        assert_eq!(results.rows()[0].state, RunState::Unavailable);
        assert!(results.rows()[1].status().1);
    });
    // A finished host: its exit code, and its output on click.
    results.update(cx, |results, cx| {
        let output = ExecOutput {
            stdout: b"/dev/sda1 40G\n".to_vec(),
            exit_status: Some(0),
            truncated: false,
            timed_out: false,
        };
        results.show_row(
            0,
            RunState::Done(TargetOutcome::Finished(output)),
            false,
            cx,
        );
    });
    results.read_with(cx, |results, _| {
        assert_eq!(
            results.rows()[0].status(),
            (strings::SUCCESS.to_owned(), false)
        );
        assert_eq!(results.rows()[0].detail(), "Exit code 0");
        assert_eq!(results.succeeded(), 1);
    });
    ui.click(cx, format!("snippet-result-{web}"));
    assert!(ui.visible(cx, format!("snippet-output-{web}")));
    ui.click(cx, "snippet-results-close");
    assert!(!ui.visible(cx, "snippet-results"));
    ui.finish(cx);
}

#[gpui_kit::test]
fn a_snippet_without_targets_opens_instead_of_running(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, &[json!({"address": "web.example.com"})]);
    let snippet = ui.save(cx, json!({"script": "uptime"}));
    let id = snippet.record.record_id;
    ui.act(cx, |screen, window, cx| screen.run(id, window, cx));
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.selected()),
        Some(id)
    );
    assert!(ui.visible(cx, "snippet-form-targets"));
    assert!(!ui.visible(cx, "snippet-variables-form"));
    assert!(!ui.visible(cx, "snippet-confirm"));
    ui.finish(cx);
}

#[gpui_kit::test]
fn search_remove_undo_and_packages(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, &[json!({"address": "web.example.com"})]);
    let tools = ui.package(cx, "Tools");
    let a = ui.save(cx, json!({"label": "Uptime", "script": "uptime"}));
    let b = ui.save(
        cx,
        json!({"label": "Logs", "script": "journalctl -n 50", "package_id": tools}),
    );
    let (a, b) = (a.record.record_id, b.record.record_id);
    // A search looks inside packages too.
    ui.type_into(cx, "snippet-search", "journal");
    assert_eq!(ui.screen.read_with(cx, |screen, _| screen.shown_ids()), [b]);
    ui.type_into(cx, "snippet-search", "nothing like it");
    assert!(ui.visible(cx, "snippet-no-matches"));
    ui.type_into(cx, "snippet-search", "");
    assert_eq!(ui.screen.read_with(cx, |screen, _| screen.shown_ids()), [a]);

    // Remove asks first; Undo brings it back.
    ui.click(cx, card_id(a));
    ui.click(cx, "snippet-panel-remove");
    ui.click(cx, "snippet-remove-confirm");
    assert_eq!(ui.snippets(cx).len(), 1);
    assert!(!ui.visible(cx, "snippet-panel"));
    ui.act(cx, |screen, window, cx| screen.undo_remove(window, cx));
    assert_eq!(ui.snippets(cx).len(), 2);

    // Renaming and removing a package: its snippet stays, outside it.
    ui.act(cx, |screen, window, cx| {
        assert!(screen.save_package(Some(tools), "Admin".into(), window, cx))
    });
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen
            .package_view(tools)
            .map(|p| p.name.clone())),
        Some("Admin".into())
    );
    ui.act(cx, |screen, window, cx| {
        screen.confirm_remove_package(tools, window, cx)
    });
    assert!(ui.visible(cx, "package-remove-dialog"));
    ui.click(cx, "package-remove-confirm");
    let left = ui.snippets(cx);
    assert_eq!(left.len(), 2);
    assert!(left.iter().all(|snippet| snippet.package_id.is_none()));
    assert!(
        ui.screen
            .read_with(cx, |screen, _| screen.packages().is_empty())
    );
    ui.finish(cx);
}

/// A snippet without variables on one host runs at once: no form, no
/// confirmation (and no nested update of the screen).
#[gpui_kit::test]
fn a_plain_snippet_on_one_host_runs_at_once(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, &[json!({"address": "web.example.com", "label": "Web"})]);
    let web = ui.host_id(cx, "web.example.com");
    let snippet = ui.save(cx, json!({"script": "uptime", "target_host_ids": [web]}));
    ui.frame(cx, |window, cx| {
        window.double_click(card_id(snippet.record.record_id), cx)
    });
    assert!(!ui.visible(cx, "snippet-variables-form"));
    assert!(!ui.visible(cx, "snippet-confirm"));
    assert!(ui.visible(cx, "snippet-results"));
    let results = ui
        .screen
        .read_with(cx, |screen, _| screen.results().cloned())
        .unwrap();
    assert_eq!(results.read_with(cx, |results, _| results.rows().len()), 1);
    ui.finish(cx);
}

/// Shell History (9.1): every host's commands, a host filter, a search and
/// "Save as snippet" with the command and its host.
#[gpui_kit::test]
fn shell_history_filters_and_saves_a_snippet(cx: &mut TestAppContext) {
    use shellpp_core::service::dto::{HistoryCandidate, RecordRef};
    use shellpp_core::service::host_api::HistoryRecordInput;

    let ui = Screen::open(
        cx,
        &[
            json!({"address": "web.example.com", "label": "Web"}),
            json!({"address": "db.example.com", "label": "DB"}),
        ],
    );
    let vault = ui.vault(cx);
    let web = ui.host_id(cx, "web.example.com");
    let db = ui.host_id(cx, "db.example.com");
    for (host, command) in [(web, "systemctl status nginx"), (db, "psql -c 'select 1'")] {
        ui.core(cx)
            .history_record(HistoryRecordInput {
                host: Some(RecordRef {
                    vault_id: vault,
                    record_id: host,
                }),
                session_id: Uuid::now_v7(),
                candidate: HistoryCandidate {
                    command: command.to_owned(),
                    prompt_text: "alice@host:~$".to_owned(),
                    typed_printable: true,
                    alt_screen: false,
                },
                injected: false,
                record_history: true,
            })
            .unwrap();
    }
    ui.click(cx, "snippet-shell-history");
    let history = ui
        .screen
        .read_with(cx, |screen, _| screen.history().cloned())
        .expect("Shell History is shown");
    ui.frame(cx, |_, _| {});
    let commands = |cx: &mut TestAppContext| {
        history.read_with(cx, |history, _| {
            history
                .entries()
                .iter()
                .map(|entry| entry.command.clone())
                .collect::<Vec<_>>()
        })
    };
    assert_eq!(
        commands(cx),
        ["psql -c 'select 1'", "systemctl status nginx"]
    );
    assert!(ui.visible(cx, "shell-history-list"));

    // The host filter and the search.
    history.update(cx, |history, cx| history.set_host(Some(web), cx));
    ui.frame(cx, |_, _| {});
    assert_eq!(commands(cx), ["systemctl status nginx"]);
    history.update(cx, |history, cx| history.set_host(None, cx));
    ui.type_into(cx, "shell-history-search", "PSQL");
    assert_eq!(commands(cx), ["psql -c 'select 1'"]);

    // Save as snippet: a new snippet with the command, on its host.
    let entry = history.read_with(cx, |history, _| history.entries()[0].record_id);
    ui.click(cx, history::save_id(entry));
    let form = ui.form(cx);
    assert_eq!(form.read_with(cx, |form, _| form.mode()), FormMode::New);
    assert_eq!(
        form.read_with(cx, |form, cx| form.script_text(cx)),
        "psql -c 'select 1'"
    );
    assert_eq!(form.read_with(cx, |form, _| form.targets().to_vec()), [db]);

    // Back to the snippets.
    ui.click(cx, "snippet-back");
    assert!(
        ui.screen
            .read_with(cx, |screen, _| screen.history().is_none())
    );
    ui.finish(cx);
}

/// Enter on the confirmation starts one run and leaves its results open;
/// Run from the menu of the open snippet saves first; a removed host
/// leaves the targets.
#[gpui_kit::test]
fn the_review_cases_of_the_run(cx: &mut TestAppContext) {
    let ui = Screen::open(
        cx,
        &[
            json!({"address": "web.example.com", "label": "Web"}),
            json!({"address": "db.example.com", "label": "DB"}),
        ],
    );
    let web = ui.host_id(cx, "web.example.com");
    let db = ui.host_id(cx, "db.example.com");
    let id = ui
        .save(
            cx,
            json!({"script": "uptime", "target_host_ids": [web, db]}),
        )
        .record
        .record_id;
    ui.act(cx, |screen, window, cx| screen.run(id, window, cx));
    assert!(ui.visible(cx, "snippet-confirm"));
    ui.frame(cx, |window, cx| window.press("enter", cx));
    assert!(ui.visible(cx, "snippet-results"));
    assert!(!ui.visible(cx, "snippet-confirm"));
    ui.click(cx, "snippet-results-close");

    // A removed host leaves the targets of the form; Save works.
    ui.state
        .update(cx, |state, cx| state.delete_records(vec![db], cx))
        .unwrap();
    ui.act(cx, |screen, _, cx| screen.reload(cx));
    ui.click(cx, card_id(id));
    let form = ui.form(cx);
    assert_eq!(form.read_with(cx, |form, _| form.targets().to_vec()), [web]);
    // Run from the menu of the open snippet: what is typed runs, after
    // "Save and run" (D68).
    ui.type_into(cx, "snippet-script", "uptime -p");
    ui.act(cx, |screen, window, cx| {
        screen.run_from_menu(id, window, cx)
    });
    assert!(ui.visible(cx, "snippet-save-run-dialog"));
    assert!(!ui.visible(cx, "snippet-results"));
    ui.click(cx, "snippet-save-run-confirm");
    let saved = ui.snippets(cx);
    assert_eq!(saved[0].script, "uptime -p");
    assert_eq!(saved[0].target_host_ids, [web]);
    assert!(ui.visible(cx, "snippet-results"));
    // Selecting the open snippet again keeps its form.
    ui.click(cx, "snippet-results-close");
    let before = ui.form(cx);
    ui.click(cx, card_id(id));
    assert_eq!(ui.form(cx).entity_id(), before.entity_id());
    ui.finish(cx);
}

/// D68: leaving the panel with changes not saved (another card, the
/// collapse button, a new snippet) asks Save, Discard changes or Stay; the
/// header marks them "Not saved"; a refused Save stays; Run asks "Save
/// and run".
#[gpui_kit::test]
fn changes_not_saved_are_asked_about_before_they_are_left(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, &[json!({"address": "web.example.com", "label": "Web"})]);
    let web = ui.host_id(cx, "web.example.com");
    let a = ui
        .save(
            cx,
            json!({"label": "Uptime", "script": "uptime", "target_host_ids": [web]}),
        )
        .record
        .record_id;
    let b = ui
        .save(cx, json!({"label": "Disk", "script": "df -h"}))
        .record
        .record_id;
    let dirty = |cx: &mut TestAppContext| {
        ui.screen
            .read_with(cx, |screen, cx| screen.has_unsaved_changes(cx))
    };
    ui.click(cx, card_id(a));
    assert!(!dirty(cx));
    assert!(!ui.visible(cx, "panel-note"));
    // A typed change: the header says so.
    ui.type_into(cx, "snippet-label", "Uptime now");
    assert!(dirty(cx));
    assert!(ui.visible(cx, "panel-note"));
    // Typing the saved value back is no change.
    ui.type_into(cx, "snippet-label", "Uptime");
    assert!(!dirty(cx));
    ui.type_into(cx, "snippet-label", "Uptime now");

    // Another card: the question; Stay keeps the panel and what is typed.
    ui.click(cx, card_id(b));
    assert!(ui.visible(cx, "snippet-leave-dialog"));
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.selected()),
        Some(a)
    );
    ui.click(cx, "snippet-leave-stay");
    assert!(!ui.visible(cx, "snippet-leave-dialog"));
    assert!(dirty(cx));
    // Discard changes: the other card opens, nothing is saved.
    ui.click(cx, card_id(b));
    ui.click(cx, "snippet-leave-discard");
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.selected()),
        Some(b)
    );
    assert!(!dirty(cx));
    let label = |cx: &mut TestAppContext, id: Uuid| {
        ui.snippets(cx)
            .into_iter()
            .find(|snippet| snippet.record.record_id == id)
            .and_then(|snippet| snippet.label)
    };
    assert_eq!(label(cx, a).as_deref(), Some("Uptime"));

    // The collapse button with a change, then Save: saved and closed.
    ui.type_into(cx, "snippet-script", "df -h /");
    ui.click(cx, "panel-close");
    assert!(ui.visible(cx, "snippet-leave-dialog"));
    ui.click(cx, "snippet-leave-save");
    assert!(!ui.visible(cx, "snippet-panel"));
    let saved = ui.snippets(cx);
    let disk = saved.iter().find(|s| s.record.record_id == b).unwrap();
    assert_eq!(disk.script, "df -h /");

    // A new snippet untouched is no change; one with a label only cannot
    // be saved, so Save keeps the panel open with the reason.
    ui.click(cx, "snippet-new");
    assert!(!dirty(cx));
    ui.click(cx, card_id(a));
    assert!(!ui.visible(cx, "snippet-leave-dialog"));
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.selected()),
        Some(a)
    );
    ui.click(cx, "snippet-new");
    ui.type_into(cx, "snippet-label", "Half done");
    ui.click(cx, "panel-close");
    // Enter on the question is Save.
    ui.frame(cx, |window, cx| window.press("enter", cx));
    assert!(ui.visible(cx, "snippet-panel"));
    assert_eq!(
        ui.form(cx)
            .read_with(cx, |form, _| form.errors().get("script").copied()),
        Some(strings::ERROR_SCRIPT)
    );
    assert_eq!(ui.snippets(cx).len(), 2);
    // Esc on the question stays.
    ui.click(cx, card_id(b));
    assert!(ui.visible(cx, "snippet-leave-dialog"));
    ui.frame(cx, |window, cx| window.press("escape", cx));
    assert!(!ui.visible(cx, "snippet-leave-dialog"));
    assert_eq!(
        ui.form(cx).read_with(cx, |form, _| form.mode()),
        FormMode::New
    );
    ui.click(cx, card_id(b));
    ui.click(cx, "snippet-leave-discard");

    // Run with a change: "Save and run" first; Cancel saves nothing.
    ui.click(cx, card_id(a));
    ui.type_into(cx, "snippet-script", "uptime -p");
    ui.click(cx, "snippet-run");
    assert!(ui.visible(cx, "snippet-save-run-dialog"));
    ui.click(cx, "snippet-save-run-cancel");
    assert!(!ui.visible(cx, "snippet-results"));
    let script = |cx: &mut TestAppContext| {
        ui.snippets(cx)
            .into_iter()
            .find(|snippet| snippet.record.record_id == a)
            .unwrap()
            .script
    };
    assert_eq!(script(cx), "uptime");
    ui.click(cx, "snippet-run");
    ui.click(cx, "snippet-save-run-confirm");
    assert_eq!(script(cx), "uptime -p");
    assert!(ui.visible(cx, "snippet-results"));
    ui.click(cx, "snippet-results-close");
    // Without changes Run runs at once.
    ui.click(cx, "snippet-run");
    assert!(!ui.visible(cx, "snippet-save-run-dialog"));
    assert!(ui.visible(cx, "snippet-results"));
    ui.finish(cx);
}

/// Save as snippet from Shell History is not saved yet: leaving it asks.
#[gpui_kit::test]
fn a_snippet_from_history_counts_as_not_saved(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, &[json!({"address": "web.example.com"})]);
    let web = ui.host_id(cx, "web.example.com");
    ui.act(cx, |screen, window, cx| {
        screen.new_snippet_from("uptime", Some(web), window, cx)
    });
    assert!(
        ui.screen
            .read_with(cx, |screen, cx| screen.has_unsaved_changes(cx))
    );
    ui.click(cx, "panel-close");
    assert!(ui.visible(cx, "snippet-leave-dialog"));
    ui.click(cx, "snippet-leave-save");
    assert!(!ui.visible(cx, "snippet-panel"));
    assert_eq!(ui.snippets(cx)[0].script, "uptime");
    ui.finish(cx);
}

/// D74: tags are chips with the vault's other tags as suggestions; × and
/// Backspace in the empty field remove one; the highlighted script is
/// stored as typed.
#[gpui_kit::test]
fn tags_are_chips_and_the_script_keeps_its_text(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, &[json!({"address": "web.example.com"})]);
    ui.save(
        cx,
        json!({"script": "uptime", "tags": ["production", "web"]}),
    );
    ui.click(cx, "snippet-new");
    let form = ui.form(cx);
    let tags = |cx: &mut TestAppContext| form.read_with(cx, |form, _| form.tags().to_vec());
    // Typed into the tag field as it is (`type_into` clears it with
    // Backspace, which in an empty tag field takes the last chip).
    let type_tag = |cx: &mut TestAppContext, text: &'static str| {
        ui.frame(cx, |window, cx| {
            window.click("snippet-tags", cx);
            window.input(text, cx);
        })
    };
    // A suggestion from the other snippet.
    type_tag(cx, "prod");
    assert!(ui.visible(cx, "snippet-tag-suggest-0"));
    ui.click(cx, "snippet-tag-suggest-0");
    assert_eq!(tags(cx), ["production"]);
    // A new one with Enter; the same one again is not added twice.
    type_tag(cx, "db");
    ui.frame(cx, |window, cx| window.press("enter", cx));
    type_tag(cx, "DB");
    ui.frame(cx, |window, cx| window.press("enter", cx));
    assert_eq!(tags(cx), ["production", "db"]);
    // Backspace in the empty field takes the last chip; × takes any.
    ui.frame(cx, |window, cx| {
        window.click("snippet-tags", cx);
        window.press("backspace", cx);
    });
    assert_eq!(tags(cx), ["production"]);
    ui.click(cx, "snippet-tag-remove-0");
    assert!(tags(cx).is_empty());
    // The script: shell highlighting colors it, the text stays as typed.
    let script = "if [ -f \"$HOME/{{file}}\" ]; then cat 'x' # read\nfi";
    ui.type_into(cx, "snippet-script", script);
    assert_eq!(form.read_with(cx, |form, cx| form.script_text(cx)), script);
    type_tag(cx, "ops");
    ui.click(cx, "snippet-create");
    let saved = ui
        .snippets(cx)
        .into_iter()
        .find(|snippet| snippet.tags == ["ops"])
        .expect("the new snippet");
    assert_eq!(saved.script, script);
    assert_eq!(saved.variables, ["file"]);
    ui.finish(cx);
}

/// D68, D69: the other ways of leaving the panel ask too: New snippet, Esc
/// and the arrow keys on the cards.
#[gpui_kit::test]
fn every_way_of_leaving_the_panel_asks(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, &[json!({"address": "web.example.com"})]);
    let a = ui
        .save(cx, json!({"label": "A", "script": "uptime"}))
        .record
        .record_id;
    let b = ui
        .save(cx, json!({"label": "B", "script": "df -h"}))
        .record
        .record_id;
    let selected = |cx: &mut TestAppContext| ui.screen.read_with(cx, |screen, _| screen.selected());
    ui.click(cx, card_id(a));
    ui.type_into(cx, "snippet-label", "A changed");
    // New snippet.
    ui.click(cx, "snippet-new");
    assert!(ui.visible(cx, "snippet-leave-dialog"));
    ui.click(cx, "snippet-leave-stay");
    assert_eq!(selected(cx), Some(a));
    // Esc on the cards (a click on the open card keeps its form and takes
    // the keyboard to the cards).
    ui.click(cx, card_id(a));
    ui.frame(cx, |window, cx| window.press("escape", cx));
    assert!(ui.visible(cx, "snippet-leave-dialog"));
    ui.click(cx, "snippet-leave-stay");
    assert!(ui.visible(cx, "snippet-panel"));
    // The arrow keys move to the next card: asked first, then Discard.
    ui.click(cx, card_id(a));
    ui.frame(cx, |window, cx| window.press("right", cx));
    assert!(ui.visible(cx, "snippet-leave-dialog"));
    assert_eq!(selected(cx), Some(a));
    ui.click(cx, "snippet-leave-discard");
    assert_eq!(selected(cx), Some(b));
    ui.finish(cx);
}

/// Plan of run 4, item 7.4: the arrows reach the folder cards too (up from
/// the first snippet row); Enter opens the folder, Esc leaves it.
#[gpui_kit::test]
fn folders_are_reached_with_the_arrows(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, &[]);
    let deploy = ui.package(cx, "Deploy");
    let top = ui.save(cx, json!({"label": "Uptime", "script": "uptime"}));
    let top = top.record.record_id;
    // A click on the card selects it and focuses the cards.
    ui.click(cx, card_id(top));
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.selected()),
        Some(top)
    );
    ui.frame(cx, |window, cx| window.press("up", cx));
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.selected()),
        None,
        "the cursor is on the folder"
    );
    ui.frame(cx, |window, cx| window.press("enter", cx));
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.open_package()),
        Some(deploy)
    );
    ui.frame(cx, |window, cx| window.press("escape", cx));
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.open_package()),
        None
    );
    ui.finish(cx);
}
