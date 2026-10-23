//! The SFTP tab in GPUI tests (stage 4): the panels on temporary local
//! folders (sorting, the filter, hidden files, "..", back and forward, the
//! selection), New Folder, Rename and Delete, copying from one panel into
//! the other through the transfer queue with a name conflict, the empty
//! panel's host selection, and the pure helpers. A remote panel is the
//! session layer's SFTP session; `tests/sessions.rs` covers it against
//! OpenSSH.

use std::path::Path;

use gpui::{
    AnyWindowHandle, AppContext as _, Bounds, Entity, TestAppContext, WindowBounds, WindowOptions,
    px, size,
};
use gpui_kit::test::TestWindowExt as _;
use serde_json::{Map, Value, json};

use super::listing::Column;
use super::menu::parse_octal;
use super::panel::{FilePanel, Side, date_text, valid_name};
use super::queue::Transfers;
use super::screen::SftpScreen;
use crate::state::AppState;
use crate::state::selection::SelectMode;
use crate::testing::TestProfile;

struct Ui {
    state: Entity<AppState>,
    screen: Entity<SftpScreen>,
    handle: AnyWindowHandle,
    password: secrecy::SecretString,
}

impl Ui {
    fn open(cx: &mut TestAppContext, hosts: &[Value]) -> Self {
        cx.update(crate::testing::init_ui);
        // Local listings and copies run on real threads.
        cx.executor().allow_parking();
        let profile = TestProfile::new();
        let password = profile.password().clone();
        let state = cx.new(|cx| AppState::new(profile.into_unlocked(), cx));
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
        let mut screen = None;
        let (handle, _) = cx
            .update(|cx| {
                gpui_kit::open_window(options, cx, |window, cx| {
                    let made = cx.new(|cx| SftpScreen::new(state.clone(), window, cx));
                    screen = Some(made.clone());
                    made
                })
            })
            .unwrap();
        let ui = Self {
            state,
            screen: screen.unwrap(),
            handle,
            password,
        };
        ui.settle(cx);
        ui
    }

    fn settle(&self, cx: &mut TestAppContext) {
        for _ in 0..3 {
            cx.run_until_parked();
            cx.update_window(self.handle, |_, window, cx| window.render_frame(cx))
                .unwrap();
            // Local listings and copies finish on real threads.
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        cx.run_until_parked();
    }

    fn panel(&self, cx: &mut TestAppContext, side: Side) -> Entity<FilePanel> {
        self.screen
            .read_with(cx, |screen, _| screen.panel(side).clone())
    }

    fn transfers(&self, cx: &mut TestAppContext) -> Entity<Transfers> {
        self.screen
            .read_with(cx, |screen, _| screen.transfers().clone())
    }

    fn names(&self, cx: &mut TestAppContext, side: Side) -> Vec<String> {
        let panel = self.panel(cx, side);
        panel.read_with(cx, |panel, _| {
            panel.rows().iter().map(|row| row.name.clone()).collect()
        })
    }

    fn update_panel<R>(
        &self,
        cx: &mut TestAppContext,
        side: Side,
        f: impl FnOnce(&mut FilePanel, &mut gpui::Window, &mut gpui::Context<FilePanel>) -> R,
    ) -> R {
        let panel = self.panel(cx, side);
        let result = cx
            .update_window(self.handle, |_, window, cx| {
                panel.update(cx, |panel, cx| f(panel, window, cx))
            })
            .unwrap();
        self.settle(cx);
        result
    }

    fn open_local(&self, cx: &mut TestAppContext, side: Side, dir: &Path) {
        let dir = dir.to_path_buf();
        self.update_panel(cx, side, |panel, _, cx| panel.open_local(dir, cx));
        // The listing ends on a background thread.
        for _ in 0..50 {
            let panel = self.panel(cx, side);
            if panel.read_with(cx, |panel, _| *panel.load() == super::panel::Load::Idle) {
                break;
            }
            self.settle(cx);
        }
    }

    fn click(&self, cx: &mut TestAppContext, id: &str) {
        let id = gpui::SharedString::from(id.to_owned());
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.click(id, cx);
        })
        .unwrap();
        self.settle(cx);
    }

    fn visible(&self, cx: &mut TestAppContext, id: &str) -> bool {
        let id = gpui::SharedString::from(id.to_owned());
        cx.update_window(self.handle, |_, window, cx| {
            window.render_frame(cx);
            window.try_find(id).is_some_and(|found| found.visible())
        })
        .unwrap()
    }

    /// Waits until `condition` holds (real threads copy).
    fn until(
        &self,
        cx: &mut TestAppContext,
        what: &str,
        mut condition: impl FnMut(&mut TestAppContext) -> bool,
    ) {
        for _ in 0..200 {
            if condition(cx) {
                return;
            }
            self.settle(cx);
        }
        panic!("timed out waiting for {what}");
    }
}

fn folder() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::write(root.join("b.txt"), b"0123456789").unwrap();
    std::fs::write(root.join("a10.log"), b"x").unwrap();
    std::fs::write(root.join("a2.log"), b"xyz").unwrap();
    std::fs::write(root.join(".hidden"), b"").unwrap();
    std::fs::create_dir(root.join("docs")).unwrap();
    std::fs::write(root.join("docs").join("inside.md"), b"# doc").unwrap();
    // On Windows a dot-file is not hidden: mark it so with the attribute.
    #[cfg(windows)]
    {
        let _ = std::process::Command::new("attrib")
            .arg("+h")
            .arg(root.join(".hidden"))
            .status();
    }
    dir
}

#[gpui_kit::test]
fn the_tab_starts_with_local_files_and_an_empty_panel(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, &[json!({"address": "web.example.com", "label": "web"})]);
    let left = ui.panel(cx, Side::Left);
    assert!(left.read_with(cx, |panel, _| panel.is_local()));
    assert!(ui.visible(cx, "sftp-right-empty"));
    assert!(ui.visible(cx, "sftp-right-choose-host"));
    assert!(
        !ui.visible(cx, "sftp-queue"),
        "the queue appears with a transfer"
    );

    // The host selection lists the saved hosts and searches them.
    ui.click(cx, "sftp-right-choose-host");
    assert!(ui.visible(cx, "sftp-right-picker"));
    let right = ui.panel(cx, Side::Right);
    let hosts = cx.update(|cx| right.read(cx).picker_hosts(cx));
    assert_eq!(hosts.len(), 1);
    assert_eq!(hosts[0].1.as_ref(), "web");
    ui.click(cx, "sftp-right-picker-cancel");
    assert!(ui.visible(cx, "sftp-right-choose-host"));
    let _ = &ui.state;
}

#[gpui_kit::test]
fn a_local_panel_sorts_filters_and_navigates(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, &[]);
    let dir = folder();
    ui.open_local(cx, Side::Left, dir.path());
    // Folders first, natural order, hidden files left out.
    assert_eq!(
        ui.names(cx, Side::Left),
        ["docs", "a2.log", "a10.log", "b.txt"]
    );
    assert!(ui.visible(cx, "sftp-left-parent"), "the .. row");

    ui.update_panel(cx, Side::Left, |panel, _, cx| {
        panel.set_show_hidden(true, cx)
    });
    assert!(ui.names(cx, Side::Left).contains(&".hidden".to_owned()));
    ui.update_panel(cx, Side::Left, |panel, _, cx| {
        panel.set_show_hidden(false, cx)
    });

    // Size, descending after a second click; folders stay first.
    ui.click(cx, "sftp-left-column-size");
    assert_eq!(
        ui.names(cx, Side::Left),
        ["docs", "a10.log", "a2.log", "b.txt"]
    );
    ui.click(cx, "sftp-left-column-size");
    assert_eq!(
        ui.names(cx, Side::Left),
        ["docs", "b.txt", "a2.log", "a10.log"]
    );
    ui.update_panel(cx, Side::Left, |panel, _, cx| {
        panel.set_sort(Column::Name, cx)
    });

    // A double click on a folder opens it; Back returns.
    let docs = ui.panel(cx, Side::Left).read_with(cx, |panel, _| {
        panel
            .rows()
            .iter()
            .find(|row| row.name == "docs")
            .cloned()
            .unwrap()
    });
    ui.update_panel(cx, Side::Left, |panel, _, cx| panel.open_entry(&docs, cx));
    ui.until(cx, "docs listed", |cx| {
        ui.names(cx, Side::Left) == ["inside.md"]
    });
    let path = ui
        .panel(cx, Side::Left)
        .read_with(cx, |panel, _| panel.path().to_owned());
    assert!(path.ends_with("docs"), "{path}");
    ui.click(cx, "sftp-left-back");
    ui.until(cx, "back", |cx| ui.names(cx, Side::Left).len() == 4);
    ui.click(cx, "sftp-left-forward");
    ui.until(cx, "forward", |cx| {
        ui.names(cx, Side::Left) == ["inside.md"]
    });
    ui.update_panel(cx, Side::Left, |panel, _, cx| panel.up(cx));
    ui.until(cx, "up", |cx| ui.names(cx, Side::Left).len() == 4);
}

#[gpui_kit::test]
fn the_selection_follows_ctrl_and_shift(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, &[]);
    let dir = folder();
    ui.open_local(cx, Side::Left, dir.path());
    let selected = |ui: &Ui, cx: &mut TestAppContext| -> Vec<String> {
        ui.panel(cx, Side::Left).read_with(cx, |panel, _| {
            panel
                .selection()
                .into_iter()
                .map(|entry| entry.name)
                .collect()
        })
    };
    // A plain click on a row selects it.
    ui.click(cx, "sftp-left-row-a2.log");
    assert_eq!(selected(&ui, cx), ["a2.log"]);
    ui.update_panel(cx, Side::Left, |panel, _, cx| {
        panel.click("b.txt", SelectMode::Range, cx)
    });
    assert_eq!(selected(&ui, cx), ["a2.log", "a10.log", "b.txt"]);
    ui.update_panel(cx, Side::Left, |panel, _, cx| {
        panel.click("a10.log", SelectMode::Toggle, cx)
    });
    assert_eq!(selected(&ui, cx), ["a2.log", "b.txt"]);
    ui.update_panel(cx, Side::Left, |panel, _, cx| panel.select_all(cx));
    assert_eq!(selected(&ui, cx).len(), 4);
}

/// Plan of run 4, item 7.4: the file list works from the keyboard; the
/// arrows move the selection (Shift extends it), Enter opens a folder,
/// Backspace goes up, Ctrl+A selects all.
#[gpui_kit::test]
fn the_file_list_works_from_the_keyboard(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, &[]);
    let dir = folder();
    ui.open_local(cx, Side::Left, dir.path());
    let selected = |ui: &Ui, cx: &mut TestAppContext| -> Vec<String> {
        ui.panel(cx, Side::Left).read_with(cx, |panel, _| {
            panel
                .selection()
                .into_iter()
                .map(|entry| entry.name)
                .collect()
        })
    };
    let press = |ui: &Ui, key: &str, cx: &mut TestAppContext| {
        cx.update_window(ui.handle, |_, window, cx| {
            window.render_frame(cx);
            window.press(key, cx);
        })
        .unwrap();
        ui.settle(cx);
    };
    // A click focuses the list.
    ui.click(cx, "sftp-left-row-a2.log");
    press(&ui, "down", cx);
    assert_eq!(selected(&ui, cx), ["a10.log"]);
    press(&ui, "shift-down", cx);
    assert_eq!(selected(&ui, cx), ["a10.log", "b.txt"]);
    press(&ui, "down", cx);
    assert_eq!(selected(&ui, cx), ["b.txt"], "the last row stays");
    press(&ui, "ctrl-a", cx);
    assert_eq!(selected(&ui, cx).len(), 4);
    for _ in 0..3 {
        press(&ui, "up", cx);
    }
    assert_eq!(selected(&ui, cx), ["docs"]);
    press(&ui, "enter", cx);
    ui.until(cx, "docs listed", |cx| {
        ui.names(cx, Side::Left) == ["inside.md"]
    });
    press(&ui, "backspace", cx);
    ui.until(cx, "up", |cx| ui.names(cx, Side::Left).len() == 4);
}

#[gpui_kit::test]
fn new_folder_rename_and_delete_in_a_local_panel(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, &[]);
    let dir = folder();
    ui.open_local(cx, Side::Left, dir.path());
    ui.update_panel(cx, Side::Left, |panel, window, cx| {
        panel.new_folder("made", window, cx)
    });
    ui.until(cx, "the new folder", |cx| {
        ui.names(cx, Side::Left).contains(&"made".to_owned())
    });
    assert!(dir.path().join("made").is_dir());
    // A name with a separator is refused before anything happens.
    ui.update_panel(cx, Side::Left, |panel, window, cx| {
        panel.new_folder("a/b", window, cx)
    });
    assert!(!dir.path().join("a").exists());

    ui.update_panel(cx, Side::Left, |panel, window, cx| {
        panel.rename("b.txt", "c.txt", window, cx)
    });
    ui.until(cx, "the rename", |cx| {
        ui.names(cx, Side::Left).contains(&"c.txt".to_owned())
    });
    assert!(dir.path().join("c.txt").is_file() && !dir.path().join("b.txt").exists());

    let doomed = ui.panel(cx, Side::Left).read_with(cx, |panel, _| {
        panel
            .rows()
            .iter()
            .filter(|row| row.name == "docs" || row.name == "c.txt")
            .cloned()
            .collect::<Vec<_>>()
    });
    ui.update_panel(cx, Side::Left, |panel, window, cx| {
        panel.delete(doomed, window, cx)
    });
    ui.until(cx, "the delete", |cx| {
        let names = ui.names(cx, Side::Left);
        !names.contains(&"docs".to_owned()) && !names.contains(&"c.txt".to_owned())
    });
    assert!(!dir.path().join("docs").exists());
}

/// Run 8, C5 (security.md 8): a name conflict of a transfer that runs while
/// the app is locked asks only after the unlock; nothing opens over the
/// lock screen.
#[gpui_kit::test]
fn a_name_conflict_waits_while_the_app_is_locked(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, &[]);
    let source = folder();
    let target = tempfile::tempdir().unwrap();
    std::fs::write(target.path().join("b.txt"), b"older").unwrap();
    ui.open_local(cx, Side::Left, source.path());
    ui.open_local(cx, Side::Right, target.path());
    let entries: Vec<_> = ui.panel(cx, Side::Left).read_with(cx, |panel, _| {
        panel
            .rows()
            .iter()
            .filter(|row| row.name == "b.txt")
            .cloned()
            .collect()
    });
    ui.state.update(cx, |state, cx| state.lock(cx)).unwrap();
    ui.settle(cx);
    cx.update_window(ui.handle, |_, window, cx| {
        ui.screen.update(cx, |screen, cx| {
            screen.copy(Side::Left, entries, window, cx)
        })
    })
    .unwrap();
    let transfers = ui.transfers(cx);
    ui.until(cx, "the conflict", |cx| {
        transfers.read_with(cx, |transfers, _| {
            transfers
                .transfers()
                .iter()
                .any(|transfer| transfer.state == shellpp_ssh::transfer_queue::State::Conflict)
        })
    });
    ui.settle(cx);
    assert!(!ui.visible(cx, "sftp-conflict"), "a dialog over the lock");

    let password = ui.password.clone();
    ui.state
        .update(cx, |state, cx| state.unlock(password, cx))
        .detach();
    ui.until(cx, "the conflict dialog after the unlock", |cx| {
        ui.visible(cx, "sftp-conflict")
    });
    ui.click(cx, "sftp-conflict-rename");
    ui.until(cx, "the copy under a new name", |_| {
        target.path().join("b (2).txt").is_file()
    });
}

#[gpui_kit::test]
fn copying_between_panels_goes_through_the_queue(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, &[]);
    let source = folder();
    let target = tempfile::tempdir().unwrap();
    ui.open_local(cx, Side::Left, source.path());
    ui.open_local(cx, Side::Right, target.path());
    let entries = ui.panel(cx, Side::Left).read_with(cx, |panel, _| {
        panel
            .rows()
            .iter()
            .filter(|row| row.name == "docs" || row.name == "b.txt")
            .cloned()
            .collect::<Vec<_>>()
    });
    cx.update_window(ui.handle, |_, window, cx| {
        ui.screen.update(cx, |screen, cx| {
            screen.copy(Side::Left, entries.clone(), window, cx)
        })
    })
    .unwrap();
    ui.until(cx, "the copy", |_| {
        target.path().join("b.txt").is_file()
            && target.path().join("docs").join("inside.md").is_file()
    });
    assert_eq!(
        std::fs::read(target.path().join("b.txt")).unwrap(),
        b"0123456789"
    );
    assert!(ui.visible(cx, "sftp-queue"));
    ui.until(cx, "the target listed again", |cx| {
        ui.names(cx, Side::Right) == ["docs", "b.txt"]
    });

    // The same file again: the name conflict is asked; Keep both.
    let again: Vec<_> = entries
        .into_iter()
        .filter(|row| row.name == "b.txt")
        .collect();
    cx.update_window(ui.handle, |_, window, cx| {
        ui.screen
            .update(cx, |screen, cx| screen.copy(Side::Left, again, window, cx))
    })
    .unwrap();
    ui.until(cx, "the conflict dialog", |cx| {
        ui.visible(cx, "sftp-conflict")
    });
    ui.click(cx, "sftp-conflict-rename");
    ui.until(cx, "the copy under a new name", |_| {
        target.path().join("b (2).txt").is_file()
    });
    let transfers = ui.transfers(cx);
    ui.until(cx, "every transfer done", |cx| {
        transfers.read_with(cx, |transfers, _| transfers.active() == 0)
    });
    ui.click(cx, "sftp-queue-clear");
    assert!(transfers.read_with(cx, |transfers, _| transfers.is_empty()));
}

#[test]
fn names_modes_and_dates() {
    assert!(valid_name("report.txt") && valid_name(" spaced "));
    for bad in ["", "  ", ".", "..", "a/b", "a\\b"] {
        assert!(!valid_name(bad), "{bad:?}");
    }
    assert_eq!(parse_octal("755"), Some(0o755));
    assert_eq!(parse_octal("0644"), Some(0o644));
    for bad in ["", "8", "1777", "77777", "rwx"] {
        assert_eq!(parse_octal(bad), None, "{bad:?}");
    }
    assert_eq!(date_text(None), "—");
    crate::dates::set_fixed_display(Some(crate::dates::FixedDisplay {
        offset_minutes: 120,
    }));
    assert_eq!(date_text(Some(0)), "01.01.1970 02:00");
    crate::dates::set_fixed_display(None);
}

/// The review of stage 4: a local copy onto itself is refused and leaves
/// the file whole; a rename never replaces an existing item; names with
/// control or bidirectional characters are shown visibly; Open asks first
/// for a file the system would run.
#[gpui_kit::test]
fn copies_onto_themselves_and_renames_over_items_are_refused(cx: &mut TestAppContext) {
    let ui = Ui::open(cx, &[]);
    let dir = folder();
    ui.open_local(cx, Side::Left, dir.path());
    ui.open_local(cx, Side::Right, dir.path());
    let file = ui.panel(cx, Side::Left).read_with(cx, |panel, _| {
        panel
            .rows()
            .iter()
            .find(|row| row.name == "b.txt")
            .cloned()
            .unwrap()
    });
    cx.update_window(ui.handle, |_, window, cx| {
        ui.screen.update(cx, |screen, cx| {
            screen.copy(Side::Left, vec![file.clone()], window, cx)
        })
    })
    .unwrap();
    // The conflict is asked first; Replace would copy the file onto itself.
    ui.until(cx, "the conflict dialog", |cx| {
        ui.visible(cx, "sftp-conflict")
    });
    ui.click(cx, "sftp-conflict-replace");
    let transfers = ui.transfers(cx);
    ui.until(cx, "the refusal", |cx| {
        transfers.read_with(cx, |transfers, _| transfers.active() == 0)
    });
    let failed = transfers.read_with(cx, |transfers, _| {
        transfers.transfers().iter().any(|transfer| {
            transfer.state
                == shellpp_ssh::transfer_queue::State::Failed(super::strings::SAME_FILE.to_owned())
        })
    });
    assert!(failed);
    assert_eq!(
        std::fs::read(dir.path().join("b.txt")).unwrap(),
        b"0123456789"
    );

    ui.update_panel(cx, Side::Left, |panel, window, cx| {
        panel.rename("a2.log", "b.txt", window, cx)
    });
    assert!(
        dir.path().join("a2.log").is_file(),
        "not renamed over b.txt"
    );
    assert_eq!(
        std::fs::read(dir.path().join("b.txt")).unwrap(),
        b"0123456789"
    );
}

#[test]
fn names_are_shown_safely_and_programs_are_named() {
    use super::{copy_needs_question, display_name, runnable_extension};
    assert_eq!(display_name("report.txt"), "report.txt");
    assert_eq!(display_name("photo\u{202e}gpj.exe"), "photo<U+202E>gpj.exe");
    assert_eq!(display_name("a\u{7}b"), "a<U+0007>b");
    assert_eq!(runnable_extension("setup.EXE").as_deref(), Some("exe"));
    assert_eq!(runnable_extension("deploy.sh").as_deref(), Some("sh"));
    assert_eq!(runnable_extension("notes.txt"), None);
    assert_eq!(runnable_extension("Makefile"), None);
    // Types Windows runs that the first list missed (run 8, C5).
    for name in [
        "console.msc",
        "app.appref-ms",
        "help.chm",
        "x.settingcontent-ms",
    ] {
        assert!(runnable_extension(name).is_some(), "{name}");
    }
    // The copy of a link is named after its target: "notes.txt" pointing to
    // "run.cmd" asks before it runs, unless the user already agreed to run
    // a .cmd, or chose the program (Open with).
    assert_eq!(
        copy_needs_question("run.cmd", true, None).as_deref(),
        Some("cmd")
    );
    assert_eq!(copy_needs_question("run.cmd", true, Some("cmd")), None);
    assert_eq!(
        copy_needs_question("run.cmd", true, Some("exe")).as_deref(),
        Some("cmd")
    );
    assert_eq!(copy_needs_question("run.cmd", false, None), None);
    assert_eq!(copy_needs_question("notes.txt", true, None), None);
    // The messages of an opened copy show a remote name safely too.
    let tricky = "x\u{202e}gpj.exe";
    for text in [
        super::strings::uploaded_back(tricky),
        super::strings::changed_on_server(tricky),
        super::strings::kept_server_file(tricky),
    ] {
        assert!(!text.contains('\u{202e}'), "{text}");
        assert!(text.contains("<U+202E>"), "{text}");
    }
}
