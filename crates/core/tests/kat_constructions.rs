//! Known-answer tests of the constructions whose expected values are computed
//! independently with the Python 3 standard library (`vectors/gen_vectors.py`
//! -> `vectors/kat_constructions.json`): labels, KDF JSON, Argon2id salts,
//! every HKDF derivation, wrap and record AADs, padding and the recovery key
//! text (docs/security.md, section 5; docs/storage-format.md, sections 5, 6, 14).

mod kat_support;

use secrecy::ExposeSecret;
use shellpp_core::crypto::{
    self, KdfParams, Key32, PlatformClass, WrapPurpose, argon2_salt, decode_recovery_key,
    derive_auth_key, derive_chain_key, derive_device_wrap_key, derive_epoch_commit,
    derive_export_key, derive_kek, derive_local_secrets_key, derive_record_key,
    derive_recording_key, derive_recovery_auth_key, derive_recovery_kek, encode_recovery_key,
    normalize_email, pad, padded_len, record_aad, refresh_token_aad, sha256, unpad, vault_wrap_aad,
};
use shellpp_sync_api::envelope::EnvelopeHeader;

use kat_support::{array, constructions, hex, text, uuid};

fn key(v: &serde_json::Value, name: &str) -> Key32 {
    Key32::from_bytes(array::<32>(v, name))
}

#[test]
fn labels_match_the_vectors() {
    let v = constructions();
    let expected: Vec<&str> = v["labels"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| l.as_str().unwrap())
        .collect();
    let actual: Vec<&str> = crypto::labels::ALL
        .iter()
        .map(|l| std::str::from_utf8(l).unwrap())
        .collect();
    assert_eq!(actual, expected);
}

#[test]
fn kdf_json_presets() {
    let v = constructions();
    assert_eq!(
        KdfParams::preset(PlatformClass::Desktop).to_json(),
        text(&v["kdf_json"], "desktop")
    );
    assert_eq!(
        KdfParams::preset(PlatformClass::Mobile).to_json(),
        text(&v["kdf_json"], "mobile")
    );
}

#[test]
fn argon2_salts() {
    let v = constructions();
    let salt: [u8; 16] = array(&v["inputs"], "kdf_salt");
    let s = &v["argon2_salt"];
    assert_eq!(hex(&argon2_salt(None, &salt)), text(s, "no_email"));
    assert_eq!(hex(&argon2_salt(Some(""), &salt)), text(s, "no_email"));
    assert_eq!(
        hex(&argon2_salt(Some("alice@example.com"), &salt)),
        text(s, "alice")
    );
    assert_eq!(
        hex(&argon2_salt(
            Some(&normalize_email("Zo\u{00eb}@example.org")),
            &salt
        )),
        text(s, "unicode")
    );
}

#[test]
fn email_normalization() {
    let v = constructions();
    for case in v["email_normalization"].as_array().unwrap() {
        assert_eq!(
            normalize_email(text(case, "input")),
            text(case, "normalized")
        );
    }
}

#[test]
fn hkdf_derivations() {
    let v = constructions();
    let i = &v["inputs"];
    let h = &v["hkdf"];
    let master = key(i, "master_key");
    let recovery = key(i, "recovery_key");
    let device = key(i, "device_key");
    let vault = key(i, "vault_key");
    let vault_id = uuid(i, "vault_id");
    let origin = text(i, "origin");

    let cases: [(&str, Key32); 11] = [
        ("kek", derive_kek(&master)),
        ("auth_key", derive_auth_key(&master, origin)),
        ("recovery_kek", derive_recovery_kek(&recovery)),
        (
            "recovery_auth_key",
            derive_recovery_auth_key(&recovery, origin),
        ),
        (
            "device_wrap_key",
            derive_device_wrap_key(&device, &uuid(i, "db_id")),
        ),
        ("record_key_e1", derive_record_key(&vault, &vault_id, 1)),
        ("record_key_e7", derive_record_key(&vault, &vault_id, 7)),
        ("chain_key_e2", derive_chain_key(&vault, &vault_id, 2)),
        (
            "local_secrets_key_e1",
            derive_local_secrets_key(&vault, &uuid(i, "device_id"), 1),
        ),
        (
            "recording_key",
            derive_recording_key(&vault, &uuid(i, "file_id")),
        ),
        ("export_key", derive_export_key(&master)),
    ];
    for (name, derived) in &cases {
        assert_eq!(hex(derived.expose()), text(h, name), "{name}");
    }
    assert_eq!(
        hex(&derive_epoch_commit(&vault, &vault_id, 1)),
        text(h, "epoch_commit_e1")
    );
}

#[test]
fn wrap_and_record_aads() {
    let v = constructions();
    let i = &v["inputs"];
    let a = &v["aad"];
    let vault_id = uuid(i, "vault_id");
    assert_eq!(
        hex(&vault_wrap_aad(WrapPurpose::Kek, &vault_id, 1)),
        text(a, "wrap_kek_e1")
    );
    assert_eq!(
        hex(&vault_wrap_aad(WrapPurpose::Recovery, &vault_id, 1)),
        text(a, "wrap_recovery_e1")
    );
    assert_eq!(
        hex(&vault_wrap_aad(WrapPurpose::Device, &vault_id, 1)),
        text(a, "wrap_device_e1")
    );
    // The chain row of epoch 2 wraps the key of epoch 1: AAD names epoch 1.
    assert_eq!(
        hex(&vault_wrap_aad(WrapPurpose::EpochChain, &vault_id, 2 - 1)),
        text(a, "wrap_chain_e2")
    );
    assert_eq!(
        hex(&refresh_token_aad(&uuid(i, "device_id"))),
        text(a, "refresh_token")
    );

    let header = EnvelopeHeader {
        class: 0x02,
        tombstone: false,
        vault_id: *vault_id.as_bytes(),
        record_id: *uuid(i, "record_id").as_bytes(),
        revision: 3,
        key_epoch: 1,
    };
    let header_bytes = header.encode();
    assert_eq!(hex(&header_bytes), text(a, "record_header"));
    let aad = record_aad(&header_bytes);
    assert_eq!(aad.len(), 65);
    assert_eq!(hex(&aad), text(a, "record"));
}

#[test]
fn padding_boundaries() {
    let v = constructions();
    for case in v["padding"].as_array().unwrap() {
        let n = case["payload_len"].as_u64().unwrap() as usize;
        let payload = vec![b'a'; n];
        let padded = pad(&payload).unwrap();
        assert_eq!(
            padded.len() as u64,
            case["padded_len"].as_u64().unwrap(),
            "{n}"
        );
        assert_eq!(padded_len(n), padded.len());
        assert_eq!(hex(&sha256(&padded)), text(case, "padded_sha256"), "{n}");
        assert_eq!(unpad(&padded).unwrap(), &payload[..]);
    }
    assert!(pad(&vec![b'a'; crypto::MAX_PAYLOAD_LEN + 1]).is_err());
}

#[test]
fn recovery_key_text() {
    let v = constructions();
    for case in v["recovery_key_text"].as_array().unwrap() {
        let key = Key32::from_bytes(array(case, "key"));
        let expected = text(case, "text");
        let encoded = encode_recovery_key(&key);
        assert_eq!(encoded.expose_secret(), expected);
        assert!(decode_recovery_key(expected).unwrap().ct_eq(&key));
        let relaxed = expected.to_lowercase().replace('-', " ");
        assert!(decode_recovery_key(&relaxed).unwrap().ct_eq(&key));
    }
}

/// Re-runs the Python generator in `--check` mode when Python 3 is available:
/// the JSON file must match a fresh, independent computation. With the
/// optional packages argon2-cffi and PyNaCl the script also verifies the
/// recorded AEAD and Argon2id vectors.
#[test]
fn python_generator_agrees() {
    let Some(python) = kat_support::python3() else {
        kat_support::skip("no Python 3 on PATH");
        return;
    };
    let script = kat_support::vectors_dir().join("gen_vectors.py");
    let out = kat_support::run_python(&python, &script, &["--check"], &[]);
    assert!(
        out.status.success(),
        "gen_vectors.py --check failed: {}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}
