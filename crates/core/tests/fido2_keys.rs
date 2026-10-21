//! FIDO2 security keys in the Keychain at the format level (night plan,
//! stage 3 item 6; docs/data-model.md, section 9.3): OpenSSH
//! `sk-ssh-ed25519@openssh.com` and `sk-ecdsa-sha2-nistp256@openssh.com`
//! private keys (application, flags and key handle; no secret scalar) are
//! imported with their `fido2` member, which Import Data keeps. No
//! authenticator is used: the keys
//! are built here from random public keys and key handles. Signing with
//! them comes with the authenticator support (see the night report).

mod common;
mod stage2;

use common::first_run;
use secrecy::SecretString;
use shellpp_core::service::dto::KeyImportInput;
use ssh_key::private::KeypairData;
use ssh_key::{LineEnding, PrivateKey};

fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    getrandom::fill(&mut bytes).expect("os random");
    bytes
}

/// An `sk-ssh-ed25519` private key file with `flags`.
fn sk_ed25519(flags: u8) -> String {
    let seed: [u8; 32] = random();
    let public = ssh_key::private::Ed25519Keypair::from_seed(&seed).public;
    let public = ssh_key::public::SkEd25519::new(public, "ssh:");
    let private =
        ssh_key::private::SkEd25519::new(public, flags, random::<64>().to_vec()).expect("sk key");
    PrivateKey::new(KeypairData::SkEd25519(private), "yubikey")
        .expect("key")
        .to_openssh(LineEnding::LF)
        .expect("openssh")
        .to_string()
}

/// An `sk-ecdsa-sha2-nistp256` private key file.
fn sk_ecdsa(flags: u8) -> String {
    let ecdsa = stage2::generate(stage2::KeyKind::EcdsaP256, "");
    let point = match ecdsa.public_key().key_data() {
        ssh_key::public::KeyData::Ecdsa(ssh_key::public::EcdsaPublicKey::NistP256(point)) => *point,
        _ => unreachable!("a P-256 key"),
    };
    let public = ssh_key::public::SkEcdsaSha2NistP256::new(point, "ssh:");
    let private =
        ssh_key::private::SkEcdsaSha2NistP256::new(public, flags, random::<48>().to_vec())
            .expect("sk key");
    PrivateKey::new(KeypairData::SkEcdsaSha2NistP256(private), "")
        .expect("key")
        .to_openssh(LineEnding::LF)
        .expect("openssh")
        .to_string()
}

#[test]
fn security_keys_are_imported_with_their_fido2_member() {
    let u = first_run();
    let import = |text: String| {
        u.core
            .key_import(KeyImportInput {
                vault_id: u.vault_id,
                label: None,
                private_key: SecretString::from(text),
                passphrase: None,
                save_passphrase: false,
            })
            .expect("security key import")
            .key
    };
    // User presence (0x01) and a resident (discoverable) credential (0x20).
    let ed = import(sk_ed25519(0x01 | 0x20));
    assert_eq!(ed.algorithm, "sk-ssh-ed25519@openssh.com");
    assert_eq!(ed.label, "yubikey");
    assert!(ed.fingerprint.is_some());
    let ecdsa = import(sk_ecdsa(0x01));
    assert_eq!(ecdsa.algorithm, "sk-ecdsa-sha2-nistp256@openssh.com");

    // The stored records carry {"resident": ...} (read back through the
    // export, which holds every member).
    let export = u.core.data_export(Default::default()).expect("export");
    let document: serde_json::Value =
        serde_json::from_str(secrecy::ExposeSecret::expose_secret(&export.text)).unwrap();
    let fido2 = |record_id: uuid::Uuid| {
        document["vaults"][0]["records"]
            .as_array()
            .unwrap()
            .iter()
            .find(|record| record["record_id"] == record_id.to_string())
            .unwrap()["data"]["fido2"]
            .clone()
    };
    assert_eq!(fido2(ed.record_id), serde_json::json!({"resident": true}));
    assert_eq!(
        fido2(ecdsa.record_id),
        serde_json::json!({"resident": false})
    );

    // Import Data into another store keeps the member (it comes from the
    // key itself).
    let target = first_run();
    let summary = target
        .core
        .data_import(shellpp_core::service::dto::DataImportInput {
            apply_settings: false,
            ..shellpp_core::service::dto::DataImportInput::new(export.text.clone(), None)
        })
        .expect("import");
    assert_eq!(summary.added, 2);
    let again = target.core.data_export(Default::default()).expect("export");
    let copied: serde_json::Value =
        serde_json::from_str(secrecy::ExposeSecret::expose_secret(&again.text)).unwrap();
    let resident = copied["vaults"][0]["records"]
        .as_array()
        .unwrap()
        .iter()
        .find(|record| record["record_id"] == ed.record_id.to_string())
        .unwrap()["data"]["fido2"]
        .clone();
    assert_eq!(resident, serde_json::json!({"resident": true}));
}
