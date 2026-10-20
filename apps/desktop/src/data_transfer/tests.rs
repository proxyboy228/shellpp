//! Headless tests of Export Data and Import Data (GPUI `TestAppContext`,
//! no real window; D38): encrypted export and import between two temporary
//! profiles, the plain export's "I understand", password checks, a wrong
//! password, a file that is not an export. Every profile is a `TestProfile`
//! (in-memory secret store); files live in temporary directories; the save
//! and open dialogs are the test platform's.

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, IntoElement, Render,
    TestAppContext, WindowBounds, WindowOptions, div, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Map, Value, json};

use super::*;
use crate::state::AppState;
use crate::testing::TestProfile;
use crate::theme::{self, Appearance};

struct Blank;

impl Render for Blank {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div().size_full()
    }
}

struct Ui {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
}

impl Ui {
    fn open(cx: &mut TestAppContext, hosts: &[Value]) -> Self {
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::install(Appearance::Dark, cx);
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
        let (handle, _) = cx
            .update(|cx| gpui_kit::open_window(options, cx, |_, cx| cx.new(|_| Blank)))
            .unwrap();
        let ui = Self { state, handle };
        ui.frame(cx, |_, _| {});
        ui
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

    fn click(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) {
        let id = id.into();
        self.frame(cx, |window, cx| window.click(id, cx));
    }

    fn visible(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) -> bool {
        let id = id.into();
        self.frame(cx, |window, _| {
            window.try_find(id).is_some_and(|found| found.visible())
        })
    }

    fn export(&self, cx: &mut TestAppContext) -> Entity<ExportDialog> {
        let state = self.state.clone();
        let dialog = self.frame(cx, |window, cx| open_export(state, window, cx));
        self.frame(cx, |_, _| {});
        dialog
    }

    fn import(&self, cx: &mut TestAppContext) -> Entity<ImportDialog> {
        let state = self.state.clone();
        let dialog = self.frame(cx, |window, cx| open_import(state, window, cx));
        self.frame(cx, |_, _| {});
        dialog
    }

    fn hosts(&self, cx: &mut TestAppContext) -> Vec<String> {
        let core = self
            .state
            .read_with(cx, |state, _| state.core().cloned().unwrap());
        let text = core
            .data_export(Default::default())
            .unwrap()
            .text
            .expose_secret()
            .to_owned();
        let document: Value = serde_json::from_str(&text).unwrap();
        document["vaults"][0]["records"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|record| record["type"] == "host")
            .map(|record| record["data"]["address"].as_str().unwrap().to_owned())
            .collect()
    }
}

fn password() -> String {
    format!(
        "export-{}",
        shellpp_core::crypto::random_array::<8>()
            .unwrap()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>()
    )
}

#[gpui_kit::test]
fn an_encrypted_export_imports_into_another_profile(cx: &mut TestAppContext) {
    let source = Ui::open(cx, &[json!({"address": "web.example.com", "label": "web"})]);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("backup.encrypted.json");
    let secret = password();

    let export = source.export(cx);
    assert!(source.visible(cx, "data-export-dialog"));
    let typed = secret.clone();
    source.frame(cx, |window, cx| {
        export.update(cx, |dialog, cx| {
            dialog.set_passwords(&typed, &typed, window, cx)
        })
    });
    source.click(cx, "data-export-run");
    let chosen = file.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    source.frame(cx, |_, _| {});
    let written = std::fs::read_to_string(&file).expect("written");
    assert!(written.contains("shellpp.export.encrypted.v1"));
    assert!(!written.contains("web.example.com"), "nothing in the clear");
    assert!(
        !source.visible(cx, "data-export-dialog"),
        "closed after saving"
    );

    let target = Ui::open(cx, &[]);
    let import = target.import(cx);
    target.frame(cx, |window, cx| {
        import.update(cx, |dialog, cx| dialog.set_path(&file, window, cx))
    });
    target.click(cx, "data-import-run");
    assert_eq!(
        import.read_with(cx, |dialog, _| dialog.step().clone()),
        ImportStep::Password { error: None }
    );
    // A wrong password is asked again.
    target.frame(cx, |window, cx| {
        import.update(cx, |dialog, cx| {
            dialog.set_password("wrong password", window, cx)
        })
    });
    target.click(cx, "data-import-run");
    assert_eq!(
        import.read_with(cx, |dialog, _| dialog.step().clone()),
        ImportStep::Password {
            error: Some(strings::PASSWORD_WRONG)
        }
    );
    let typed = secret.clone();
    target.frame(cx, |window, cx| {
        import.update(cx, |dialog, cx| dialog.set_password(&typed, window, cx))
    });
    target.click(cx, "data-import-run");
    let ImportStep::Summary(summary) = import.read_with(cx, |dialog, _| dialog.step().clone())
    else {
        panic!("no summary");
    };
    assert_eq!(summary.added, 1);
    assert!(target.visible(cx, "data-import-summary"));
    assert_eq!(target.hosts(cx), ["web.example.com"]);
    target.click(cx, "data-import-close");
    assert!(!target.visible(cx, "data-import-dialog"));
}

#[gpui_kit::test]
fn a_plain_export_needs_i_understand(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, &[json!({"address": "db.example.com"})]);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("plain.json");
    let export = ui.export(cx);
    export.update(cx, |dialog, cx| dialog.set_encrypt(false, cx));
    assert!(ui.visible(cx, "data-export-warning"));
    assert!(!export.read_with(cx, |dialog, _| dialog.can_run()));
    // Without "I understand" nothing happens.
    ui.click(cx, "data-export-run");
    assert_eq!(
        export.read_with(cx, |dialog, _| dialog.step()),
        ExportStep::Form
    );
    export.update(cx, |dialog, cx| dialog.set_understood(true, cx));
    ui.click(cx, "data-export-run");
    let chosen = file.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    ui.frame(cx, |_, _| {});
    let written = std::fs::read_to_string(&file).expect("written");
    assert!(written.contains("shellpp.export.v1"));
    assert!(written.contains("db.example.com"));
}

#[gpui_kit::test]
fn short_and_mismatched_passwords_are_refused(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, &[]);
    let export = ui.export(cx);
    ui.frame(cx, |window, cx| {
        export.update(cx, |dialog, cx| {
            dialog.set_passwords("short", "short", window, cx)
        })
    });
    ui.click(cx, "data-export-run");
    export.read_with(cx, |dialog, _| {
        assert_eq!(dialog.step(), ExportStep::Form);
        assert_eq!(dialog.password_error, Some(strings::PASSWORD_TOO_SHORT));
    });
    ui.frame(cx, |window, cx| {
        export.update(cx, |dialog, cx| {
            dialog.set_passwords("long enough one", "long enough two", window, cx)
        })
    });
    ui.click(cx, "data-export-run");
    export.read_with(cx, |dialog, _| {
        assert_eq!(dialog.step(), ExportStep::Form);
        assert_eq!(dialog.password_error, None);
        assert_eq!(dialog.repeat_error, Some(strings::PASSWORD_MISMATCH));
    });
    // Cancelling the save dialog leaves the form as it was.
    ui.frame(cx, |window, cx| {
        export.update(cx, |dialog, cx| {
            dialog.set_passwords("long enough one", "long enough one", window, cx)
        })
    });
    ui.click(cx, "data-export-run");
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
    assert_eq!(
        export.read_with(cx, |dialog, _| dialog.step()),
        ExportStep::Form
    );
    assert!(ui.visible(cx, "data-export-dialog"));
}

#[gpui_kit::test]
fn a_file_that_is_not_an_export_is_refused(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, &[]);
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("other.json");
    std::fs::write(&file, r#"{"format":"something else"}"#).unwrap();
    let import = ui.import(cx);
    ui.click(cx, "data-import-run");
    assert_eq!(
        import.read_with(cx, |dialog, _| dialog.step().clone()),
        ImportStep::File {
            error: Some(strings::FILE_MISSING)
        }
    );
    ui.frame(cx, |window, cx| {
        import.update(cx, |dialog, cx| dialog.set_path(&file, window, cx))
    });
    ui.click(cx, "data-import-run");
    assert_eq!(
        import.read_with(cx, |dialog, _| dialog.step().clone()),
        ImportStep::File {
            error: Some(strings::FILE_NOT_EXPORT)
        }
    );
    let missing = dir.path().join("missing.json");
    ui.frame(cx, |window, cx| {
        import.update(cx, |dialog, cx| dialog.set_path(&missing, window, cx))
    });
    ui.click(cx, "data-import-run");
    assert_eq!(
        import.read_with(cx, |dialog, _| dialog.step().clone()),
        ImportStep::File {
            error: Some(strings::FILE_NOT_FOUND)
        }
    );
}
