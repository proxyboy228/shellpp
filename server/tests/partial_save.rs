//! An editor's save writes only the members it changed (D220): a version
//! that a pull brought between the editor's read and its save keeps every
//! member the edit did not touch, and one that changed the same members is
//! kept as a conflict copy on both devices, never lost.

mod support;

use serde_json::{Value, json};
use shellpp_core::service::SignInOutcome;
use shellpp_core::service::dto::RecordType;
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
