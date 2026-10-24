//! PEM key import against OpenSSL (stage 3; UI 7.1; docs/data-model.md,
//! section 9.1): keys that the `openssl` command generates at test time,
//! written as PKCS#8 (plain, PBES2 with PBKDF2 and with scrypt), SEC1 and
//! PKCS#1 (plain and with the legacy encryption, `-traditional`). Every
//! variant of one key must give the same record.
//!
//! Runs when `openssl` is on the PATH (Linux runners; Git for Windows ships
//! one in `usr/bin`); otherwise it says so on stderr and checks nothing.

#![allow(clippy::print_stderr)]

mod common;

use std::path::Path;
use std::process::{Command, Stdio};

use common::{first_run, reasons};
use secrecy::SecretString;
use shellpp_core::ValidationReason;
use shellpp_core::keys::convert::Format;
use shellpp_core::service::dto::KeyImportInput;

/// The variants of one key (file, format, encrypted) and its algorithm.
type KeyVariants = (Vec<(String, Format, bool)>, &'static str);

fn openssl(dir: &Path, args: &[&str]) {
    let status = Command::new("openssl")
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .expect("openssl runs");
    assert!(status.success(), "openssl {}", args.join(" "));
}

/// OpenSSL 3 or later: `-traditional` (the PKCS#1 files) is there. macOS
/// ships LibreSSL, which lacks it (CI's macOS job, run 12), and OpenSSL 1.1
/// writes PKCS#1 without the flag; the test is skipped on both.
fn available() -> bool {
    Command::new("openssl")
        .arg("version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .is_ok_and(|output| {
            let version = String::from_utf8_lossy(&output.stdout);
            output.status.success()
                && version.starts_with("OpenSSL ")
                && !version.starts_with("OpenSSL 0")
                && !version.starts_with("OpenSSL 1")
        })
}

#[test]
fn pem_keys_from_openssl_are_imported() {
    if !available() {
        eprintln!("key_formats_openssl: no OpenSSL 3 or later on the PATH");
        return;
    }
    let temp = tempfile::tempdir().expect("temp dir");
    let dir = temp.path();
    let bytes: [u8; 12] = shellpp_core::crypto::random_array().expect("random");
    let pass: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    let passout = format!("pass:{pass}");
    let passout = passout.as_str();

    // (variants of one key: file, format, encrypted), algorithm.
    let mut keys: Vec<KeyVariants> = Vec::new();
    openssl(dir, &["genpkey", "-algorithm", "ed25519", "-out", "ed.pem"]);
    openssl(
        dir,
        &[
            "pkcs8",
            "-topk8",
            "-in",
            "ed.pem",
            "-v2",
            "aes-256-cbc",
            "-passout",
            passout,
            "-out",
            "ed.pbkdf2.pem",
        ],
    );
    openssl(
        dir,
        &[
            "pkcs8",
            "-topk8",
            "-in",
            "ed.pem",
            "-scrypt",
            "-passout",
            passout,
            "-out",
            "ed.scrypt.pem",
        ],
    );
    keys.push((
        vec![
            ("ed.pem".to_owned(), Format::Pkcs8, false),
            ("ed.pbkdf2.pem".to_owned(), Format::Pkcs8, true),
            ("ed.scrypt.pem".to_owned(), Format::Pkcs8, true),
        ],
        "ssh-ed25519",
    ));
    for (curve, algorithm) in [
        ("P-256", "ecdsa-sha2-nistp256"),
        ("P-384", "ecdsa-sha2-nistp384"),
        ("P-521", "ecdsa-sha2-nistp521"),
    ] {
        let p8 = format!("{curve}.pem");
        let sec1 = format!("{curve}.sec1.pem");
        let sec1_enc = format!("{curve}.sec1.enc.pem");
        openssl(
            dir,
            &[
                "genpkey",
                "-algorithm",
                "EC",
                "-pkeyopt",
                &format!("ec_paramgen_curve:{curve}"),
                "-out",
                &p8,
            ],
        );
        openssl(dir, &["ec", "-in", &p8, "-out", &sec1]);
        openssl(
            dir,
            &[
                "ec", "-in", &p8, "-aes256", "-passout", passout, "-out", &sec1_enc,
            ],
        );
        keys.push((
            vec![
                (p8, Format::Pkcs8, false),
                (sec1, Format::Sec1, false),
                (sec1_enc, Format::Sec1, true),
            ],
            algorithm,
        ));
    }
    openssl(dir, &["genrsa", "-out", "rsa.pem", "3072"]);
    openssl(
        dir,
        &[
            "rsa",
            "-in",
            "rsa.pem",
            "-traditional",
            "-out",
            "rsa.pkcs1.pem",
        ],
    );
    openssl(
        dir,
        &[
            "rsa",
            "-in",
            "rsa.pem",
            "-traditional",
            "-aes128",
            "-passout",
            passout,
            "-out",
            "rsa.pkcs1.enc.pem",
        ],
    );
    openssl(
        dir,
        &[
            "pkcs8",
            "-topk8",
            "-in",
            "rsa.pem",
            "-v2",
            "aes-128-cbc",
            "-passout",
            passout,
            "-out",
            "rsa.p8.enc.pem",
        ],
    );
    keys.push((
        vec![
            ("rsa.pem".to_owned(), Format::Pkcs8, false),
            ("rsa.pkcs1.pem".to_owned(), Format::Pkcs1, false),
            ("rsa.pkcs1.enc.pem".to_owned(), Format::Pkcs1, true),
            ("rsa.p8.enc.pem".to_owned(), Format::Pkcs8, true),
        ],
        "ssh-rsa",
    ));

    let u = first_run();
    let pass = SecretString::from(pass);
    let wrong = SecretString::from("not the passphrase".to_owned());
    for (variants, algorithm) in keys {
        let mut record = None;
        for (file, format, encrypted) in variants {
            let file = file.as_str();
            let text = std::fs::read_to_string(dir.join(file)).expect(file);
            assert_eq!(
                shellpp_core::keys::inspect(&text),
                Ok((format, encrypted)),
                "{file}"
            );
            let import = |passphrase: Option<&SecretString>| {
                u.core.key_import(KeyImportInput {
                    vault_id: u.vault_id,
                    label: Some(file.to_owned()),
                    private_key: SecretString::from(text.clone()),
                    passphrase: passphrase.cloned(),
                    save_passphrase: false,
                })
            };
            if encrypted {
                assert_eq!(
                    reasons(import(None).unwrap_err()),
                    vec![("passphrase".to_owned(), ValidationReason::Required)],
                    "{file}"
                );
                assert_eq!(
                    reasons(import(Some(&wrong)).unwrap_err()),
                    vec![("passphrase".to_owned(), ValidationReason::Invalid)],
                    "{file}"
                );
            }
            let result = import(encrypted.then_some(&pass)).expect(file);
            assert_eq!(result.key.algorithm, algorithm, "{file}");
            // Every variant of one key is the same record (same fingerprint).
            let id = *record.get_or_insert(result.key.record_id);
            assert_eq!(result.key.record_id, id, "{file}");
        }
    }
}
