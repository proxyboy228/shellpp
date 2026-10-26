//! An identity dialog's save with the version it started from (D221, D223
//! items 2 and 10): only the members it changed are written onto the
//! identity as it is now. Another version written meanwhile (here a whole
//! write, as a pull or another window makes one) keeps the members the edit
//! left alone; in the same members it is kept as a conflict copy.

mod common;

use common::{Unlocked, first_run};
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Map, json};
use shellpp_core::CoreError;
use shellpp_core::keys::generate::KeyKind;
use shellpp_core::model::identity::PasswordChange;
use shellpp_core::service::dto::{KeyGenerateInput, RecordBase, RecordRef, RecordsRef};
use shellpp_core::service::keychain::{IdentitySaveInput, IdentityView};
use uuid::Uuid;

fn input(
    app: &Unlocked,
    record_id: Option<Uuid>,
    label: &str,
    username: &str,
) -> IdentitySaveInput {
    IdentitySaveInput {
        vault_id: app.vault_id,
        record_id,
        label: label.to_owned(),
        username: username.to_owned(),
        key_id: None,
        password: PasswordChange::Keep,
        base: None,
    }
}

/// The base a dialog opened on `view` sends.
fn base_of(view: &IdentityView) -> RecordBase {
    let mut data = Map::new();
    data.insert("label".into(), json!(view.label));
    data.insert("username".into(), json!(view.username));
    data.insert("key_id".into(), json!(view.key_id));
    RecordBase {
        revision: view.revision,
        data,
    }
}

fn get(app: &Unlocked, id: Uuid) -> IdentityView {
    app.core
        .identities_list()
        .expect("identities")
        .into_iter()
        .find(|view| view.record_id == id)
        .expect("identity")
}

fn password(app: &Unlocked, id: Uuid) -> Option<String> {
    app.core
        .identity_password(RecordRef {
            vault_id: app.vault_id,
            record_id: id,
        })
        .ok()
        .map(|password| password.expose_secret().to_owned())
}

/// The identity "ops" (user "deploy", password "first").
fn ops(app: &Unlocked) -> IdentityView {
    app.core
        .identity_save(IdentitySaveInput {
            password: PasswordChange::Set(SecretString::from("first")),
            ..input(app, None, "ops", "deploy")
        })
        .expect("created")
}

#[test]
fn members_the_edit_left_alone_keep_the_newer_version() {
    let app = first_run();
    let view = ops(&app);
    let base = base_of(&view);
    // Another version changes the username (a whole write).
    app.core
        .identity_save(input(&app, Some(view.record_id), "ops", "admin"))
        .expect("other version");

    let saved = app
        .core
        .identity_save(IdentitySaveInput {
            base: Some(base),
            ..input(&app, Some(view.record_id), "ops edited", "deploy")
        })
        .expect("saved");
    assert_eq!(saved.label, "ops edited");
    assert_eq!(saved.username.as_deref(), Some("admin"));
    assert_eq!(password(&app, view.record_id).as_deref(), Some("first"));
    assert!(app.core.sync_conflicts().expect("conflicts").is_empty());
}

#[test]
fn the_same_member_changed_by_both_keeps_the_other_value_in_a_conflict_copy() {
    let app = first_run();
    let view = ops(&app);
    let base = base_of(&view);
    app.core
        .identity_save(input(&app, Some(view.record_id), "other label", "admin"))
        .expect("other version");

    let saved = app
        .core
        .identity_save(IdentitySaveInput {
            base: Some(base),
            ..input(&app, Some(view.record_id), "edited", "deploy")
        })
        .expect("saved");
    assert_eq!(saved.label, "edited");
    assert_eq!(saved.username.as_deref(), Some("admin"));
    let copies = app.core.sync_conflicts().expect("conflicts");
    assert_eq!(copies.len(), 1);
    let losing = copies[0].losing_data.as_ref().expect("losing data");
    assert_eq!(losing["label"], "other label");
    assert_eq!(losing["username"], "admin");
    assert_eq!(copies[0].original_record_id, view.record_id);
}

#[test]
fn a_save_that_changes_nothing_writes_no_version() {
    let app = first_run();
    let view = ops(&app);
    app.env.take_events();
    let saved = app
        .core
        .identity_save(IdentitySaveInput {
            base: Some(base_of(&view)),
            // The dialog sends the label as the user left it: the same value.
            ..input(&app, Some(view.record_id), " ops ", "deploy")
        })
        .expect("saved");
    assert_eq!(saved.revision, view.revision);
    assert!(app.env.take_events().is_empty());
}

#[test]
fn a_cleared_username_is_cleared_and_a_newer_label_stays() {
    let app = first_run();
    let view = ops(&app);
    let base = base_of(&view);
    app.core
        .identity_save(input(&app, Some(view.record_id), "newer", "deploy"))
        .expect("other version");
    let saved = app
        .core
        .identity_save(IdentitySaveInput {
            base: Some(base),
            ..input(&app, Some(view.record_id), "ops", "")
        })
        .expect("saved");
    assert_eq!(saved.label, "newer");
    assert_eq!(saved.username, None);
    assert!(app.core.sync_conflicts().expect("conflicts").is_empty());
}

#[test]
fn a_password_after_a_newer_version_keeps_that_version_whole() {
    let app = first_run();
    let view = ops(&app);

    // Without a newer version the password is simply written.
    app.core
        .identity_save(IdentitySaveInput {
            base: Some(base_of(&view)),
            password: PasswordChange::Set(SecretString::from("second")),
            ..input(&app, Some(view.record_id), "ops", "deploy")
        })
        .expect("saved");
    assert_eq!(password(&app, view.record_id).as_deref(), Some("second"));
    assert!(app.core.sync_conflicts().expect("conflicts").is_empty());

    // After a newer version the dialog cannot tell whether it changed the
    // password too (the view holds none): that version is kept whole.
    let base = base_of(&get(&app, view.record_id));
    app.core
        .identity_save(input(&app, Some(view.record_id), "ops", "newer user"))
        .expect("other version");
    app.core
        .identity_save(IdentitySaveInput {
            base: Some(base),
            password: PasswordChange::Set(SecretString::from("third")),
            ..input(&app, Some(view.record_id), "ops", "deploy")
        })
        .expect("saved");
    assert_eq!(password(&app, view.record_id).as_deref(), Some("third"));
    assert_eq!(
        get(&app, view.record_id).username.as_deref(),
        Some("newer user")
    );
    let copies = app.core.sync_conflicts().expect("conflicts");
    assert_eq!(copies.len(), 1);
    assert_eq!(
        copies[0].losing_data.as_ref().expect("data")["username"],
        "newer user"
    );

    // Removing the password after a newer version keeps it in a copy too.
    let base = base_of(&get(&app, view.record_id));
    app.core
        .identity_save(input(
            &app,
            Some(view.record_id),
            "ops renamed",
            "newer user",
        ))
        .expect("other version");
    app.core
        .identity_save(IdentitySaveInput {
            base: Some(base),
            password: PasswordChange::Clear,
            ..input(&app, Some(view.record_id), "ops", "newer user")
        })
        .expect("saved");
    assert_eq!(password(&app, view.record_id), None);
    assert_eq!(get(&app, view.record_id).label, "ops renamed");
    assert_eq!(app.core.sync_conflicts().expect("conflicts").len(), 2);
}

#[test]
fn a_key_deleted_meanwhile_does_not_block_the_other_members() {
    let app = first_run();
    let key = app
        .core
        .key_generate(KeyGenerateInput {
            vault_id: app.vault_id,
            label: "ops key".into(),
            kind: KeyKind::Ed25519,
            passphrase: None,
            save_passphrase: false,
        })
        .expect("key")
        .key;
    let view = app
        .core
        .identity_save(IdentitySaveInput {
            key_id: Some(key.record_id),
            ..input(&app, None, "ops", "deploy")
        })
        .expect("created");
    let base = base_of(&view);
    app.core
        .record_delete(RecordsRef {
            vault_id: app.vault_id,
            record_ids: vec![key.record_id],
        })
        .expect("key deleted");

    // The dialog still shows the key it opened with; only the label changed.
    let saved = app
        .core
        .identity_save(IdentitySaveInput {
            key_id: Some(key.record_id),
            base: Some(base.clone()),
            ..input(&app, Some(view.record_id), "ops renamed", "deploy")
        })
        .expect("saved");
    assert_eq!(saved.label, "ops renamed");

    // Linking another key that does not exist is still refused.
    let error = app
        .core
        .identity_save(IdentitySaveInput {
            key_id: Some(Uuid::now_v7()),
            base: Some(base_of(&saved)),
            ..input(&app, Some(view.record_id), "ops renamed", "deploy")
        })
        .expect_err("refused");
    assert!(matches!(error, CoreError::Validation(_)), "{error:?}");
}

#[test]
fn a_new_identity_and_a_save_without_base_write_the_input_whole() {
    let app = first_run();
    let view = ops(&app);
    app.core
        .identity_save(input(&app, Some(view.record_id), "other", "admin"))
        .expect("other version");
    // No base: the input is the whole new state (imports, the old dialog).
    let saved = app
        .core
        .identity_save(input(&app, Some(view.record_id), "ops", "deploy"))
        .expect("saved");
    assert_eq!(saved.label, "ops");
    assert_eq!(saved.username.as_deref(), Some("deploy"));
    assert!(app.core.sync_conflicts().expect("conflicts").is_empty());
}
