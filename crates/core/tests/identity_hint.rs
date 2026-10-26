//! The identity hint of the host panel (UI 3.1; docs/data-model.md, section
//! 9.4; night plan, stage 3 item 7): hosts with the same effective username
//! and the same password, or the same key, become hosts of one identity in
//! one action. Passwords are generated at run time.

mod common;

use common::{as_host, first_run, save};
use secrecy::ExposeSecret;
use serde_json::{Value, json};
use shellpp_core::keys::generate::KeyKind;
use shellpp_core::model::identity::PasswordChange;
use shellpp_core::service::dto::SessionTarget;
use shellpp_core::service::dto::{
    KeyGenerateInput, QueryScope, RecordRef, RecordType, RecordsQuery, SortMode,
};
use shellpp_core::service::keychain::IdentitySaveInput;
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

/// The hosts of a hint and their stored `ssh` blocks and secrets.
fn host_states(u: &common::Unlocked, ids: &[uuid::Uuid]) -> Vec<(Value, Vec<String>)> {
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
    ids.iter()
        .map(|id| {
            let host = hosts.iter().find(|host| host.record_id == *id).unwrap();
            (
                host.data.get("ssh").cloned().unwrap_or(Value::Null),
                host.secrets_set.clone(),
            )
        })
        .collect()
}

#[test]
fn every_credential_the_hosts_share_moves_and_nothing_else_is_lost() {
    let u = first_run();
    let vault = u.vault_id;
    let key = u
        .core
        .key_generate(KeyGenerateInput {
            vault_id: vault,
            label: "shared key".into(),
            kind: KeyKind::Ed25519,
            passphrase: None,
            save_passphrase: false,
        })
        .unwrap()
        .key;
    let reference = |record_id| RecordRef {
        vault_id: vault,
        record_id,
    };
    // The same username, password and key on three hosts (the servers may
    // take only the key): the identity holds both.
    let password = format!("pw-{}", common::random_hex());
    let host = |address: &str, secret: &str| {
        save(
            &u.core,
            vault,
            None,
            RecordType::Host,
            json!({"address": address, "ssh": {"username": "app", "key_id": key.record_id.to_string()}}),
            &[("ssh.password", Some(secret))],
        )
        .unwrap()
        .record_id()
    };
    let shared = [
        host("192.0.2.21", &password),
        host("192.0.2.22", &password),
        host("192.0.2.23", &password),
    ];
    let hint = u.core.identity_hint(reference(shared[0])).unwrap().unwrap();
    assert_eq!(hint.kind, IdentityHintKind::Password);
    let identity = u.core.identity_from_hint(reference(shared[0])).unwrap();
    let view = u
        .core
        .identities_list()
        .unwrap()
        .into_iter()
        .find(|view| view.record_id == identity)
        .unwrap();
    assert_eq!(
        view.key_id,
        Some(key.record_id),
        "the shared key came along"
    );
    assert_eq!(
        u.core
            .identity_password(reference(identity))
            .unwrap()
            .expose_secret(),
        password
    );
    for (ssh, secrets) in host_states(&u, &shared) {
        assert!(ssh.get("key_id").is_none_or(Value::is_null));
        assert!(secrets.is_empty());
    }

    // The same key with a password of its own on each host: the identity
    // takes the key, every host keeps its own password.
    let own = [
        format!("pw-{}", common::random_hex()),
        format!("pw-{}", common::random_hex()),
    ];
    let other_key = u
        .core
        .key_generate(KeyGenerateInput {
            vault_id: vault,
            label: "other key".into(),
            kind: KeyKind::Ed25519,
            passphrase: None,
            save_passphrase: false,
        })
        .unwrap()
        .key;
    let keyed: Vec<_> = own
        .iter()
        .enumerate()
        .map(|(n, secret)| {
            save(
                &u.core,
                vault,
                None,
                RecordType::Host,
                json!({
                    "address": format!("198.51.100.{}", n + 1),
                    "ssh": {"username": "batch", "key_id": other_key.record_id.to_string()}
                }),
                &[("ssh.password", Some(secret.as_str()))],
            )
            .unwrap()
            .record_id()
        })
        .collect();
    let hint = u.core.identity_hint(reference(keyed[0])).unwrap().unwrap();
    assert_eq!(hint.kind, IdentityHintKind::Key);
    let identity = u.core.identity_from_hint(reference(keyed[0])).unwrap();
    assert!(
        u.core.identity_password(reference(identity)).is_err(),
        "no password the hosts do not share"
    );
    for (ssh, secrets) in host_states(&u, &keyed) {
        assert_eq!(ssh["identity_id"], json!(identity.to_string()));
        assert!(ssh.get("key_id").is_none_or(Value::is_null));
        assert_eq!(
            secrets,
            vec!["ssh.password".to_owned()],
            "its own password stays"
        );
    }
}

/// A connection reads what the identity holds (data-model 8.3): after the
/// hint, the hosts still sign in with the username, password and key they
/// had.
#[test]
fn hosts_moved_onto_an_identity_still_sign_in() {
    let u = first_run();
    let vault = u.vault_id;
    let key = u
        .core
        .key_generate(KeyGenerateInput {
            vault_id: vault,
            label: "app key".into(),
            kind: KeyKind::Ed25519,
            passphrase: None,
            save_passphrase: false,
        })
        .unwrap()
        .key;
    let password = format!("pw-{}", common::random_hex());
    let ids: Vec<_> = ["192.0.2.31", "192.0.2.32"]
        .iter()
        .map(|address| {
            save(
                &u.core,
                vault,
                None,
                RecordType::Host,
                json!({"address": address, "ssh": {"username": "app", "key_id": key.record_id.to_string()}}),
                &[("ssh.password", Some(password.as_str()))],
            )
            .unwrap()
            .record_id()
        })
        .collect();
    let target = |record_id| SessionTarget::Host {
        vault_id: vault,
        record_id,
    };
    let before = u.core.session_config(&target(ids[0])).unwrap();
    u.core
        .identity_from_hint(RecordRef {
            vault_id: vault,
            record_id: ids[0],
        })
        .unwrap();
    for id in &ids {
        let config = u.core.session_config(&target(*id)).unwrap();
        assert_eq!(config.username.as_deref(), Some("app"));
        assert_eq!(
            config
                .password
                .as_ref()
                .map(|value| value.expose_secret().to_owned()),
            Some(password.clone())
        );
        assert_eq!(
            config.key.as_ref().map(|key| key.label.clone()),
            before.key.as_ref().map(|key| key.label.clone())
        );
    }
}

/// The identity of a group is inherited with it; a host's own value wins
/// over every identity (data-model 8.3).
#[test]
fn a_group_identity_supplies_its_hosts_and_own_values_win() {
    let u = first_run();
    let vault = u.vault_id;
    let secret = format!("pw-{}", common::random_hex());
    let identity = u
        .core
        .identity_save(IdentitySaveInput {
            vault_id: vault,
            record_id: None,
            label: "ops".into(),
            username: "ops".into(),
            key_id: None,
            password: PasswordChange::Set(secrecy::SecretString::from(secret.clone())),
            base: None,
        })
        .unwrap();
    let group = save(
        &u.core,
        vault,
        None,
        RecordType::Group,
        json!({"name": "prod", "ssh": {"identity_id": identity.record_id.to_string()}}),
        &[],
    )
    .unwrap();
    let in_group = |data: serde_json::Value, secrets: &[(&str, Option<&str>)]| {
        let mut data = data;
        data["group_id"] = json!(group.record_id().to_string());
        save(&u.core, vault, None, RecordType::Host, data, secrets)
            .unwrap()
            .record_id()
    };
    let plain = in_group(json!({"address": "192.0.2.41"}), &[]);
    let own = in_group(
        json!({"address": "192.0.2.42", "ssh": {"username": "root"}}),
        &[("ssh.password", Some("own password"))],
    );
    let config = |record_id| {
        u.core
            .session_config(&SessionTarget::Host {
                vault_id: vault,
                record_id,
            })
            .unwrap()
    };
    let inherited = config(plain);
    assert_eq!(inherited.username.as_deref(), Some("ops"));
    assert_eq!(
        inherited
            .password
            .map(|value| value.expose_secret().to_owned()),
        Some(secret)
    );
    let overridden = config(own);
    assert_eq!(overridden.username.as_deref(), Some("root"));
    assert_eq!(
        overridden
            .password
            .map(|value| value.expose_secret().to_owned()),
        Some("own password".to_owned())
    );
}

/// A host that names an identity uses that one: when it is missing (for
/// example deleted on another device), the host does not fall back to its
/// group's identity (data-model 8.3).
#[test]
fn a_missing_own_identity_does_not_fall_back_to_the_group() {
    let u = first_run();
    let vault = u.vault_id;
    let group_secret = format!("pw-{}", common::random_hex());
    let group_identity = u
        .core
        .identity_save(IdentitySaveInput {
            vault_id: vault,
            record_id: None,
            label: "ops".into(),
            username: "ops".into(),
            key_id: None,
            password: PasswordChange::Set(secrecy::SecretString::from(group_secret)),
            base: None,
        })
        .unwrap()
        .record_id;
    let group = save(
        &u.core,
        vault,
        None,
        RecordType::Group,
        json!({"name": "prod", "ssh": {"identity_id": group_identity.to_string()}}),
        &[],
    )
    .unwrap();
    let host = save(
        &u.core,
        vault,
        None,
        RecordType::Host,
        json!({
            "address": "192.0.2.51",
            "group_id": group.record_id().to_string(),
            "ssh": {"identity_id": uuid::Uuid::now_v7().to_string()}
        }),
        &[],
    )
    .unwrap()
    .record_id();
    let config = u
        .core
        .session_config(&SessionTarget::Host {
            vault_id: vault,
            record_id: host,
        })
        .unwrap();
    assert!(
        config.password.is_none(),
        "never the group identity's password"
    );
}
