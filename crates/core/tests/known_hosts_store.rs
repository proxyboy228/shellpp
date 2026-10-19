//! Known hosts (docs/data-model.md, section 9.5; UI 10.1, 10.2; stage 2 plan,
//! section 4.4 and decision 21): TOFU, changed and revoked keys, patterns,
//! hashed entries built with HMAC-SHA1 in the test, listing and removal.
//! Host keys are generated at run time.

mod common;
mod stage2;

use common::{first_run, reasons};
use data_encoding::BASE64;
use serde_json::json;
use shellpp_core::known_hosts::{fingerprint_sha256, hash_host};
use shellpp_core::service::dto::{
    CoreEventKind, KnownHostAddInput, KnownHostSource, RecordsRef, SessionTarget,
};
use shellpp_core::service::host_api::{HostKeyCheck, HostKeyStatus};
use shellpp_core::{CoreError, ValidationReason};
use stage2::{KeyKind, generate};
use uuid::Uuid;

/// A host key: (key type, base64 blob).
fn host_key(kind: KeyKind) -> (String, String) {
    let key = generate(kind, "");
    let public = key.public_key();
    (
        public.algorithm().as_str().to_owned(),
        BASE64.encode(&public.to_bytes().unwrap()),
    )
}

fn check(address: &str, port: u16, key: &(String, String)) -> HostKeyCheck {
    HostKeyCheck {
        address: address.to_owned(),
        port,
        key_type: key.0.clone(),
        public_key: key.1.clone(),
    }
}

fn add(host: &str, port: Option<u16>, key: &(String, String), revoked: bool) -> KnownHostAddInput {
    KnownHostAddInput {
        host: host.to_owned(),
        port,
        key_type: key.0.clone(),
        public_key: key.1.clone(),
        revoked,
        comment: None,
    }
}

#[test]
fn tofu_accept_then_match_and_changed_then_replace() {
    let u = first_run();
    let key = host_key(KeyKind::Ed25519);
    let first = check("Web1.Example.com", 2222, &key);
    assert_eq!(
        u.core.known_host_check(&first).unwrap(),
        HostKeyStatus::Unknown
    );
    u.env.take_events();
    let view = u.core.known_host_accept(&first).unwrap();
    assert!(u.env.take_events().iter().any(|event| matches!(
        &event.kind,
        CoreEventKind::RecordsChanged { classes, .. } if classes == &vec![8]
    )));
    assert_eq!(view.host, "web1.example.com");
    assert_eq!(view.port, Some(2222));
    assert_eq!(
        view.display_address.as_deref(),
        Some("[web1.example.com]:2222")
    );
    assert_eq!(view.source, KnownHostSource::Tofu);
    assert!(!view.hashed && !view.revoked);
    assert_eq!(view.key_type, "ssh-ed25519");
    assert_eq!(Some(view.fingerprint.clone()), fingerprint_sha256(&key.1));
    assert_eq!(
        u.core.known_host_check(&first).unwrap(),
        HostKeyStatus::Match {
            record_id: view.record_id
        }
    );
    // Host names compare case-insensitively; the port must match.
    assert_eq!(
        u.core
            .known_host_check(&check("WEB1.example.COM", 2222, &key))
            .unwrap(),
        HostKeyStatus::Match {
            record_id: view.record_id
        }
    );
    assert_eq!(
        u.core
            .known_host_check(&check("web1.example.com", 22, &key))
            .unwrap(),
        HostKeyStatus::Unknown
    );
    // Accepting again returns the same record.
    assert_eq!(
        u.core.known_host_accept(&first).unwrap().record_id,
        view.record_id
    );
    assert_eq!(u.core.known_hosts_list().unwrap().len(), 1);

    // Another key for the same host: changed, with the old fingerprint.
    let other = host_key(KeyKind::Ed25519);
    let changed = check("web1.example.com", 2222, &other);
    assert_eq!(
        u.core.known_host_check(&changed).unwrap(),
        HostKeyStatus::Changed {
            record_id: view.record_id,
            old_key_type: "ssh-ed25519".into(),
            old_fingerprint: view.fingerprint.clone(),
        }
    );
    let replaced = u.core.known_host_replace(view.record_id, &changed).unwrap();
    assert_eq!(replaced.record_id, view.record_id);
    assert_eq!(replaced.source, KnownHostSource::Replaced);
    assert_eq!(
        Some(replaced.fingerprint.clone()),
        fingerprint_sha256(&other.1)
    );
    assert_eq!(replaced.port, Some(2222));
    assert_eq!(
        u.core.known_host_check(&changed).unwrap(),
        HostKeyStatus::Match {
            record_id: view.record_id
        }
    );
    // The old key is now the changed one.
    assert!(matches!(
        u.core.known_host_check(&first).unwrap(),
        HostKeyStatus::Changed { .. }
    ));
    assert!(matches!(
        u.core
            .known_host_replace(Uuid::now_v7(), &changed)
            .unwrap_err(),
        CoreError::NotFound
    ));
}

#[test]
fn ipv6_addresses_and_port_22() {
    let u = first_run();
    let key = host_key(KeyKind::EcdsaP256);
    let view = u
        .core
        .known_host_accept(&check("2001:DB8::10", 22, &key))
        .unwrap();
    assert_eq!(view.host, "2001:db8::10");
    assert_eq!(view.port, None);
    assert_eq!(view.display_address.as_deref(), Some("2001:db8::10"));
    assert_eq!(view.key_type, "ecdsa-sha2-nistp256");
    assert!(matches!(
        u.core
            .known_host_check(&check("2001:db8::10", 22, &key))
            .unwrap(),
        HostKeyStatus::Match { .. }
    ));
    let on_port = u
        .core
        .known_host_accept(&check("2001:db8::10", 2200, &key))
        .unwrap();
    assert_eq!(
        on_port.display_address.as_deref(),
        Some("[2001:db8::10]:2200")
    );
    assert_ne!(on_port.record_id, view.record_id);
}

#[test]
fn hashed_entries_patterns_and_revoked_keys() {
    let u = first_run();
    let salt: [u8; 20] = shellpp_core::crypto::random_array().unwrap();
    let hashed_key = host_key(KeyKind::Ed25519);
    let hashed_host = hash_host(&salt, "db.example.com", 22);
    let hashed = u
        .core
        .known_host_add(add(&hashed_host, None, &hashed_key, false))
        .unwrap();
    assert!(hashed.hashed);
    assert_eq!(hashed.host, hashed_host);
    assert_eq!(hashed.display_address, None);
    assert_eq!(hashed.port, None);
    assert_eq!(hashed.source, KnownHostSource::Import);
    assert!(matches!(
        u.core.known_host_check(&check("DB.example.com", 22, &hashed_key)).unwrap(),
        HostKeyStatus::Match { record_id } if record_id == hashed.record_id
    ));
    assert_eq!(
        u.core
            .known_host_check(&check("db.example.com", 2222, &hashed_key))
            .unwrap(),
        HostKeyStatus::Unknown
    );
    // A hashed entry for a non-standard port hashes `[host]:port`.
    let hashed_port = u
        .core
        .known_host_add(add(
            &hash_host(&salt, "198.51.100.20", 2022),
            None,
            &hashed_key,
            false,
        ))
        .unwrap();
    assert!(matches!(
        u.core.known_host_check(&check("198.51.100.20", 2022, &hashed_key)).unwrap(),
        HostKeyStatus::Match { record_id } if record_id == hashed_port.record_id
    ));
    // A port next to a hashed host is refused.
    assert_eq!(
        reasons(
            u.core
                .known_host_add(add(&hashed_host, Some(2222), &hashed_key, false))
                .unwrap_err()
        ),
        vec![("port".into(), ValidationReason::NotAllowed)]
    );

    // Wildcard patterns.
    let pattern_key = host_key(KeyKind::Ed25519);
    let pattern = u
        .core
        .known_host_add(add("*.Prod.example.com", None, &pattern_key, false))
        .unwrap();
    assert_eq!(pattern.host, "*.prod.example.com");
    for address in ["a.prod.example.com", "X.PROD.example.com"] {
        assert!(matches!(
            u.core.known_host_check(&check(address, 22, &pattern_key)).unwrap(),
            HostKeyStatus::Match { record_id } if record_id == pattern.record_id
        ));
    }
    assert_eq!(
        u.core
            .known_host_check(&check("prod.example.com", 22, &pattern_key))
            .unwrap(),
        HostKeyStatus::Unknown
    );
    let question = u
        .core
        .known_host_add(add("web?.example.org", Some(2200), &pattern_key, false))
        .unwrap();
    assert!(matches!(
        u.core.known_host_check(&check("web7.example.org", 2200, &pattern_key)).unwrap(),
        HostKeyStatus::Match { record_id } if record_id == question.record_id
    ));

    // A revoked key blocks on any host, whatever its pattern.
    let revoked_key = host_key(KeyKind::Ed25519);
    let revoked = u
        .core
        .known_host_add(add("old.example.net", None, &revoked_key, true))
        .unwrap();
    assert!(revoked.revoked);
    for (address, port) in [
        ("old.example.net", 22),
        ("a.prod.example.com", 22),
        ("203.0.113.9", 2222),
    ] {
        assert_eq!(
            u.core
                .known_host_check(&check(address, port, &revoked_key))
                .unwrap(),
            HostKeyStatus::Revoked {
                record_id: revoked.record_id
            },
            "{address}"
        );
    }
    // A revoked record cannot be replaced.
    assert!(matches!(
        u.core
            .known_host_replace(
                revoked.record_id,
                &check("old.example.net", 22, &pattern_key)
            )
            .unwrap_err(),
        CoreError::WrongState
    ));
    // The same entry twice is one record; revoked and not revoked are two.
    assert_eq!(
        u.core
            .known_host_add(add("old.example.net", Some(22), &revoked_key, true))
            .unwrap()
            .record_id,
        revoked.record_id
    );
    let unrevoked = u
        .core
        .known_host_add(add("old.example.net", None, &revoked_key, false))
        .unwrap();
    assert_ne!(unrevoked.record_id, revoked.record_id);
}

/// "Replace saved key" for a host covered by a wildcard pattern adds a
/// plain entry for that host only; the pattern and the trust of the other
/// hosts it covers stay as they were.
#[test]
fn replacing_a_pattern_key_never_rewrites_the_pattern() {
    let u = first_run();
    let old = host_key(KeyKind::Ed25519);
    let pattern = u
        .core
        .known_host_add(add("*.corp.example.com", None, &old, false))
        .unwrap();
    let presented = host_key(KeyKind::Ed25519);
    let changed = check("A.corp.example.com", 22, &presented);
    assert!(matches!(
        u.core.known_host_check(&changed).unwrap(),
        HostKeyStatus::Changed { record_id, .. } if record_id == pattern.record_id
    ));
    let replaced = u
        .core
        .known_host_replace(pattern.record_id, &changed)
        .unwrap();
    assert_ne!(replaced.record_id, pattern.record_id);
    assert_eq!(replaced.host, "a.corp.example.com");
    assert_eq!(replaced.port, None);
    assert_eq!(replaced.source, KnownHostSource::Replaced);
    assert_eq!(
        Some(replaced.fingerprint.clone()),
        fingerprint_sha256(&presented.1)
    );
    // This host now matches its own entry.
    assert_eq!(
        u.core.known_host_check(&changed).unwrap(),
        HostKeyStatus::Match {
            record_id: replaced.record_id
        }
    );
    // The pattern is unchanged: other hosts keep the old key ...
    let views = u.core.known_hosts_list().unwrap();
    let kept = views
        .iter()
        .find(|view| view.record_id == pattern.record_id)
        .unwrap();
    assert_eq!(kept.host, "*.corp.example.com");
    assert_eq!(kept.fingerprint, pattern.fingerprint);
    assert_eq!(kept.source, KnownHostSource::Import);
    assert_eq!(
        u.core
            .known_host_check(&check("b.corp.example.com", 22, &old))
            .unwrap(),
        HostKeyStatus::Match {
            record_id: pattern.record_id
        }
    );
    // ... and the presented key is not trusted for them.
    assert!(matches!(
        u.core
            .known_host_check(&check("b.corp.example.com", 22, &presented))
            .unwrap(),
        HostKeyStatus::Changed { record_id, .. } if record_id == pattern.record_id
    ));
    // A record for another host is never rewritten either.
    let other = u
        .core
        .known_host_add(add("other.example.net", None, &old, false))
        .unwrap();
    let elsewhere = check("third.example.net", 22, &presented);
    let added = u
        .core
        .known_host_replace(other.record_id, &elsewhere)
        .unwrap();
    assert_ne!(added.record_id, other.record_id);
    assert_eq!(added.host, "third.example.net");
    assert_eq!(
        u.core
            .known_host_check(&check("other.example.net", 22, &old))
            .unwrap(),
        HostKeyStatus::Match {
            record_id: other.record_id
        }
    );
}

#[test]
fn preferred_key_types_come_from_matching_entries() {
    let u = first_run();
    let rsa = host_key(KeyKind::Rsa(2048));
    let ed = host_key(KeyKind::Ed25519);
    u.core
        .known_host_add(add("multi.example.com", Some(2222), &rsa, false))
        .unwrap();
    u.core
        .known_host_add(add("*.example.com", Some(2222), &ed, false))
        .unwrap();
    u.core
        .known_host_add(add(
            "multi.example.com",
            Some(2222),
            &host_key(KeyKind::EcdsaP256),
            true,
        ))
        .unwrap();
    let host = u.host(json!({"address": "multi.example.com", "ssh": {"port": 2222}}));
    let config = u
        .core
        .session_config(&SessionTarget::Host {
            vault_id: u.vault_id,
            record_id: host.record_id,
        })
        .unwrap();
    assert_eq!(
        config.known_key_types,
        vec!["ssh-rsa".to_owned(), "ssh-ed25519".to_owned()]
    );
    // A server presenting its Ed25519 key matches the pattern entry, not "changed".
    assert!(matches!(
        u.core
            .known_host_check(&check("multi.example.com", 2222, &ed))
            .unwrap(),
        HostKeyStatus::Match { .. }
    ));
    let quick = u
        .core
        .session_config(&SessionTarget::Quick {
            address: "other.example.com".into(),
            port: Some(2222),
            username: None,
        })
        .unwrap();
    assert_eq!(quick.known_key_types, vec!["ssh-ed25519".to_owned()]);
}

#[test]
fn list_order_saved_hosts_and_removal() {
    let mut u = first_run();
    let key = host_key(KeyKind::Ed25519);
    let salt: [u8; 20] = shellpp_core::crypto::random_array().unwrap();
    let hashed = u
        .core
        .known_host_add(add(
            &hash_host(&salt, "h.example.com", 22),
            None,
            &key,
            false,
        ))
        .unwrap();
    let zulu = u
        .core
        .known_host_add(add("Zulu.example.com", None, &key, false))
        .unwrap();
    let alpha = u
        .core
        .known_host_add(add("alpha.example.com", Some(2222), &key, false))
        .unwrap();
    let beta = u
        .core
        .known_host_add(add("beta.example.com", None, &key, false))
        .unwrap();
    let order: Vec<Uuid> = u
        .core
        .known_hosts_list()
        .unwrap()
        .into_iter()
        .map(|view| view.record_id)
        .collect();
    assert_eq!(
        order,
        vec![
            alpha.record_id,
            beta.record_id,
            zulu.record_id,
            hashed.record_id
        ]
    );

    // Saved hosts whose address and effective port match an entry; the
    // port is inherited from a group.
    let group = u.group(json!({"name": "ports", "ssh": {"port": 2222}}));
    let inherits = u.host(json!({"address": "ALPHA.example.com", "group_id": group.record_id}));
    let plain = u.host(json!({"address": "alpha.example.com"}));
    let hashed_host = u.host(json!({"address": "h.example.com"}));
    let views = u.core.known_hosts_list().unwrap();
    let find = |id: Uuid| views.iter().find(|view| view.record_id == id).unwrap();
    assert_eq!(find(alpha.record_id).host_ids, vec![inherits.record_id]);
    assert_eq!(find(hashed.record_id).host_ids, vec![hashed_host.record_id]);
    assert!(find(beta.record_id).host_ids.is_empty());
    let _ = plain;

    // Removal: not_found for ids that are not known hosts.
    assert!(matches!(
        u.core
            .known_host_remove(RecordsRef {
                vault_id: u.vault_id,
                record_ids: vec![alpha.record_id, inherits.record_id],
            })
            .unwrap_err(),
        CoreError::NotFound
    ));
    assert!(matches!(
        u.core
            .known_host_remove(RecordsRef {
                vault_id: Uuid::now_v7(),
                record_ids: vec![alpha.record_id],
            })
            .unwrap_err(),
        CoreError::NotFound
    ));
    u.env.take_events();
    u.core
        .known_host_remove(RecordsRef {
            vault_id: u.vault_id,
            record_ids: vec![alpha.record_id, alpha.record_id, beta.record_id],
        })
        .unwrap();
    assert!(u.env.take_events().iter().any(|event| matches!(
        &event.kind,
        CoreEventKind::RecordsChanged { classes, record_ids, .. }
            if classes == &vec![8] && record_ids.len() == 2
    )));
    assert_eq!(u.core.known_hosts_list().unwrap().len(), 2);
    assert_eq!(
        u.core
            .known_host_check(&check("alpha.example.com", 2222, &key))
            .unwrap(),
        HostKeyStatus::Unknown
    );

    // Persisted across a restart.
    u.reopen();
    u.core.unlock(u.password.clone()).unwrap();
    let after: Vec<Uuid> = u
        .core
        .known_hosts_list()
        .unwrap()
        .into_iter()
        .map(|view| view.record_id)
        .collect();
    assert_eq!(after, vec![zulu.record_id, hashed.record_id]);
    u.core.lock().unwrap();
    assert!(matches!(
        u.core.known_hosts_list().unwrap_err(),
        CoreError::Locked
    ));
    assert!(matches!(
        u.core
            .known_host_check(&check("zulu.example.com", 22, &key))
            .unwrap_err(),
        CoreError::Locked
    ));
}

#[test]
fn add_validation() {
    let u = first_run();
    let key = host_key(KeyKind::Ed25519);
    let rsa = host_key(KeyKind::Rsa(2048));
    let cases: Vec<(KnownHostAddInput, Vec<(String, ValidationReason)>)> = vec![
        (
            add("", None, &key, false),
            vec![("host".into(), ValidationReason::Required)],
        ),
        (
            add("a b", None, &key, false),
            vec![("host".into(), ValidationReason::Invalid)],
        ),
        (
            add("a,b", None, &key, false),
            vec![("host".into(), ValidationReason::Invalid)],
        ),
        (
            add("[a]:22", None, &key, false),
            vec![("host".into(), ValidationReason::Invalid)],
        ),
        (
            add("!a.example.com", None, &key, false),
            vec![("host".into(), ValidationReason::Invalid)],
        ),
        (
            add("a.example.com", Some(0), &key, false),
            vec![("port".into(), ValidationReason::OutOfRange)],
        ),
        (
            KnownHostAddInput {
                key_type: "ssh-rsa".into(),
                ..add("a.example.com", None, &key, false)
            },
            vec![("public_key".into(), ValidationReason::Invalid)],
        ),
        (
            KnownHostAddInput {
                public_key: rsa.1.clone(),
                ..add("a.example.com", None, &key, false)
            },
            vec![("public_key".into(), ValidationReason::Invalid)],
        ),
        (
            KnownHostAddInput {
                key_type: "Bad Type".into(),
                ..add("a.example.com", None, &key, false)
            },
            vec![("key_type".into(), ValidationReason::Invalid)],
        ),
        (
            KnownHostAddInput {
                comment: Some("x".repeat(257)),
                ..add("a.example.com", None, &key, false)
            },
            vec![("comment".into(), ValidationReason::TooLong)],
        ),
    ];
    for (input, expected) in cases {
        assert_eq!(reasons(u.core.known_host_add(input).unwrap_err()), expected);
    }
    let with_comment = u
        .core
        .known_host_add(KnownHostAddInput {
            comment: Some(" imported ".into()),
            ..add("a.example.com", None, &key, false)
        })
        .unwrap();
    assert_eq!(with_comment.comment.as_deref(), Some("imported"));
    // An invalid presented key is refused by the check.
    assert_eq!(
        reasons(
            u.core
                .known_host_check(&HostKeyCheck {
                    public_key: "!!".into(),
                    ..check("a.example.com", 22, &key)
                })
                .unwrap_err()
        ),
        vec![("public_key".into(), ValidationReason::Invalid)]
    );
    // "Accept and save" never stores a pattern.
    for address in ["web*.example.com", "web?.example.com"] {
        assert_eq!(
            reasons(
                u.core
                    .known_host_accept(&check(address, 22, &key))
                    .unwrap_err()
            ),
            vec![("host".into(), ValidationReason::Invalid)]
        );
    }
}
