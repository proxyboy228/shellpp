//! Known-answer tests of wrap blobs and record envelopes with fixed nonces
//! (docs/storage-format.md, sections 5 and 6.1-6.2): exact bytes, round trips,
//! decryption with the bare XChaCha20-Poly1305 primitive, tamper detection for
//! every header field, and the padding boundaries.

mod kat_support;

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use shellpp_core::crypto::{
    self, CryptoError, Key32, MAX_PAYLOAD_LEN, WRAPPED_KEY_LEN, open_envelope, seal_envelope,
    testing, unwrap_key, wrap_open,
};
use shellpp_sync_api::envelope::{EnvelopeError, EnvelopeHeader};

use kat_support::{aead, array, bytes, hex, text, uuid};

/// Decrypts with the primitive only, independent of the crate's helpers.
fn primitive_open(key: &[u8], nonce: &[u8], aad: &[u8], ct: &[u8]) -> Vec<u8> {
    let key = Key::try_from(key).unwrap();
    let nonce = XNonce::try_from(nonce).unwrap();
    XChaCha20Poly1305::new(&key)
        .decrypt(&nonce, Payload { msg: ct, aad })
        .unwrap()
}

#[test]
fn wrap_blobs() {
    let v = aead();
    let wraps = v["wraps"].as_object().unwrap();
    assert_eq!(wraps.len(), 5);
    for (name, w) in wraps {
        let key = Key32::from_bytes(array(w, "key"));
        let aad = bytes(w, "aad");
        let plaintext = bytes(w, "plaintext");
        let nonce: [u8; 24] = array(w, "nonce");
        let blob = bytes(w, "blob");

        let sealed = testing::wrap_seal_with_fixed_nonce(&key, &aad, &plaintext, &nonce).unwrap();
        assert_eq!(hex(&sealed), hex(&blob), "{name}");
        assert_eq!(blob[0], 0x01);
        assert_eq!(&blob[1..25], &nonce[..]);
        assert_eq!(blob.len(), 1 + 24 + plaintext.len() + 16);

        assert_eq!(&wrap_open(&key, &aad, &blob).unwrap()[..], &plaintext[..]);
        assert_eq!(
            primitive_open(key.expose(), &blob[1..25], &aad, &blob[25..]),
            plaintext
        );
        if plaintext.len() == 32 {
            assert_eq!(blob.len(), WRAPPED_KEY_LEN);
            assert_eq!(
                unwrap_key(&key, &aad, &blob).unwrap().expose()[..],
                plaintext[..]
            );
        }

        // Every bit position after the version byte is authenticated.
        for i in 1..blob.len() {
            let mut bad = blob.clone();
            bad[i] ^= 0x80;
            assert!(
                matches!(wrap_open(&key, &aad, &bad), Err(CryptoError::Aead)),
                "{name} byte {i}"
            );
        }
        let mut bad = blob.clone();
        bad[0] = 0x02;
        assert!(matches!(
            wrap_open(&key, &aad, &bad),
            Err(CryptoError::Malformed(_))
        ));
        let mut other_aad = aad.clone();
        *other_aad.last_mut().unwrap() ^= 1;
        assert!(matches!(
            wrap_open(&key, &other_aad, &blob),
            Err(CryptoError::Aead)
        ));
    }
}

#[test]
fn wrap_vectors_use_the_construction_keys() {
    // The wrapping keys in kat_aead.json are the HKDF outputs of
    // kat_constructions.json, so the two files describe one chain.
    let c = kat_support::constructions();
    let v = aead();
    let w = &v["wraps"];
    assert_eq!(text(&w["kek_e1"], "key"), text(&c["hkdf"], "kek"));
    assert_eq!(text(&w["kek_e1"], "aad"), text(&c["aad"], "wrap_kek_e1"));
    assert_eq!(
        text(&w["recovery_e1"], "key"),
        text(&c["hkdf"], "recovery_kek")
    );
    assert_eq!(
        text(&w["device_e1"], "key"),
        text(&c["hkdf"], "device_wrap_key")
    );
    assert_eq!(
        text(&w["chain_e2"], "key"),
        text(&c["hkdf"], "chain_key_e2")
    );
    assert_eq!(
        text(&w["chain_e2"], "aad"),
        text(&c["aad"], "wrap_chain_e2")
    );
    assert_eq!(
        text(&w["refresh_token"], "key"),
        text(&c["hkdf"], "local_secrets_key_e1")
    );
    for e in v["envelopes"].as_object().unwrap().values() {
        assert_eq!(text(e, "record_key"), text(&c["hkdf"], "record_key_e1"));
    }
}

fn header_of(e: &serde_json::Value) -> EnvelopeHeader {
    EnvelopeHeader {
        class: e["class"].as_u64().unwrap() as u8,
        tombstone: e["tombstone"].as_bool().unwrap(),
        vault_id: *uuid(e, "vault_id").as_bytes(),
        record_id: *uuid(e, "record_id").as_bytes(),
        revision: e["revision"].as_u64().unwrap(),
        key_epoch: e["key_epoch"].as_u64().unwrap() as u32,
    }
}

#[test]
fn envelopes() {
    let v = aead();
    let envelopes = v["envelopes"].as_object().unwrap();
    assert_eq!(envelopes.len(), 3);
    for (name, e) in envelopes {
        let key = Key32::from_bytes(array(e, "record_key"));
        let header = header_of(e);
        let payload = text(e, "payload").as_bytes();
        let nonce: [u8; 24] = array(e, "nonce");
        let expected = bytes(e, "envelope");

        let sealed =
            testing::seal_envelope_with_fixed_nonce(&key, &header, payload, &nonce).unwrap();
        assert_eq!(hex(&sealed), hex(&expected), "{name}");
        assert_eq!(expected.len(), 88 + crypto::padded_len(payload.len()));
        assert_eq!(&expected[..48], &header.encode()[..]);
        assert_eq!(&expected[48..72], &nonce[..]);

        let (h, p) = open_envelope(&key, &expected).unwrap();
        assert_eq!(h, header);
        assert_eq!(&p[..], payload);

        // Independent decryption: AAD = label || header bytes.
        let mut aad = b"shellpp/v1/record".to_vec();
        aad.extend_from_slice(&expected[..48]);
        let padded = primitive_open(key.expose(), &expected[48..72], &aad, &expected[72..]);
        assert_eq!(padded.len() % 256, 0);
        assert_eq!(&padded[..payload.len()], payload);
        assert_eq!(padded[payload.len()], 0x80);
        assert!(padded[payload.len() + 1..].iter().all(|&b| b == 0));
    }
}

/// Changes one header field and expects the given outcome.
fn expect(
    key: &Key32,
    env: &[u8],
    change: impl Fn(&mut Vec<u8>),
    want: fn(&CryptoError) -> bool,
    what: &str,
) {
    let mut bad = env.to_vec();
    change(&mut bad);
    match open_envelope(key, &bad) {
        Err(e) if want(&e) => {}
        other => panic!("{what}: unexpected {other:?}"),
    }
}

#[test]
fn tampering_with_any_header_field_is_detected() {
    let v = aead();
    let e = &v["envelopes"]["host_r3"];
    let key = Key32::from_bytes(array(e, "record_key"));
    let env = bytes(e, "envelope");
    let aead_fail = |e: &CryptoError| matches!(e, CryptoError::Aead);

    // version
    expect(
        &key,
        &env,
        |b| b[0] = 0x02,
        |e| matches!(e, CryptoError::Envelope(EnvelopeError::UnknownVersion(2))),
        "version",
    );
    expect(
        &key,
        &env,
        |b| b[0] = 0x00,
        |e| matches!(e, CryptoError::Envelope(EnvelopeError::UnknownVersion(0))),
        "version 0",
    );
    // class: another known class fails authentication, an unknown one fails check 3
    expect(&key, &env, |b| b[1] = 0x03, aead_fail, "class");
    expect(
        &key,
        &env,
        |b| b[1] = 0x0B,
        |e| matches!(e, CryptoError::Envelope(EnvelopeError::UnknownClass(0x0B))),
        "class reserved",
    );
    expect(
        &key,
        &env,
        |b| b[1] = 0x00,
        |e| matches!(e, CryptoError::Envelope(EnvelopeError::UnknownClass(0))),
        "class 0",
    );
    // flags: the tombstone bit is authenticated, reserved bits fail check 3
    expect(&key, &env, |b| b[2] ^= 0x01, aead_fail, "tombstone flag");
    for bit in 1..8 {
        expect(
            &key,
            &env,
            move |b| b[2] |= 1 << bit,
            |e| matches!(e, CryptoError::Envelope(EnvelopeError::ReservedFlags)),
            "reserved flag",
        );
    }
    // reserved byte
    expect(
        &key,
        &env,
        |b| b[3] = 0x01,
        |e| matches!(e, CryptoError::Envelope(EnvelopeError::ReservedByte)),
        "reserved byte",
    );
    // vault_id, record_id: every byte
    for i in 4..36 {
        expect(&key, &env, move |b| b[i] ^= 0x01, aead_fail, "id byte");
    }
    // revision: every byte, and zero
    for i in 36..44 {
        expect(
            &key,
            &env,
            move |b| b[i] ^= 0x10,
            aead_fail,
            "revision byte",
        );
    }
    expect(
        &key,
        &env,
        |b| b[36..44].fill(0),
        |e| matches!(e, CryptoError::Envelope(EnvelopeError::ZeroRevision)),
        "revision 0",
    );
    // key_epoch: every byte, and zero
    for i in 44..48 {
        expect(&key, &env, move |b| b[i] ^= 0x10, aead_fail, "epoch byte");
    }
    expect(
        &key,
        &env,
        |b| b[44..48].fill(0),
        |e| matches!(e, CryptoError::Envelope(EnvelopeError::ZeroEpoch)),
        "epoch 0",
    );
    // nonce, ciphertext and tag
    for i in [
        48,
        60,
        71,
        72,
        200,
        env.len() - 17,
        env.len() - 16,
        env.len() - 1,
    ] {
        expect(&key, &env, move |b| b[i] ^= 0x01, aead_fail, "body byte");
    }
    // length
    expect(
        &key,
        &env,
        |b| {
            b.pop();
        },
        |e| matches!(e, CryptoError::Envelope(EnvelopeError::BadLength(343))),
        "truncated",
    );
    expect(
        &key,
        &env,
        |b| b.extend_from_slice(&[0; 256]),
        aead_fail,
        "extended by a block",
    );
    expect(
        &key,
        &env,
        |b| b.push(0),
        |e| matches!(e, CryptoError::Envelope(EnvelopeError::BadLength(345))),
        "one byte more",
    );

    // Wrong key: the record key of another epoch.
    let c = kat_support::constructions();
    let other = Key32::from_bytes(array(&c["hkdf"], "record_key_e7"));
    assert!(matches!(
        open_envelope(&other, &env),
        Err(CryptoError::Aead)
    ));

    // A ciphertext moved to another record id (same key) fails.
    let t = bytes(&v["envelopes"]["tombstone_r4"], "envelope");
    let mut spliced = env[..72].to_vec();
    spliced.extend_from_slice(&t[72..]);
    assert!(matches!(
        open_envelope(&key, &spliced),
        Err(CryptoError::Aead)
    ));
}

fn json_payload(len: usize) -> Vec<u8> {
    // {"x":"aaa...a"} of exactly `len` bytes (len >= 8).
    let mut p = b"{\"x\":\"".to_vec();
    p.resize(len - 2, b'a');
    p.extend_from_slice(b"\"}");
    assert_eq!(p.len(), len);
    p
}

#[test]
fn envelope_padding_boundaries() {
    let key = Key32::generate().unwrap();
    let header = EnvelopeHeader {
        class: 0x02,
        tombstone: false,
        vault_id: [0x11; 16],
        record_id: [0x22; 16],
        revision: 1,
        key_epoch: 1,
    };
    for (len, env_len) in [
        (8usize, 344usize),
        (255, 344),
        (256, 600),
        (511, 600),
        (512, 856),
        (MAX_PAYLOAD_LEN, 262_232),
    ] {
        let payload = json_payload(len);
        let env = seal_envelope(&key, &header, &payload).unwrap();
        assert_eq!(env.len(), env_len, "{len}");
        let (_, p) = open_envelope(&key, &env).unwrap();
        assert_eq!(&p[..], &payload[..]);
    }
    assert!(matches!(
        seal_envelope(&key, &header, &json_payload(MAX_PAYLOAD_LEN + 1)),
        Err(CryptoError::TooLarge)
    ));
}
