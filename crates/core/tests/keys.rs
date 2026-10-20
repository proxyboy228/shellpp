//! Key import for the short "+ -> Key" path (stage 2 plan, sections 4.4 and
//! 5.1; docs/data-model.md, section 9.1). Every key is generated at run time.

mod common;
mod stage2;

use common::{first_run, reasons};
use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use shellpp_core::service::dto::{CoreEventKind, KeyImportInput, RecordRef, SessionTarget};
use shellpp_core::{CoreError, ValidationReason};
use stage2::{KeyKind, fingerprint, generate, openssh, passphrase, with_kdf_rounds};
use uuid::Uuid;

fn import(
    vault_id: Uuid,
    text: &str,
    pass: Option<&SecretString>,
    save: bool,
    label: Option<&str>,
) -> KeyImportInput {
    KeyImportInput {
        vault_id,
        label: label.map(str::to_owned),
        private_key: SecretString::from(text.to_owned()),
        passphrase: pass.cloned(),
        save_passphrase: save,
    }
}

#[test]
fn import_table_plain_and_encrypted() {
    let u = first_run();
    let cases = [
        (KeyKind::Ed25519, "ssh-ed25519"),
        (KeyKind::EcdsaP256, "ecdsa-sha2-nistp256"),
        (KeyKind::EcdsaP384, "ecdsa-sha2-nistp384"),
        (KeyKind::Rsa(3072), "ssh-rsa"),
    ];
    for (kind, algorithm) in cases {
        // Plain key, label from the comment.
        let key = generate(kind, &format!("plain {algorithm}"));
        let result = u
            .core
            .key_import(import(u.vault_id, &openssh(&key, None), None, false, None))
            .expect("plain import");
        assert!(result.created, "{algorithm}");
        assert_eq!(result.key.algorithm, algorithm);
        assert_eq!(
            result.key.fingerprint.as_deref(),
            Some(fingerprint(&key).as_str())
        );
        assert_eq!(result.key.label, format!("plain {algorithm}"));
        assert!(!result.key.encrypted);
        assert!(!result.key.passphrase_saved);

        // Encrypted key with the passphrase saved.
        let key = generate(kind, "");
        let pass = passphrase();
        let text = openssh(&key, Some(pass.expose_secret()));
        let result = u
            .core
            .key_import(import(
                u.vault_id,
                &text,
                Some(&pass),
                true,
                Some(" Deploy "),
            ))
            .expect("encrypted import");
        assert!(result.created);
        assert_eq!(result.key.label, "Deploy");
        assert!(result.key.encrypted, "{algorithm}");
        assert!(result.key.passphrase_saved);
        assert_eq!(
            result.key.fingerprint.as_deref(),
            Some(fingerprint(&key).as_str())
        );

        // Encrypted key, "ask on every connection": checked but not stored.
        let key = generate(kind, "");
        let pass = passphrase();
        let text = openssh(&key, Some(pass.expose_secret()));
        let result = u
            .core
            .key_import(import(u.vault_id, &text, Some(&pass), false, None))
            .expect("encrypted import without saving");
        assert!(result.key.encrypted);
        assert!(!result.key.passphrase_saved);
        // Without a comment the label is the algorithm name.
        assert_eq!(result.key.label, algorithm);
    }
    assert_eq!(u.core.keys_list().unwrap().len(), 12);
}

/// A key file that declares an absurd bcrypt-pbkdf rounds count is refused
/// before anything is decrypted (the KDF cannot be interrupted).
#[test]
fn keys_with_too_many_kdf_rounds_are_refused() {
    let u = first_run();
    let pass = passphrase();
    let key = generate(KeyKind::Ed25519, "rounds");
    let encrypted = openssh(&key, Some(pass.expose_secret()));
    // The patch itself keeps a normal key usable.
    let normal = with_kdf_rounds(&encrypted, 16);
    assert!(shellpp_core::keys::parse(&normal).is_ok());
    let crafted = with_kdf_rounds(&encrypted, u32::MAX);
    let started = std::time::Instant::now();
    assert_eq!(
        reasons(
            u.core
                .key_import(import(u.vault_id, &crafted, Some(&pass), true, None))
                .unwrap_err()
        ),
        vec![("private_key".into(), ValidationReason::NotAllowed)]
    );
    assert_eq!(
        shellpp_core::keys::passphrase_opens(&crafted, &pass)
            .unwrap_err()
            .reason,
        ValidationReason::Invalid
    );
    let above = with_kdf_rounds(&encrypted, shellpp_core::keys::MAX_KDF_ROUNDS + 1);
    assert_eq!(
        shellpp_core::keys::parse(&above).unwrap_err().reason,
        ValidationReason::NotAllowed
    );
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[test]
fn import_errors() {
    let u = first_run();
    let field = |error: CoreError| reasons(error);
    let pass = passphrase();
    let key = generate(KeyKind::Ed25519, "k");
    let encrypted = openssh(&key, Some(pass.expose_secret()));
    let plain = openssh(&generate(KeyKind::Ed25519, "k2"), None);

    // Encrypted without a passphrase, an empty one, or a wrong one.
    assert_eq!(
        field(
            u.core
                .key_import(import(u.vault_id, &encrypted, None, true, None))
                .unwrap_err()
        ),
        vec![("passphrase".into(), ValidationReason::Required)]
    );
    let empty = SecretString::from(String::new());
    assert_eq!(
        field(
            u.core
                .key_import(import(u.vault_id, &encrypted, Some(&empty), true, None))
                .unwrap_err()
        ),
        vec![("passphrase".into(), ValidationReason::Required)]
    );
    let wrong = passphrase();
    assert_eq!(
        field(
            u.core
                .key_import(import(u.vault_id, &encrypted, Some(&wrong), true, None))
                .unwrap_err()
        ),
        vec![("passphrase".into(), ValidationReason::Invalid)]
    );
    // A passphrase for a plain key.
    assert_eq!(
        field(
            u.core
                .key_import(import(u.vault_id, &plain, Some(&pass), false, None))
                .unwrap_err()
        ),
        vec![("passphrase".into(), ValidationReason::NotAllowed)]
    );
    // A damaged PEM key, public keys and garbage are refused.
    let pem = "-----BEGIN PRIVATE KEY-----\nMC4CAQAwBQYDK2VwBCIEIA==\n-----END PRIVATE KEY-----\n";
    let public = key.public_key().to_openssh().unwrap();
    let truncated = plain[..plain.len() / 2].to_owned();
    for text in [pem, public.as_str(), "garbage", truncated.as_str()] {
        assert_eq!(
            field(
                u.core
                    .key_import(import(u.vault_id, text, None, false, None))
                    .unwrap_err()
            ),
            vec![("private_key".into(), ValidationReason::Invalid)],
            "{}",
            &text[..text.len().min(20)]
        );
    }
    assert_eq!(
        field(
            u.core
                .key_import(import(u.vault_id, "  \n ", None, false, None))
                .unwrap_err()
        ),
        vec![("private_key".into(), ValidationReason::Required)]
    );
    let huge = format!(
        "-----BEGIN OPENSSH PRIVATE KEY-----\n{}",
        "A".repeat(40_000)
    );
    assert_eq!(
        field(
            u.core
                .key_import(import(u.vault_id, &huge, None, false, None))
                .unwrap_err()
        ),
        vec![("private_key".into(), ValidationReason::TooLong)]
    );
    // RSA under 2048 bits is refused.
    let small = generate(KeyKind::Rsa(1024), "small");
    assert_eq!(
        field(
            u.core
                .key_import(import(
                    u.vault_id,
                    &openssh(&small, None),
                    None,
                    false,
                    None
                ))
                .unwrap_err()
        ),
        vec![("private_key".into(), ValidationReason::NotAllowed)]
    );
    // Label errors, also together with key errors.
    assert_eq!(
        field(
            u.core
                .key_import(import(
                    u.vault_id,
                    &plain,
                    None,
                    false,
                    Some(&"x".repeat(257))
                ))
                .unwrap_err()
        ),
        vec![("label".into(), ValidationReason::TooLong)]
    );
    assert_eq!(
        field(
            u.core
                .key_import(import(u.vault_id, "garbage", None, false, Some("a\u{7}b")))
                .unwrap_err()
        ),
        vec![
            ("private_key".into(), ValidationReason::Invalid),
            ("label".into(), ValidationReason::Invalid)
        ]
    );
    // Another vault and the locked state.
    assert!(matches!(
        u.core
            .key_import(import(Uuid::now_v7(), &plain, None, false, None))
            .unwrap_err(),
        CoreError::NotFound
    ));
    assert!(u.core.keys_list().unwrap().is_empty());
    u.core.lock().unwrap();
    assert!(matches!(
        u.core
            .key_import(import(u.vault_id, &plain, None, false, None))
            .unwrap_err(),
        CoreError::Locked
    ));
    assert!(matches!(u.core.keys_list().unwrap_err(), CoreError::Locked));
}

#[test]
fn duplicate_fingerprints_return_the_existing_key() {
    let u = first_run();
    let key = generate(KeyKind::Ed25519, "dup");
    let plain = openssh(&key, None);
    let first = u
        .core
        .key_import(import(u.vault_id, &plain, None, false, None))
        .unwrap();
    // CRLF line endings and surrounding whitespace are the same key.
    let crlf = format!("\r\n  {}  \r\n", plain.replace('\n', "\r\n"));
    let again = u
        .core
        .key_import(import(u.vault_id, &crlf, None, false, Some("other label")))
        .unwrap();
    assert!(!again.created);
    assert_eq!(again.key.record_id, first.key.record_id);
    assert_eq!(again.key.label, "dup");
    // The encrypted form of the same key is the same key too.
    let pass = passphrase();
    let encrypted = openssh(&key, Some(pass.expose_secret()));
    let third = u
        .core
        .key_import(import(u.vault_id, &encrypted, Some(&pass), true, None))
        .unwrap();
    assert!(!third.created);
    assert_eq!(third.key.record_id, first.key.record_id);
    assert!(!third.key.encrypted);
    assert!(!third.key.passphrase_saved);
    assert_eq!(u.core.keys_list().unwrap().len(), 1);
}

#[test]
fn an_existing_key_gets_a_passphrase_that_opens_it() {
    let u = first_run();
    let key = generate(KeyKind::EcdsaP256, "enc");
    let pass = passphrase();
    let text = openssh(&key, Some(pass.expose_secret()));
    let first = u
        .core
        .key_import(import(u.vault_id, &text, Some(&pass), false, None))
        .unwrap();
    assert!(!first.key.passphrase_saved);
    // The same text with "save": the existing record gets the passphrase.
    let saved = u
        .core
        .key_import(import(u.vault_id, &text, Some(&pass), true, None))
        .unwrap();
    assert!(!saved.created);
    assert_eq!(saved.key.record_id, first.key.record_id);
    assert!(saved.key.passphrase_saved);

    // The same key encrypted with another passphrase does not open the
    // stored text: the passphrase is not saved.
    let key = generate(KeyKind::Ed25519, "two");
    let pass_a = passphrase();
    let pass_b = passphrase();
    let text_a = openssh(&key, Some(pass_a.expose_secret()));
    let text_b = openssh(&key, Some(pass_b.expose_secret()));
    let first = u
        .core
        .key_import(import(u.vault_id, &text_a, Some(&pass_a), false, None))
        .unwrap();
    let other = u
        .core
        .key_import(import(u.vault_id, &text_b, Some(&pass_b), true, None))
        .unwrap();
    assert!(!other.created);
    assert_eq!(other.key.record_id, first.key.record_id);
    assert!(!other.key.passphrase_saved);
    // The given text is checked first: `text_b` does not open with `pass_a`.
    let refused = u
        .core
        .key_import(import(u.vault_id, &text_b, Some(&pass_a), true, None));
    assert_eq!(
        reasons(refused.unwrap_err()),
        vec![("passphrase".into(), ValidationReason::Invalid)]
    );
    // The stored text's own passphrase can still be saved.
    u.core
        .save_key_passphrase(
            RecordRef {
                vault_id: u.vault_id,
                record_id: first.key.record_id,
            },
            pass_a,
        )
        .unwrap();
    let listed = u.core.keys_list().unwrap();
    let stored = listed
        .iter()
        .find(|view| view.record_id == first.key.record_id)
        .unwrap();
    assert!(stored.passphrase_saved);
}

#[test]
fn save_key_passphrase_rules() {
    let u = first_run();
    let key = generate(KeyKind::Ed25519, "later");
    let pass = passphrase();
    let text = openssh(&key, Some(pass.expose_secret()));
    let view = u
        .core
        .key_import(import(u.vault_id, &text, Some(&pass), false, None))
        .unwrap()
        .key;
    let key_ref = RecordRef {
        vault_id: u.vault_id,
        record_id: view.record_id,
    };
    assert_eq!(
        reasons(
            u.core
                .save_key_passphrase(key_ref, passphrase())
                .unwrap_err()
        ),
        vec![("passphrase".into(), ValidationReason::Invalid)]
    );
    assert_eq!(
        reasons(
            u.core
                .save_key_passphrase(key_ref, SecretString::from(String::new()))
                .unwrap_err()
        ),
        vec![("passphrase".into(), ValidationReason::Required)]
    );
    u.env.take_events();
    u.core.save_key_passphrase(key_ref, pass.clone()).unwrap();
    let events = u.env.take_events();
    assert!(events.iter().any(|event| matches!(
        &event.kind,
        CoreEventKind::RecordsChanged { classes, .. } if classes == &vec![3]
    )));
    assert!(u.core.keys_list().unwrap()[0].passphrase_saved);

    let plain = u
        .core
        .key_import(import(
            u.vault_id,
            &openssh(&generate(KeyKind::Ed25519, "plain"), None),
            None,
            false,
            None,
        ))
        .unwrap()
        .key;
    assert_eq!(
        reasons(
            u.core
                .save_key_passphrase(
                    RecordRef {
                        vault_id: u.vault_id,
                        record_id: plain.record_id
                    },
                    pass.clone()
                )
                .unwrap_err()
        ),
        vec![("passphrase".into(), ValidationReason::NotAllowed)]
    );
    assert!(matches!(
        u.core
            .save_key_passphrase(
                RecordRef {
                    vault_id: u.vault_id,
                    record_id: Uuid::now_v7()
                },
                pass
            )
            .unwrap_err(),
        CoreError::NotFound
    ));
}

#[test]
fn keys_are_listed_by_label_and_survive_a_restart() {
    let mut u = first_run();
    for label in ["beta", "Alpha", "gamma"] {
        let text = openssh(&generate(KeyKind::Ed25519, label), None);
        u.env.take_events();
        u.core
            .key_import(import(u.vault_id, &text, None, false, None))
            .unwrap();
        let events = u.env.take_events();
        assert!(events.iter().any(|event| matches!(
            &event.kind,
            CoreEventKind::RecordsChanged { classes, .. } if classes == &vec![3]
        )));
    }
    let labels: Vec<String> = u
        .core
        .keys_list()
        .unwrap()
        .into_iter()
        .map(|view| view.label)
        .collect();
    assert_eq!(labels, vec!["Alpha", "beta", "gamma"]);
    u.reopen();
    u.core.unlock(u.password.clone()).unwrap();
    assert_eq!(u.core.keys_list().unwrap().len(), 3);
}

#[test]
fn a_linked_key_reaches_the_session_config() {
    let u = first_run();
    let key = generate(KeyKind::Ed25519, "linked");
    let pass = passphrase();
    let text = openssh(&key, Some(pass.expose_secret()));
    let view = u
        .core
        .key_import(import(
            u.vault_id,
            &format!("{text}\r\n"),
            Some(&pass),
            true,
            Some("Deploy key"),
        ))
        .unwrap()
        .key;
    let host = u.host(json!({"address": "k.example.com", "ssh": {"key_id": view.record_id}}));
    let config = u
        .core
        .session_config(&SessionTarget::Host {
            vault_id: u.vault_id,
            record_id: host.record_id,
        })
        .unwrap();
    let session_key = config.key.expect("effective key");
    assert_eq!(session_key.key.record_id, view.record_id);
    assert_eq!(session_key.label, "Deploy key");
    assert!(session_key.can_save_passphrase);
    // The stored text is the normalized given text, still encrypted.
    assert_eq!(session_key.private_key.expose_secret(), text.trim_end());
    assert_eq!(
        session_key
            .passphrase
            .as_ref()
            .map(|p| p.expose_secret().to_owned()),
        Some(pass.expose_secret().to_owned())
    );
    // Debug output names no secret.
    let debug = format!("{session_key:?}");
    assert!(!debug.contains(pass.expose_secret()));
    assert!(!debug.contains("OPENSSH"));
}
