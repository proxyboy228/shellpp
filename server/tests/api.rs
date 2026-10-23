//! Server integration tests at the API (docs/sync-protocol.md, section 12,
//! "Server integration tests on PostgreSQL and SQLite"): the refresh retry
//! and reuse rules, the push acceptance order, `seq` under concurrent
//! pushes, cursors with class filters, `410` for purged and ahead cursors
//! and two concurrent recovery key reissues. Opaque test envelopes (headers
//! the server checks, ciphertext it never reads) and the client's own HTTP
//! layer; on PostgreSQL with `SHELLPP_TEST_POSTGRES=docker`.

mod support;

use std::sync::Arc;

use shellpp_core::sync::{HttpApi, HttpError, TrustMode};
use shellpp_sync_api::api::{self as wire, B64, ItemStatus};
use shellpp_sync_api::envelope::EnvelopeHeader;
use support::*;
use uuid::Uuid;

fn api(server: &Server) -> HttpApi {
    HttpApi::new(&server.origin, TrustMode::System, "0.7.0").expect("api")
}

fn key(byte: u8) -> B64 {
    B64(vec![byte; 32])
}

struct Account {
    vault_id: Uuid,
    tokens: wire::Tokens,
}

fn register(api: &HttpApi, auth: u8) -> Account {
    let vault_id = Uuid::now_v7();
    let device_id = Uuid::now_v7();
    let response = api
        .register(&wire::RegisterRequest {
            email: email(),
            registration_code: None,
            kdf: wire::Kdf::desktop(),
            kdf_salt: B64(vec![1; 16]),
            auth_key: key(auth),
            recovery_auth_key: key(auth.wrapping_add(1)),
            personal_vault: wire::PersonalVaultRegistration {
                vault_id,
                epoch: 1,
                kek_wrap: B64(vec![4; 73]),
                recovery_wrap: B64(vec![5; 73]),
                epoch_chain: Vec::new(),
            },
            x25519_public_key: B64(vec![6; 32]),
            ssh_id_public_key: "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK0 shellpp:x".into(),
            device_id,
        })
        .expect("registered");
    Account {
        vault_id,
        tokens: response.tokens,
    }
}

/// An opaque envelope of the given header (344 bytes: the smallest).
fn envelope(vault_id: Uuid, record_id: Uuid, class: u8, revision: u64, epoch: u32) -> B64 {
    let header = EnvelopeHeader {
        class,
        tombstone: false,
        vault_id: *vault_id.as_bytes(),
        record_id: *record_id.as_bytes(),
        revision,
        key_epoch: epoch,
    };
    let mut bytes = header.encode().to_vec();
    bytes.extend_from_slice(&[7u8; 24 + 256 + 16]);
    B64(bytes)
}

fn write(base: u64, envelope: B64) -> wire::RecordWrite {
    wire::RecordWrite {
        base_revision: base,
        envelope,
    }
}

fn status_of(error: &HttpError) -> (u16, String) {
    (error.status().unwrap_or(0), error.code())
}

#[test]
fn refresh_rotates_retries_once_and_reuse_revokes_the_family() {
    let server = Server::start();
    let api = api(&server);
    let account = register(&api, 2);
    let first = account.tokens.refresh_token.0.clone();
    let second = api.refresh(&first).expect("rotated");
    // The previous token once more within 60 s (a lost answer): accepted,
    // and it answers with a successor again.
    let retried = api.refresh(&first).expect("one retry within 60 s");
    // A third use of the same token is reuse: the family is revoked.
    let reused = api.refresh(&first).unwrap_err();
    assert_eq!(status_of(&reused), (401, "refresh_token_reused".into()));
    for token in [&second.refresh_token.0, &retried.refresh_token.0] {
        let error = api.refresh(token).unwrap_err();
        assert_eq!(error.status(), Some(401), "the whole family is revoked");
    }
    // Its access tokens too.
    assert!(api.account(&second.access_token.0).is_err());
}

#[test]
fn push_acceptance_order_and_a_record_id_of_another_vault() {
    let server = Server::start();
    let api = api(&server);
    let a = register(&api, 2);
    let b = register(&api, 9);
    let token = a.tokens.access_token.0.clone();
    let record = Uuid::now_v7();
    // B stores the record id first.
    let ok = api
        .push(
            &b.tokens.access_token.0,
            b.vault_id,
            vec![write(0, envelope(b.vault_id, record, 2, 1, 1))],
        )
        .expect("push");
    assert_eq!(ok.results[0].status, ItemStatus::Ok);
    let results = api
        .push(
            &token,
            a.vault_id,
            vec![
                // Another vault's id in the header: invalid envelope.
                write(0, envelope(b.vault_id, Uuid::now_v7(), 2, 1, 1)),
                // An epoch the vault does not have: stale.
                write(0, envelope(a.vault_id, Uuid::now_v7(), 2, 1, 2)),
                // A record id that belongs to another vault: invalid.
                write(0, envelope(a.vault_id, record, 2, 1, 1)),
                // A wrong base revision: conflict with `current: null`.
                write(3, envelope(a.vault_id, Uuid::now_v7(), 2, 4, 1)),
                // A good one.
                write(0, envelope(a.vault_id, Uuid::now_v7(), 2, 1, 1)),
            ],
        )
        .expect("push")
        .results;
    let statuses: Vec<ItemStatus> = results.iter().map(|r| r.status).collect();
    assert_eq!(
        statuses,
        vec![
            ItemStatus::InvalidEnvelope,
            ItemStatus::StaleKeyEpoch,
            ItemStatus::InvalidEnvelope,
            ItemStatus::Conflict,
            ItemStatus::Ok,
        ]
    );
    assert_eq!(results[3].current, Some(None), "absent on the server: null");
}

#[test]
fn seq_is_monotonic_under_concurrent_pushes() {
    let server = Server::start();
    let shared = Arc::new(api(&server));
    let account = register(&shared, 2);
    let token = account.tokens.access_token.0.clone();
    let vault = account.vault_id;
    let handles: Vec<_> = (0..8)
        .map(|_| {
            let api = shared.clone();
            let token = token.clone();
            std::thread::spawn(move || {
                let items = (0..20)
                    .map(|_| write(0, envelope(vault, Uuid::now_v7(), 2, 1, 1)))
                    .collect();
                api.push(&token, vault, items).expect("push")
            })
        })
        .collect();
    let mut seqs: Vec<u64> = handles
        .into_iter()
        .flat_map(|handle| handle.join().expect("thread").results)
        .map(|result| result.seq.expect("seq"))
        .collect();
    let count = seqs.len();
    seqs.sort_unstable();
    seqs.dedup();
    assert_eq!(seqs.len(), count, "every seq is unique");
    // Pulled in pages, the order is strictly increasing.
    let mut cursor = 0;
    let mut last = 0;
    let mut seen = 0;
    loop {
        let page = shared
            .pull(&token, vault, cursor, Some(17), None)
            .expect("pull");
        for record in &page.records {
            assert!(record.seq > last, "increasing");
            last = record.seq;
            seen += 1;
        }
        cursor = page.next_cursor;
        if !page.has_more {
            break;
        }
    }
    assert_eq!(seen, count);
}

#[test]
fn cursors_with_class_filters_and_410_purged_or_ahead() {
    let server = Server::with(Options {
        tombstone_retention_days: 0,
        ..Options::default()
    });
    let api = api(&server);
    let account = register(&api, 2);
    let token = account.tokens.access_token.0.clone();
    let vault = account.vault_id;
    let items = vec![
        write(0, envelope(vault, Uuid::now_v7(), 2, 1, 1)),
        write(0, envelope(vault, Uuid::now_v7(), 3, 1, 1)),
        write(0, envelope(vault, Uuid::now_v7(), 2, 1, 1)),
    ];
    api.push(&token, vault, items).expect("push");
    // Only class 3: one record, and the cursor still moves past the others.
    let page = api.pull(&token, vault, 0, None, Some(&[3])).expect("pull");
    assert_eq!(page.records.len(), 1);
    assert_eq!(
        page.next_cursor, 3,
        "the cursor covers the filtered records"
    );
    // Ahead of the server: 410 ahead.
    let ahead = api.pull(&token, vault, 99, None, None).unwrap_err();
    match &ahead {
        HttpError::Api {
            status: 410,
            details,
            ..
        } => {
            assert_eq!(
                details.get("reason").and_then(|v| v.as_str()),
                Some("ahead")
            );
        }
        other => panic!("expected 410 ahead, got {other:?}"),
    }
    // A tombstone purged at once (retention 0): a cursor below it is purged.
    let id = Uuid::now_v7();
    api.push(&token, vault, vec![write(0, envelope(vault, id, 2, 1, 1))])
        .expect("push");
    let header = EnvelopeHeader {
        class: 2,
        tombstone: true,
        vault_id: *vault.as_bytes(),
        record_id: *id.as_bytes(),
        revision: 2,
        key_epoch: 1,
    };
    let mut bytes = header.encode().to_vec();
    bytes.extend_from_slice(&[7u8; 24 + 256 + 16]);
    api.push(&token, vault, vec![write(1, B64(bytes))])
        .expect("tombstone");
    std::thread::sleep(std::time::Duration::from_millis(1100));
    shellpp_server::admin::purge(&server.db, 0).expect("purge");
    let purged = api.pull(&token, vault, 1, None, None).unwrap_err();
    match &purged {
        HttpError::Api {
            status: 410,
            details,
            ..
        } => {
            assert_eq!(
                details.get("reason").and_then(|v| v.as_str()),
                Some("purged")
            );
        }
        other => panic!("expected 410 purged, got {other:?}"),
    }
}

#[test]
fn two_concurrent_recovery_key_reissues_commit_exactly_one() {
    let server = Server::start();
    let shared = Arc::new(api(&server));
    let account = register(&shared, 2);
    let token = account.tokens.access_token.0.clone();
    let vault = account.vault_id;
    let recovery_record = Uuid::now_v7();
    shared
        .push(
            &token,
            vault,
            vec![write(0, envelope(vault, recovery_record, 1, 1, 1))],
        )
        .expect("the recovery_key record");
    let handles: Vec<_> = (0..2u8)
        .map(|i| {
            let api = shared.clone();
            let token = token.clone();
            std::thread::spawn(move || {
                api.recovery_key_reissue(
                    &token,
                    &wire::RecoveryKeyReissueRequest {
                        current_auth_key: key(2),
                        recovery_auth_key: key(40 + i),
                        kek_wrap: B64(vec![8; 73]),
                        rotation: wire::Rotation {
                            epoch: 2,
                            prev_key_wrap: B64(vec![9; 73]),
                            recovery_wrap: B64(vec![10; 73]),
                        },
                        record: write(1, envelope(vault, recovery_record, 1, 2, 2)),
                    },
                )
            })
        })
        .collect();
    let results: Vec<_> = handles
        .into_iter()
        .map(|h| h.join().expect("thread"))
        .collect();
    let ok = results.iter().filter(|r| r.is_ok()).count();
    assert_eq!(ok, 1, "exactly one reissue commits: {results:?}");
}

/// `recover/complete` for another email than the account's answers 422
/// with `details.field = "email"` (section 5.2).
#[test]
fn recover_complete_with_another_email_is_refused() {
    let server = Server::start();
    let api = api(&server);
    let account = register(&api, 2);
    let info = api
        .account(&account.tokens.access_token.0)
        .expect("account");
    let device = Uuid::now_v7();
    let begun = api
        .recover_begin(&wire::RecoverBeginRequest {
            email: info.email.clone(),
            recovery_auth_key: key(3),
            device_id: device,
        })
        .expect("begin");
    let error = api
        .recover_complete(&wire::RecoverCompleteRequest {
            recovery_ticket: begun.recovery_ticket,
            totp_code: None,
            email: format!("other-{}", info.email),
            kdf: wire::Kdf::desktop(),
            kdf_salt: B64(vec![1; 16]),
            auth_key: key(11),
            kek_wrap: B64(vec![4; 73]),
            rotation: wire::Rotation {
                epoch: 2,
                prev_key_wrap: B64(vec![9; 73]),
                recovery_wrap: B64(vec![10; 73]),
            },
            device_id: device,
        })
        .unwrap_err();
    match error {
        HttpError::Api {
            status: 422,
            details,
            ..
        } => {
            assert_eq!(details.get("field").and_then(|v| v.as_str()), Some("email"));
        }
        other => panic!("expected 422, got {other:?}"),
    }
}
