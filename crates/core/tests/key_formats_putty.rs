//! PuTTY `.ppk` import (stage 3; UI 7.1; docs/data-model.md, section 9.1):
//! keys that PuTTY's own puttygen generates at test time, saved as `.ppk`
//! version 2 and 3, plain and encrypted (tests/putty). Their OpenSSH public
//! keys, also written by puttygen, give the expected fingerprints.
//!
//! The keys come from:
//! - `SHELLPP_TEST_PUTTY_DIR`: a folder that `tests/putty/generate.sh` filled;
//! - else `SHELLPP_TEST_PUTTYGEN=docker`: the test runs the image
//!   `SHELLPP_PUTTY_IMAGE` (default `shellpp-test-putty:local`, built from
//!   tests/putty) into a temporary folder; the CI integration job does this;
//! - else the test says so on stderr and checks nothing (no puttygen here).

#![allow(clippy::print_stderr)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{first_run, reasons};
use secrecy::SecretString;
use shellpp_core::ValidationReason;
use shellpp_core::keys::convert::Format;
use shellpp_core::service::dto::KeyImportInput;
use ssh_key::{HashAlg, PublicKey};

const KEYS: [(&str, &str); 5] = [
    ("ed25519", "ssh-ed25519"),
    ("ecdsa-256", "ecdsa-sha2-nistp256"),
    ("ecdsa-384", "ecdsa-sha2-nistp384"),
    ("ecdsa-521", "ecdsa-sha2-nistp521"),
    ("rsa-2048", "ssh-rsa"),
];

/// The folder with the generated keys (and the temporary folder that holds
/// it, removed on drop), or `None` without puttygen.
fn keys_dir() -> Option<(PathBuf, Option<tempfile::TempDir>)> {
    if let Some(dir) = std::env::var_os("SHELLPP_TEST_PUTTY_DIR") {
        return Some((PathBuf::from(dir), None));
    }
    if std::env::var("SHELLPP_TEST_PUTTYGEN").as_deref() != Ok("docker") {
        eprintln!(
            "key_formats_putty: no puttygen (set SHELLPP_TEST_PUTTYGEN=docker with the tests/putty image built)"
        );
        return None;
    }
    let image = std::env::var("SHELLPP_PUTTY_IMAGE")
        .unwrap_or_else(|_| "shellpp-test-putty:local".to_owned());
    let temp = tempfile::tempdir().expect("temp dir");
    let status = Command::new("docker")
        .arg("run")
        .arg("--rm")
        .arg("-v")
        .arg(format!("{}:/out", temp.path().display()))
        .arg(&image)
        .status()
        .expect("docker runs");
    assert!(status.success(), "puttygen in {image} failed");
    Some((temp.path().to_path_buf(), Some(temp)))
}

fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_else(|_| panic!("{name}"))
}

#[test]
fn ppk_keys_from_puttygen_are_imported() {
    let Some((dir, _temp)) = keys_dir() else {
        return;
    };
    let pass = SecretString::from(read(&dir, "passphrase").trim().to_owned());
    let wrong = SecretString::from("not the passphrase".to_owned());
    let u = first_run();
    let import = |text: &str, passphrase: Option<&SecretString>| {
        u.core.key_import(KeyImportInput {
            vault_id: u.vault_id,
            label: None,
            private_key: SecretString::from(text.to_owned()),
            passphrase: passphrase.cloned(),
            save_passphrase: false,
        })
    };
    for (name, algorithm) in KEYS {
        let expected = PublicKey::from_openssh(read(&dir, &format!("{name}.pub")).trim())
            .expect("public key")
            .fingerprint(HashAlg::Sha256)
            .to_string();
        let mut record = None;
        for version in ["v3", "v2"] {
            let plain = read(&dir, &format!("{name}.{version}.ppk"));
            assert_eq!(
                shellpp_core::keys::inspect(&plain),
                Ok((Format::Putty, false)),
                "{name} {version}"
            );
            let result = import(&plain, None).expect("plain ppk");
            assert_eq!(result.key.algorithm, algorithm, "{name} {version}");
            assert_eq!(result.key.fingerprint.as_deref(), Some(expected.as_str()));
            assert!(!result.key.encrypted);
            // The label is the key comment puttygen wrote.
            assert_eq!(result.key.label, format!("shellpp-test-{name}"));
            // The same key again (another version, or encrypted) is the same
            // record.
            let id = *record.get_or_insert(result.key.record_id);
            assert_eq!(result.key.record_id, id);

            let encrypted = read(&dir, &format!("{name}.{version}.enc.ppk"));
            assert_eq!(
                shellpp_core::keys::inspect(&encrypted),
                Ok((Format::Putty, true)),
                "{name} {version}"
            );
            assert_eq!(
                reasons(import(&encrypted, None).unwrap_err()),
                vec![("passphrase".to_owned(), ValidationReason::Required)],
                "{name} {version}"
            );
            assert_eq!(
                reasons(import(&encrypted, Some(&wrong)).unwrap_err()),
                vec![("passphrase".to_owned(), ValidationReason::Invalid)],
                "{name} {version}"
            );
            let result = import(&encrypted, Some(&pass)).expect("encrypted ppk");
            assert_eq!(result.key.fingerprint.as_deref(), Some(expected.as_str()));
            assert_eq!(result.key.record_id, id, "{name} {version}");
        }
    }

    // An encrypted key imported first is stored encrypted again.
    let fresh = first_run();
    let encrypted = read(&dir, "ed25519.v3.enc.ppk");
    let result = fresh
        .core
        .key_import(KeyImportInput {
            vault_id: fresh.vault_id,
            label: None,
            private_key: SecretString::from(encrypted),
            passphrase: Some(pass),
            save_passphrase: true,
        })
        .expect("encrypted ppk");
    assert!(result.key.encrypted);
    assert!(result.key.passphrase_saved);
}
