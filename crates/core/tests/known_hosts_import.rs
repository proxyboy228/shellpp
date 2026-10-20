//! Known hosts import, export and undo of Remove (UI 10.1, D21;
//! docs/data-model.md, sections 9.5 and 18.4; docs/storage-format.md,
//! section 13.6). Host keys and known_hosts files are generated at run time;
//! no real file is read.

mod common;
mod stage2;

use std::time::Duration;

use common::{first_run, reasons};
use data_encoding::BASE64;
use shellpp_core::known_hosts::hash_host;
use shellpp_core::known_hosts::openssh::{IMPORT_BATCH, MAX_IMPORT_BYTES};
use shellpp_core::service::dto::{
    CoreEventKind, KnownHostSource, KnownHostsImportSummary, RecordsRef,
};
use shellpp_core::service::host_api::{HostKeyCheck, HostKeyStatus};
use shellpp_core::{CoreError, ValidationReason};
use stage2::{KeyKind, generate};

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

fn summary(
    imported: usize,
    duplicates: usize,
    skipped: usize,
    invalid: usize,
) -> KnownHostsImportSummary {
    KnownHostsImportSummary {
        imported,
        duplicates,
        skipped,
        invalid,
    }
}

#[test]
fn import_counts_entries_and_connections_match_them() {
    let u = first_run();
    let ed = host_key(KeyKind::Ed25519);
    let ecdsa = host_key(KeyKind::EcdsaP256);
    let revoked = host_key(KeyKind::Ed25519);
    let salt: [u8; 20] = shellpp_core::crypto::random_array().unwrap();
    let hashed = hash_host(&salt, "db.example.com", 2222);
    let file = format!(
        "# generated for the test\n\
         \n\
         web.example.com,192.0.2.10 {} {} laptop\n\
         [web.example.com]:2200 {} {}\n\
         {hashed} {} {}\n\
         @revoked old.example.net {} {}\n\
         @cert-authority *.example.com {} {}\n\
         !skip.example.com,keep.example.com {} {}\n\
         broken line\n\
         web.example.com ssh-rsa {}\n",
        ed.0,
        ed.1,
        ecdsa.0,
        ecdsa.1,
        ed.0,
        ed.1,
        revoked.0,
        revoked.1,
        ed.0,
        ed.1,
        ed.0,
        ed.1,
        ed.1,
    );
    u.env.take_events();
    let result = u.core.known_hosts_import(file.as_bytes()).unwrap();
    assert_eq!(result, summary(6, 0, 2, 2));
    // One transaction, one event for the known_hosts class.
    let events = u.env.take_events();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(
                &event.kind,
                CoreEventKind::RecordsChanged { classes, record_ids, .. }
                    if classes == &vec![8] && record_ids.len() == 6
            ))
            .count(),
        1
    );

    let views = u.core.known_hosts_list().unwrap();
    assert_eq!(views.len(), 6);
    assert!(
        views
            .iter()
            .all(|view| view.source == KnownHostSource::Import)
    );
    let web = views
        .iter()
        .find(|view| view.display_address.as_deref() == Some("web.example.com"))
        .unwrap();
    assert_eq!(web.comment.as_deref(), Some("laptop"));
    let hashed_view = views.iter().find(|view| view.hashed).unwrap();
    assert_eq!(hashed_view.host, hashed);
    assert_eq!(hashed_view.display_address, None);
    assert_eq!(hashed_view.key_type, ed.0);
    assert!(views.iter().any(|view| view.revoked));

    // Connections check against the imported entries: the hashed one by
    // HMAC, the bracketed one with its port.
    assert!(matches!(
        u.core.known_host_check(&check("db.example.com", 2222, &ed)).unwrap(),
        HostKeyStatus::Match { record_id } if record_id == hashed_view.record_id
    ));
    assert!(matches!(
        u.core
            .known_host_check(&check("db.example.com", 2222, &ecdsa))
            .unwrap(),
        HostKeyStatus::Changed { .. }
    ));
    assert!(matches!(
        u.core
            .known_host_check(&check("web.example.com", 2200, &ecdsa))
            .unwrap(),
        HostKeyStatus::Match { .. }
    ));
    assert!(matches!(
        u.core
            .known_host_check(&check("192.0.2.10", 22, &ed))
            .unwrap(),
        HostKeyStatus::Match { .. }
    ));
    assert!(matches!(
        u.core
            .known_host_check(&check("anything.example.org", 22, &revoked))
            .unwrap(),
        HostKeyStatus::Revoked { .. }
    ));
    // The negated pattern was skipped, the other one of its line imported;
    // the @cert-authority line added nothing.
    assert!(matches!(
        u.core
            .known_host_check(&check("keep.example.com", 22, &ed))
            .unwrap(),
        HostKeyStatus::Match { .. }
    ));
    assert_eq!(
        u.core
            .known_host_check(&check("skip.example.com", 22, &ed))
            .unwrap(),
        HostKeyStatus::Unknown
    );

    // Importing the same file again adds nothing and changes nothing.
    let again = u.core.known_hosts_import(file.as_bytes()).unwrap();
    assert_eq!(again, summary(0, 6, 2, 2));
    assert!(u.env.take_events().is_empty());
    assert_eq!(u.core.known_hosts_list().unwrap().len(), 6);
}

#[test]
fn duplicates_inside_a_file_and_of_tofu_entries() {
    let u = first_run();
    let key = host_key(KeyKind::Ed25519);
    u.core
        .known_host_accept(&check("tofu.example.com", 22, &key))
        .unwrap();
    let file = format!(
        "tofu.example.com {0} {1}\n\
         new.example.com,NEW.example.com {0} {1}\n\
         new.example.com {0} {1}\n\
         @revoked new.example.com {0} {1}\n",
        key.0, key.1
    );
    // A revoked entry differs from the plain one by its marker.
    assert_eq!(
        u.core.known_hosts_import(file.as_bytes()).unwrap(),
        summary(2, 3, 0, 0)
    );
    let views = u.core.known_hosts_list().unwrap();
    assert_eq!(views.len(), 3);
    let tofu = views
        .iter()
        .find(|view| view.host == "tofu.example.com")
        .unwrap();
    assert_eq!(tofu.source, KnownHostSource::Tofu);
}

#[test]
fn export_writes_known_hosts_and_imports_back_identically() {
    let u = first_run();
    let ed = host_key(KeyKind::Ed25519);
    let rsa = host_key(KeyKind::Rsa(2048));
    let old = host_key(KeyKind::Ed25519);
    let salt: [u8; 20] = shellpp_core::crypto::random_array().unwrap();
    let hashed = hash_host(&salt, "h.example.com", 22);
    let file = format!(
        "zulu.example.com {} {} a comment\n\
         [alpha.example.com]:2222 {} {}\n\
         @revoked old.example.net {} {}\n\
         {hashed} {} {}\n",
        ed.0, ed.1, rsa.0, rsa.1, old.0, old.1, ed.0, ed.1,
    );
    assert_eq!(
        u.core.known_hosts_import(file.as_bytes()).unwrap(),
        summary(4, 0, 0, 0)
    );
    let exported = u.core.known_hosts_export().unwrap();
    assert_eq!(
        exported,
        format!(
            "[alpha.example.com]:2222 {} {}\n\
             @revoked old.example.net {} {}\n\
             zulu.example.com {} {}\n\
             {hashed} {} {}\n",
            rsa.0, rsa.1, old.0, old.1, ed.0, ed.1, ed.0, ed.1,
        )
    );
    // Into another profile: the same entries, matching the same way.
    let other = first_run();
    assert_eq!(
        other.core.known_hosts_import(exported.as_bytes()).unwrap(),
        summary(4, 0, 0, 0)
    );
    assert_eq!(other.core.known_hosts_export().unwrap(), exported);
    assert!(matches!(
        other
            .core
            .known_host_check(&check("h.example.com", 22, &ed))
            .unwrap(),
        HostKeyStatus::Match { .. }
    ));
    assert!(matches!(
        other
            .core
            .known_host_check(&check("h.example.com", 22, &old))
            .unwrap(),
        HostKeyStatus::Revoked { .. }
    ));
    // And back into the first: all duplicates.
    assert_eq!(
        u.core.known_hosts_import(exported.as_bytes()).unwrap(),
        summary(0, 4, 0, 0)
    );
    let empty = first_run();
    assert_eq!(empty.core.known_hosts_export().unwrap(), "");
}

#[test]
fn limits_and_lock() {
    let u = first_run();
    let too_big = vec![b'#'; MAX_IMPORT_BYTES + 1];
    assert_eq!(
        reasons(u.core.known_hosts_import(&too_big).unwrap_err()),
        vec![("file".into(), ValidationReason::TooLong)]
    );
    assert_eq!(u.core.known_hosts_import(b"").unwrap(), summary(0, 0, 0, 0));
    u.core.lock().unwrap();
    assert!(matches!(
        u.core.known_hosts_import(b"").unwrap_err(),
        CoreError::Locked
    ));
    assert!(matches!(
        u.core.known_hosts_export().unwrap_err(),
        CoreError::Locked
    ));
}

/// A file larger than one batch is written in several transactions, and a
/// line with thousands of host patterns is invalid (stage 2b limits).
#[test]
fn large_files_are_written_in_batches_and_pattern_floods_are_invalid() {
    let u = first_run();
    let key = host_key(KeyKind::Ed25519);
    let count = IMPORT_BATCH + 500;
    let mut file = String::new();
    for n in 0..count {
        file.push_str(&format!("h{n}.example.com {} {}\n", key.0, key.1));
    }
    let flood: Vec<String> = (0..10_000).map(|n| format!("f{n}")).collect();
    file.push_str(&format!("{} {} {}\n", flood.join(","), key.0, key.1));
    u.env.take_events();
    assert_eq!(
        u.core.known_hosts_import(file.as_bytes()).unwrap(),
        summary(count, 0, 0, 1)
    );
    let batches = u
        .env
        .take_events()
        .iter()
        .filter(|event| matches!(&event.kind, CoreEventKind::RecordsChanged { .. }))
        .count();
    assert_eq!(batches, 2);
    assert_eq!(u.core.known_hosts_list().unwrap().len(), count);
    // Again: every entry is a duplicate, across the batches too.
    assert_eq!(
        u.core.known_hosts_import(file.as_bytes()).unwrap(),
        summary(0, count, 0, 1)
    );
}

#[test]
fn remove_can_be_undone_within_the_window() {
    let u = first_run();
    let key = host_key(KeyKind::Ed25519);
    let file = format!(
        "a.example.com {0} {1} note\nb.example.com {0} {1}\n",
        key.0, key.1
    );
    u.core.known_hosts_import(file.as_bytes()).unwrap();
    let views = u.core.known_hosts_list().unwrap();
    let ids: Vec<_> = views.iter().map(|view| view.record_id).collect();
    u.core
        .known_host_remove(RecordsRef {
            vault_id: u.vault_id,
            record_ids: vec![ids[0]],
        })
        .unwrap();
    assert_eq!(
        u.core
            .known_host_check(&check("a.example.com", 22, &key))
            .unwrap(),
        HostKeyStatus::Unknown
    );
    u.env.take_events();
    let restored = u
        .core
        .known_host_restore(RecordsRef {
            vault_id: u.vault_id,
            record_ids: vec![ids[0]],
        })
        .unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].record_id, ids[0]);
    assert_eq!(restored[0].comment.as_deref(), Some("note"));
    assert_eq!(restored[0].created_at, views[0].created_at);
    assert!(u.env.take_events().iter().any(|event| matches!(
        &event.kind,
        CoreEventKind::RecordsChanged { classes, .. } if classes == &vec![8]
    )));
    assert!(matches!(
        u.core
            .known_host_check(&check("a.example.com", 22, &key))
            .unwrap(),
        HostKeyStatus::Match { record_id } if record_id == ids[0]
    ));
    // Restoring again, or something never removed: not found.
    for id in [ids[0], ids[1]] {
        assert!(matches!(
            u.core
                .known_host_restore(RecordsRef {
                    vault_id: u.vault_id,
                    record_ids: vec![id],
                })
                .unwrap_err(),
            CoreError::NotFound
        ));
    }
    // After the window the removal is final.
    u.core
        .known_host_remove(RecordsRef {
            vault_id: u.vault_id,
            record_ids: vec![ids[1]],
        })
        .unwrap();
    u.env.mem.clock.advance(Duration::from_secs(61));
    assert!(matches!(
        u.core
            .known_host_restore(RecordsRef {
                vault_id: u.vault_id,
                record_ids: vec![ids[1]],
            })
            .unwrap_err(),
        CoreError::NotFound
    ));
    assert_eq!(u.core.known_hosts_list().unwrap().len(), 1);
}
