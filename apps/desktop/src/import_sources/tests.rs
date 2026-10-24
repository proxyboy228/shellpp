//! Headless tests of the import dialog (UI 2.5; D39): the `~/.ssh/config`
//! path with its preview, a host left unchecked, the passphrase of an
//! encrypted key file and the summary; the hand-over of the JSON and
//! known_hosts sources; disabled sources; a missing file; PuTTY sessions
//! from a fake registry (D140). Every profile is a `TestProfile`; the
//! config and the keys are written at run time into a temporary home folder
//! (never the real `~/.ssh` or registry).

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, IntoElement, Render, TestAppContext,
    WindowBounds, WindowOptions, div, px, size,
};
use std::sync::Arc;

use gpui_kit::test::TestWindowExt as _;
use secrecy::ExposeSecret as _;
use shellpp_core::import::csv::Role;
use shellpp_core::import::putty::{
    FixedSessions, Session, SessionSource, Sessions, Value as PuttyValue,
};

use super::*;
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
    home: tempfile::TempDir,
}

impl Ui {
    fn open(cx: &mut TestAppContext) -> Self {
        cx.update(|cx| {
            crate::testing::init_ui(cx);
        });
        let home = tempfile::tempdir().unwrap();
        let config = home.path().join(".ssh").join("config");
        std::fs::create_dir_all(config.parent().unwrap()).unwrap();
        cx.update(|cx| {
            set_default_path(Some(config), cx);
            crate::known_hosts::set_import_source(None, cx);
        });
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
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
        let ui = Self {
            state,
            handle,
            home,
        };
        ui.frame(cx, |_, _| {});
        ui
    }

    fn ssh(&self) -> std::path::PathBuf {
        self.home.path().join(".ssh")
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

    fn click(&self, cx: &mut TestAppContext, id: &'static str) {
        self.frame(cx, |window, cx| window.click(id, cx));
    }

    fn visible(&self, cx: &mut TestAppContext, id: &'static str) -> bool {
        self.frame(cx, |window, _| {
            window.try_find(id).is_some_and(|found| found.visible())
        })
    }

    fn dialog(&self, cx: &mut TestAppContext) -> Entity<ImportSourcesDialog> {
        let state = self.state.clone();
        let dialog = self.frame(cx, |window, cx| open(state, window, cx));
        self.frame(cx, |_, _| {});
        dialog
    }
}

/// An OpenSSH private key text, encrypted with `passphrase` when given.
fn key_text(passphrase: Option<&str>) -> String {
    let seed: [u8; 32] = shellpp_core::crypto::random_array().unwrap();
    let keypair = ssh_key::private::Ed25519Keypair::from_seed(&seed);
    let key =
        ssh_key::PrivateKey::new(ssh_key::private::KeypairData::Ed25519(keypair), "test").unwrap();
    let key = match passphrase {
        Some(passphrase) => key.encrypt(&mut getrandom::SysRng, passphrase).unwrap(),
        None => key,
    };
    key.to_openssh(ssh_key::LineEnding::LF).unwrap().to_string()
}

#[gpui_kit::test]
fn a_config_is_previewed_chosen_and_imported(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let passphrase = "a passphrase for the test";
    std::fs::write(
        ui.ssh().join("config"),
        "Host web\n  HostName web.example.com\n  User deploy\n  IdentityFile ~/.ssh/id_web\n\
         Host db\n  HostName 192.0.2.44\n\
         Host legacy\n  ProxyCommand nc %h %p\n\
         Host *\n  ServerAliveInterval 30\n",
    )
    .unwrap();
    std::fs::write(ui.ssh().join("id_web"), key_text(Some(passphrase))).unwrap();

    let dialog = ui.dialog(cx);
    assert!(ui.visible(cx, "import-dialog"));
    assert!(ui.visible(cx, "import-sources"));
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.source()),
        Source::SshConfig
    );
    ui.click(cx, "import-next");
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.step().clone()),
        Step::File { error: None }
    );
    ui.click(cx, "import-next");
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.step().clone()),
        Step::Preview
    );
    assert!(ui.visible(cx, "import-preview"));
    let preview = dialog.read_with(cx, |dialog, _| dialog.preview().cloned().unwrap());
    let aliases: Vec<&str> = preview
        .hosts
        .iter()
        .map(|host| host.alias.as_str())
        .collect();
    assert_eq!(aliases, ["web", "db", "legacy"]);
    assert_eq!(preview.templates, 1);
    let key_path = preview.keys[0].path.clone();
    assert_eq!(preview.keys[0].status, KeyFileStatus::NeedsPassphrase);

    // "db" stays out; the key gets its passphrase.
    dialog.update(cx, |dialog, cx| dialog.set_chosen("db", false, cx));
    ui.frame(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| {
            dialog.set_passphrase(&key_path, passphrase, window, cx)
        })
    });
    ui.click(cx, "import-next");
    let Step::Summary(summary) = dialog.read_with(cx, |dialog, _| dialog.step().clone()) else {
        panic!("no summary");
    };
    assert_eq!(summary.added, 1);
    assert_eq!(summary.keys_added, 1);
    assert!(ui.visible(cx, "import-summary"));
    // legacy's ProxyCommand: counted, never run.
    assert_eq!(summary.commands_skipped, 1);
    assert!(ui.visible(cx, "import-commands-skipped"));
    assert!(!ui.visible(cx, "import-jump-left-out"));

    // The core holds "web" with its key; "db" and "legacy" are not there.
    let core = ui
        .state
        .read_with(cx, |state, _| state.core().cloned().unwrap());
    let export = core.data_export(Default::default()).unwrap();
    let text = export.text.expose_secret();
    assert!(text.contains("web.example.com"));
    assert!(!text.contains("192.0.2.44"));
    assert_eq!(core.keys_list().unwrap().len(), 1);
    ui.click(cx, "import-close");
    assert!(!ui.visible(cx, "import-dialog"));
}

/// D193: `ForwardAgent` from a file other than the user's own config (the
/// suggested file stands in for `~/.ssh/config`) is imported turned off and
/// counted in the summary; the user's own config keeps mapping it (D60).
#[gpui_kit::test]
fn forward_agent_turns_on_only_from_the_users_own_config(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let own = ui.ssh().join("config");
    std::fs::write(
        &own,
        "Host own\n  HostName own.example.com\n  ForwardAgent yes\n",
    )
    .unwrap();
    let other = ui.home.path().join("Downloads").join("config");
    std::fs::create_dir_all(other.parent().unwrap()).unwrap();
    std::fs::write(
        &other,
        "Host other\n  HostName other.example.com\n  ForwardAgent yes\n",
    )
    .unwrap();
    ui.frame(cx, |_, cx| {
        assert!(is_own_config(&own, cx));
        assert!(!is_own_config(&other, cx));
        assert!(!is_own_config(&ui.home.path().join("missing"), cx));
    });

    for (path, off) in [(&other, 1), (&own, 0)] {
        let dialog = ui.dialog(cx);
        ui.click(cx, "import-next");
        ui.frame(cx, |window, cx| {
            dialog.update(cx, |dialog, cx| dialog.set_path(path, window, cx))
        });
        ui.click(cx, "import-next");
        ui.click(cx, "import-next");
        let Step::Summary(summary) = dialog.read_with(cx, |dialog, _| dialog.step().clone()) else {
            panic!("no summary");
        };
        assert_eq!(summary.added, 1);
        assert_eq!(summary.hosts_agent_forwarding_off, off);
        assert_eq!(ui.visible(cx, "import-agent-off"), off > 0);
        ui.click(cx, "import-close");
    }
}

#[gpui_kit::test]
fn other_sources_hand_over(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let dialog = ui.dialog(cx);
    // A Shell++ export goes on in Import Data.
    dialog.update(cx, |dialog, cx| dialog.select(Source::Json, cx));
    ui.click(cx, "import-next");
    assert!(!ui.visible(cx, "import-dialog"));
    assert!(ui.visible(cx, "data-import-dialog"));
    ui.click(cx, "data-import-cancel");

    // known_hosts goes on in the Known Hosts import.
    let dialog = ui.dialog(cx);
    dialog.update(cx, |dialog, cx| dialog.select(Source::KnownHosts, cx));
    ui.click(cx, "import-next");
    assert!(!ui.visible(cx, "import-dialog"));
    assert!(ui.visible(cx, "kh-import-dialog"));
}

/// A registry that cannot be read.
struct Unreadable;

impl SessionSource for Unreadable {
    fn sessions(&self) -> std::io::Result<Sessions> {
        Err(std::io::Error::from(std::io::ErrorKind::PermissionDenied))
    }
}

fn putty_session(key: &str, host: &str) -> Session {
    Session {
        key: key.to_owned(),
        values: [
            ("HostName".to_owned(), PuttyValue::Text(host.to_owned())),
            ("Protocol".to_owned(), PuttyValue::Text("ssh".to_owned())),
        ]
        .into(),
    }
}

#[gpui_kit::test]
fn putty_sessions_skip_the_file_step(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    cx.update(|cx| {
        putty_registry::set_source(
            Arc::new(FixedSessions(Sessions {
                sessions: vec![
                    putty_session("Default%20Settings", ""),
                    putty_session("web%20prod", "web.example.com"),
                    putty_session("db", "192.0.2.44"),
                ],
                unreadable: Vec::new(),
                code_page: None,
            })),
            cx,
        )
    });
    let dialog = ui.dialog(cx);
    dialog.update(cx, |dialog, cx| dialog.select(Source::Putty, cx));
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.source()),
        Source::Putty
    );
    // Next goes straight to the preview: there is no file to choose.
    ui.click(cx, "import-next");
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.step().clone()),
        Step::Preview
    );
    let preview = dialog.read_with(cx, |dialog, _| dialog.preview().cloned().unwrap());
    let names: Vec<&str> = preview
        .hosts
        .iter()
        .map(|host| host.alias.as_str())
        .collect();
    assert_eq!(names, ["web prod", "db"]);
    // Back returns to the sources.
    ui.click(cx, "import-back");
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.step().clone()),
        Step::Source
    );
    ui.click(cx, "import-next");
    dialog.update(cx, |dialog, cx| dialog.set_chosen("db", false, cx));
    ui.click(cx, "import-next");
    let Step::Summary(summary) = dialog.read_with(cx, |dialog, _| dialog.step().clone()) else {
        panic!("no summary");
    };
    assert_eq!(summary.added, 1);
    let core = ui
        .state
        .read_with(cx, |state, _| state.core().cloned().unwrap());
    let export = core.data_export(Default::default()).unwrap();
    let text = export.text.expose_secret();
    assert!(text.contains("putty:web prod"));
    assert!(!text.contains("192.0.2.44"));
}

#[gpui_kit::test]
fn an_unreadable_registry_is_reported(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    cx.update(|cx| putty_registry::set_source(Arc::new(Unreadable), cx));
    let dialog = ui.dialog(cx);
    dialog.update(cx, |dialog, cx| dialog.select(Source::Putty, cx));
    ui.click(cx, "import-next");
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.step().clone()),
        Step::Source
    );
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.putty_error()),
        Some(strings::PUTTY_READ_FAILED)
    );
    // The PuTTY row, and its error with it, is shown on Windows only (UI
    // 2.5; the Linux run of run 12).
    assert_eq!(ui.visible(cx, "import-putty-error"), cfg!(windows));
}

#[gpui_kit::test]
fn a_csv_file_is_mapped_previewed_and_imported(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let file = ui.home.path().join("hosts.csv");
    // Semicolons; the first column has no known name; a password column.
    std::fs::write(
        &file,
        "Name;Host;Port;Password\nweb;web.example.com;22;s3cret-for-test\n\
         db;db.example.com;5432;\nbad;;22;\n",
    )
    .unwrap();
    let dialog = ui.dialog(cx);
    dialog.update(cx, |dialog, cx| dialog.select(Source::Csv, cx));
    ui.click(cx, "import-next");
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.step().clone()),
        Step::File { error: None }
    );
    assert!(ui.visible(cx, "import-csv-template"));
    // No file is suggested for CSV.
    ui.click(cx, "import-next");
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.step().clone()),
        Step::File {
            error: Some(strings::FILE_MISSING)
        }
    );
    ui.frame(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.set_path(&file, window, cx))
    });
    ui.click(cx, "import-next");
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.step().clone()),
        Step::Mapping
    );
    assert!(ui.visible(cx, "import-csv-mapping"));
    let csv = dialog.read_with(cx, |dialog, _| dialog.csv().cloned().unwrap());
    assert_eq!(csv.inspection.delimiter, ';');
    assert_eq!(
        csv.roles,
        [Role::Label, Role::Address, Role::Port, Role::Password]
    );
    assert!(ui.visible(cx, "import-csv-passwords"));

    // Without an address column Next is disabled and says why.
    dialog.update(cx, |dialog, cx| dialog.set_csv_role(1, Role::Notes, cx));
    assert!(ui.visible(cx, "import-csv-no-address"));
    ui.click(cx, "import-next");
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.step().clone()),
        Step::Mapping
    );
    // Mapping a role moves it: address from notes back to column 1, and
    // the password column is not imported.
    dialog.update(cx, |dialog, cx| dialog.set_csv_role(1, Role::Address, cx));
    dialog.update(cx, |dialog, cx| dialog.set_csv_role(3, Role::Ignore, cx));
    assert!(!ui.visible(cx, "import-csv-passwords"));
    ui.click(cx, "import-next");
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.step().clone()),
        Step::Preview
    );
    let preview = dialog.read_with(cx, |dialog, _| {
        dialog.csv().unwrap().preview.clone().unwrap()
    });
    let statuses: Vec<ImportStatus> = preview.rows.iter().map(|row| row.status).collect();
    assert_eq!(
        statuses,
        [ImportStatus::New, ImportStatus::New, ImportStatus::Error]
    );
    // Back goes to the mapping, then forward again.
    ui.click(cx, "import-back");
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.step().clone()),
        Step::Mapping
    );
    ui.click(cx, "import-next");
    ui.click(cx, "import-next");
    let Step::CsvSummary(summary) = dialog.read_with(cx, |dialog, _| dialog.step().clone()) else {
        panic!("no summary");
    };
    assert_eq!(summary.added, 2);
    assert_eq!(summary.errors.len(), 1);
    assert_eq!(summary.passwords, 0);
    assert!(ui.visible(cx, "import-summary"));
    let core = ui
        .state
        .read_with(cx, |state, _| state.core().cloned().unwrap());
    let export = core.data_export(Default::default()).unwrap();
    let text = export.text.expose_secret();
    assert!(text.contains("db.example.com"));
    assert!(!text.contains("s3cret-for-test"));
    ui.click(cx, "import-close");
    assert!(!ui.visible(cx, "import-dialog"));
}

#[gpui_kit::test]
fn the_csv_template_is_written(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let dialog = ui.dialog(cx);
    let file = ui.home.path().join("template.csv");
    ui.frame(cx, |window, cx| {
        dialog.update(cx, |dialog, cx| dialog.write_template(&file, window, cx))
    });
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.starts_with("label,address,port,username,group,tags,notes"));
}

#[gpui_kit::test]
fn a_missing_file_is_reported(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let dialog = ui.dialog(cx);
    ui.click(cx, "import-next");
    // No config written: the suggested file does not exist.
    ui.click(cx, "import-next");
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.step().clone()),
        Step::File {
            error: Some(strings::FILE_NOT_FOUND)
        }
    );
    // Back to the sources.
    ui.click(cx, "import-back");
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.step().clone()),
        Step::Source
    );
}

#[test]
fn key_paths_under_the_home_folder_are_written_with_a_tilde() {
    let home = std::path::Path::new(if cfg!(windows) {
        r"C:\Users\someone"
    } else {
        "/home/someone"
    });
    let sep = std::path::MAIN_SEPARATOR;
    let inside = format!("{}{sep}.ssh/id_web", home.display());
    assert_eq!(tilde(&inside, home), "~/.ssh/id_web");
    // Another folder that only starts with the same text stays whole.
    let beside = format!("{}2{sep}.ssh/id_web", home.display());
    assert_eq!(tilde(&beside, home), beside);
    assert_eq!(tilde("/etc/ssh/key", home), "/etc/ssh/key");
}

#[test]
fn the_csv_counts_agree_with_their_numbers() {
    assert_eq!(
        strings::csv_counts(3, 1, 1, 1),
        "3 new, 1 already exists, 1 not imported; 1 new group"
    );
    assert_eq!(
        strings::csv_counts(1, 2, 0, 2),
        "1 new, 2 already exist, 0 not imported; 2 new groups"
    );
    assert_eq!(
        strings::csv_counts(0, 0, 0, 0),
        "0 new, 0 already exist, 0 not imported"
    );
}
