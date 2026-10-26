//! An editor's save writes only the members it changed (D220): a version
//! that a pull brought between the editor's read and its save keeps every
//! member the edit did not touch, and one that changed the same members is
//! kept as a conflict copy on both devices, never lost.

mod support;

use serde_json::{Value, json};
use shellpp_core::model::identity::PasswordChange;
use shellpp_core::service::SignInOutcome;
use shellpp_core::service::dto::{RecordBase, RecordType};
use shellpp_core::service::keychain::{IdentitySaveInput, IdentityView};
use support::*;
use uuid::Uuid;

/// Two devices of one account with a host both of them hold.
fn two_devices() -> (Server, Client, Client, Uuid) {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    assert!(matches!(
        b.sign_in(&server, &email, &password),
        SignInOutcome::Done(_)
    ));
    let id = a.save(
        None,
        RecordType::Host,
        json!({"address": "web.example.com", "label": "web", "notes": "first"}),
    );
    a.sync();
    b.sync();
    assert_eq!(b.data(id)["label"], "web");
    (server, a, b, id)
}

/// The edited form: the base's data with `member` set to `value`.
fn edited(base: &shellpp_core::service::dto::RecordBase, member: &str, value: &str) -> Value {
    let mut data = Value::Object(base.data.clone());
    data[member] = Value::String(value.to_owned());
    data
}

#[test]
fn a_pull_between_read_and_save_keeps_the_members_the_edit_left_alone() {
    let (_server, a, b, id) = two_devices();
    // A's editor reads the host; B changes the notes and pushes; A pulls.
    let base = a.base(id);
    let mut data = b.data(id);
    data["notes"] = json!("from B");
    b.save(Some(id), RecordType::Host, data);
    b.sync();
    a.sync();
    assert_eq!(a.data(id)["notes"], "from B");

    // A's editor saves its label: B's notes stay.
    a.save_from(
        id,
        RecordType::Host,
        &base,
        edited(&base, "label", "from A"),
    )
    .expect("saved");
    assert_eq!(a.data(id)["label"], "from A");
    assert_eq!(a.data(id)["notes"], "from B");
    assert!(a.core.sync_conflicts().expect("conflicts").is_empty());

    a.sync();
    b.sync();
    assert_eq!(b.data(id)["label"], "from A");
    assert_eq!(b.data(id)["notes"], "from B");
    assert!(b.core.sync_conflicts().expect("conflicts").is_empty());
}

#[test]
fn a_pull_of_the_same_member_is_kept_as_a_conflict_copy_on_both_devices() {
    let (_server, a, b, id) = two_devices();
    let base = a.base(id);
    let mut data = b.data(id);
    data["label"] = json!("from B");
    data["notes"] = json!("notes of B");
    b.save(Some(id), RecordType::Host, data);
    b.sync();
    a.sync();

    // The edit is the newer version of the label; B's label goes to a copy,
    // B's notes (not edited) stay in the record.
    a.save_from(
        id,
        RecordType::Host,
        &base,
        edited(&base, "label", "from A"),
    )
    .expect("saved");
    assert_eq!(a.data(id)["label"], "from A");
    assert_eq!(a.data(id)["notes"], "notes of B");
    let copies = a.core.sync_conflicts().expect("conflicts");
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].original_record_id, id);
    assert_eq!(
        copies[0].losing_data.as_ref().expect("data")["label"],
        "from B"
    );

    a.sync();
    b.sync();
    assert_eq!(b.data(id)["label"], "from A");
    let copies = b.core.sync_conflicts().expect("conflicts");
    assert_eq!(copies.len(), 1);
    assert_eq!(
        copies[0].losing_data.as_ref().expect("data")["label"],
        "from B"
    );
}

#[test]
fn a_pull_that_deleted_the_record_gives_way_to_the_edit_and_keeps_the_deletion() {
    let (_server, a, b, id) = two_devices();
    let base = a.base(id);
    b.delete(id);
    b.sync();
    a.sync();
    assert!(!a.addresses().contains(&"web.example.com".to_owned()));

    // The edit brings the host back from Recently deleted, with its edit.
    a.save_from(id, RecordType::Host, &base, edited(&base, "label", "kept"))
        .expect("saved");
    assert_eq!(a.data(id)["label"], "kept");
    assert_eq!(a.data(id)["notes"], "first");
    assert!(
        a.core
            .recently_deleted_list()
            .expect("trash")
            .iter()
            .all(|entry| entry.record_id != id)
    );
    let copies = a.core.sync_conflicts().expect("conflicts");
    assert_eq!(copies.len(), 1);
    assert!(copies[0].losing_deleted);

    a.sync();
    b.sync();
    assert_eq!(b.data(id)["label"], "kept");
    let copies = b.core.sync_conflicts().expect("conflicts");
    assert_eq!(copies.len(), 1);
    assert!(copies[0].losing_deleted);
}

#[test]
fn a_newer_version_before_the_deletion_is_kept_when_the_edit_brings_the_record_back() {
    let (_server, a, b, id) = two_devices();
    let base = a.base(id);
    let mut data = b.data(id);
    data["label"] = json!("from B");
    b.save(Some(id), RecordType::Host, data);
    b.sync();
    a.sync();
    b.delete(id);
    b.sync();
    a.sync();

    a.save_from(id, RecordType::Host, &base, edited(&base, "label", "kept"))
        .expect("saved");
    assert_eq!(a.data(id)["label"], "kept");
    let copies = a.core.sync_conflicts().expect("conflicts");
    assert_eq!(copies.len(), 2, "the deletion and the version it deleted");
    assert!(copies.iter().any(|copy| copy.losing_deleted));
    assert!(copies.iter().any(|copy| {
        copy.losing_data
            .as_ref()
            .and_then(|data| data.get("label"))
            .and_then(Value::as_str)
            == Some("from B")
    }));
    a.sync();
    b.sync();
    assert_eq!(b.core.sync_conflicts().expect("conflicts").len(), 2);
}

#[test]
fn a_save_that_changes_nothing_does_not_bring_a_deleted_record_back() {
    let (_server, a, b, id) = two_devices();
    let base = a.base(id);
    b.delete(id);
    b.sync();
    a.sync();
    let unchanged = Value::Object(base.data.clone());
    assert!(matches!(
        a.save_from(id, RecordType::Host, &base, unchanged),
        Err(shellpp_core::CoreError::NotFound)
    ));
    assert!(!a.addresses().contains(&"web.example.com".to_owned()));
}

/// An identity both devices of one account hold (label "ops", user
/// "deploy"), and its view on A (D223 items 2 and 10).
fn identity_on_two_devices() -> (Server, Client, Client, IdentityView) {
    let server = Server::start();
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.register(&server, &email, &password);
    let b = Client::empty("B");
    assert!(matches!(
        b.sign_in(&server, &email, &password),
        SignInOutcome::Done(_)
    ));
    let view = a
        .core
        .identity_save(identity_input(&a, None, "ops", "deploy", None))
        .expect("identity");
    a.sync();
    b.sync();
    assert_eq!(
        identity(&b, view.record_id).username.as_deref(),
        Some("deploy")
    );
    (server, a, b, view)
}

fn identity_input(
    client: &Client,
    record_id: Option<Uuid>,
    label: &str,
    username: &str,
    base: Option<RecordBase>,
) -> IdentitySaveInput {
    IdentitySaveInput {
        vault_id: client.vault_id(),
        record_id,
        label: label.to_owned(),
        username: username.to_owned(),
        key_id: None,
        password: PasswordChange::Keep,
        base,
    }
}

fn identity(client: &Client, id: Uuid) -> IdentityView {
    client
        .core
        .identities_list()
        .expect("identities")
        .into_iter()
        .find(|view| view.record_id == id)
        .expect("identity")
}

/// The base the identity dialog sends: the view it opened.
fn identity_base(view: &IdentityView) -> RecordBase {
    let mut data = serde_json::Map::new();
    data.insert("label".into(), json!(view.label));
    data.insert("username".into(), json!(view.username));
    data.insert("key_id".into(), json!(view.key_id));
    RecordBase {
        revision: view.revision,
        data,
    }
}

#[test]
fn an_identity_pulled_between_read_and_save_keeps_the_members_the_edit_left_alone() {
    let (_server, a, b, view) = identity_on_two_devices();
    // A's dialog reads the identity; B changes the username and pushes; A pulls.
    let base = identity_base(&view);
    b.core
        .identity_save(identity_input(
            &b,
            Some(view.record_id),
            "ops",
            "from B",
            None,
        ))
        .expect("B's version");
    b.sync();
    a.sync();
    assert_eq!(
        identity(&a, view.record_id).username.as_deref(),
        Some("from B")
    );

    // A's dialog saves its label: B's username stays, on both devices.
    let saved = a
        .core
        .identity_save(identity_input(
            &a,
            Some(view.record_id),
            "ops of A",
            "deploy",
            Some(base),
        ))
        .expect("saved");
    assert_eq!(saved.label, "ops of A");
    assert_eq!(saved.username.as_deref(), Some("from B"));
    assert!(a.core.sync_conflicts().expect("conflicts").is_empty());
    a.sync();
    b.sync();
    let on_b = identity(&b, view.record_id);
    assert_eq!(on_b.label, "ops of A");
    assert_eq!(on_b.username.as_deref(), Some("from B"));
    assert!(b.core.sync_conflicts().expect("conflicts").is_empty());
}

#[test]
fn an_identity_pulled_in_the_same_member_is_kept_as_a_conflict_copy_on_both_devices() {
    let (_server, a, b, view) = identity_on_two_devices();
    let base = identity_base(&view);
    b.core
        .identity_save(identity_input(
            &b,
            Some(view.record_id),
            "label of B",
            "user of B",
            None,
        ))
        .expect("B's version");
    b.sync();
    a.sync();

    let saved = a
        .core
        .identity_save(identity_input(
            &a,
            Some(view.record_id),
            "label of A",
            "deploy",
            Some(base),
        ))
        .expect("saved");
    assert_eq!(saved.label, "label of A");
    assert_eq!(saved.username.as_deref(), Some("user of B"));
    let copies = a.core.sync_conflicts().expect("conflicts");
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].original_record_id, view.record_id);
    assert_eq!(
        copies[0].losing_data.as_ref().expect("data")["label"],
        "label of B"
    );
    a.sync();
    b.sync();
    assert_eq!(identity(&b, view.record_id).label, "label of A");
    assert_eq!(b.core.sync_conflicts().expect("conflicts").len(), 1);
}
