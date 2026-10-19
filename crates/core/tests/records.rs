//! Record commands through the facade: CRUD, validation, the draft rule,
//! group removal, moves, order keys, duplicate, undo, queries, inheritance,
//! settings and notifications (stage 1 plan 5.4-5.8).

mod common;

use std::collections::BTreeMap;
use std::time::Duration;

use common::{Env, as_group, as_host, changed_ids, first_run, names, reasons, save};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use shellpp_core::service::dto::{
    CoreEventKind, MovePosition, NewNotification, NotificationLevel, NotificationSource,
    NotificationSourceKind, QueryScope, RecordMoveInput, RecordRef, RecordType, RecordsQuery,
    RecordsRef, SecretFieldRef, SettingsSetInput, SortMode,
};
use shellpp_core::{CoreError, ValidationReason};
use uuid::Uuid;

const BOTH: &[RecordType] = &[RecordType::Group, RecordType::Host];

fn move_input(
    app: &common::Unlocked,
    ids: &[Uuid],
    target: Option<Uuid>,
    position: MovePosition,
) -> RecordMoveInput {
    RecordMoveInput {
        vault_id: app.vault_id,
        record_ids: ids.to_vec(),
        target_group_id: target,
        position,
    }
}

#[test]
fn draft_rule_crud_and_validation() {
    let app = first_run();
    let core = &app.core;
    let vault = app.vault_id;

    // A draft without a valid address is never written.
    let error = save(
        core,
        vault,
        None,
        RecordType::Host,
        json!({"label": "web"}),
        &[],
    )
    .unwrap_err();
    assert_eq!(
        reasons(error),
        vec![("address".to_owned(), ValidationReason::Required)]
    );
    let error = save(
        core,
        vault,
        None,
        RecordType::Host,
        json!({"address": "web.example.com", "ssh": {"port": 0, "password": "x"}}),
        &[],
    )
    .unwrap_err();
    assert_eq!(
        reasons(error),
        vec![
            ("ssh.password".to_owned(), ValidationReason::NotAllowed),
            ("ssh.port".to_owned(), ValidationReason::OutOfRange),
        ]
    );
    assert!(app.env.take_events().is_empty());
    assert!(
        app.query(BOTH, QueryScope::All, SortMode::Manual)
            .is_empty()
    );

    // Valid: created, autosaved updates are new revisions.
    let created = as_host(
        save(
            core,
            vault,
            None,
            RecordType::Host,
            json!({"address": " web.example.com ", "tags": ["prod"], "x_future": 7}),
            &[("ssh.password", Some("pw-123"))],
        )
        .unwrap(),
    );
    assert_eq!(created.revision, 1);
    assert_eq!(created.data["address"], json!("web.example.com"));
    assert_eq!(created.data["order_key"], json!("V"));
    assert_eq!(created.secrets_set, vec!["ssh.password".to_owned()]);
    assert!(created.data["ssh"].get("password").is_none());
    let events = app.env.take_events();
    assert_eq!(names(&events), vec!["records:changed"]);
    match &events[0].kind {
        CoreEventKind::RecordsChanged {
            vault_id,
            record_ids,
            classes,
        } => {
            assert_eq!(*vault_id, vault);
            assert_eq!(record_ids, &vec![created.record_id]);
            assert_eq!(classes, &vec![0x02]);
        }
        other => panic!("unexpected {other:?}"),
    }

    // Update: the frontend sends the view's data back with a change.
    let mut data = created.data.clone();
    data["label"] = json!("Web");
    data["ssh"]["port"] = json!(2222);
    data["order_key"] = json!("zzz");
    let updated = as_host(
        save(
            core,
            vault,
            Some(created.record_id),
            RecordType::Host,
            data.clone(),
            &[],
        )
        .unwrap(),
    );
    assert_eq!(updated.revision, 2);
    assert_eq!(updated.data["label"], json!("Web"));
    assert_eq!(updated.data["x_future"], json!(7));
    assert_eq!(updated.data["order_key"], json!("V"), "managed member kept");
    assert_eq!(
        updated.secrets_set,
        vec!["ssh.password".to_owned()],
        "secret kept"
    );

    // An invalid value is refused and the last valid value stays.
    data["ssh"]["port"] = json!(70000);
    let error = save(
        core,
        vault,
        Some(created.record_id),
        RecordType::Host,
        data.clone(),
        &[],
    )
    .unwrap_err();
    assert_eq!(
        reasons(error),
        vec![("ssh.port".to_owned(), ValidationReason::OutOfRange)]
    );
    let current = as_host(
        core.record_get(RecordRef {
            vault_id: vault,
            record_id: created.record_id,
        })
        .unwrap(),
    );
    assert_eq!(current.revision, 2);
    assert_eq!(current.data["ssh"]["port"], json!(2222));

    // Clearing the secret.
    data["ssh"]["port"] = json!(2222);
    let cleared = as_host(
        save(
            core,
            vault,
            Some(created.record_id),
            RecordType::Host,
            data,
            &[("ssh.password", None)],
        )
        .unwrap(),
    );
    assert!(cleared.secrets_set.is_empty());
    assert!(matches!(
        core.secret_reveal(SecretFieldRef {
            vault_id: vault,
            record_id: created.record_id,
            path: "ssh.password".into()
        }),
        Err(CoreError::NotFound)
    ));

    // Not found: unknown id, wrong type, other vault.
    let unknown = Uuid::now_v7();
    assert!(matches!(
        core.record_get(RecordRef {
            vault_id: vault,
            record_id: unknown
        }),
        Err(CoreError::NotFound)
    ));
    assert!(matches!(
        save(
            core,
            vault,
            Some(created.record_id),
            RecordType::Group,
            json!({"name": "g"}),
            &[]
        ),
        Err(CoreError::NotFound)
    ));
    assert!(matches!(
        save(
            core,
            Uuid::now_v7(),
            None,
            RecordType::Host,
            json!({"address": "a.example.com"}),
            &[]
        ),
        Err(CoreError::NotFound)
    ));
    assert!(matches!(
        core.records_query(RecordsQuery {
            vault_id: Some(Uuid::now_v7()),
            types: vec![RecordType::Host],
            scope: QueryScope::All,
            text: None,
            tags: vec![],
            sort: SortMode::Manual,
        }),
        Err(CoreError::NotFound)
    ));

    // Groups: name required; secrets refused.
    let error = save(
        core,
        vault,
        None,
        RecordType::Group,
        json!({"color": "jade"}),
        &[("ssh.password", Some("x"))],
    )
    .unwrap_err();
    assert_eq!(
        reasons(error),
        vec![
            ("name".to_owned(), ValidationReason::Required),
            ("ssh.password".to_owned(), ValidationReason::NotAllowed),
        ]
    );
    // Persisted across lock and unlock.
    core.lock().unwrap();
    core.unlock(app.password.clone()).unwrap();
    let after = as_host(
        core.record_get(RecordRef {
            vault_id: vault,
            record_id: created.record_id,
        })
        .unwrap(),
    );
    assert_eq!(after.data["label"], json!("Web"));
    assert_eq!(after.revision, 3);
}

#[test]
fn groups_tree_rules_inheritance_and_removal() {
    let app = first_run();
    let core = &app.core;
    let vault = app.vault_id;
    let a = app.group(json!({"name": "A", "ssh": {"port": 2222, "username": "root"}}));
    let b =
        app.group(json!({"name": "B", "parent_id": a.record_id, "ssh": {"username": "deploy"}}));
    let c = app.group(json!({"name": "C", "parent_id": b.record_id}));
    let in_a = app.host(json!({"address": "a1.example.com", "group_id": a.record_id}));
    let h1 = app.host(json!({"address": "b1.example.com", "group_id": b.record_id}));
    let h2 = app
        .host(json!({"address": "b2.example.com", "group_id": b.record_id, "ssh": {"port": 22}}));

    // Inheritance placeholders: nearest group per field.
    let view = as_host(
        core.record_get(RecordRef {
            vault_id: vault,
            record_id: h1.record_id,
        })
        .unwrap(),
    );
    assert_eq!(view.inherited.port.as_ref().unwrap().value, 2222);
    assert_eq!(
        view.inherited.port.as_ref().unwrap().from_group_id,
        a.record_id
    );
    assert_eq!(view.inherited.username.as_ref().unwrap().value, "deploy");
    assert_eq!(
        view.inherited.username.as_ref().unwrap().from_group_id,
        b.record_id
    );
    assert_eq!(view.path, vec![a.record_id, b.record_id]);
    let c_view = as_group(
        core.record_get(RecordRef {
            vault_id: vault,
            record_id: c.record_id,
        })
        .unwrap(),
    );
    assert_eq!(c_view.inherited.username.as_ref().unwrap().value, "deploy");
    let a_view = as_group(
        core.record_get(RecordRef {
            vault_id: vault,
            record_id: a.record_id,
        })
        .unwrap(),
    );
    assert_eq!(a_view.host_count, 3);
    assert_eq!(a_view.child_count, 2);

    // Search finds a host by its inherited username.
    let found = core
        .records_query(RecordsQuery {
            vault_id: None,
            types: BOTH.to_vec(),
            scope: QueryScope::All,
            text: Some("DEPLOY".into()),
            tags: vec![],
            sort: SortMode::Name,
        })
        .unwrap();
    assert_eq!(common::ids(&found), vec![h1.record_id, h2.record_id]);

    // Cycle and depth rules on save and move.
    let error = save(
        core,
        vault,
        Some(a.record_id),
        RecordType::Group,
        json!({"name": "A", "parent_id": c.record_id}),
        &[],
    )
    .unwrap_err();
    assert_eq!(
        reasons(error),
        vec![("parent_id".to_owned(), ValidationReason::Cycle)]
    );
    let error = save(
        core,
        vault,
        Some(a.record_id),
        RecordType::Group,
        json!({"name": "A", "parent_id": a.record_id}),
        &[],
    )
    .unwrap_err();
    assert_eq!(
        reasons(error),
        vec![("parent_id".to_owned(), ValidationReason::Cycle)]
    );
    let error = core
        .record_move(move_input(
            &app,
            &[a.record_id],
            Some(c.record_id),
            MovePosition::End,
        ))
        .unwrap_err();
    assert_eq!(
        reasons(error),
        vec![("target_group_id".to_owned(), ValidationReason::Cycle)]
    );
    let error = core
        .record_move(move_input(
            &app,
            &[h1.record_id],
            Some(Uuid::now_v7()),
            MovePosition::End,
        ))
        .unwrap_err();
    assert_eq!(
        reasons(error),
        vec![("target_group_id".to_owned(), ValidationReason::MissingRef)]
    );
    let error = core
        .record_move(move_input(
            &app,
            &[h1.record_id],
            Some(b.record_id),
            MovePosition::Before {
                record_id: in_a.record_id,
            },
        ))
        .unwrap_err();
    assert_eq!(
        reasons(error),
        vec![("position".to_owned(), ValidationReason::Invalid)]
    );
    assert!(matches!(
        core.record_move(move_input(&app, &[Uuid::now_v7()], None, MovePosition::End)),
        Err(CoreError::NotFound)
    ));

    // A chain of 32 levels: nothing more fits below it.
    let mut parent = None;
    let mut deepest = Uuid::nil();
    for level in 0..32 {
        let mut data = json!({"name": format!("L{level}")});
        if let Some(parent) = parent {
            data["parent_id"] = json!(parent);
        }
        deepest = app.group(data).record_id;
        parent = Some(deepest);
    }
    let error = save(
        core,
        vault,
        None,
        RecordType::Group,
        json!({"name": "too deep", "parent_id": deepest}),
        &[],
    )
    .unwrap_err();
    assert_eq!(
        reasons(error),
        vec![("parent_id".to_owned(), ValidationReason::TooDeep)]
    );
    let error = core
        .record_move(move_input(
            &app,
            &[c.record_id],
            Some(deepest),
            MovePosition::End,
        ))
        .unwrap_err();
    assert_eq!(
        reasons(error),
        vec![("target_group_id".to_owned(), ValidationReason::TooDeep)]
    );
    // A host still fits (hosts are not a level).
    app.host(json!({"address": "deep.example.com", "group_id": deepest}));

    // Removing B moves C, h1 and h2 one level up into A, in one write batch.
    app.env.take_events();
    let result = core
        .record_delete(RecordsRef {
            vault_id: vault,
            record_ids: vec![b.record_id],
        })
        .unwrap();
    assert_eq!(result.deleted, vec![b.record_id]);
    let mut moved = result.moved.clone();
    moved.sort();
    let mut expected = vec![c.record_id, h1.record_id, h2.record_id];
    expected.sort();
    assert_eq!(moved, expected);
    let events = app.env.take_events();
    assert_eq!(names(&events), vec!["records:changed"]);
    let mut changed = changed_ids(&events);
    changed.sort();
    let mut all = expected.clone();
    all.push(b.record_id);
    all.sort();
    assert_eq!(changed, all);
    let in_a_now = app.query(
        BOTH,
        QueryScope::Group {
            group_id: a.record_id,
        },
        SortMode::Manual,
    );
    assert_eq!(
        in_a_now,
        vec![c.record_id, in_a.record_id, h1.record_id, h2.record_id]
    );
    let h1_view = as_host(
        core.record_get(RecordRef {
            vault_id: vault,
            record_id: h1.record_id,
        })
        .unwrap(),
    );
    assert_eq!(h1_view.data["group_id"], json!(a.record_id));
    assert_eq!(h1_view.inherited.username.as_ref().unwrap().value, "root");

    // Undo restores B; its children stay in A.
    let restored = core
        .record_restore(RecordsRef {
            vault_id: vault,
            record_ids: vec![b.record_id],
        })
        .unwrap();
    assert_eq!(common::ids(&restored), vec![b.record_id]);
    let b_view = as_group(restored.into_iter().next().unwrap());
    assert_eq!(b_view.child_count, 0);
    assert_eq!(b_view.path, vec![a.record_id]);

    // Removing a top-level group moves its children to the top level.
    core.record_delete(RecordsRef {
        vault_id: vault,
        record_ids: vec![a.record_id],
    })
    .unwrap();
    let top = app.query(BOTH, QueryScope::TopLevel, SortMode::Manual);
    assert!(top.contains(&b.record_id));
    assert!(top.contains(&in_a.record_id));
    let in_a_view = as_host(
        core.record_get(RecordRef {
            vault_id: vault,
            record_id: in_a.record_id,
        })
        .unwrap(),
    );
    assert!(in_a_view.data.get("group_id").is_none());
}

#[test]
fn manual_order_moves_and_duplicate() {
    let app = first_run();
    let core = &app.core;
    let vault = app.vault_id;
    let h1 =
        app.host(json!({"address": "h1.example.com", "label": "First", "import_key": "ignored"}));
    let h2 = app.host(json!({"address": "h2.example.com"}));
    let h3 = app.host(json!({"address": "h3.example.com"}));
    let top = |app: &common::Unlocked| {
        app.query(&[RecordType::Host], QueryScope::TopLevel, SortMode::Manual)
    };
    assert_eq!(top(&app), vec![h1.record_id, h2.record_id, h3.record_id]);

    // Drag h3 before h1.
    core.record_move(move_input(
        &app,
        &[h3.record_id],
        None,
        MovePosition::Before {
            record_id: h1.record_id,
        },
    ))
    .unwrap();
    assert_eq!(top(&app), vec![h3.record_id, h1.record_id, h2.record_id]);
    // Drag h3 after h2.
    core.record_move(move_input(
        &app,
        &[h3.record_id],
        None,
        MovePosition::After {
            record_id: h2.record_id,
        },
    ))
    .unwrap();
    assert_eq!(top(&app), vec![h1.record_id, h2.record_id, h3.record_id]);
    // Several records keep the given order.
    core.record_move(move_input(
        &app,
        &[h3.record_id, h2.record_id],
        None,
        MovePosition::Before {
            record_id: h1.record_id,
        },
    ))
    .unwrap();
    assert_eq!(top(&app), vec![h3.record_id, h2.record_id, h1.record_id]);

    // Duplicate goes right after the original, with " Copy" and no import key.
    let copy = as_host(
        core.record_duplicate(RecordRef {
            vault_id: vault,
            record_id: h2.record_id,
        })
        .unwrap(),
    );
    assert_eq!(copy.data["label"], json!("h2.example.com Copy"));
    assert!(copy.data.get("import_key").is_none());
    assert_eq!(
        top(&app),
        vec![h3.record_id, h2.record_id, copy.record_id, h1.record_id]
    );
    let first_copy = as_host(
        core.record_duplicate(RecordRef {
            vault_id: vault,
            record_id: h1.record_id,
        })
        .unwrap(),
    );
    assert_eq!(first_copy.data["label"], json!("First Copy"));
    let group = app.group(json!({"name": "g"}));
    let error = core
        .record_duplicate(RecordRef {
            vault_id: vault,
            record_id: group.record_id,
        })
        .unwrap_err();
    assert_eq!(
        reasons(error),
        vec![("type".to_owned(), ValidationReason::Invalid)]
    );

    // Drop on a group card: appended at the end of the group, any sort mode.
    let inside = app.host(json!({"address": "in.example.com", "group_id": group.record_id}));
    core.record_move(move_input(
        &app,
        &[h1.record_id],
        Some(group.record_id),
        MovePosition::End,
    ))
    .unwrap();
    assert_eq!(
        app.query(
            &[RecordType::Host],
            QueryScope::Group {
                group_id: group.record_id
            },
            SortMode::Manual
        ),
        vec![inside.record_id, h1.record_id]
    );
    // Changing group_id through a save also appends at the end.
    let mut data = h2.data.clone();
    data["group_id"] = json!(group.record_id);
    save(core, vault, Some(h2.record_id), RecordType::Host, data, &[]).unwrap();
    assert_eq!(
        app.query(
            &[RecordType::Host],
            QueryScope::Group {
                group_id: group.record_id
            },
            SortMode::Manual
        ),
        vec![inside.record_id, h1.record_id, h2.record_id]
    );
    // Many insertions at the same place stay ordered (rebalancing when needed).
    for _ in 0..80 {
        core.record_move(move_input(
            &app,
            &[h2.record_id],
            Some(group.record_id),
            MovePosition::After {
                record_id: inside.record_id,
            },
        ))
        .unwrap();
        core.record_move(move_input(
            &app,
            &[h1.record_id],
            Some(group.record_id),
            MovePosition::After {
                record_id: inside.record_id,
            },
        ))
        .unwrap();
    }
    assert_eq!(
        app.query(
            &[RecordType::Host],
            QueryScope::Group {
                group_id: group.record_id
            },
            SortMode::Manual
        ),
        vec![inside.record_id, h1.record_id, h2.record_id]
    );
}

#[test]
fn undo_window_and_multi_removal() {
    let app = first_run();
    let core = &app.core;
    let vault = app.vault_id;
    let a = app.host_with_password(json!({"address": "a.example.com"}), "undo-secret-1");
    let b = app.host(json!({"address": "b.example.com"}));
    let result = core
        .record_delete(RecordsRef {
            vault_id: vault,
            record_ids: vec![a.record_id, b.record_id, a.record_id],
        })
        .unwrap();
    assert_eq!(result.deleted, vec![a.record_id, b.record_id]);
    assert!(
        app.query(BOTH, QueryScope::All, SortMode::Manual)
            .is_empty()
    );
    assert!(matches!(
        core.record_delete(RecordsRef {
            vault_id: vault,
            record_ids: vec![a.record_id]
        }),
        Err(CoreError::NotFound)
    ));
    app.env.mem.clock.advance(Duration::from_secs(30));
    let restored = core
        .record_restore(RecordsRef {
            vault_id: vault,
            record_ids: vec![a.record_id, b.record_id],
        })
        .unwrap();
    assert_eq!(common::ids(&restored), vec![a.record_id, b.record_id]);
    let secret = core
        .secret_reveal(SecretFieldRef {
            vault_id: vault,
            record_id: a.record_id,
            path: "ssh.password".into(),
        })
        .unwrap();
    assert_eq!(secret.expose_secret(), "undo-secret-1");
    assert!(matches!(
        core.record_restore(RecordsRef {
            vault_id: vault,
            record_ids: vec![a.record_id]
        }),
        Err(CoreError::NotFound)
    ));

    // After 60 seconds the removal is final.
    core.record_delete(RecordsRef {
        vault_id: vault,
        record_ids: vec![a.record_id],
    })
    .unwrap();
    app.env.mem.clock.advance(Duration::from_secs(61));
    core.tick();
    assert!(matches!(
        core.record_restore(RecordsRef {
            vault_id: vault,
            record_ids: vec![a.record_id]
        }),
        Err(CoreError::NotFound)
    ));
    // Lock wipes the undo buffer.
    core.record_delete(RecordsRef {
        vault_id: vault,
        record_ids: vec![b.record_id],
    })
    .unwrap();
    core.lock().unwrap();
    core.unlock(app.password.clone()).unwrap();
    assert!(matches!(
        core.record_restore(RecordsRef {
            vault_id: vault,
            record_ids: vec![b.record_id]
        }),
        Err(CoreError::NotFound)
    ));
}

#[test]
fn query_scope_text_tags_and_sort() {
    let app = first_run();
    let core = &app.core;
    let prod = app.group(json!({"name": "Production"}));
    let web = app.host(json!({"address": "web.example.com", "label": "Web", "tags": ["prod", "http"], "group_id": prod.record_id}));
    let db = app.host(json!({"address": "db.example.org", "tags": ["prod", "sql"], "ssh": {"username": "postgres"}}));
    let lab = app.host(json!({"address": "192.0.2.44", "label": "Lab box"}));
    let query = |text: Option<&str>, tags: &[&str], scope: QueryScope, sort: SortMode| {
        common::ids(
            &core
                .records_query(RecordsQuery {
                    vault_id: Some(app.vault_id),
                    types: BOTH.to_vec(),
                    scope,
                    text: text.map(str::to_owned),
                    tags: tags.iter().map(|tag| (*tag).to_owned()).collect(),
                    sort,
                })
                .unwrap(),
        )
    };
    assert_eq!(
        query(None, &[], QueryScope::TopLevel, SortMode::Manual),
        vec![prod.record_id, db.record_id, lab.record_id]
    );
    assert_eq!(
        query(Some("prod"), &[], QueryScope::All, SortMode::Manual),
        vec![prod.record_id, web.record_id, db.record_id]
    );
    assert_eq!(
        query(Some("POSTGRES"), &[], QueryScope::All, SortMode::Manual),
        vec![db.record_id]
    );
    assert_eq!(
        query(Some("192.0.2"), &[], QueryScope::All, SortMode::Manual),
        vec![lab.record_id]
    );
    assert_eq!(
        query(None, &["PROD"], QueryScope::All, SortMode::Name),
        vec![db.record_id, web.record_id]
    );
    assert_eq!(
        query(None, &["prod", "sql"], QueryScope::All, SortMode::Name),
        vec![db.record_id]
    );
    assert_eq!(
        query(None, &[], QueryScope::All, SortMode::Created),
        vec![prod.record_id, lab.record_id, db.record_id, web.record_id]
    );
    // Modified: an edit moves a record to the front.
    let mut data = web.data.clone();
    data["label"] = json!("Web 2");
    save(
        core,
        app.vault_id,
        Some(web.record_id),
        RecordType::Host,
        data,
        &[],
    )
    .unwrap();
    assert_eq!(
        query(None, &[], QueryScope::All, SortMode::Modified),
        vec![prod.record_id, web.record_id, lab.record_id, db.record_id]
    );
    assert_eq!(
        query(Some("nothing here"), &[], QueryScope::All, SortMode::Manual),
        Vec::<Uuid>::new()
    );
}

#[test]
fn settings_are_readable_while_locked_and_validated() {
    let env = Env::new();
    let core = env.open();
    let values = core.settings_get().unwrap();
    assert_eq!(values["security.auto_lock_minutes"], json!(15));
    assert_eq!(values["view.hosts.sort"], json!("manual"));
    assert_eq!(values["shortcuts.preset"], json!("pc"));
    assert!(matches!(
        core.settings_set(SettingsSetInput::default()),
        Err(CoreError::Locked)
    ));
    let password = common::new_password();
    core.first_run_setup(password.clone()).unwrap();
    env.take_events();

    let mut values = BTreeMap::new();
    values.insert("view.hosts.layout".to_owned(), json!("list"));
    values.insert("view.details_panel_width".to_owned(), json!(600));
    values.insert("no.such.key".to_owned(), json!(1));
    let error = core
        .settings_set(SettingsSetInput {
            values,
            reset: vec![],
        })
        .unwrap_err();
    assert_eq!(
        reasons(error),
        vec![
            ("no.such.key".to_owned(), ValidationReason::Invalid),
            (
                "view.details_panel_width".to_owned(),
                ValidationReason::OutOfRange
            ),
        ]
    );
    assert!(env.take_events().is_empty());
    assert_eq!(
        core.settings_get().unwrap()["view.hosts.layout"],
        json!("grid"),
        "nothing written"
    );

    let mut values = BTreeMap::new();
    values.insert("view.hosts.layout".to_owned(), json!("list"));
    values.insert("view.hosts.sort".to_owned(), json!("manual"));
    values.insert("security.auto_lock_minutes".to_owned(), Value::Null);
    let result = core
        .settings_set(SettingsSetInput {
            values,
            reset: vec![],
        })
        .unwrap();
    assert_eq!(result["view.hosts.layout"], json!("list"));
    let events = env.take_events();
    match &events[..] {
        [event] => match &event.kind {
            CoreEventKind::SettingsChanged { keys } => assert_eq!(
                keys,
                &vec![
                    "security.auto_lock_minutes".to_owned(),
                    "view.hosts.layout".to_owned()
                ]
            ),
            other => panic!("unexpected {other:?}"),
        },
        other => panic!("unexpected {other:?}"),
    }

    // Persisted and readable after a restart without unlocking.
    drop(core);
    let core = env.open();
    let values = core.settings_get().unwrap();
    assert_eq!(values["view.hosts.layout"], json!("list"));
    assert_eq!(values["security.auto_lock_minutes"], Value::Null);
    assert_eq!(core.snapshot().settings["view.hosts.layout"], json!("list"));
    core.unlock(password).unwrap();
    core.settings_set(SettingsSetInput {
        values: BTreeMap::new(),
        reset: vec!["view.hosts.layout".into()],
    })
    .unwrap();
    assert_eq!(
        core.settings_get().unwrap()["view.hosts.layout"],
        json!("grid")
    );
}

#[test]
fn notifications_are_memory_only_and_capped() {
    let env = Env::new();
    let core = env.open();
    let note = |key: &str| NewNotification {
        level: NotificationLevel::Success,
        message_key: key.to_owned(),
        params: BTreeMap::new(),
        source: NotificationSource {
            kind: NotificationSourceKind::App,
            reference: None,
        },
    };
    let first = core.notify(note("app.test.one"));
    assert_eq!(first.id, 1);
    let events = env.take_events();
    match &events[0].kind {
        CoreEventKind::NotificationsChanged { unread, latest } => {
            assert_eq!(*unread, 1);
            assert_eq!(latest.as_ref().unwrap().id, 1);
        }
        other => panic!("unexpected {other:?}"),
    }
    assert_eq!(
        events[0].payload()["latest"]["message_key"],
        json!("app.test.one")
    );
    for i in 0..205 {
        core.notify(note(&format!("app.test.{i}")));
    }
    let list = core.notifications_list();
    assert_eq!(list.items.len(), 200);
    assert_eq!(list.items[0].id, 206);
    assert_eq!(list.unread, 200);
    env.take_events();
    let list = core.notifications_mark_read();
    assert_eq!(list.unread, 0);
    let events = env.take_events();
    assert_eq!(events.len(), 1);
    assert!(events[0].payload().get("latest").is_none());
    assert_eq!(events[0].payload()["unread"], json!(0));
    // Nothing to mark: no event.
    core.notifications_mark_read();
    assert!(env.take_events().is_empty());
    assert_eq!(core.snapshot().notifications.items.len(), 200);
    // Lost at exit.
    drop(core);
    assert!(env.open().notifications_list().items.is_empty());
}

#[test]
fn oversized_payloads_are_refused() {
    let app = first_run();
    let big = "x".repeat(300_000);
    let error = save(
        &app.core,
        app.vault_id,
        None,
        RecordType::Host,
        json!({"address": "big.example.com", "x_blob": big}),
        &[],
    )
    .unwrap_err();
    assert!(matches!(error, CoreError::TooLarge));
    assert_eq!(error.code(), "too_large");
    assert!(
        app.query(BOTH, QueryScope::All, SortMode::Manual)
            .is_empty()
    );
}
