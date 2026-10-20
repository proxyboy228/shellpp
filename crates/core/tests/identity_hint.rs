//! The identity hint of the host panel (UI 3.1; docs/data-model.md, section
//! 9.4; night plan, stage 3 item 7): hosts with the same effective username
//! and the same password, or the same key, become hosts of one identity in
//! one action. Passwords are generated at run time.

mod common;

use common::{as_host, first_run, save};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use shellpp_core::keys::generate::KeyKind;
use shellpp_core::service::dto::{
    KeyGenerateInput, QueryScope, RecordRef, RecordType, RecordsQuery, SortMode,
};
use shellpp_core::service::merge::IdentityHintKind;

#[test]
fn hosts_with_the_same_password_move_onto_one_identity() {
    let u = first_run();
    let vault = u.vault_id;
    let password = format!("pw-{}", common::random_hex());
    let other = format!("pw-{}", common::random_hex());
    // The username of two hosts comes from their group.
    let group = save(
        &u.core,
        vault,
        None,
        RecordType::Group,
        json!({"name": "prod", "ssh": {"username": "deploy"}}),
        &[],
    )
    .unwrap();
    let in_group = |address: &str| {
        save(
            &u.core,
            vault,
            None,
            RecordType::Host,
            json!({"address": address, "group_id": group.record_id().to_string()}),
            &[("ssh.password", Some(password.as_str()))],
        )
        .unwrap()
        .record_id()
    };
    let a = in_group("192.0.2.1");
    let b = in_group("192.0.2.2");
    let with = |address: &str, username: &str, secret: &str| {
        save(
            &u.core,
            vault,
            None,
            RecordType::Host,
            json!({"address": address, "ssh": {"username": username}}),
            &[("ssh.password", Some(secret))],
        )
        .unwrap()
        .record_id()
    };
    let c = with("192.0.2.3", "deploy", &password);
    // Another password, another username: not part of the hint.
    let d = with("192.0.2.4", "deploy", &other);
    let e = with("192.0.2.5", "root", &password);

    let reference = |record_id| RecordRef {
        vault_id: vault,
        record_id,
    };
    let hint = u.core.identity_hint(reference(a)).unwrap().expect("a hint");
    assert_eq!(hint.kind, IdentityHintKind::Password);
    assert_eq!(hint.username, "deploy");
    let mut expected = vec![a, b, c];
    expected.sort();
    assert_eq!(hint.hosts, expected);
    assert!(u.core.identity_hint(reference(d)).unwrap().is_none());
    assert!(u.core.identity_hint(reference(e)).unwrap().is_none());

    let identity = u.core.identity_from_hint(reference(a)).unwrap();
    let identities = u.core.identities_list().unwrap();
    assert_eq!(identities.len(), 1);
    assert_eq!(identities[0].record_id, identity);
    assert_eq!(identities[0].label, "deploy");
    let hosts: Vec<_> = u
        .core
        .records_query(RecordsQuery {
            vault_id: None,
            types: vec![RecordType::Host],
            scope: QueryScope::All,
            text: None,
            tags: Vec::new(),
            sort: SortMode::Manual,
        })
        .unwrap()
        .into_iter()
        .map(as_host)
        .collect();
    assert_eq!(hosts.len(), 5);
    for host in &hosts {
        let ssh = host.data.get("ssh").cloned().unwrap_or(Value::Null);
        if [a, b, c].contains(&host.record_id) {
            assert_eq!(ssh["identity_id"], json!(identity.to_string()));
            assert!(ssh.get("username").is_none_or(Value::is_null));
            assert!(
                host.secrets_set.is_empty(),
                "the password moved to the identity"
            );
        } else {
            assert!(ssh.get("identity_id").is_none_or(Value::is_null));
        }
    }
    // The identity holds the password, and no hint is left.
    assert!(u.core.identity_hint(reference(a)).unwrap().is_none());
    let stored = u.core.identity_password(reference(identity)).unwrap();
    assert_eq!(stored.expose_secret(), password);
}

#[test]
fn hosts_with_the_same_key_move_onto_one_identity() {
    let u = first_run();
    let vault = u.vault_id;
    let key = u
        .core
        .key_generate(KeyGenerateInput {
            vault_id: vault,
            label: "ops key".into(),
            kind: KeyKind::Ed25519,
            passphrase: None,
            save_passphrase: false,
        })
        .unwrap()
        .key;
    let host = |address: &str| {
        save(
            &u.core,
            vault,
            None,
            RecordType::Host,
            json!({"address": address, "ssh": {"username": "ops", "key_id": key.record_id.to_string()}}),
            &[],
        )
        .unwrap()
        .record_id()
    };
    let a = host("192.0.2.11");
    let b = host("192.0.2.12");
    let reference = RecordRef {
        vault_id: vault,
        record_id: a,
    };
    let hint = u.core.identity_hint(reference).unwrap().expect("a hint");
    assert_eq!(hint.kind, IdentityHintKind::Key);
    assert_eq!(hint.hosts.len(), 2);
    assert!(hint.hosts.contains(&b));
    u.core.identity_from_hint(reference).unwrap();
    let identities = u.core.identities_list().unwrap();
    assert_eq!(identities[0].key_id, Some(key.record_id));
}
