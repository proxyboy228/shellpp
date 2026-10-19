//! "Decrypt your data manually" (docs/storage-format.md, section 14).
//!
//! Builds a database with the store and crypto APIs the way the app lays it
//! out (personal vault at epoch 2 with a chain row to epoch 1, `kek` and
//! `recovery` rows, records under both epochs), then reads it back following
//! only the steps of sections 14.1-14.4, with `rusqlite` and the bare primitive
//! crates (`argon2`, `hkdf`, `sha2`, `chacha20poly1305`, `unicode-normalization`),
//! without any `shellpp_core::crypto` function. When Python 3 with
//! argon2-cffi and PyNaCl is available, the reference script of section 14.3
//! is also run, unchanged, on the same file.

mod kat_support;

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use rusqlite::{Connection, OpenFlags};
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use shellpp_core::crypto::{
    self, KdfParams, Key32, PlatformClass, WrapPurpose, argon2_salt, derive_chain_key, derive_kek,
    derive_master_key, derive_record_key, derive_recovery_kek, encode_recovery_key, seal_envelope,
    vault_wrap_aad, wrap_key,
};
use shellpp_core::service::dto::{VaultKind, VaultStatus};
use shellpp_core::store::{KeyWrapRow, LocalAuthRow, RecordRow, Store, VaultRow, WrapKind};
use shellpp_sync_api::envelope::EnvelopeHeader;
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

const CROCKFORD: &[u8] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";

struct Fixture {
    password: SecretString,
    recovery_text: SecretString,
    vault_key_e2: [u8; 32],
    payloads: Vec<String>,
}

fn random_password() -> SecretString {
    let bytes: [u8; 12] = crypto::random_array().unwrap();
    // Generated at run time; a decomposed character checks NFC on both sides.
    SecretString::from(format!("kat-{}-cafe\u{0301}", kat_support::hex(&bytes)))
}

fn build(dir: &Path) -> Fixture {
    let (mut store, _) = Store::open(dir, "0.1.0").unwrap();
    let vault_id = Uuid::now_v7();
    let vk1 = Key32::generate().unwrap();
    let vk2 = Key32::generate().unwrap();
    let password = random_password();
    let kdf_salt: [u8; 16] = crypto::random_array().unwrap();
    let params = KdfParams::preset(PlatformClass::Desktop);
    let master = derive_master_key(
        &password,
        &params,
        &argon2_salt(None, &kdf_salt),
        PlatformClass::Desktop,
    )
    .unwrap();
    let kek = derive_kek(&master);
    let recovery_key = Key32::generate().unwrap();
    let recovery_kek = derive_recovery_kek(&recovery_key);
    let chain_key = derive_chain_key(&vk2, &vault_id, 2);

    let payloads = vec![
        r#"{"v":1,"type":"vault_meta","hlc":"0199f6a1b2c30001","device_id":"0199f6a1-0000-7000-8000-000000000002","data":{"name":"Personal","color":"graphite"}}"#.to_owned(),
        r#"{"v":1,"type":"host","hlc":"0199f6a1b2c30002","device_id":"0199f6a1-0000-7000-8000-000000000002","data":{"address":"198.51.100.7","label":"café db","ssh":{"port":22}}}"#.to_owned(),
        r#"{"v":1,"type":"group","hlc":"0199f6a1b2c30003","device_id":"0199f6a1-0000-7000-8000-000000000002","deleted":true}"#.to_owned(),
    ];
    // (class, epoch, vault key, tombstone)
    let layout: [(u8, u32, &Key32, bool); 3] = [
        (0x01, 1, &vk1, false),
        (0x02, 2, &vk2, false),
        (0x02, 2, &vk2, true),
    ];

    store
        .write(|tx| {
            tx.vault_put(&VaultRow {
                vault_id,
                kind: VaultKind::Personal,
                status: VaultStatus::Active,
                role: None,
                current_epoch: 2,
                bound: false,
                created_at: "2026-10-18T22:31:05.123Z".into(),
            })?;
            tx.local_auth_put(&LocalAuthRow {
                personal_vault_id: vault_id,
                kdf_json: params.to_json(),
                kdf_salt,
                kdf_email: None,
                updated_at: "2026-10-18T22:31:05.123Z".into(),
            })?;
            let wraps = [
                (
                    WrapKind::Kek,
                    2,
                    wrap_key(&kek, &vault_wrap_aad(WrapPurpose::Kek, &vault_id, 2), &vk2),
                ),
                (
                    WrapKind::Recovery,
                    2,
                    wrap_key(
                        &recovery_kek,
                        &vault_wrap_aad(WrapPurpose::Recovery, &vault_id, 2),
                        &vk2,
                    ),
                ),
                (
                    WrapKind::Chain,
                    2,
                    wrap_key(
                        &chain_key,
                        &vault_wrap_aad(WrapPurpose::EpochChain, &vault_id, 1),
                        &vk1,
                    ),
                ),
            ];
            for (kind, epoch, wrap) in wraps {
                tx.key_wrap_put(&KeyWrapRow {
                    vault_id,
                    epoch,
                    kind,
                    sender_user_id: None,
                    hpke_enc: None,
                    wrap: wrap.unwrap(),
                })?;
            }
            for ((class, epoch, vk, tombstone), payload) in layout.iter().zip(&payloads) {
                let record_id = Uuid::now_v7();
                let header = EnvelopeHeader {
                    class: *class,
                    tombstone: *tombstone,
                    vault_id: *vault_id.as_bytes(),
                    record_id: *record_id.as_bytes(),
                    revision: 1,
                    key_epoch: *epoch,
                };
                let record_key = derive_record_key(vk, &vault_id, *epoch);
                tx.record_put(&RecordRow {
                    vault_id,
                    record_id,
                    class: *class,
                    revision: 1,
                    synced_revision: 0,
                    key_epoch: *epoch,
                    tombstone: *tombstone,
                    tombstoned_at: None,
                    server_seq: None,
                    envelope: seal_envelope(&record_key, &header, payload.as_bytes()).unwrap(),
                    base_envelope: None,
                })?;
            }
            Ok(())
        })
        .unwrap();
    drop(store);

    Fixture {
        password,
        recovery_text: encode_recovery_key(&recovery_key),
        vault_key_e2: *vk2.expose(),
        payloads,
    }
}

// ---- Section 14.1 with bare primitives -------------------------------------

fn be32(x: u32) -> [u8; 4] {
    x.to_be_bytes()
}

fn hkdf32(ikm: &[u8], info: &[&[u8]]) -> [u8; 32] {
    let mut okm = [0u8; 32];
    Hkdf::<Sha256>::new(None, ikm)
        .expand_multi_info(info, &mut okm)
        .unwrap();
    okm
}

fn aead_open(key: &[u8], nonce: &[u8], aad: &[u8], ct: &[u8]) -> Vec<u8> {
    XChaCha20Poly1305::new(&Key::try_from(key).unwrap())
        .decrypt(&XNonce::try_from(nonce).unwrap(), Payload { msg: ct, aad })
        .unwrap()
}

fn open_wrap(blob: &[u8], key: &[u8], aad: &[u8]) -> Vec<u8> {
    assert_eq!(blob[0], 0x01, "wrap blob version");
    aead_open(key, &blob[1..25], aad, &blob[25..])
}

fn unpad(data: &[u8]) -> &[u8] {
    let end = data.iter().rposition(|&b| b != 0).unwrap();
    assert_eq!(data[end], 0x80);
    &data[..end]
}

/// Section 14.4, step 1, written from the text of the document.
fn decode_recovery_key(text: &str) -> [u8; 32] {
    let mut s: Vec<u8> = text
        .to_ascii_uppercase()
        .bytes()
        .filter(|&b| b != b'-' && b != b' ')
        .map(|b| match b {
            b'I' | b'L' => b'1',
            b'O' => b'0',
            other => other,
        })
        .collect();
    if s.len() == 58 && s.starts_with(b"RK1") {
        s.drain(..3);
    }
    assert_eq!(s.len(), 55);
    let mut bits: Vec<u8> = Vec::with_capacity(275);
    for c in s {
        let v = CROCKFORD.iter().position(|&a| a == c).unwrap() as u8;
        for i in (0..5).rev() {
            bits.push((v >> i) & 1);
        }
    }
    let raw: Vec<u8> = bits[..272]
        .chunks(8)
        .map(|byte| byte.iter().fold(0u8, |acc, &b| (acc << 1) | b))
        .collect();
    assert_eq!(&Sha256::digest(&raw[..32])[..2], &raw[32..34], "checksum");
    raw[..32].try_into().unwrap()
}

struct Db {
    conn: Connection,
    vault_id: Vec<u8>,
    epoch: u32,
}

impl Db {
    fn open(path: &Path) -> Db {
        let conn = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
        let vault_id: Vec<u8> = conn
            .query_row("SELECT personal_vault_id FROM local_auth", [], |r| r.get(0))
            .unwrap();
        let epoch: u32 = conn
            .query_row(
                "SELECT current_epoch FROM vaults WHERE vault_id = ?1",
                [&vault_id],
                |r| r.get(0),
            )
            .unwrap();
        Db {
            conn,
            vault_id,
            epoch,
        }
    }

    fn wrap(&self, kind: &str, epoch: u32) -> Vec<u8> {
        self.conn
            .query_row(
                "SELECT wrap FROM key_wraps WHERE vault_id = ?1 AND epoch = ?2 AND kind = ?3",
                rusqlite::params![self.vault_id, epoch, kind],
                |r| r.get(0),
            )
            .unwrap()
    }

    /// Steps 2-4 of section 14.2 with the password.
    fn vault_key_from_password(&self, password: &str) -> Vec<u8> {
        let (kdf_text, salt, email): (String, Vec<u8>, Option<String>) = self
            .conn
            .query_row("SELECT kdf, kdf_salt, kdf_email FROM local_auth", [], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })
            .unwrap();
        let kdf: serde_json::Value = serde_json::from_str(&kdf_text).unwrap();
        assert_eq!(kdf["alg"], "argon2id");
        assert_eq!(kdf["v"], 1);
        let e = email.unwrap_or_default().into_bytes();
        let mut h = Sha256::new();
        h.update(b"shellpp/v1/kdf-salt");
        h.update((e.len() as u16).to_be_bytes());
        h.update(&e);
        h.update(&salt);
        let argon2_salt = h.finalize();
        let normalized: String = password.nfc().collect();
        let params = Params::new(
            kdf["m_kib"].as_u64().unwrap() as u32,
            kdf["t"].as_u64().unwrap() as u32,
            kdf["p"].as_u64().unwrap() as u32,
            Some(32),
        )
        .unwrap();
        let mut master_key = [0u8; 32];
        Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
            .hash_password_into(normalized.as_bytes(), &argon2_salt, &mut master_key)
            .unwrap();
        let kek = hkdf32(&master_key, &[b"shellpp/v1/kek"]);
        open_wrap(
            &self.wrap("kek", self.epoch),
            &kek,
            &[
                &b"shellpp/v1/wrap/kek"[..],
                &self.vault_id,
                &be32(self.epoch),
            ]
            .concat(),
        )
    }

    /// Section 14.4 with the recovery key.
    fn vault_key_from_recovery(&self, text: &str) -> Vec<u8> {
        let recovery_key = decode_recovery_key(text);
        let key = hkdf32(&recovery_key, &[b"shellpp/v1/recovery-kek"]);
        open_wrap(
            &self.wrap("recovery", self.epoch),
            &key,
            &[
                &b"shellpp/v1/wrap/recovery"[..],
                &self.vault_id,
                &be32(self.epoch),
            ]
            .concat(),
        )
    }

    /// Steps 4-5 of section 14.2: walk the chain, decrypt every record.
    fn payloads(&self, vault_key: Vec<u8>) -> BTreeSet<String> {
        let mut keys: BTreeMap<u32, Vec<u8>> = BTreeMap::new();
        keys.insert(self.epoch, vault_key);
        for e in (2..=self.epoch).rev() {
            let chain_key = hkdf32(
                &keys[&e],
                &[b"shellpp/v1/epoch-chain", &self.vault_id, &be32(e)],
            );
            let older = open_wrap(
                &self.wrap("chain", e),
                &chain_key,
                &[
                    &b"shellpp/v1/wrap/epoch-chain"[..],
                    &self.vault_id,
                    &be32(e - 1),
                ]
                .concat(),
            );
            keys.insert(e - 1, older);
        }
        let mut stmt = self
            .conn
            .prepare("SELECT envelope FROM records WHERE vault_id = ?1")
            .unwrap();
        let envelopes: Vec<Vec<u8>> = stmt
            .query_map([&self.vault_id], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        envelopes
            .iter()
            .map(|env| {
                assert_eq!(env[0], 0x01);
                let key_epoch = u32::from_be_bytes(env[44..48].try_into().unwrap());
                let record_key = hkdf32(
                    &keys[&key_epoch],
                    &[b"shellpp/v1/record", &self.vault_id, &be32(key_epoch)],
                );
                let aad = [&b"shellpp/v1/record"[..], &env[..48]].concat();
                let plain = aead_open(&record_key, &env[48..72], &aad, &env[72..]);
                canonical(std::str::from_utf8(unpad(&plain)).unwrap())
            })
            .collect()
    }
}

fn canonical(json: &str) -> String {
    serde_json::from_str::<serde_json::Value>(json)
        .unwrap()
        .to_string()
}

#[test]
fn database_decrypts_by_the_documented_steps() {
    let tmp = tempfile::TempDir::new().unwrap();
    let fixture = build(tmp.path());
    let expected: BTreeSet<String> = fixture.payloads.iter().map(|p| canonical(p)).collect();
    let db = Db::open(&tmp.path().join("shellpp.db"));
    assert_eq!(db.epoch, 2);

    let from_password = db.vault_key_from_password(fixture.password.expose_secret());
    assert_eq!(from_password, fixture.vault_key_e2);
    assert_eq!(db.payloads(from_password), expected);

    let from_recovery = db.vault_key_from_recovery(fixture.recovery_text.expose_secret());
    assert_eq!(from_recovery, fixture.vault_key_e2);
    let lowercase = fixture.recovery_text.expose_secret().to_lowercase();
    assert_eq!(db.vault_key_from_recovery(&lowercase), fixture.vault_key_e2);
    drop(db);

    run_reference_script(tmp.path(), &fixture, &expected);
}

/// Section 14.3, the reference script itself (optional: needs Python 3 with
/// argon2-cffi and PyNaCl; the extraction and compilation are checked
/// whenever Python 3 is available).
fn run_reference_script(dir: &Path, fixture: &Fixture, expected: &BTreeSet<String>) {
    let Some(python) = kat_support::python3() else {
        kat_support::skip("no Python 3 on PATH: reference script not run");
        return;
    };
    let Some(md) = kat_support::storage_format_md() else {
        kat_support::skip("docs/storage-format.md not found: reference script not run");
        return;
    };
    let harness = kat_support::vectors_dir().join("run_reference_script.py");
    let db = dir.join("shellpp.db");
    let md = md.to_string_lossy().into_owned();
    let db = db.to_string_lossy().into_owned();
    for (mode, secret) in [
        ("password", fixture.password.expose_secret()),
        ("recovery", fixture.recovery_text.expose_secret()),
    ] {
        let out = kat_support::run_python(
            &python,
            &harness,
            &[&md, &db, mode],
            &[("SHELLPP_KAT_SECRET", secret)],
        );
        match out.status.code() {
            Some(0) => {}
            Some(3) => {
                kat_support::skip("argon2-cffi or PyNaCl not installed: reference script not run");
                return;
            }
            _ => panic!(
                "reference script ({mode}) failed: {}",
                String::from_utf8_lossy(&out.stderr)
            ),
        }
        let printed: BTreeSet<String> = String::from_utf8(out.stdout)
            .unwrap()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(canonical)
            .collect();
        assert_eq!(&printed, expected, "reference script output ({mode})");
    }
}
