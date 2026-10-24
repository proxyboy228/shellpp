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
            crate::testing::init_ui(cx);
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

/// Export Data opens encrypted with a password: the password and its repeat
/// show, the clear-text warning does not (UI 1.4; run 3 polish).
#[gpui_kit::test]
fn the_export_is_encrypted_by_default(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, &[json!({"address": "web.example.com"})]);
    let export = ui.export(cx);
    assert!(export.read_with(cx, |dialog, _| dialog.encrypt()));
    assert!(ui.visible(cx, "data-export-password"));
    assert!(ui.visible(cx, "data-export-repeat"));
    assert!(!ui.visible(cx, "data-export-warning"));
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
    assert!(
        shellpp_private_file::is_private(&file).unwrap(),
        "only the user can read the clear-text file"
    );

    // Saving over an existing file that others can read makes it private
    // too (D42).
    std::fs::remove_file(&file).unwrap();
    std::fs::write(&file, "an older file").unwrap();
    assert!(!shellpp_private_file::is_private(&file).unwrap());
    let export = ui.export(cx);
    export.update(cx, |dialog, cx| dialog.set_encrypt(false, cx));
    export.update(cx, |dialog, cx| dialog.set_understood(true, cx));
    ui.click(cx, "data-export-run");
    let chosen = file.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    ui.frame(cx, |_, _| {});
    assert!(
        std::fs::read_to_string(&file)
            .unwrap()
            .contains("db.example.com")
    );
    assert!(shellpp_private_file::is_private(&file).unwrap());
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

#[gpui_kit::test]
fn the_summary_names_the_settings_kept_here(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, &[]);
    let core = ui
        .state
        .read_with(cx, |state, _| state.core().cloned().unwrap());
    let text = core
        .data_export(Default::default())
        .unwrap()
        .text
        .expose_secret()
        .to_owned();
    let mut document: Value = serde_json::from_str(&text).unwrap();
    // A script for every server: never applied from a file (D41).
    document["settings"]["terminal.os_detection_script"] = json!("echo from the file");
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("shared.json");
    std::fs::write(&file, document.to_string()).unwrap();
    let import = ui.import(cx);
    ui.frame(cx, |window, cx| {
        import.update(cx, |dialog, cx| dialog.set_path(&file, window, cx))
    });
    ui.click(cx, "data-import-run");
    let ImportStep::Summary(summary) = import.read_with(cx, |dialog, _| dialog.step().clone())
    else {
        panic!("no summary");
    };
    assert_eq!(summary.settings_left_out, 1);
    assert!(ui.visible(cx, "data-import-settings-left-out"));
}

#[test]
fn the_format_is_read_without_the_rest_of_the_file() {
    assert_eq!(
        encrypted(r#"{"vaults":[{"records":[]}],"format":"shellpp.export.v1"}"#),
        Some(false)
    );
    assert_eq!(
        encrypted(r#"{"format":"shellpp.export.encrypted.v1","kdf":{}}"#),
        Some(true)
    );
    assert_eq!(encrypted(r#"{"format":"other"}"#), None);
    assert_eq!(encrypted(r#"{"vaults":[]}"#), None);
    assert_eq!(encrypted("not json"), None);
    // A file that is not UTF-8 is not an export.
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("binary.json");
    std::fs::write(&file, [0xff, 0xfe, 0x00, 0x7b]).unwrap();
    assert_eq!(read_file(&file).err(), Some(strings::FILE_NOT_EXPORT));
}

/// OpenSSH files (D43): the config and the key file land in the chosen
/// folder, the key encrypted with the passphrase and readable by the user
/// only; a second export into the same folder asks before replacing.
#[gpui_kit::test]
fn openssh_files_are_written_into_the_chosen_folder(cx: &mut TestAppContext) {
    use shellpp_core::service::dto::KeyGenerateInput;
    let ui = Ui::open(cx, &[json!({"address": "web.example.com", "label": "web"})]);
    let core = ui
        .state
        .read_with(cx, |state, _| state.core().cloned().unwrap());
    let vault_id = ui.state.read_with(cx, |state, _| state.vault_id().unwrap());
    core.key_generate(KeyGenerateInput {
        vault_id,
        label: "deploy".into(),
        kind: Default::default(),
        passphrase: None,
        save_passphrase: false,
    })
    .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let passphrase = password();

    let export = ui.export(cx);
    export.update(cx, |dialog, cx| {
        dialog.set_format(ExportFormat::OpenSsh, cx)
    });
    assert!(ui.visible(cx, "data-export-keys"));
    // A passphrase that is too short is refused before any folder is asked.
    ui.frame(cx, |window, cx| {
        export.update(cx, |dialog, cx| {
            dialog.set_passwords("short", "short", window, cx)
        })
    });
    ui.click(cx, "data-export-run");
    assert_eq!(
        export.read_with(cx, |dialog, _| dialog.step()),
        ExportStep::Form
    );
    assert!(!cx.did_prompt_for_paths());
    let typed = passphrase.clone();
    ui.frame(cx, |window, cx| {
        export.update(cx, |dialog, cx| {
            dialog.set_passwords(&typed, &typed, window, cx)
        })
    });
    ui.click(cx, "data-export-run");
    let chosen = dir.path().to_path_buf();
    cx.simulate_path_prompt_response(move |options| {
        assert!(options.directories && !options.files);
        Some(vec![chosen])
    });
    cx.run_until_parked();
    ui.frame(cx, |_, _| {});
    assert!(
        !ui.visible(cx, "data-export-dialog"),
        "closed after writing"
    );
    let config = std::fs::read_to_string(dir.path().join("shellpp-ssh-config")).unwrap();
    assert!(config.contains("HostName web.example.com"), "{config}");
    let key_file = dir.path().join("shellpp_deploy");
    let key = ssh_key::PrivateKey::from_openssh(std::fs::read(&key_file).unwrap()).unwrap();
    assert!(key.is_encrypted(), "the passphrase protects the key file");
    assert!(key.decrypt(passphrase.as_bytes()).is_ok());
    assert!(shellpp_private_file::is_private(&key_file).unwrap());

    // The same folder again: nothing is replaced before "Replace".
    std::fs::write(dir.path().join("shellpp-ssh-config"), "# mine\n").unwrap();
    let export = ui.export(cx);
    export.update(cx, |dialog, cx| {
        dialog.set_format(ExportFormat::OpenSsh, cx);
        dialog.set_include_keys(false, cx);
    });
    assert!(
        !ui.visible(cx, "data-export-encrypt"),
        "no passphrase without key files"
    );
    ui.click(cx, "data-export-run");
    let chosen = dir.path().to_path_buf();
    cx.simulate_path_prompt_response(move |_| Some(vec![chosen]));
    cx.run_until_parked();
    assert_eq!(
        export.read_with(cx, |dialog, _| dialog.step()),
        ExportStep::Replace
    );
    assert!(ui.visible(cx, "data-export-replace-text"));
    assert_eq!(
        std::fs::read_to_string(dir.path().join("shellpp-ssh-config")).unwrap(),
        "# mine\n"
    );
    ui.click(cx, "data-export-replace");
    cx.run_until_parked();
    ui.frame(cx, |_, _| {});
    let config = std::fs::read_to_string(dir.path().join("shellpp-ssh-config")).unwrap();
    assert!(config.contains("HostName web.example.com"));
    assert!(!ui.visible(cx, "data-export-dialog"));
}

/// Records one command of a local terminal (no host) in `ui`'s profile.
fn record_command(ui: &Ui, cx: &mut TestAppContext, command: &str) {
    use shellpp_core::service::dto::HistoryCandidate;
    use shellpp_core::service::host_api::{HistoryDecision, HistoryRecordInput};
    let core = ui
        .state
        .read_with(cx, |state, _| state.core().cloned().unwrap());
    let decision = core
        .history_record(HistoryRecordInput {
            host: None,
            session_id: uuid::Uuid::now_v7(),
            candidate: HistoryCandidate {
                command: command.to_owned(),
                prompt_text: "me@here:~$".to_owned(),
                typed_printable: true,
                alt_screen: false,
            },
            injected: false,
            record_history: true,
        })
        .unwrap();
    assert_eq!(decision, HistoryDecision::Recorded);
}

/// D50 and D51: history only in an export that asks for it; Import Data
/// then asks whether to take it (on), and the full restore warns first.
#[gpui_kit::test]
fn history_travels_only_when_asked_for(cx: &mut TestAppContext) {
    let source = Ui::open(cx, &[json!({"address": "web.example.com"})]);
    record_command(&source, cx, "uptime");
    let dir = tempfile::tempdir().unwrap();
    let export_to = |ui: &Ui, cx: &mut TestAppContext, file: &Path, history: bool| {
        let export = ui.export(cx);
        assert!(ui.visible(cx, "data-export-history"));
        assert!(ui.visible(cx, "data-export-connection-log"));
        if history {
            ui.click(cx, "data-export-history");
        }
        export.update(cx, |dialog, cx| dialog.set_encrypt(false, cx));
        export.update(cx, |dialog, cx| dialog.set_understood(true, cx));
        ui.click(cx, "data-export-run");
        let chosen = file.to_path_buf();
        cx.simulate_new_path_selection(move |_| Some(chosen));
        cx.run_until_parked();
        ui.frame(cx, |_, _| {});
    };
    let without = dir.path().join("without.json");
    export_to(&source, cx, &without, false);
    assert!(
        !std::fs::read_to_string(&without)
            .unwrap()
            .contains("uptime")
    );
    let with = dir.path().join("with.json");
    export_to(&source, cx, &with, true);
    assert!(std::fs::read_to_string(&with).unwrap().contains("uptime"));

    // A file without history goes straight to the summary.
    let target = Ui::open(cx, &[]);
    let import = target.import(cx);
    target.frame(cx, |window, cx| {
        import.update(cx, |dialog, cx| dialog.set_path(&without, window, cx))
    });
    target.click(cx, "data-import-run");
    assert!(matches!(
        import.read_with(cx, |dialog, _| dialog.step().clone()),
        ImportStep::Summary(_)
    ));
    target.click(cx, "data-import-close");

    // The full restore warns before anything runs.
    let import = target.import(cx);
    assert!(!target.visible(cx, "data-import-full-restore-warning"));
    target.click(cx, "data-import-full-restore");
    assert!(target.visible(cx, "data-import-full-restore-warning"));
    target.click(cx, "data-import-full-restore");
    assert!(!target.visible(cx, "data-import-full-restore-warning"));

    // A file with history asks, with the history checked.
    target.frame(cx, |window, cx| {
        import.update(cx, |dialog, cx| dialog.set_path(&with, window, cx))
    });
    target.click(cx, "data-import-run");
    assert_eq!(
        import.read_with(cx, |dialog, _| dialog.step().clone()),
        ImportStep::Contents(DataImportContents {
            history: 1,
            connection_log: 0
        })
    );
    assert!(target.visible(cx, "data-import-history"));
    assert!(!target.visible(cx, "data-import-connection-log"));
    assert!(import.read_with(cx, |dialog, _| dialog.import_history));
    target.click(cx, "data-import-run");
    assert!(matches!(
        import.read_with(cx, |dialog, _| dialog.step().clone()),
        ImportStep::Summary(_)
    ));
    let core = target
        .state
        .read_with(cx, |state, _| state.core().cloned().unwrap());
    let listed = core.history_list(Default::default()).unwrap();
    assert_eq!(
        listed
            .iter()
            .map(|entry| entry.command.as_str())
            .collect::<Vec<_>>(),
        ["uptime"]
    );
}

/// Run 10 (UI 14): the export of Recently deleted offers the JSON formats
/// only; "OpenSSH files" would write the vault's hosts and keys, not the
/// trash's, so it is neither shown nor taken.
#[gpui_kit::test]
fn the_export_of_recently_deleted_offers_json_only(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, &[json!({"address": "web.example.com"})]);
    let export = ui.export(cx);
    assert!(
        ui.visible(cx, "data-export-format-openssh"),
        "an ordinary export"
    );
    ui.frame(cx, |window, cx| {
        gpui_kit::component::WindowExt::close_dialog(window, cx)
    });
    let export_of_trash = ui.export(cx);
    export_of_trash.update(cx, |dialog, cx| {
        dialog.recently_deleted = vec![uuid::Uuid::now_v7()];
        dialog.set_format(super::ExportFormat::OpenSsh, cx);
    });
    assert_eq!(
        export_of_trash.read_with(cx, |dialog, _| dialog.format()),
        super::ExportFormat::Data
    );
    assert!(!ui.visible(cx, "data-export-format-openssh"));
    assert!(ui.visible(cx, "data-export-format-data"));
    assert!(!ui.visible(cx, "data-export-history"));
    assert!(!ui.visible(cx, "data-export-connection-log"));
    drop(export);
}
