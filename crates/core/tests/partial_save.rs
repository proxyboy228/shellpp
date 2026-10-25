//! An editor's save with the version it started from (D220): only the
//! members it changed are written onto the record as it is now. Another
//! version written meanwhile (here a whole write, as a pull or another
//! window makes one) keeps the members the edit left alone; in the same
//! members it is kept as a conflict copy.

mod common;

use std::collections::BTreeMap;

use common::{Unlocked, as_group, as_host, first_run, map, save};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use shellpp_core::service::dto::{
    HostView, RecordBase, RecordRef, RecordSaveInput, RecordType, RecordView, SecretFieldRef,
};
use shellpp_core::{CoreError, ValidationReason};
use uuid::Uuid;

fn base_of(view: &RecordView) -> RecordBase {
    let (revision, data) = match view {
        RecordView::Host(host) => (host.revision, host.data.clone()),
        RecordView::Group(group) => (group.revision, group.data.clone()),
    };
    RecordBase {
        revision,
        data: map(data),
    }
}

fn get(app: &Unlocked, id: Uuid) -> RecordView {
    app.core
        .record_get(RecordRef {
            vault_id: app.vault_id,
            record_id: id,
        })
        .expect("record")
}

/// An editor's save of `form` that started from `base`.
fn save_from(
    app: &Unlocked,
    id: Uuid,
    record_type: RecordType,
    base: &RecordBase,
    form: Value,
    secrets: &[(&str, Option<&str>)],
) -> Result<RecordView, CoreError> {
    app.core.record_save(RecordSaveInput {
        vault_id: app.vault_id,
        record_id: Some(id),
        record_type,
        data: map(form),
        secrets: secrets
            .iter()
            .map(|(path, value)| ((*path).to_owned(), value.map(SecretString::from)))
            .collect::<BTreeMap<_, _>>(),
        base: Some(base.clone()),
    })
}

/// A whole write of `data` onto the record (the other version).
fn write_whole(app: &Unlocked, id: Uuid, record_type: RecordType, data: Value) {
    save(&app.core, app.vault_id, Some(id), record_type, data, &[]).expect("written");
}

fn with(base: &RecordBase, changes: Value) -> Value {
    let mut data = Value::Object(base.data.clone());
    for (key, value) in map(changes) {
        match value {
            Value::Object(inner) => {
                for (inner_key, inner_value) in inner {
                    data[&key][inner_key] = inner_value;
                }
            }
            other => data[&key] = other,
        }
    }
    data
}

fn web(app: &Unlocked) -> HostView {
    app.host(json!({
        "address": "web.example.com",
        "label": "web",
        "notes": "first",
        "ssh": {"port": 22, "username": "root"}
    }))
}

#[test]
fn members_the_edit_left_alone_keep_the_newer_version() {
    let app = first_run();
    let host = web(&app);
    let base = base_of(&get(&app, host.record_id));
    // Another version changes the notes and the port.
    let mut other = host.data.clone();
    other["notes"] = json!("from the other version");
    other["ssh"]["port"] = json!(2222);
    write_whole(&app, host.record_id, RecordType::Host, other);

    let saved = as_host(
        save_from(
            &app,
            host.record_id,
            RecordType::Host,
            &base,
            with(
                &base,
                json!({"label": "edited", "ssh": {"username": "admin"}}),
            ),
            &[],
        )
        .expect("saved"),
    );
    assert_eq!(saved.data["label"], "edited");
    assert_eq!(saved.data["ssh"]["username"], "admin");
    assert_eq!(saved.data["notes"], "from the other version");
    assert_eq!(saved.data["ssh"]["port"], 2222);
    assert!(app.core.sync_conflicts().expect("conflicts").is_empty());
}

#[test]
fn the_same_member_changed_by_both_keeps_the_other_value_in_a_conflict_copy() {
    let app = first_run();
    let host = web(&app);
    let base = base_of(&get(&app, host.record_id));
    let mut other = host.data.clone();
    other["label"] = json!("other label");
    other["notes"] = json!("other notes");
    write_whole(&app, host.record_id, RecordType::Host, other);

    let saved = as_host(
        save_from(
            &app,
            host.record_id,
            RecordType::Host,
            &base,
            with(&base, json!({"label": "edited"})),
            &[],
        )
        .expect("saved"),
    );
    assert_eq!(saved.data["label"], "edited");
    assert_eq!(saved.data["notes"], "other notes");
    let copies = app.core.sync_conflicts().expect("conflicts");
    assert_eq!(copies.len(), 1);
    let losing = copies[0].losing_data.as_ref().expect("losing data");
    assert_eq!(losing["label"], "other label");
    assert_eq!(losing["notes"], "other notes");
    assert_eq!(copies[0].original_record_id, host.record_id);
}

#[test]
fn the_same_value_in_the_same_member_makes_no_copy() {
    let app = first_run();
    let host = web(&app);
    let base = base_of(&get(&app, host.record_id));
    let mut other = host.data.clone();
    other["label"] = json!("same");
    write_whole(&app, host.record_id, RecordType::Host, other);
    save_from(
        &app,
        host.record_id,
        RecordType::Host,
        &base,
        with(&base, json!({"label": "same"})),
        &[],
    )
    .expect("saved");
    assert!(app.core.sync_conflicts().expect("conflicts").is_empty());
}

#[test]
fn a_cleared_member_is_cleared_and_others_stay() {
    let app = first_run();
    let host = web(&app);
    let base = base_of(&get(&app, host.record_id));
    let mut other = host.data.clone();
    other["label"] = json!("other label");
    write_whole(&app, host.record_id, RecordType::Host, other);
    let mut form = Value::Object(base.data.clone());
    form["notes"] = Value::Null;
    form["ssh"]["username"] = json!("");
    let saved = as_host(
        save_from(&app, host.record_id, RecordType::Host, &base, form, &[]).expect("saved"),
    );
    assert!(saved.data.get("notes").is_none());
    assert!(saved.data["ssh"].get("username").is_none());
    assert_eq!(saved.data["label"], "other label");
    assert!(app.core.sync_conflicts().expect("conflicts").is_empty());
}

#[test]
fn a_save_that_changes_nothing_writes_no_version() {
    let app = first_run();
    let host = web(&app);
    let base = base_of(&get(&app, host.record_id));
    app.env.take_events();
    let saved = as_host(
        save_from(
            &app,
            host.record_id,
            RecordType::Host,
            &base,
            // The form sends the port as the user typed it: the same value.
            with(&base, json!({"ssh": {"port": 22}})),
            &[],
        )
        .expect("saved"),
    );
    assert_eq!(saved.revision, host.revision);
    assert!(app.env.take_events().is_empty());
}

#[test]
fn a_password_after_a_newer_version_keeps_that_version_whole() {
    let app = first_run();
    let view = as_host(
        save(
            &app.core,
            app.vault_id,
            None,
            RecordType::Host,
            json!({"address": "db.example.com", "label": "db"}),
            &[("ssh.password", Some("old secret"))],
        )
        .expect("saved"),
    );
    let id = view.record_id;
    let reveal = || {
        app.core
            .secret_reveal(SecretFieldRef {
                vault_id: app.vault_id,
                record_id: id,
                path: "ssh.password".into(),
            })
            .expect("revealed")
            .expose_secret()
            .to_owned()
    };

    // Without a newer version the password is simply written.
    let base = base_of(&get(&app, id));
    save_from(
        &app,
        id,
        RecordType::Host,
        &base,
        Value::Object(base.data.clone()),
        &[("ssh.password", Some("second"))],
    )
    .expect("saved");
    assert_eq!(reveal(), "second");
    assert!(app.core.sync_conflicts().expect("conflicts").is_empty());

    // After a newer version the editor cannot tell whether it changed the
    // password too (the view holds none): that version is kept whole.
    let base = base_of(&get(&app, id));
    let mut other = as_host(get(&app, id)).data;
    other["notes"] = json!("newer");
    write_whole(&app, id, RecordType::Host, other);
    save_from(
        &app,
        id,
        RecordType::Host,
        &base,
        Value::Object(base.data.clone()),
        &[("ssh.password", Some("third"))],
    )
    .expect("saved");
    assert_eq!(reveal(), "third");
    let saved = as_host(get(&app, id));
    assert_eq!(saved.data["notes"], "newer");
    let copies = app.core.sync_conflicts().expect("conflicts");
    assert_eq!(copies.len(), 1);
    assert_eq!(
        copies[0].losing_data.as_ref().expect("data")["notes"],
        "newer"
    );
}

#[test]
fn a_group_editor_keeps_a_newer_parent_and_writes_its_name() {
    let app = first_run();
    let top = app.group(json!({"name": "top"}));
    let group = app.group(json!({"name": "team", "notes": "a"}));
    let base = base_of(&get(&app, group.record_id));
    // Moved meanwhile.
    let mut other = group.data.clone();
    other["parent_id"] = json!(top.record_id);
    write_whole(&app, group.record_id, RecordType::Group, other);

    let saved = as_group(
        save_from(
            &app,
            group.record_id,
            RecordType::Group,
            &base,
            with(&base, json!({"name": "renamed"})),
            &[],
        )
        .expect("saved"),
    );
    assert_eq!(saved.data["name"], "renamed");
    assert_eq!(saved.data["parent_id"], json!(top.record_id));
    assert_eq!(saved.path, vec![top.record_id]);
}

#[test]
fn an_invalid_edit_is_refused_and_writes_nothing() {
    let app = first_run();
    let host = web(&app);
    let base = base_of(&get(&app, host.record_id));
    let error = save_from(
        &app,
        host.record_id,
        RecordType::Host,
        &base,
        with(&base, json!({"ssh": {"port": 70000}})),
        &[],
    )
    .unwrap_err();
    match error {
        CoreError::Validation(errors) => {
            assert_eq!(errors[0].field, "ssh.port");
            assert_eq!(errors[0].reason, ValidationReason::OutOfRange);
        }
        other => panic!("expected validation_failed, got {}", other.code()),
    }
    assert_eq!(as_host(get(&app, host.record_id)).revision, host.revision);
}

#[test]
fn a_record_deleted_here_without_a_trace_is_not_found() {
    // A vault that was never bound deletes a record that never synced at
    // once (storage-format 4.6): nothing to bring back.
    let app = first_run();
    let host = web(&app);
    let base = base_of(&get(&app, host.record_id));
    app.core
        .record_delete(shellpp_core::service::dto::RecordsRef {
            vault_id: app.vault_id,
            record_ids: vec![host.record_id],
        })
        .expect("deleted");
    let error = save_from(
        &app,
        host.record_id,
        RecordType::Host,
        &base,
        with(&base, json!({"label": "late"})),
        &[],
    )
    .unwrap_err();
    assert!(matches!(error, CoreError::NotFound));
}

/// A host saved with `secrets`, its view.
fn host_with(app: &Unlocked, data: Value, secrets: &[(&str, Option<&str>)]) -> HostView {
    as_host(
        save(
            &app.core,
            app.vault_id,
            None,
            RecordType::Host,
            data,
            secrets,
        )
        .expect("saved"),
    )
}

fn proxy_password(app: &Unlocked, id: Uuid) -> Option<String> {
    app.core
        .secret_reveal(SecretFieldRef {
            vault_id: app.vault_id,
            record_id: id,
            path: "ssh.proxy.password".into(),
        })
        .ok()
        .map(|secret| secret.expose_secret().to_owned())
}

#[test]
fn a_newer_version_that_already_holds_the_new_password_makes_no_copy() {
    let app = first_run();
    let host = host_with(
        &app,
        json!({"address": "db.example.com"}),
        &[("ssh.password", Some("old"))],
    );
    let base = base_of(&get(&app, host.record_id));
    let other = as_host(get(&app, host.record_id)).data;
    save(
        &app.core,
        app.vault_id,
        Some(host.record_id),
        RecordType::Host,
        other,
        &[("ssh.password", Some("same"))],
    )
    .expect("written");
    save_from(
        &app,
        host.record_id,
        RecordType::Host,
        &base,
        Value::Object(base.data.clone()),
        &[("ssh.password", Some("same"))],
    )
    .expect("saved");
    assert!(app.core.sync_conflicts().expect("conflicts").is_empty());
}

#[test]
fn a_removed_proxy_keeps_a_newer_proxy_password_in_a_copy() {
    let app = first_run();
    let host = host_with(
        &app,
        json!({"address": "px.example.com", "ssh": {"proxy": {"type": "http", "address": "proxy.example.com", "port": 8080}}}),
        &[("ssh.proxy.password", Some("old"))],
    );
    let base = base_of(&get(&app, host.record_id));
    let other = as_host(get(&app, host.record_id)).data;
    save(
        &app.core,
        app.vault_id,
        Some(host.record_id),
        RecordType::Host,
        other,
        &[("ssh.proxy.password", Some("newer"))],
    )
    .expect("written");
    let mut form = Value::Object(base.data.clone());
    form["ssh"].as_object_mut().unwrap().remove("proxy");
    save_from(&app, host.record_id, RecordType::Host, &base, form, &[]).expect("saved");
    assert!(
        as_host(get(&app, host.record_id)).data["ssh"]
            .get("proxy")
            .is_none()
    );
    assert_eq!(proxy_password(&app, host.record_id), None);
    let copies = app.core.sync_conflicts().expect("conflicts");
    assert_eq!(
        copies.len(),
        1,
        "the newer proxy password is kept in a copy"
    );
}

#[test]
fn a_parent_changed_elsewhere_does_not_refuse_a_rename() {
    let app = first_run();
    let p = app.group(json!({"name": "P"}));
    let a = app.group(json!({"name": "A", "parent_id": p.record_id}));
    let base = base_of(&get(&app, a.record_id));
    // Elsewhere: A goes to the top level and P under A.
    let mut a_data = as_group(get(&app, a.record_id)).data;
    a_data.as_object_mut().unwrap().remove("parent_id");
    write_whole(&app, a.record_id, RecordType::Group, a_data);
    let mut p_data = as_group(get(&app, p.record_id)).data;
    p_data["parent_id"] = json!(a.record_id);
    write_whole(&app, p.record_id, RecordType::Group, p_data);
    let saved = as_group(
        save_from(
            &app,
            a.record_id,
            RecordType::Group,
            &base,
            with(&base, json!({"name": "A renamed"})),
            &[],
        )
        .expect("saved"),
    );
    assert_eq!(saved.data["name"], "A renamed");
    assert!(saved.data.get("parent_id").is_none());
}

#[test]
fn a_theme_reference_is_written_whole() {
    let app = first_run();
    let host = app.host(json!({"address": "t.example.com", "terminal_theme": {"builtin": "nord"}}));
    let base = base_of(&get(&app, host.record_id));
    let mut other = host.data.clone();
    other["terminal_theme"] = json!({"builtin": "paper"});
    write_whole(&app, host.record_id, RecordType::Host, other);
    let saved = as_host(
        save_from(
            &app,
            host.record_id,
            RecordType::Host,
            &base,
            with(&base, json!({"terminal_theme": {"builtin": "dracula"}})),
            &[],
        )
        .expect("saved"),
    );
    assert_eq!(saved.data["terminal_theme"], json!({"builtin": "dracula"}));
    assert_eq!(app.core.sync_conflicts().expect("conflicts").len(), 1);
}

#[test]
fn telnet_removed_elsewhere_and_changed_here_goes_to_the_edit_with_a_copy() {
    let app = first_run();
    let host = app.host(json!({"address": "tn.example.com", "telnet": {"port": 2323}}));
    let base = base_of(&get(&app, host.record_id));
    let mut other = host.data.clone();
    other.as_object_mut().unwrap().remove("telnet");
    write_whole(&app, host.record_id, RecordType::Host, other);
    let saved = as_host(
        save_from(
            &app,
            host.record_id,
            RecordType::Host,
            &base,
            with(&base, json!({"telnet": {"port": 2424}})),
            &[],
        )
        .expect("saved"),
    );
    // Telnet is one member: the edit wins and the other version is kept.
    assert_eq!(saved.data["telnet"], json!({"port": 2424}));
    let copies = app.core.sync_conflicts().expect("conflicts");
    assert_eq!(copies.len(), 1);
    assert!(
        copies[0]
            .losing_data
            .as_ref()
            .unwrap()
            .get("telnet")
            .is_none()
    );
}

#[test]
fn a_proxy_removed_elsewhere_and_edited_here_comes_back_whole() {
    let app = first_run();
    let host = app.host(json!({"address": "px.example.com", "ssh": {"proxy": {"type": "http", "address": "proxy.example.com", "port": 8080}}}));
    let base = base_of(&get(&app, host.record_id));
    let mut other = host.data.clone();
    other["ssh"].as_object_mut().unwrap().remove("proxy");
    write_whole(&app, host.record_id, RecordType::Host, other);
    let mut form = Value::Object(base.data.clone());
    form["ssh"]["proxy"]["port"] = json!(3128);
    let saved = as_host(
        save_from(&app, host.record_id, RecordType::Host, &base, form, &[]).expect("saved"),
    );
    assert_eq!(
        saved.data["ssh"]["proxy"],
        json!({"type": "http", "address": "proxy.example.com", "port": 3128})
    );
    assert_eq!(app.core.sync_conflicts().expect("conflicts").len(), 1);
}

#[test]
fn a_base_in_another_normalization_is_no_change() {
    let app = first_run();
    let host = app.host(json!({"address": "[2001:db8::1]", "label": "v6"}));
    let stored = as_host(get(&app, host.record_id));
    assert_eq!(stored.data["address"], "2001:db8::1");
    // The editor's base holds the address as it was typed.
    let mut sent = stored.data.clone();
    sent["address"] = json!("[2001:db8::1]");
    let base = RecordBase {
        revision: stored.revision,
        data: map(sent.clone()),
    };
    let mut other = stored.data.clone();
    other["address"] = json!("2001:db8::2");
    write_whole(&app, host.record_id, RecordType::Host, other);
    let mut form = sent;
    form["label"] = json!("v6 edited");
    let saved = as_host(
        save_from(&app, host.record_id, RecordType::Host, &base, form, &[]).expect("saved"),
    );
    assert_eq!(saved.data["address"], "2001:db8::2");
    assert_eq!(saved.data["label"], "v6 edited");
    assert!(app.core.sync_conflicts().expect("conflicts").is_empty());
}

#[test]
fn a_newer_proxy_password_never_goes_to_the_proxy_the_edit_writes() {
    let app = first_run();
    let host = host_with(
        &app,
        json!({"address": "px.example.com", "ssh": {"proxy": {"type": "http", "address": "proxy-a.example.com", "port": 8080}}}),
        &[("ssh.proxy.password", Some("pass-a"))],
    );
    let base = base_of(&get(&app, host.record_id));
    // Elsewhere: another proxy, with its own password.
    let mut other = as_host(get(&app, host.record_id)).data;
    other["ssh"]["proxy"]["address"] = json!("proxy-b.example.com");
    save(
        &app.core,
        app.vault_id,
        Some(host.record_id),
        RecordType::Host,
        other,
        &[("ssh.proxy.password", Some("pass-b"))],
    )
    .expect("written");
    // Here: only the port of the proxy the editor shows (A).
    let mut form = Value::Object(base.data.clone());
    form["ssh"]["proxy"]["port"] = json!(3128);
    let saved = as_host(
        save_from(&app, host.record_id, RecordType::Host, &base, form, &[]).expect("saved"),
    );
    assert_eq!(saved.data["ssh"]["proxy"]["address"], "proxy-a.example.com");
    assert_eq!(
        proxy_password(&app, host.record_id),
        None,
        "B's password is not sent to A"
    );
    let copies = app.core.sync_conflicts().expect("conflicts");
    assert_eq!(copies.len(), 1, "B and its password are kept in a copy");
    assert_eq!(
        copies[0].losing_data.as_ref().unwrap()["ssh"]["proxy"]["address"],
        "proxy-b.example.com"
    );
}

#[test]
fn an_own_proxy_edit_keeps_its_password() {
    let app = first_run();
    let host = host_with(
        &app,
        json!({"address": "px.example.com", "ssh": {"proxy": {"type": "http", "address": "proxy-a.example.com", "port": 8080}}}),
        &[("ssh.proxy.password", Some("pass-a"))],
    );
    let base = base_of(&get(&app, host.record_id));
    let mut form = Value::Object(base.data.clone());
    form["ssh"]["proxy"]["address"] = json!("proxy-a2.example.com");
    save_from(&app, host.record_id, RecordType::Host, &base, form, &[]).expect("saved");
    assert_eq!(
        proxy_password(&app, host.record_id).as_deref(),
        Some("pass-a")
    );
    assert!(app.core.sync_conflicts().expect("conflicts").is_empty());
}
