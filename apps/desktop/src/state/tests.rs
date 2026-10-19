//! AppState against a real core in a temporary profile (GPUI context tests,
//! no windows).

use gpui::{Entity, TestAppContext};
use serde_json::{Map, Value, json};
use shellpp_core::CoreError;
use shellpp_core::service::dto::LockState;

use super::*;
use crate::testing::TestProfile;

fn map(value: Value) -> Map<String, Value> {
    value.as_object().cloned().unwrap()
}

fn unlocked_state(cx: &mut TestAppContext) -> Entity<AppState> {
    let opened = TestProfile::new().into_unlocked();
    cx.new(|cx| AppState::new(opened, cx))
}

#[gpui_kit::test]
fn loads_saves_and_navigates(cx: &mut TestAppContext) {
    let state = unlocked_state(cx);
    let events = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let seen = events.clone();
    cx.update(|cx| {
        cx.subscribe(&state, move |_, event: &AppStateEvent, _| {
            seen.borrow_mut().push(event.clone())
        })
        .detach()
    });

    state.update(cx, |state, cx| {
        assert!(state.is_unlocked());
        assert!(state.vault_id().is_some());
        assert!(state.hosts().is_empty());

        let group = state
            .save_group(None, map(json!({"name": "Production"})), cx)
            .unwrap();
        let host = state
            .save_host(
                None,
                map(json!({"address": "web1.example.com", "group_id": group.record_id})),
                cx,
            )
            .unwrap();
        let top = state
            .save_host(
                None,
                map(json!({"address": "192.0.2.10", "label": "Top"})),
                cx,
            )
            .unwrap();
        assert_eq!(state.hosts().len(), 2);
        assert_eq!(state.groups_in(None).len(), 1);
        assert_eq!(state.hosts_in(None).len(), 1);
        assert_eq!(
            state.hosts_in(Some(group.record_id))[0].record_id,
            host.record_id
        );
        assert_eq!(state.host(top.record_id).unwrap().title(), "Top");

        // Edit: start from the stored data, change one member.
        let mut data = state.host(host.record_id).unwrap().data_map();
        data.insert("label".to_owned(), json!("Web 1"));
        let edited = state.save_host(Some(host.record_id), data, cx).unwrap();
        assert_eq!(edited.label.as_deref(), Some("Web 1"));
        assert_eq!(edited.group_id, Some(group.record_id));

        // Validation errors come back with the failing field.
        let error = state
            .save_host(None, map(json!({"address": "bad host"})), cx)
            .unwrap_err();
        assert!(matches!(error, CoreError::Validation(_)));

        state.set_location(Some(group.record_id), cx);
        assert_eq!(state.location(), Some(group.record_id));
        assert_eq!(state.location_path().len(), 1);

        let order = vec![host.record_id, top.record_id];
        state.select(host.record_id, SelectMode::Replace, &order, cx);
        state.select(top.record_id, SelectMode::Toggle, &order, cx);
        assert_eq!(state.selection().len(), 2);

        // Deleting the group moves its host to the top level.
        state.delete_records(vec![group.record_id], cx).unwrap();
        assert_eq!(state.location(), None);
        assert!(state.groups().is_empty());
        assert_eq!(state.hosts_in(None).len(), 2);
    });
    cx.run_until_parked();
    let events = events.borrow();
    assert!(events.contains(&AppStateEvent::RecordsChanged));
    assert!(events.contains(&AppStateEvent::SelectionChanged));
    assert!(events.contains(&AppStateEvent::LocationChanged(None)));
}

#[gpui_kit::test]
async fn lock_and_unlock_through_the_state(cx: &mut TestAppContext) {
    let (opened, password) = TestProfile::new().into_opened();
    let state = cx.new(|cx| AppState::new(opened, cx));
    assert_eq!(
        state.read_with(cx, |state, _| state.lock_state()),
        LockState::FirstRun
    );

    // First run with the generated password (real KDF parameters).
    let setup = state.update(cx, |state, cx| state.first_run_setup(password.clone(), cx));
    assert!(setup.await.is_ok());
    state.read_with(cx, |state, _| assert!(state.is_unlocked()));

    state.update(cx, |state, cx| {
        state
            .save_host(None, map(json!({"address": "db.example.com"})), cx)
            .unwrap();
        state.lock(cx).unwrap();
        assert_eq!(state.lock_state(), LockState::Locked);
        // Nothing stays loaded while locked.
        assert!(state.hosts().is_empty());
        assert!(matches!(
            state.save_host(None, map(json!({"address": "x.example.com"})), cx),
            Err(CoreError::Locked)
        ));
    });

    let wrong = state.update(cx, |state, cx| {
        state.unlock(
            secrecy::SecretString::from("not the password".to_owned()),
            cx,
        )
    });
    assert!(matches!(wrong.await, Err(CoreError::InvalidPassword)));

    let unlock = state.update(cx, |state, cx| state.unlock(password, cx));
    assert!(unlock.await.is_ok());
    state.read_with(cx, |state, _| {
        assert!(state.is_unlocked());
        assert_eq!(state.hosts().len(), 1);
    });
}

#[gpui_kit::test]
fn shutdown_removes_a_temporary_profile(cx: &mut TestAppContext) {
    let profile = TestProfile::new();
    let path = profile.path().to_path_buf();
    let opened = profile.into_unlocked();
    let state = cx.new(|cx| AppState::new(opened, cx));
    assert!(path.is_dir());
    state.update(cx, |state, _| state.shutdown());
    assert!(
        !path.exists(),
        "the temporary profile is deleted at shutdown"
    );
    state.read_with(cx, |state, _| assert!(state.core().is_none()));
}

/// Owner's step 3.2: the profile starts empty; demo data exists only in a
/// dev-tools build started with `--demo` (a temporary profile).
#[gpui_kit::test]
async fn a_new_profile_starts_empty(cx: &mut TestAppContext) {
    let (opened, password) = TestProfile::new().into_opened();
    let state = cx.new(|cx| AppState::new(opened, cx));
    let setup = state.update(cx, |state, cx| state.first_run_setup(password, cx));
    assert!(setup.await.is_ok());
    state.read_with(cx, |state, _| {
        assert!(state.is_unlocked());
        assert!(state.hosts().is_empty(), "no demo hosts");
        assert!(state.groups().is_empty(), "no demo groups");
    });
    state.update(cx, |state, _| state.shutdown());
}

#[test]
fn demo_data_needs_the_dev_tools_build_and_demo_flag() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let read = |path: &str| std::fs::read_to_string(root.join(path)).expect(path);
    // The dev module (the seed) is compiled only with the feature, and the
    // feature is off by default (release builds never enable it).
    assert!(
        read("src/lib.rs").contains("#[cfg(feature = \"dev-tools\")]\npub mod dev;"),
        "src/dev is behind the dev-tools feature"
    );
    assert!(read("Cargo.toml").contains("default = []"));
    // Outside src/dev only `run_demo` of main.rs (behind the feature,
    // reached by `--demo`) opens the demo profile.
    let main = read("src/main.rs");
    let uses: Vec<usize> = main
        .match_indices("dev::demo")
        .map(|(index, _)| index)
        .collect();
    assert!(!uses.is_empty());
    for index in uses {
        let before = &main[..index];
        let function = before.rfind("fn run_demo").expect("inside run_demo");
        let gate = before[..function].rfind("#[cfg(feature = \"dev-tools\")]");
        assert!(
            gate.is_some_and(|gate| function - gate < 64),
            "run_demo is behind the dev-tools feature"
        );
    }
    let mut stack = vec![root.join("src")];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                if !path.ends_with("dev") {
                    stack.push(path);
                }
                continue;
            }
            if path.extension().is_none_or(|ext| ext != "rs") || path.ends_with("main.rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).unwrap();
            // This test names the module it looks for.
            if path.ends_with(std::path::Path::new("state").join("tests.rs")) {
                continue;
            }
            assert!(
                !text.contains("dev::demo") && !text.contains("demo::seed"),
                "{} opens demo data",
                path.display()
            );
        }
    }
}

/// Copy to (lead decision 7) copies every field and the secrets, with the
/// original label, into the target group; the original stays.
#[gpui_kit::test]
fn copy_to_copies_fields_and_secrets(cx: &mut TestAppContext) {
    use secrecy::ExposeSecret as _;
    use shellpp_core::service::dto::{RecordSaveInput, RecordType, SecretFieldRef};

    let state = unlocked_state(cx);
    state.update(cx, |state, cx| {
        let vault_id = state.vault_id().unwrap();
        let lab = state
            .save_group(None, map(json!({"name": "Lab"})), cx)
            .unwrap()
            .record_id;
        let secret = format!("pw-{}", uuid::Uuid::now_v7().simple());
        let mut secrets = std::collections::BTreeMap::new();
        secrets.insert(
            "ssh.password".to_owned(),
            Some(secrecy::SecretString::from(secret.clone())),
        );
        let original = state
            .core()
            .unwrap()
            .record_save(RecordSaveInput {
                vault_id,
                record_id: None,
                record_type: RecordType::Host,
                data: map(json!({
                    "address": "db.example.com",
                    "label": "DB",
                    "tags": ["prod"],
                    "ssh": {"username": "pg", "port": 5022}
                })),
                secrets,
            })
            .unwrap()
            .record_id();
        let unlabeled = state
            .save_host(None, map(json!({"address": "192.0.2.5"})), cx)
            .unwrap()
            .record_id;
        state.reload_and_notify(cx);

        let copies = state
            .copy_hosts_to(vec![original, unlabeled], Some(lab), cx)
            .unwrap();
        assert_eq!(copies.len(), 2);
        let copy = state.host(copies[0]).unwrap().clone();
        let source = state.host(original).unwrap().clone();
        assert_eq!(copy.label.as_deref(), Some("DB"), "the same label");
        assert_eq!(copy.group_id, Some(lab));
        assert_eq!(source.group_id, None, "the original stays");
        assert_eq!(copy.tags, source.tags);
        assert_eq!(copy.username.as_deref(), Some("pg"));
        assert_eq!(copy.port, Some(5022));
        let revealed = state
            .core()
            .unwrap()
            .secret_reveal(SecretFieldRef {
                vault_id,
                record_id: copies[0],
                path: "ssh.password".to_owned(),
            })
            .unwrap();
        assert_eq!(revealed.expose_secret(), secret);
        // No label stays no label (the title is the address).
        let copy = state.host(copies[1]).unwrap();
        assert_eq!(copy.label, None);
        assert_eq!(copy.title(), "192.0.2.5");
        // Copy to the top level.
        let top = state.copy_hosts_to(vec![copies[1]], None, cx).unwrap();
        assert_eq!(state.host(top[0]).unwrap().group_id, None);
        // An unknown group is refused, nothing is copied.
        let before = state.hosts().len();
        assert!(matches!(
            state.copy_hosts_to(vec![original], Some(uuid::Uuid::now_v7()), cx),
            Err(CoreError::NotFound)
        ));
        assert_eq!(state.hosts().len(), before);
    });
    state.update(cx, |state, _| state.shutdown());
}
