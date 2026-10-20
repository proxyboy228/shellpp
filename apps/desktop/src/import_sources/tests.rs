//! Headless tests of the import dialog (UI 2.5; D39): the `~/.ssh/config`
//! path with its preview, a host left unchecked, the passphrase of an
//! encrypted key file and the summary; the hand-over of the JSON and
//! known_hosts sources; disabled sources; a missing file. Every profile is
//! a `TestProfile`; the config and the keys are written at run time into a
//! temporary home folder (never the real `~/.ssh`).

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, IntoElement, Render, TestAppContext,
    WindowBounds, WindowOptions, div, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use secrecy::ExposeSecret as _;

use super::*;
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
    home: tempfile::TempDir,
}

impl Ui {
    fn open(cx: &mut TestAppContext) -> Self {
        cx.update(|cx| {
            gpui_kit::init(cx);
            theme::install(Appearance::Dark, cx);
            cx.set_reduce_motion(true);
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

#[gpui_kit::test]
fn other_sources_hand_over_or_are_disabled(cx: &mut TestAppContext) {
    let ui = Ui::open(cx);
    let dialog = ui.dialog(cx);
    // Later sources cannot be selected.
    dialog.update(cx, |dialog, cx| dialog.select(Source::Csv, cx));
    assert_eq!(
        dialog.read_with(cx, |dialog, _| dialog.source()),
        Source::SshConfig
    );
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
