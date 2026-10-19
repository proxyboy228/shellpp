//! Keychain records written from the host panel's "+" (UI 3.1;
//! docs/data-model.md, sections 9.2 and 9.4): identities and user
//! certificates on keys. Keys and certificates are generated at run time.

mod common;
mod stage2;

use common::{first_run, reasons};
use secrecy::{ExposeSecret, SecretString};
use shellpp_core::ValidationReason;
use shellpp_core::model::identity::PasswordChange;
use shellpp_core::service::dto::{KeyImportInput, RecordRef};
use shellpp_core::service::keychain::{IdentitySaveInput, KeyCertificateInput};
use ssh_key::PrivateKey;
use ssh_key::certificate::{Builder, CertType};
use stage2::{KeyKind, generate, openssh};
use uuid::Uuid;

fn identity(vault_id: Uuid, record_id: Option<Uuid>, label: &str) -> IdentitySaveInput {
    IdentitySaveInput {
        vault_id,
        record_id,
        label: label.to_owned(),
        username: String::new(),
        key_id: None,
        password: PasswordChange::Keep,
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
