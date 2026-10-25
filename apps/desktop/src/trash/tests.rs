//! Headless tests of the Recently deleted screen (UI 14) over a temporary
//! profile in local mode: objects are deleted through the core as the
//! screens do. Passwords are generated at run time.

use std::collections::BTreeMap;

use gpui::{
    AnyWindowHandle, App, AppContext as _, Bounds, ElementId, InputEvent as _, Modifiers,
    MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, TestAppContext, Window,
    WindowBounds, WindowOptions, px, size,
};
use gpui_kit::test::TestWindowExt as _;
use secrecy::SecretString;
use serde_json::json;
use shellpp_core::service::dto::{
    QueryScope, RecordSaveInput, RecordType, RecordsQuery, RecordsRef, SortMode,
};

use super::*;
use crate::testing::TestProfile;

struct Screen {
    state: Entity<AppState>,
    screen: Entity<TrashScreen>,
    handle: AnyWindowHandle,
}

impl Screen {
    fn open(cx: &mut TestAppContext) -> Self {
        cx.update(|cx| {
            crate::testing::init_ui(cx);
            cx.set_reduce_motion(true);
        });
        let state = cx.new(|cx| AppState::new(TestProfile::new().into_unlocked(), cx));
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
                    cx.new(|cx| TrashScreen::new(screen_state, window, cx))
                })
            })
            .unwrap();
        let this = Self {
            state,
            screen,
            handle,
        };
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

    fn empty_title(&self, cx: &mut TestAppContext) -> &'static str {
        self.screen.read_with(cx, |screen, _| screen.empty_title())
    }

    fn cells(&self, cx: &mut TestAppContext, id: Uuid) -> [String; 5] {
        self.screen.read_with(cx, |screen, cx| {
            let item = screen
                .items()
                .iter()
                .find(|item| item.record_id == id)
                .unwrap();
            screen.cells(item, cx)
        })
    }

    fn click(&self, cx: &mut TestAppContext, id: impl Into<ElementId>) {
        let id = id.into();
        self.frame(cx, |window, cx| window.click(id, cx));
    }

    /// A left click with modifiers held (Ctrl, Shift).
    fn click_with(&self, cx: &mut TestAppContext, id: impl Into<ElementId>, modifiers: Modifiers) {
        let id = id.into();
        self.frame(cx, |window, cx| {
            let position = window.find(id).bounds().center();
            window.dispatch_event(
                MouseMoveEvent {
                    position,
                    pressed_button: None,
                    modifiers,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseDownEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: 1,
                    first_mouse: false,
                }
                .to_platform_input(),
                cx,
            );
            window.dispatch_event(
                MouseUpEvent {
                    button: MouseButton::Left,
                    position,
                    modifiers,
                    click_count: 1,
                }
                .to_platform_input(),
                cx,
            );
        });
    }

    fn press(&self, cx: &mut TestAppContext, keys: &str) {
        self.frame(cx, |window, cx| window.press(keys, cx));
    }

    fn type_into(&self, cx: &mut TestAppContext, id: &'static str, text: &str) {
        self.frame(cx, |window, cx| {
            window.click(id, cx);
            window.press("ctrl-a", cx);
            window.press("backspace", cx);
            window.input(text, cx);
        });
    }

    fn core(&self, cx: &mut TestAppContext) -> std::sync::Arc<shellpp_core::Core> {
        self.state
            .read_with(cx, |state, _| state.core().cloned().unwrap())
    }

    fn vault(&self, cx: &mut TestAppContext) -> Uuid {
        self.state
            .read_with(cx, |state, _| state.vault_id().unwrap())
    }

    /// Saves a record and returns its id.
    fn save(
        &self,
        cx: &mut TestAppContext,
        record_type: RecordType,
        data: serde_json::Value,
        password: Option<&str>,
    ) -> Uuid {
        let mut secrets = BTreeMap::new();
        if let Some(password) = password {
            secrets.insert(
                "ssh.password".to_owned(),
                Some(SecretString::from(password.to_owned())),
            );
        }
        let vault_id = self.vault(cx);
        self.core(cx)
            .record_save(RecordSaveInput {
                base: None,
                vault_id,
                record_id: None,
                record_type,
                data: data.as_object().cloned().unwrap(),
                secrets,
            })
            .unwrap()
            .record_id()
    }

    /// Deletes records as the screens do; the screen reloads on the event.
    fn delete(&self, cx: &mut TestAppContext, ids: &[Uuid]) {
        let vault_id = self.vault(cx);
        self.core(cx)
            .record_delete(RecordsRef {
                vault_id,
                record_ids: ids.to_vec(),
            })
            .unwrap();
        self.state.update(cx, |state, cx| state.reload_records(cx));
        self.frame(cx, |_, _| {});
    }

    fn shown(&self, cx: &mut TestAppContext) -> Vec<Uuid> {
        self.screen.read_with(cx, |screen, cx| {
            screen.shown(cx).iter().map(|item| item.record_id).collect()
        })
    }

    fn selected(&self, cx: &mut TestAppContext) -> Vec<Uuid> {
        self.screen
            .read_with(cx, |screen, _| screen.selected().to_vec())
    }

    fn live_hosts(&self, cx: &mut TestAppContext) -> Vec<Uuid> {
        self.core(cx)
            .records_query(RecordsQuery {
                vault_id: None,
                types: vec![RecordType::Host],
                scope: QueryScope::All,
                text: None,
                tags: Vec::new(),
                sort: SortMode::Name,
            })
            .unwrap()
            .iter()
            .map(|view| view.record_id())
            .collect()
    }

    fn finish(self, cx: &mut TestAppContext) {
        let _ = cx.update_window(self.handle, |_, window, _| window.remove_window());
        cx.run_until_parked();
        self.state.update(cx, |state, _| state.shutdown());
    }
}

fn generated_password() -> String {
    format!("pw-{}", Uuid::new_v4().simple())
}

#[test]
fn fields_flatten_nested_objects_without_bookkeeping() {
    let data = json!({
        "label": "web",
        "order_key": "zw",
        "group_id": "01a168dc-8537-762d-a5f2-a02e2f3dbd63",
        "ssh": {"port": 22, "password": "••••••", "username": "deploy", "key_id": "x"},
        "notes": null,
        "tags": ["prod"]
    });
    assert_eq!(
        fields(data.as_object().unwrap()),
        [
            ("label".to_owned(), "web".to_owned()),
            ("ssh.password".to_owned(), "••••••".to_owned()),
            ("ssh.port".to_owned(), "22".to_owned()),
            ("ssh.username".to_owned(), "deploy".to_owned()),
            ("tags".to_owned(), "[\"prod\"]".to_owned()),
        ]
    );
}

#[test]
fn types_and_counts_read_as_the_spec_words() {
    assert_eq!(strings::type_label("host"), "Host");
    assert_eq!(strings::type_label("snippet_package"), "Snippet package");
    assert_eq!(strings::type_label("connection_log_entry"), "Log entry");
    assert_eq!(strings::type_label("highlight_rule"), "Highlighting rule");
    assert_eq!(strings::type_label("some_new_kind"), "Some new kind");
    assert_eq!(strings::objects(1), "1 object");
    assert_eq!(strings::restored(3), "3 objects restored");
    assert!(strings::delete_body(2).starts_with("2 objects will be removed"));
    assert!(strings::empty_body(4).starts_with("All 4 objects here"));
}

/// UI 14: the line on top, the empty state, then the table newest first
/// with type, vault, time and where, in local mode (no account).
#[gpui_kit::test]
fn lists_deleted_objects_newest_first_in_local_mode(cx: &mut TestAppContext) {
    let ui = Screen::open(cx);
    assert!(ui.visible(cx, "trash-hint"));
    assert!(ui.visible(cx, "trash-empty-state"));
    assert_eq!(ui.empty_title(cx), strings::EMPTY);
    let status = ui
        .state
        .read_with(cx, |state, _| state.sync_status().cloned());
    assert!(status.is_none_or(|status| !status.bound), "local mode");

    let web = ui.save(
        cx,
        RecordType::Host,
        json!({"label": "web", "address": "web.example.com"}),
        None,
    );
    let databases = ui.save(cx, RecordType::Group, json!({"name": "Databases"}), None);
    ui.delete(cx, &[web]);
    ui.delete(cx, &[databases]);

    assert!(!ui.visible(cx, "trash-empty-state"));
    assert_eq!(ui.shown(cx), [databases, web], "newest first");
    assert!(ui.visible(cx, row_id(web)));
    let [name, kind, vault, deleted, place] = ui.cells(cx, web);
    assert_eq!(
        [name.as_str(), kind.as_str(), vault.as_str(), place.as_str()],
        ["web", "Host", "Personal", "This device"]
    );
    assert!(
        !deleted.is_empty() && !deleted.ends_with('Z'),
        "local time: {deleted}"
    );
    assert_eq!(ui.cells(cx, databases)[1], "Group");
    ui.finish(cx);
}

/// UI 14: a search by name and a filter by type.
#[gpui_kit::test]
fn search_and_type_filter_narrow_the_list(cx: &mut TestAppContext) {
    let ui = Screen::open(cx);
    let web = ui.save(
        cx,
        RecordType::Host,
        json!({"label": "web", "address": "web.example.com"}),
        None,
    );
    let mail = ui.save(
        cx,
        RecordType::Host,
        json!({"label": "Mail", "address": "mail.example.org"}),
        None,
    );
    let group = ui.save(cx, RecordType::Group, json!({"name": "Webfarm"}), None);
    ui.delete(cx, &[web, mail, group]);

    ui.type_into(cx, "trash-search", "WEB");
    let mut shown = ui.shown(cx);
    shown.sort();
    let mut expected = vec![web, group];
    expected.sort();
    assert_eq!(shown, expected, "the name, ignoring case");

    ui.screen.update(cx, |screen, cx| {
        assert_eq!(screen.types(), ["group", "host"]);
        screen.set_type_filter(Some("host".into()), cx)
    });
    assert_eq!(ui.shown(cx), [web]);
    ui.screen.read_with(cx, |screen, _| {
        assert_eq!(screen.type_filter(), Some("host"))
    });

    ui.type_into(cx, "trash-search", "nothing-like-it");
    assert!(ui.shown(cx).is_empty());
    assert!(ui.visible(cx, "trash-empty-state"));
    assert_eq!(ui.empty_title(cx), strings::NO_MATCH);
    ui.finish(cx);
}

/// UI 14: a click selects, Ctrl adds, Shift takes the range, Ctrl+A all;
/// one selected object opens read-only in the panel, its password masked.
#[gpui_kit::test]
fn selection_and_the_read_only_panel(cx: &mut TestAppContext) {
    let ui = Screen::open(cx);
    let password = generated_password();
    let a = ui.save(
        cx,
        RecordType::Host,
        json!({"label": "a", "address": "a.example.com"}),
        Some(&password),
    );
    let b = ui.save(
        cx,
        RecordType::Host,
        json!({"label": "b", "address": "b.example.com"}),
        None,
    );
    let c = ui.save(
        cx,
        RecordType::Host,
        json!({"label": "c", "address": "c.example.com"}),
        None,
    );
    ui.delete(cx, &[a]);
    ui.delete(cx, &[b]);
    ui.delete(cx, &[c]);
    assert_eq!(ui.shown(cx), [c, b, a]);
    assert!(!ui.visible(cx, "trash-panel"));

    ui.click(cx, row_id(a));
    assert_eq!(ui.selected(cx), [a]);
    assert!(ui.visible(cx, "trash-panel"));
    assert!(ui.visible(cx, "trash-field-address"));
    assert!(ui.visible(cx, "trash-field-ssh.password"));
    let fields = ui.screen.read_with(cx, |screen, _| screen.panel_fields());
    assert!(fields.contains(&("address".to_owned(), "a.example.com".to_owned())));
    assert!(fields.contains(&("ssh.password".to_owned(), "••••••".to_owned())));
    assert!(
        fields.iter().all(|(_, value)| !value.contains(&password)),
        "the password never reaches the screen"
    );

    ui.click_with(cx, row_id(c), Modifiers::secondary_key());
    assert_eq!(ui.selected(cx), [a, c]);
    assert!(!ui.visible(cx, "trash-panel"), "two objects: no panel");
    ui.click(cx, row_id(c));
    ui.click_with(cx, row_id(a), Modifiers::shift());
    assert_eq!(ui.selected(cx), [c, b, a]);
    ui.click(cx, row_id(b));
    ui.press(cx, "ctrl-a");
    assert_eq!(ui.selected(cx), [c, b, a]);
    ui.press(cx, "escape");
    assert!(ui.selected(cx).is_empty());
    ui.finish(cx);
}

/// UI 14: Restore brings the objects back under their ids and they leave
/// the list.
#[gpui_kit::test]
fn restore_brings_objects_back(cx: &mut TestAppContext) {
    let ui = Screen::open(cx);
    let a = ui.save(
        cx,
        RecordType::Host,
        json!({"label": "a", "address": "a.example.com"}),
        None,
    );
    let b = ui.save(
        cx,
        RecordType::Host,
        json!({"label": "b", "address": "b.example.com"}),
        None,
    );
    ui.delete(cx, &[a, b]);
    assert!(ui.live_hosts(cx).is_empty());

    ui.click(cx, row_id(a));
    ui.click(cx, "trash-restore");
    assert_eq!(ui.live_hosts(cx), [a], "back under its id");
    assert_eq!(ui.shown(cx), [b]);
    assert!(ui.selected(cx).is_empty());
    ui.finish(cx);
}

/// UI 14: Delete forever and Empty trash ask first, naming the count;
/// Cancel keeps everything.
#[gpui_kit::test]
fn delete_forever_and_empty_trash_confirm_first(cx: &mut TestAppContext) {
    let ui = Screen::open(cx);
    let ids: Vec<Uuid> = ["a", "b", "c"]
        .into_iter()
        .map(|name| {
            ui.save(
                cx,
                RecordType::Host,
                json!({"label": name, "address": format!("{name}.example.com")}),
                None,
            )
        })
        .collect();
    ui.delete(cx, &ids);

    ui.click(cx, row_id(ids[0]));
    ui.click_with(cx, row_id(ids[1]), Modifiers::secondary_key());
    ui.click(cx, "trash-delete");
    assert!(ui.visible(cx, "trash-delete-dialog"));
    ui.click(cx, "trash-delete-cancel");
    assert_eq!(ui.shown(cx).len(), 3, "Cancel keeps them");

    ui.click(cx, "trash-delete");
    ui.click(cx, "trash-delete-confirm");
    assert_eq!(ui.shown(cx), [ids[2]]);
    assert!(ui.live_hosts(cx).is_empty(), "nothing restored");

    ui.click(cx, "trash-empty");
    assert!(ui.visible(cx, "trash-empty-dialog"));
    ui.click(cx, "trash-empty-confirm");
    assert!(ui.shown(cx).is_empty());
    assert_eq!(ui.empty_title(cx), strings::EMPTY);
    let left = ui.core(cx).recently_deleted_list().unwrap();
    assert!(left.is_empty(), "removed from the device");
    ui.finish(cx);
}

/// Locking clears the list from the screen and from memory.
#[gpui_kit::test]
fn a_lock_clears_the_list(cx: &mut TestAppContext) {
    let ui = Screen::open(cx);
    let a = ui.save(
        cx,
        RecordType::Host,
        json!({"label": "a", "address": "a.example.com"}),
        None,
    );
    ui.delete(cx, &[a]);
    ui.click(cx, row_id(a));
    assert!(ui.visible(cx, "trash-panel"));
    ui.state.update(cx, |state, cx| state.lock(cx)).unwrap();
    ui.frame(cx, |_, _| {});
    ui.screen.read_with(cx, |screen, _| {
        assert!(screen.items().is_empty());
        assert!(screen.selected().is_empty());
    });
    assert!(!ui.visible(cx, "trash-panel"));
    ui.finish(cx);
}

/// Review finding: Empty trash removes the entries its confirmation
/// counted; one that arrives while the confirmation is open stays.
#[gpui_kit::test]
fn empty_trash_removes_only_what_it_counted(cx: &mut TestAppContext) {
    let ui = Screen::open(cx);
    let first = ui.save(
        cx,
        RecordType::Host,
        json!({"label": "a", "address": "a.example.com"}),
        None,
    );
    let later = ui.save(
        cx,
        RecordType::Host,
        json!({"label": "b", "address": "b.example.com"}),
        None,
    );
    ui.delete(cx, &[first]);
    ui.click(cx, "trash-empty");
    assert!(ui.visible(cx, "trash-empty-dialog"));
    // Deleted while the confirmation is open (as a sync would bring it).
    let vault_id = ui.vault(cx);
    ui.core(cx)
        .record_delete(RecordsRef {
            vault_id,
            record_ids: vec![later],
        })
        .unwrap();
    ui.click(cx, "trash-empty-confirm");
    let left: Vec<Uuid> = ui
        .core(cx)
        .recently_deleted_list()
        .unwrap()
        .into_iter()
        .map(|item| item.record_id)
        .collect();
    assert_eq!(left, [later], "only the counted entry went");
    ui.finish(cx);
}
