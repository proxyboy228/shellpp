//! Keychain records written from the host panel's "+" (UI 3.1;
//! docs/data-model.md, sections 9.2 and 9.4): identities and user
//! certificates on keys. Keys and certificates are generated at run time.

mod common;
mod stage2;

use common::{first_run, reasons};
use secrecy::{ExposeSecret, SecretString};
use shellpp_core::ValidationReason;
use shellpp_core::model::identity::PasswordChange;
use shellpp_core::service::dto::{KeyImportInput, RecordRef, SecretCopyTarget, SecretFieldRef};
use shellpp_core::service::keychain::{IdentitySaveInput, KeyCertificateInput, KeyLabelInput};
use ssh_key::PrivateKey;
use ssh_key::certificate::{Builder, CertType};
use stage2::{KeyKind, generate, openssh, passphrase};
use uuid::Uuid;

fn identity(vault_id: Uuid, record_id: Option<Uuid>, label: &str) -> IdentitySaveInput {
    IdentitySaveInput {
        vault_id,
        record_id,
        label: label.to_owned(),
        username: String::new(),
        key_id: None,
        password: PasswordChange::Keep,
        base: None,
    }
}

/// An OpenSSH certificate line for `subject`, signed by `ca`.
fn certificate(subject: &PrivateKey, ca: &PrivateKey, cert_type: CertType) -> String {
    let mut builder = Builder::new(
        vec![7u8; 32],
        subject.public_key().key_data().clone(),
        0,
        u64::from(u32::MAX),
    )
    .expect("builder");
    builder.cert_type(cert_type).expect("type");
    builder.key_id("test-cert").expect("key id");
    builder.valid_principal("ops").expect("principal");
    builder
        .sign(ca)
        .expect("signed")
        .to_openssh()
        .expect("openssh")
}

#[test]
fn identities_are_created_listed_edited_and_survive_a_restart() {
    let mut u = first_run();
    let vault_id = u.vault_id;
    assert_eq!(
        reasons(
            u.core
                .identity_save(identity(vault_id, None, " "))
                .unwrap_err()
        ),
        vec![("label".into(), ValidationReason::Required)]
    );
    let created = u
        .core
        .identity_save(IdentitySaveInput {
            username: " ops ".into(),
            password: PasswordChange::Set(SecretString::from("generated-Pw-1")),
            ..identity(vault_id, None, "Ops login")
        })
        .expect("created");
    assert_eq!(created.label, "Ops login");
    assert_eq!(created.username.as_deref(), Some("ops"));
    assert!(created.password_set);
    let other = u
        .core
        .identity_save(identity(vault_id, None, "admin"))
        .expect("second");
    let labels: Vec<String> = u
        .core
        .identities_list()
        .unwrap()
        .into_iter()
        .map(|view| view.label)
        .collect();
    assert_eq!(labels, ["admin", "Ops login"]);

    // A key that does not exist is refused; the stored values stay.
    assert_eq!(
        reasons(
            u.core
                .identity_save(IdentitySaveInput {
                    key_id: Some(Uuid::now_v7()),
                    ..identity(vault_id, Some(created.record_id), "Ops login")
                })
                .unwrap_err()
        ),
        vec![("key_id".into(), ValidationReason::MissingRef)]
    );

    // Editing keeps the password unless asked; clearing the username works.
    let edited = u
        .core
        .identity_save(identity(vault_id, Some(created.record_id), "Ops"))
        .expect("edited");
    assert_eq!(edited.label, "Ops");
    assert_eq!(edited.username, None);
    assert!(edited.password_set);
    let reference = RecordRef {
        vault_id,
        record_id: created.record_id,
    };
    assert_eq!(
        u.core.identity_password(reference).unwrap().expose_secret(),
        "generated-Pw-1"
    );

    u.reopen();
    u.core.unlock(u.password.clone()).expect("unlock");
    let after: Vec<(Uuid, String, bool)> = u
        .core
        .identities_list()
        .unwrap()
        .into_iter()
        .map(|view| (view.record_id, view.label, view.password_set))
        .collect();
    assert_eq!(
        after,
        [
            (other.record_id, "admin".to_owned(), false),
            (created.record_id, "Ops".to_owned(), true),
        ]
    );
    let cleared = u
        .core
        .identity_save(IdentitySaveInput {
            password: PasswordChange::Clear,
            ..identity(vault_id, Some(created.record_id), "Ops")
        })
        .unwrap();
    assert!(!cleared.password_set);
}

#[test]
fn a_user_certificate_is_stored_on_the_key_it_certifies() {
    let u = first_run();
    let vault_id = u.vault_id;
    let subject = generate(KeyKind::Ed25519, "subject");
    let stranger = generate(KeyKind::Ed25519, "stranger");
    let ca = generate(KeyKind::Ed25519, "ca");
    let key = u
        .core
        .key_import(KeyImportInput {
            vault_id,
            label: None,
            private_key: SecretString::from(openssh(&subject, None)),
            passphrase: None,
            save_passphrase: false,
        })
        .expect("key imported")
        .key;
    let key_ref = RecordRef {
        vault_id,
        record_id: key.record_id,
    };
    let set = |text: &str| {
        u.core.key_certificate_set(KeyCertificateInput {
            key: key_ref,
            certificate: text.to_owned(),
        })
    };
    // Not a certificate, a host certificate, another key's certificate.
    assert_eq!(
        reasons(set("ssh-ed25519-cert-v01@openssh.com AAAA").unwrap_err()),
        vec![("certificate".into(), ValidationReason::Invalid)]
    );
    assert_eq!(
        reasons(set(&certificate(&subject, &ca, CertType::Host)).unwrap_err()),
        vec![("certificate".into(), ValidationReason::NotAllowed)]
    );
    assert_eq!(
        reasons(set(&certificate(&stranger, &ca, CertType::User)).unwrap_err()),
        vec![("certificate".into(), ValidationReason::NotAllowed)]
    );
    assert!(!u.core.key_has_certificate(key_ref).unwrap());

    let line = certificate(&subject, &ca, CertType::User);
    let view = set(&format!("  {line}\n")).expect("certificate stored");
    assert_eq!(view.record_id, key.record_id);
    assert!(u.core.key_has_certificate(key_ref).unwrap());
    // An identity can link the key.
    let linked = u
        .core
        .identity_save(IdentitySaveInput {
            key_id: Some(key.record_id),
            ..identity(vault_id, None, "With key")
        })
        .unwrap();
    assert_eq!(linked.key_id, Some(key.record_id));
    // An empty text removes the certificate.
    set("").expect("removed");
    assert!(!u.core.key_has_certificate(key_ref).unwrap());
}

/// The Keychain's key panel (UI 7.3): the label is renamed (required, at
/// most 256 characters; nothing else changes), the public key is the stored
/// line, and the private key is revealed and copied through the secret
/// paths, the copy cleared after 30 seconds.
#[test]
fn a_key_is_renamed_shows_its_public_key_and_copies_its_private_key() {
    let u = first_run();
    let vault_id = u.vault_id;
    let pair = generate(KeyKind::Ed25519, "panel@test");
    let pass = passphrase();
    let text = openssh(&pair, Some(pass.expose_secret()));
    let key = u
        .core
        .key_import(KeyImportInput {
            vault_id,
            label: Some("Deploy".into()),
            private_key: SecretString::from(text.clone()),
            passphrase: Some(pass.clone()),
            save_passphrase: true,
        })
        .expect("imported")
        .key;
    let key_ref = RecordRef {
        vault_id,
        record_id: key.record_id,
    };
    let rename = |label: &str| {
        u.core.key_label_set(KeyLabelInput {
            key: key_ref,
            label: label.to_owned(),
        })
    };
    assert_eq!(
        reasons(rename("  ").unwrap_err()),
        vec![("label".into(), ValidationReason::Required)]
    );
    assert_eq!(
        reasons(rename(&"x".repeat(257)).unwrap_err()),
        vec![("label".into(), ValidationReason::TooLong)]
    );
    let renamed = rename(" Deploy (CI) ").expect("renamed");
    assert_eq!(renamed.label, "Deploy (CI)");
    assert_eq!(renamed.fingerprint, key.fingerprint);
    assert!(renamed.encrypted && renamed.passphrase_saved);
    assert_eq!(u.core.keys_list().unwrap()[0].label, "Deploy (CI)");
    // Unchanged: no write, the same view.
    assert_eq!(
        rename("Deploy (CI)").unwrap().modified_at,
        renamed.modified_at
    );
    let missing = u.core.key_label_set(KeyLabelInput {
        key: RecordRef {
            vault_id,
            record_id: Uuid::now_v7(),
        },
        label: "x".into(),
    });
    assert!(matches!(missing, Err(shellpp_core::CoreError::NotFound)));

    // The public key line (an encrypted key keeps its comment inside the
    // encrypted part, so the stored line has none).
    let public = u.core.key_public_key(key_ref).expect("public key");
    let expected = pair.public_key().to_openssh().unwrap();
    let fields = |line: &str| {
        line.split_whitespace()
            .take(2)
            .collect::<Vec<_>>()
            .join(" ")
    };
    assert_eq!(fields(&public), fields(&expected));

    // The private key: revealed as stored, copied as a sensitive value.
    let field = |path: &str| SecretFieldRef {
        vault_id,
        record_id: key.record_id,
        path: path.to_owned(),
    };
    let revealed = u
        .core
        .secret_reveal(field("private_key"))
        .expect("revealed");
    assert_eq!(revealed.expose_secret(), text.trim());
    assert_eq!(
        u.core
            .secret_reveal(field("passphrase"))
            .unwrap()
            .expose_secret(),
        pass.expose_secret()
    );
    assert!(matches!(
        u.core.secret_reveal(field("public_key")),
        Err(shellpp_core::CoreError::Validation(_))
    ));
    let clipboard = &u.env.mem.clipboard;
    u.core
        .secret_copy(SecretCopyTarget::RecordField {
            vault_id,
            record_id: key.record_id,
            path: "private_key".into(),
        })
        .expect("copied");
    assert_eq!(clipboard.text().as_deref(), Some(text.trim()));
    assert!(clipboard.last_was_sensitive());
    u.env.mem.clock.advance(std::time::Duration::from_secs(31));
    u.core.tick();
    assert_eq!(clipboard.text(), None);
}

/// D50, item 2: Remove of a key lists who uses it first; the removal can
/// be undone, and the users keep their (now missing) reference.
#[test]
fn a_key_lists_its_users_and_is_removed_and_restored() {
    use serde_json::json;
    use shellpp_core::service::dto::{RecordType, RecordsRef};

    let u = first_run();
    let vault_id = u.vault_id;
    let pair = generate(KeyKind::Ed25519, "users@test");
    let key = u
        .core
        .key_import(KeyImportInput {
            vault_id,
            label: Some("Shared".into()),
            private_key: SecretString::from(openssh(&pair, None)),
            passphrase: None,
            save_passphrase: false,
        })
        .expect("imported")
        .key;
    let key_ref = RecordRef {
        vault_id,
        record_id: key.record_id,
    };
    assert!(u.core.key_users(key_ref).unwrap().is_empty());
    let id = key.record_id.to_string();
    let save = |kind, data| common::save(&u.core, vault_id, None, kind, data, &[]).unwrap();
    let web = save(
        RecordType::Host,
        json!({"label": "web", "address": "web.example.com", "ssh": {"key_id": id}}),
    );
    let bare = save(
        RecordType::Host,
        json!({"address": "10.0.0.7", "ssh": {"certificate_key_id": id}}),
    );
    save(RecordType::Host, json!({"address": "other.example.com"}));
    let group = save(
        RecordType::Group,
        json!({"name": "prod", "ssh": {"key_id": id}}),
    );
    let mut ops = identity(vault_id, None, "ops");
    ops.key_id = Some(key.record_id);
    let ops = u.core.identity_save(ops).unwrap();

    let users = u.core.key_users(key_ref).unwrap();
    let names = |list: &[shellpp_core::service::keychain::KeyUser]| {
        list.iter()
            .map(|user| user.name.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        names(&users.hosts),
        ["10.0.0.7", "web"],
        "label, else address"
    );
    assert_eq!(users.hosts[1].record_id, web.record_id());
    assert_eq!(users.hosts[0].record_id, bare.record_id());
    assert_eq!(names(&users.groups), ["prod"]);
    assert_eq!(users.groups[0].record_id, group.record_id());
    assert_eq!(names(&users.identities), ["ops"]);
    assert_eq!(users.identities[0].record_id, ops.record_id);

    let removal = RecordsRef {
        vault_id,
        record_ids: vec![key.record_id],
    };
    u.core.record_delete(removal.clone()).expect("removed");
    assert!(u.core.keys_list().unwrap().is_empty());
    assert!(matches!(
        u.core.key_users(key_ref),
        Err(shellpp_core::CoreError::NotFound)
    ));
    u.core.record_restore(removal).expect("restored");
    let keys = u.core.keys_list().unwrap();
    assert_eq!(keys.len(), 1);
    assert_eq!(keys[0].label, "Shared");
    assert_eq!(u.core.key_users(key_ref).unwrap().hosts.len(), 2);
}
