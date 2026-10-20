//! User certificates in the Keychain (night plan, stage 3 item 5; UI 7.1
//! "Certificate", 7.3 block 5; docs/data-model.md, section 9.2): a
//! `*-cert.pub` attaches to the key it certifies, and the panel's info
//! shows principals, the validity window, the CA fingerprint and whether it
//! is valid, not yet valid or expired. Keys and CAs are generated at run
//! time.

mod common;
mod stage2;

use common::{first_run, reasons};
use secrecy::SecretString;
use shellpp_core::ValidationReason;
use shellpp_core::keys::certificate::{CertificateStatus, certificate_info};
use shellpp_core::service::dto::{KeyImportInput, RecordRef};
use ssh_key::PrivateKey;
use ssh_key::certificate::{Builder, CertType};
use stage2::{KeyKind, generate, openssh};

fn certificate(subject: &PrivateKey, ca: &PrivateKey, after: u64, before: u64) -> String {
    let mut builder = Builder::new(
        vec![9u8; 32],
        subject.public_key().key_data().clone(),
        after,
        before,
    )
    .expect("builder");
    builder.cert_type(CertType::User).expect("type");
    builder.serial(42).expect("serial");
    builder.key_id("ops-cert").expect("key id");
    builder.valid_principal("ops").expect("principal");
    builder.valid_principal("deploy").expect("principal");
    builder
        .sign(ca)
        .expect("signed")
        .to_openssh()
        .expect("openssh")
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}

#[test]
fn a_certificate_attaches_to_its_key_and_shows_its_validity() {
    let u = first_run();
    let subject = generate(KeyKind::Ed25519, "subject");
    let other = generate(KeyKind::Ed25519, "other");
    let ca = generate(KeyKind::Ed25519, "ca");
    let import = |key: &PrivateKey| {
        u.core
            .key_import(KeyImportInput {
                vault_id: u.vault_id,
                label: None,
                private_key: SecretString::from(openssh(key, None)),
                passphrase: None,
                save_passphrase: false,
            })
            .expect("key")
            .key
    };
    let _other = import(&other);
    let subject_view = import(&subject);
    let line = certificate(&subject, &ca, now() - 60, now() + 3600);

    // The Certificate action finds the key it certifies.
    let attached = u
        .core
        .certificate_import(u.vault_id, &format!("{line}\n"))
        .expect("attached");
    assert_eq!(attached.record_id, subject_view.record_id);
    let reference = RecordRef {
        vault_id: u.vault_id,
        record_id: subject_view.record_id,
    };
    let info = u
        .core
        .key_certificate_info(reference)
        .unwrap()
        .expect("info");
    assert_eq!(info.key_id, "ops-cert");
    assert_eq!(info.serial, 42);
    assert_eq!(info.principals, ["ops", "deploy"]);
    assert_eq!(info.status, CertificateStatus::Valid);
    assert!(info.valid_after.is_some() && info.valid_before.is_some());
    assert_eq!(
        info.ca_fingerprint,
        ca.fingerprint(ssh_key::HashAlg::Sha256).to_string()
    );

    // A certificate of a key that is not in the Keychain, and not one at all.
    let stranger = generate(KeyKind::Ed25519, "stranger");
    assert_eq!(
        reasons(
            u.core
                .certificate_import(u.vault_id, &certificate(&stranger, &ca, 0, u64::MAX))
                .unwrap_err()
        ),
        vec![("certificate".to_owned(), ValidationReason::MissingRef)]
    );
    assert_eq!(
        reasons(
            u.core
                .certificate_import(u.vault_id, "garbage")
                .unwrap_err()
        ),
        vec![("certificate".to_owned(), ValidationReason::Invalid)]
    );
}

#[test]
fn expired_and_future_certificates_are_marked() {
    let subject = generate(KeyKind::Ed25519, "subject");
    let ca = generate(KeyKind::EcdsaP256, "ca");
    let at = now();
    let expired = certificate(&subject, &ca, at - 7200, at - 3600);
    assert_eq!(
        certificate_info(&expired, at).unwrap().status,
        CertificateStatus::Expired
    );
    let future = certificate(&subject, &ca, at + 3600, at + 7200);
    assert_eq!(
        certificate_info(&future, at).unwrap().status,
        CertificateStatus::NotYetValid
    );
    let forever = certificate(&subject, &ca, 0, u64::MAX);
    let info = certificate_info(&forever, at).unwrap();
    assert_eq!(info.status, CertificateStatus::Valid);
    assert_eq!(info.valid_after, None, "from the beginning of time");
    assert_eq!(info.valid_before, None, "never expires");
    assert!(certificate_info("ssh-ed25519 AAAA", at).is_none());
}
