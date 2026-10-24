//! Headless tests of the Known Hosts screen (GPUI `TestAppContext`, no real
//! window): the screen is the window's root view. Every test runs on its own
//! temporary profile (`TestProfile`: in-memory secret store, never the OS
//! keychain or the permanent profile); host keys and known_hosts files are
//! generated at run time in temporary directories, and the suggested import
//! file is always such a file (never the real `~/.ssh/known_hosts`).

use std::path::PathBuf;
use std::time::Duration;

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, ElementId, Entity, TestAppContext, Window,
    WindowBounds, WindowOptions, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Map, Value, json};
use shellpp_core::known_hosts::{encode_blob, hash_host};
use shellpp_core::service::dto::KnownHostsImportSummary;
use uuid::Uuid;

use super::*;
use crate::state::AppState;
use crate::testing::TestProfile;

/// A fresh Ed25519 host key: (key type, base64 blob).
fn host_key() -> (String, String) {
    let key: [u8; 32] = shellpp_core::crypto::random_array().expect("random");
    let mut blob = Vec::new();
    blob.extend_from_slice(&11u32.to_be_bytes());
    blob.extend_from_slice(b"ssh-ed25519");
    blob.extend_from_slice(&32u32.to_be_bytes());
    blob.extend_from_slice(&key);
    ("ssh-ed25519".to_owned(), encode_blob(&blob))
}

/// A known_hosts file of four entries: plain, bracketed with a port, hashed
/// and revoked. Returns the file and the hashed host field.
fn sample_file() -> (String, String) {
    let salt: [u8; 20] = shellpp_core::crypto::random_array().expect("random");
    let hashed = hash_host(&salt, "hidden.example.com", 22);
    let (web, db, hidden, old) = (host_key(), host_key(), host_key(), host_key());
    let file = format!(
        "web.example.com {} {} laptop\n\
         [db.example.com]:2222 {} {}\n\
         {hashed} {} {}\n\
         @revoked old.example.net {} {}\n",
        web.0, web.1, db.0, db.1, hidden.0, hidden.1, old.0, old.1
    );
    (file, hashed)
}

struct Screen {
    state: Entity<AppState>,
    screen: Entity<KnownHostsScreen>,
    handle: AnyWindowHandle,
    /// Temporary files of this test (import source, export target).
    dir: tempfile::TempDir,
}

impl Screen {
    /// A 1280 px window with the screen over a temporary profile holding
    /// the entries of `file` (imported through the core) and `hosts`.
    fn open(cx: &mut TestAppContext, file: &str, hosts: &[Value]) -> Self {
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            // Dialogs animate on the real clock: with motion on, a button's
            // position depends on timing and a click can miss it under load.
            cx.set_reduce_motion(true);
        });
        let dir = tempfile::tempdir().expect("temp dir");
        // Never the real ~/.ssh/known_hosts.
        let source = dir.path().join("known_hosts");
        cx.update(|cx| set_import_source(Some(source), cx));
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
        state.update(cx, |state, cx| {
            for host in hosts {
                let data: Map<String, Value> = host.as_object().cloned().unwrap();
                state.save_host(None, data, cx).unwrap();
            }
            if !file.is_empty() {
                state
                    .core()
                    .unwrap()
                    .known_hosts_import(file.as_bytes())
                    .unwrap();
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
                    cx.new(|cx| KnownHostsScreen::new(screen_state, window, cx))
                })
            })
            .unwrap();
        let this = Self {
            state,
            screen,
            handle,
            dir,
        };
        // The second frame lays the grid out with the measured width.
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

    fn items(&self, cx: &mut TestAppContext) -> Vec<KnownHostView> {
        self.screen
            .read_with(cx, |screen, _| screen.items().to_vec())
    }

    fn find(
        &self,
        cx: &mut TestAppContext,
        test: impl Fn(&KnownHostView) -> bool,
    ) -> KnownHostView {
        self.items(cx).into_iter().find(|view| test(view)).unwrap()
    }

    fn by_address(&self, cx: &mut TestAppContext, address: &str) -> KnownHostView {
        self.find(cx, |view| view.display_address.as_deref() == Some(address))
    }

    fn finish(self, cx: &mut TestAppContext) {
        let _ = cx.update_window(self.handle, |_, window, _| window.remove_window());
        cx.run_until_parked();
        self.state.update(cx, |state, _| state.shutdown());
    }
}

#[gpui_kit::test]
fn cards_show_addresses_hashed_entries_and_revoked_badges(cx: &mut TestAppContext) {
    let (file, hashed) = sample_file();
    let ui = Screen::open(cx, &file, &[]);
    let items = ui.items(cx);
    assert_eq!(items.len(), 4);
    for view in &items {
        assert!(ui.visible(cx, card_id(view.record_id)));
    }
    let web = ui.by_address(cx, "web.example.com");
    let db = ui.by_address(cx, "[db.example.com]:2222");
    let hidden = ui.find(cx, |view| view.hashed);
    let old = ui.find(cx, |view| view.revoked);
    // OpenSSH address forms; a hashed entry shows "Hashed host" and its type.
    assert_eq!(card_title(&web), "web.example.com");
    assert_eq!(card_title(&db), "[db.example.com]:2222");
    assert_eq!(hidden.host, hashed);
    assert_eq!(card_title(&hidden), "Hashed host");
    assert_eq!(card_subtitle(&hidden), "ssh-ed25519");
    assert!(ui.visible(cx, hashed_title_id(hidden.record_id)));
    assert!(!ui.visible(cx, hashed_title_id(web.record_id)));
    // Only the revoked key carries the badge.
    assert!(ui.visible(cx, revoked_badge_id(old.record_id)));
    for view in [&web, &db, &hidden] {
        assert!(!ui.visible(cx, revoked_badge_id(view.record_id)));
    }
    // Hashed entries come last in the address order.
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.shown_ids())
            .last(),
        Some(&hidden.record_id)
    );
    ui.finish(cx);
}

#[gpui_kit::test]
fn a_click_opens_the_details_panel(cx: &mut TestAppContext) {
    let (file, _) = sample_file();
    let ui = Screen::open(
        cx,
        &file,
        &[json!({"address": "WEB.example.com", "label": "Web front"})],
    );
    assert!(!ui.visible(cx, "kh-panel"));
    let web = ui.by_address(cx, "web.example.com");
    ui.click(cx, card_id(web.record_id));
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.selected()),
        Some(web.record_id)
    );
    for id in [
        "kh-panel",
        "kh-panel-address",
        "kh-panel-port",
        "kh-panel-key-type",
        "kh-panel-fingerprint",
        "kh-copy-fingerprint",
        "kh-panel-added",
        "kh-panel-comment",
        "kh-panel-hosts",
        "kh-panel-remove",
    ] {
        assert!(ui.visible(cx, id), "{id}");
    }
    // The saved host with this address is listed.
    assert_eq!(web.host_ids.len(), 1);
    assert!(ui.visible(cx, panel_host_id(web.host_ids[0])));
    assert_eq!(port_text(&web), "22");
    // Local time: the UTC time only in the tooltip (D68). Outside Windows
    // the zone is not read yet and the time says it is UTC (`crate::dates`).
    assert_eq!(date_text(&web.created_at).contains("UTC"), !cfg!(windows));
    assert!(
        crate::dates::timestamp_tooltip(&web.created_at).is_some_and(|utc| utc.ends_with(" UTC"))
    );

    // A hashed entry: no address, the port is in the hash; a revoked one
    // carries the badge in the panel too.
    let hidden = ui.find(cx, |view| view.hashed);
    ui.click(cx, card_id(hidden.record_id));
    assert_eq!(port_text(&hidden), "In the hash");
    assert!(!ui.visible(cx, "kh-panel-revoked"));
    let old = ui.find(cx, |view| view.revoked);
    ui.click(cx, card_id(old.record_id));
    assert!(ui.visible(cx, "kh-panel-revoked"));

    // The collapse button closes it; Esc too.
    ui.click(cx, "panel-close");
    assert!(!ui.visible(cx, "kh-panel"));
    ui.click(cx, card_id(old.record_id));
    ui.frame(cx, |window, cx| window.press("escape", cx));
    assert!(!ui.visible(cx, "kh-panel"));
    ui.finish(cx);
}

#[gpui_kit::test]
fn fingerprints_are_copied_from_the_panel_and_the_menu(cx: &mut TestAppContext) {
    let (file, _) = sample_file();
    let ui = Screen::open(cx, &file, &[]);
    let web = ui.by_address(cx, "web.example.com");
    let db = ui.by_address(cx, "[db.example.com]:2222");
    assert!(web.fingerprint.starts_with("SHA256:"));
    ui.click(cx, card_id(web.record_id));
    ui.click(cx, "kh-copy-fingerprint");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(web.fingerprint.clone())
    );
    // Right click: Copy fingerprint, Remove.
    ui.frame(cx, |window, cx| {
        window.right_click(card_id(db.record_id), cx)
    });
    assert!(ui.visible(cx, "kh-menu-copy-fingerprint"));
    assert!(ui.visible(cx, "kh-menu-remove"));
    ui.click(cx, "kh-menu-copy-fingerprint");
    assert_eq!(
        cx.read_from_clipboard().and_then(|item| item.text()),
        Some(db.fingerprint.clone())
    );
    ui.finish(cx);
}

#[gpui_kit::test]
fn remove_asks_first_and_can_be_undone(cx: &mut TestAppContext) {
    let (file, _) = sample_file();
    let ui = Screen::open(cx, &file, &[]);
    let web = ui.by_address(cx, "web.example.com");
    ui.click(cx, card_id(web.record_id));

    // Cancel keeps it.
    ui.click(cx, "kh-panel-remove");
    assert!(ui.visible(cx, "kh-remove-dialog"));
    ui.click(cx, "kh-remove-cancel");
    assert!(!ui.visible(cx, "kh-remove-dialog"));
    assert_eq!(ui.items(cx).len(), 4);

    // Remove removes it, closes the panel and offers Undo.
    ui.click(cx, "kh-panel-remove");
    ui.click(cx, "kh-remove-confirm");
    assert_eq!(ui.items(cx).len(), 3);
    assert!(!ui.visible(cx, card_id(web.record_id)));
    assert!(!ui.visible(cx, "kh-panel"));
    assert!(ui.screen.read_with(cx, |screen, _| screen.can_undo()));
    let core = ui
        .state
        .read_with(cx, |state, _| state.core().cloned().unwrap());
    assert!(
        core.known_hosts_list()
            .unwrap()
            .iter()
            .all(|view| view.record_id != web.record_id)
    );

    // Undo brings it back under the same record id.
    cx.update_window(ui.handle, |_, window, cx| {
        ui.screen
            .update(cx, |screen, cx| screen.undo_remove(window, cx))
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(ui.items(cx).len(), 4);
    assert_eq!(
        ui.by_address(cx, "web.example.com").record_id,
        web.record_id
    );
    assert!(ui.visible(cx, card_id(web.record_id)));

    // From the card menu, then the Undo window passes.
    let db = ui.by_address(cx, "[db.example.com]:2222");
    ui.frame(cx, |window, cx| {
        window.right_click(card_id(db.record_id), cx)
    });
    ui.click(cx, "kh-menu-remove");
    ui.click(cx, "kh-remove-confirm");
    assert_eq!(ui.items(cx).len(), 3);
    cx.executor()
        .advance_clock(UNDO_WINDOW + Duration::from_millis(10));
    cx.run_until_parked();
    assert!(!ui.screen.read_with(cx, |screen, _| screen.can_undo()));

    // Delete on the selected card asks too.
    let old = ui.find(cx, |view| view.revoked);
    ui.click(cx, card_id(old.record_id));
    ui.frame(cx, |window, cx| window.press("delete", cx));
    assert!(ui.visible(cx, "kh-remove-dialog"));
    ui.click(cx, "kh-remove-confirm");
    assert_eq!(ui.items(cx).len(), 2);
    // The database closes with the last core handle, so the temporary
    // profile can be removed.
    drop(core);
    ui.finish(cx);
}

/// The import dialog of the open window, once it exists.
fn dialog(ui: &Screen, cx: &mut TestAppContext) -> Entity<ImportDialog> {
    ui.screen
        .read_with(cx, |screen, _| screen.import_dialog())
        .expect("the import dialog is open")
}

#[gpui_kit::test]
fn import_reads_the_suggested_file_and_shows_the_summary(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, "", &[]);
    assert!(ui.visible(cx, "kh-empty"));
    assert!(ui.visible(cx, "kh-empty-import"));
    // Import and Export are one group: equally wide (design-system 6).
    let (import, export) = ui.frame(cx, |window, _| {
        (
            window.find("kh-import").bounds().size.width,
            window.find("kh-export").bounds().size.width,
        )
    });
    assert_eq!(import, export);
    // The suggested file, written at run time.
    let (file, _) = sample_file();
    let key = host_key();
    let source = ui.dir.path().join("known_hosts");
    std::fs::write(
        &source,
        format!(
            "{file}\
             @cert-authority *.example.com {0} {1}\n\
             web.example.com {0}\n\
             # a comment\n",
            key.0, key.1
        ),
    )
    .unwrap();

    ui.click(cx, "kh-import");
    assert!(ui.visible(cx, "kh-import-dialog"));
    let import = dialog(&ui, cx);
    assert_eq!(
        import.read_with(cx, |dialog, cx| dialog.path_text(cx)),
        source.display().to_string()
    );
    ui.click(cx, "kh-import-run");
    assert_eq!(
        import.read_with(cx, |dialog, _| dialog.step()),
        ImportStep::Summary(KnownHostsImportSummary {
            imported: 4,
            duplicates: 0,
            skipped: 1,
            invalid: 1,
        })
    );
    for id in [
        "kh-import-summary",
        "kh-import-imported",
        "kh-import-duplicates",
        "kh-import-skipped",
        "kh-import-invalid",
    ] {
        assert!(ui.visible(cx, id), "{id}");
    }
    ui.click(cx, "kh-import-close");
    assert!(!ui.visible(cx, "kh-import-dialog"));
    assert_eq!(ui.items(cx).len(), 4);
    assert!(!ui.visible(cx, "kh-empty"));

    // Again: everything is a duplicate.
    ui.click(cx, "kh-import");
    ui.click(cx, "kh-import-run");
    assert_eq!(
        dialog(&ui, cx).read_with(cx, |dialog, _| dialog.step()),
        ImportStep::Summary(KnownHostsImportSummary {
            imported: 0,
            duplicates: 4,
            skipped: 1,
            invalid: 1,
        })
    );
    ui.click(cx, "kh-import-close");
    assert_eq!(ui.items(cx).len(), 4);
    ui.finish(cx);
}

#[gpui_kit::test]
fn import_explains_a_missing_file(cx: &mut TestAppContext) {
    let ui = Screen::open(cx, "", &[]);
    // The suggested file does not exist in this test.
    ui.click(cx, "kh-import");
    ui.click(cx, "kh-import-run");
    let import = dialog(&ui, cx);
    assert_eq!(
        import.read_with(cx, |dialog, _| dialog.step()),
        ImportStep::File {
            error: Some(strings::FILE_NOT_FOUND)
        }
    );
    // An empty path asks for a file.
    let empty = PathBuf::new();
    cx.update_window(ui.handle, |_, window, cx| {
        import.update(cx, |dialog, cx| dialog.set_path(&empty, window, cx))
    })
    .unwrap();
    ui.click(cx, "kh-import-run");
    assert_eq!(
        import.read_with(cx, |dialog, _| dialog.step()),
        ImportStep::File {
            error: Some(strings::FILE_MISSING)
        }
    );
    ui.click(cx, "kh-import-cancel");
    assert!(!ui.visible(cx, "kh-import-dialog"));
    assert!(ui.items(cx).is_empty());
    ui.finish(cx);
}

#[gpui_kit::test]
fn export_writes_known_hosts_that_import_back(cx: &mut TestAppContext) {
    let (file, hashed) = sample_file();
    let ui = Screen::open(cx, &file, &[]);
    let target = ui.dir.path().join("exported");
    ui.click(cx, "kh-export");
    let chosen = target.clone();
    cx.simulate_new_path_selection(move |_| Some(chosen));
    cx.run_until_parked();
    let written = std::fs::read_to_string(&target).unwrap();
    let core = ui
        .state
        .read_with(cx, |state, _| state.core().cloned().unwrap());
    assert_eq!(written, core.known_hosts_export().unwrap());
    assert_eq!(written.lines().count(), 4);
    assert!(written.contains(&format!("{hashed} ssh-ed25519 ")));
    assert!(written.contains("@revoked old.example.net ssh-ed25519 "));
    assert!(written.contains("[db.example.com]:2222 ssh-ed25519 "));

    // Round trip: the exported file imports as four duplicates.
    cx.update(|cx| set_import_source(Some(target.clone()), cx));
    ui.click(cx, "kh-import");
    ui.click(cx, "kh-import-run");
    assert_eq!(
        dialog(&ui, cx).read_with(cx, |dialog, _| dialog.step()),
        ImportStep::Summary(KnownHostsImportSummary {
            imported: 0,
            duplicates: 4,
            skipped: 0,
            invalid: 0,
        })
    );
    ui.click(cx, "kh-import-close");
    drop(core);
    ui.finish(cx);
}

#[gpui_kit::test]
fn search_sort_and_list_layout(cx: &mut TestAppContext) {
    let (file, _) = sample_file();
    let ui = Screen::open(cx, &file, &[]);
    let web = ui.by_address(cx, "web.example.com");
    let hidden = ui.find(cx, |view| view.hashed);
    ui.frame(cx, |window, cx| {
        window.click("kh-search", cx);
        window.input("hashed", cx);
    });
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.shown_ids()),
        vec![hidden.record_id]
    );
    assert!(!ui.visible(cx, card_id(web.record_id)));
    // The fingerprint is searchable too.
    ui.frame(cx, |window, cx| {
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input(&web.fingerprint[7..20], cx);
    });
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.shown_ids()),
        vec![web.record_id]
    );
    ui.frame(cx, |window, cx| {
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
        window.input("nothing like this", cx);
    });
    assert!(ui.visible(cx, "kh-no-matches"));
    ui.frame(cx, |window, cx| {
        window.press("ctrl-a", cx);
        window.press("backspace", cx);
    });
    assert_eq!(
        ui.screen
            .read_with(cx, |screen, _| screen.shown_ids())
            .len(),
        4
    );

    // The list layout: a header over rows with the same ids.
    ui.click(cx, "kh-view-list");
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.layout()),
        KnownHostsLayout::List
    );
    assert!(ui.visible(cx, "kh-list-header"));
    assert!(ui.visible(cx, card_id(web.record_id)));
    assert!(ui.visible(cx, hashed_title_id(hidden.record_id)));
    // The choice is remembered (`view.known_hosts.layout`): a new screen
    // opens with the list.
    let state = ui.state.clone();
    let (handle, again) = cx
        .update(|cx| {
            gpui_kit::open_window(WindowOptions::default(), cx, |window, cx| {
                cx.new(|cx| KnownHostsScreen::new(state, window, cx))
            })
        })
        .unwrap();
    assert_eq!(
        again.read_with(cx, |screen, _| screen.layout()),
        KnownHostsLayout::List
    );
    let _ = cx.update_window(handle, |_, window, _| window.remove_window());
    ui.click(cx, "kh-view-tiles");
    assert!(!ui.visible(cx, "kh-list-header"));

    // Sorting by date added: newest first.
    ui.screen
        .update(cx, |screen, cx| screen.set_sort(KnownHostsSort::Added, cx));
    let items = ui.items(cx);
    let mut expected: Vec<&KnownHostView> = items.iter().collect();
    expected.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then(b.record_id.cmp(&a.record_id))
    });
    assert_eq!(
        ui.screen.read_with(cx, |screen, _| screen.shown_ids()),
        expected
            .iter()
            .map(|view| view.record_id)
            .collect::<Vec<Uuid>>()
    );
    ui.finish(cx);
}

#[gpui_kit::test]
fn the_list_follows_lock_and_changes_elsewhere(cx: &mut TestAppContext) {
    let (file, _) = sample_file();
    let ui = Screen::open(cx, &file, &[]);
    let web = ui.by_address(cx, "web.example.com");
    ui.click(cx, card_id(web.record_id));
    // A "Accept and save" elsewhere (a session's TOFU dialog) shows up.
    let core = ui
        .state
        .read_with(cx, |state, _| state.core().cloned().unwrap());
    let key = host_key();
    core.known_host_accept(&shellpp_core::service::host_api::HostKeyCheck {
        address: "new.example.org".into(),
        port: 22,
        key_type: key.0,
        public_key: key.1,
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(ui.items(cx).len(), 5);
    // Locking empties the list and hides the panel.
    ui.state.update(cx, |state, cx| state.lock(cx)).unwrap();
    cx.run_until_parked();
    assert!(ui.items(cx).is_empty());
    assert!(!ui.visible(cx, "kh-panel"));
    drop(core);
    ui.finish(cx);
}

/// The whole app in a `width` x 800 window (the shell with its sidebar,
/// toasts and this screen) over a temporary profile holding the sample
/// entries.
struct App2 {
    state: Entity<AppState>,
    handle: AnyWindowHandle,
    root: Entity<crate::app::root::RootView>,
}

impl App2 {
    fn open(cx: &mut TestAppContext, width: f32) -> Self {
        cx.executor().allow_parking();
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            crate::app::root::bind_keys(cx);
            crate::terminal::init(cx);
            set_import_source(None, cx);
            // As in `Screen::open`: no animation for the clicks to miss.
            cx.set_reduce_motion(true);
        });
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
        let (file, _) = sample_file();
        state.update(cx, |state, _| {
            state
                .core()
                .unwrap()
                .known_hosts_import(file.as_bytes())
                .unwrap();
        });
        let options = WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(Bounds::new(
                Default::default(),
                size(px(width), px(800.)),
            ))),
            ..Default::default()
        };
        let (handle, root) = cx
            .update(|cx| crate::app::open_window_with(state.clone(), options, cx))
            .unwrap();
        let app = Self {
            state,
            handle,
            root,
        };
        app.frame(cx, |_, _| {});
        app
    }

    fn frame(&self, cx: &mut TestAppContext, f: impl FnOnce(&mut Window, &mut App)) {
        cx.run_until_parked();
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            f(window, cx);
        })
        .unwrap();
        cx.run_until_parked();
    }

    fn click(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) {
        let id = id.into();
        self.frame(cx, |window, cx| window.click(id, cx));
    }

    /// The bounds of a visible element.
    fn bounds(
        &self,
        cx: &mut TestAppContext,
        id: impl Into<ElementId>,
    ) -> Option<Bounds<gpui::Pixels>> {
        let id = id.into();
        cx.update_window(self.handle, move |_, window, cx| {
            window.render_frame(cx);
            window
                .try_find(id)
                .filter(|found| found.visible())
                .map(|found| found.bounds())
        })
        .unwrap()
    }

    fn visible(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) -> bool {
        self.bounds(cx, id).is_some()
    }

    fn core(&self, cx: &mut TestAppContext) -> std::sync::Arc<shellpp_core::Core> {
        self.state
            .read_with(cx, |state, _| state.core().cloned().unwrap())
    }

    fn web(&self, cx: &mut TestAppContext) -> KnownHostView {
        self.core(cx)
            .known_hosts_list()
            .unwrap()
            .into_iter()
            .find(|view| view.display_address.as_deref() == Some("web.example.com"))
            .unwrap()
    }

    fn finish(self, cx: &mut TestAppContext) {
        self.state.update(cx, |state, _| state.shutdown());
    }
}

/// The whole window: the sidebar's Known Hosts section shows this screen
/// (not the "In development" placeholder), and Undo in the shell's toast
/// brings a removed entry back.
#[gpui_kit::test]
fn the_sidebar_section_opens_the_screen_and_the_toast_undoes(cx: &mut TestAppContext) {
    let app = App2::open(cx, 1280.);
    assert!(!app.visible(cx, "known-hosts-screen"));
    app.click(cx, "nav-known-hosts");
    assert!(app.visible(cx, "known-hosts-screen"));
    assert!(!app.visible(cx, "section-placeholder"));

    let web = app.web(cx);
    app.click(cx, card_id(web.record_id));
    assert!(app.visible(cx, "kh-panel"));
    app.click(cx, "kh-panel-remove");
    app.click(cx, "kh-remove-confirm");
    assert_eq!(app.core(cx).known_hosts_list().unwrap().len(), 3);
    assert!(app.visible(cx, "toast-undo"));
    app.click(cx, "toast-undo");
    let back = app.core(cx).known_hosts_list().unwrap();
    assert_eq!(back.len(), 4);
    assert!(back.iter().any(|view| view.record_id == web.record_id));
    assert!(app.visible(cx, card_id(web.record_id)));

    // Back to Hosts: the Hosts screen again.
    app.click(cx, "nav-hosts");
    assert!(!app.visible(cx, "known-hosts-screen"));
    app.finish(cx);
}

/// The minimum window (960 px, design-system 6): the open panel collapses
/// the sidebar into its strip of icons, as the host panel does (UI 1.2),
/// and the cards keep at least 360 px.
#[gpui_kit::test]
fn the_panel_collapses_the_sidebar_in_a_narrow_window(cx: &mut TestAppContext) {
    let app = App2::open(cx, 960.);
    app.click(cx, "nav-known-hosts");
    let sidebar = app.bounds(cx, "sidebar").unwrap();
    assert_eq!(sidebar.size.width, metrics::at_100(metrics::WIDTH_SIDEBAR));
    let web = app.web(cx);
    app.click(cx, card_id(web.record_id));
    app.frame(cx, |_, _| {});
    let sidebar = app.bounds(cx, "sidebar").unwrap();
    assert_eq!(sidebar.size.width, crate::shell::sidebar::WIDTH_COLLAPSED);
    let panel = app.bounds(cx, "kh-panel").unwrap();
    assert_eq!(panel.size.width, metrics::at_100(metrics::WIDTH_DETAILS));
    let main = app.bounds(cx, "kh-main").unwrap();
    assert!(
        main.size.width >= crate::shell::details::MAIN_AREA_MIN,
        "{main:?}"
    );
    assert!(app.visible(cx, card_id(web.record_id)));
    // Closing the panel brings the full sidebar back.
    app.click(cx, "panel-close");
    let sidebar = app.bounds(cx, "sidebar").unwrap();
    assert_eq!(sidebar.size.width, metrics::at_100(metrics::WIDTH_SIDEBAR));
    app.finish(cx);
}

/// D68: "Date added" in local time (here UTC+3) and the locale's format;
/// anything that is not a timestamp as it is.
#[test]
fn pure_texts_and_listing() {
    crate::dates::set_fixed_display(Some(crate::dates::FixedDisplay {
        offset_minutes: 180,
    }));
    assert_eq!(date_text("2026-10-20T12:34:56.789Z"), "20.10.2026 15:34");
    assert_eq!(date_text("garbage"), "garbage");
    assert_eq!(date_text(""), "");
    crate::dates::set_fixed_display(None);
}

/// A test build never suggests the real `~/.ssh/known_hosts`, even without
/// an import source and without `SHELLPP_TEST=1`.
#[gpui_kit::test]
fn a_test_build_suggests_no_real_file(cx: &mut TestAppContext) {
    cx.update(|cx| {
        assert!(cx.try_global::<ImportSource>().is_none());
        assert_eq!(default_import_path(cx), None);
    });
}

/// Writes of command history and of the connection log come with every
/// command typed on a saved host: they do not reload the list. A change of
/// the known hosts or the hosts does.
#[gpui_kit::test]
fn history_writes_do_not_reload_the_list(cx: &mut TestAppContext) {
    use shellpp_core::service::dto::{HistoryCandidate, RecordRef};
    use shellpp_core::service::host_api::{HistoryDecision, HistoryRecordInput};

    let (file, _) = sample_file();
    let ui = Screen::open(cx, &file, &[json!({"address": "web.example.com"})]);
    let reloads = |cx: &mut TestAppContext| ui.screen.read_with(cx, |screen, _| screen.reloads);
    let (core, host) = cx.update(|cx| {
        let state = ui.state.read(cx);
        let host = &state.hosts()[0];
        (
            state.core().cloned().unwrap(),
            RecordRef {
                vault_id: host.vault_id,
                record_id: host.record_id,
            },
        )
    });
    let before = reloads(cx);
    for command in ["uptime", "df -h", "ls"] {
        let decision = core
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
        assert_eq!(decision, HistoryDecision::Recorded);
        cx.run_until_parked();
    }
    assert_eq!(reloads(cx), before, "history writes do not reload");
    // A removed entry does.
    let web = ui.by_address(cx, "web.example.com");
    core.known_host_remove(RecordsRef {
        vault_id: host.vault_id,
        record_ids: vec![web.record_id],
    })
    .unwrap();
    cx.run_until_parked();
    assert_eq!(reloads(cx), before + 1);
    assert_eq!(ui.items(cx).len(), 3);
    ui.finish(cx);
}

/// A large import (several batches, one records change each) while the
/// window shows another section: the hidden screen only marks its list
/// stale and loads nothing; showing it loads once, on a background thread.
#[gpui_kit::test]
fn a_hidden_screen_loads_once_when_shown(cx: &mut TestAppContext) {
    use shellpp_core::known_hosts::openssh::IMPORT_BATCH;

    let app = App2::open(cx, 1280.);
    let screen = cx.update(|cx| {
        let shell = app.root.read(cx).shell().cloned().expect("the shell");
        shell.read(cx).known_hosts_screen().clone()
    });
    let reloads = |cx: &mut TestAppContext| screen.read_with(cx, |screen, _| screen.reloads);
    let generation = |cx: &mut TestAppContext| {
        app.state
            .read_with(cx, |state, _| state.records_generation())
    };
    assert_eq!(reloads(cx), 0, "never shown, never loaded");

    let key = host_key();
    let count = IMPORT_BATCH * 2 + 100;
    let file: String = (0..count)
        .map(|index| format!("h{index}.example.test {} {}\n", key.0, key.1))
        .collect();
    let before = generation(cx);
    let summary = app.core(cx).known_hosts_import(file.as_bytes()).unwrap();
    assert_eq!(summary.imported, count);
    app.frame(cx, |_, _| {});
    assert!(generation(cx) >= before + 3, "one records change per batch");
    assert_eq!(reloads(cx), 0, "hidden: no load per batch");

    app.click(cx, "nav-known-hosts");
    app.frame(cx, |_, _| {});
    assert_eq!(reloads(cx), 1, "shown: one load");
    assert!(!screen.read_with(cx, |screen, _| screen.is_loading()));
    assert_eq!(
        screen.read_with(cx, |screen, _| screen.items().len()),
        count + 4
    );
    // Another change while shown loads again, once.
    let web = app.web(cx);
    app.core(cx)
        .known_host_remove(RecordsRef {
            vault_id: web.vault_id,
            record_ids: vec![web.record_id],
        })
        .unwrap();
    app.frame(cx, |_, _| {});
    assert_eq!(reloads(cx), 2);
    assert_eq!(
        screen.read_with(cx, |screen, _| screen.items().len()),
        count + 3
    );
    app.finish(cx);
}
