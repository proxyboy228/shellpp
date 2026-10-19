//! Known-answer tests of the primitives from public test vectors: HKDF-SHA256
//! (RFC 5869), XChaCha20-Poly1305 (draft-irtf-cfrg-xchacha, appendix A.3.1),
//! Argon2id (RFC 9106, section 5.3) and SHA-256 (FIPS 180-2). They check the
//! crates the constructions are built on, through the same APIs.

mod kat_support;

use argon2::{Algorithm, Argon2, AssociatedData, ParamsBuilder, Version};
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use sha2::Sha256;

use kat_support::{hex, unhex};

#[test]
fn hkdf_sha256_rfc5869_case_1() {
    let ikm = [0x0b; 22];
    let salt: Vec<u8> = (0x00..=0x0c).collect();
    let info: Vec<u8> = (0xf0..=0xf9).collect();
    let (prk, hk) = Hkdf::<Sha256>::extract(Some(&salt), &ikm);
    assert_eq!(
        hex(&prk),
        "077709362c2e32df0ddc3f0dc47bba6390b6c73bb50f9c3122ec844ad7c2b3e5"
    );
    let mut okm = [0u8; 42];
    hk.expand(&info, &mut okm).unwrap();
    assert_eq!(
        hex(&okm),
        "3cb25f25faacd57a90434f64d0362f2a2d2d0a90cf1a5a4c5db02d56ecc4c5bf34007208d5b887185865"
    );
}

#[test]
fn hkdf_sha256_rfc5869_case_3_empty_salt() {
    // Case 3 uses an empty salt and empty info, as our derivations use an empty salt.
    let ikm = [0x0b; 22];
    let (prk, hk) = Hkdf::<Sha256>::extract(None, &ikm);
    assert_eq!(
        hex(&prk),
        "19ef24a32c717b167f33a91d6f648bdf96596776afdb6377ac434c1c293ccb04"
    );
    let mut okm = [0u8; 42];
    hk.expand(&[], &mut okm).unwrap();
    assert_eq!(
        hex(&okm),
        "8da4e775a563c18f715f802a063c5a31b8a11f5c5ee1879ec3454e5f3c738d2d9d201395faa4b61a96c8"
    );
}

#[test]
fn hkdf_empty_salt_equals_32_zero_bytes() {
    let ikm = [0x42; 32];
    let mut a = [0u8; 32];
    let mut b = [0u8; 32];
    let mut c = [0u8; 32];
    Hkdf::<Sha256>::new(None, &ikm)
        .expand(b"shellpp/v1/kek", &mut a)
        .unwrap();
    Hkdf::<Sha256>::new(Some(&[0u8; 32]), &ikm)
        .expand(b"shellpp/v1/kek", &mut b)
        .unwrap();
    Hkdf::<Sha256>::new(Some(&[]), &ikm)
        .expand(b"shellpp/v1/kek", &mut c)
        .unwrap();
    assert_eq!(a, b);
    assert_eq!(a, c);
}

const XCHACHA_PLAINTEXT: &[u8] = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
const XCHACHA_CIPHERTEXT: &str = "bd6d179d3e83d43b9576579493c0e939572a1700252bfaccbed2902c21396cbb731c7f1b0b4aa6440bf3a82f4eda7e39ae64c6708c54c216cb96b72e1213b4522f8c9ba40db5d945b11b69b982c1bb9e3f3fac2bc369488f76b2383565d3fff921f9664c97637da9768812f615c68b13b52e";
const XCHACHA_TAG: &str = "c0875924c1c7987947deafd8780acf49";

fn xchacha_inputs() -> (Key, XNonce, Vec<u8>) {
    let key: [u8; 32] = std::array::from_fn(|i| 0x80 + i as u8);
    let nonce: [u8; 24] = std::array::from_fn(|i| 0x40 + i as u8);
    let aad = unhex("50515253c0c1c2c3c4c5c6c7");
    (Key::from(key), XNonce::from(nonce), aad)
}

#[test]
fn xchacha20poly1305_draft_a_3_1() {
    let (key, nonce, aad) = xchacha_inputs();
    let cipher = XChaCha20Poly1305::new(&key);
    let out = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: XCHACHA_PLAINTEXT,
                aad: &aad,
            },
        )
        .unwrap();
    let (ct, tag) = out.split_at(out.len() - 16);
    assert_eq!(hex(ct), XCHACHA_CIPHERTEXT);
    assert_eq!(hex(tag), XCHACHA_TAG);

    let plain = cipher
        .decrypt(
            &nonce,
            Payload {
                msg: &out,
                aad: &aad,
            },
        )
        .unwrap();
    assert_eq!(plain, XCHACHA_PLAINTEXT);

    let mut bad = out.clone();
    bad[0] ^= 1;
    assert!(
        cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: &bad,
                    aad: &aad
                }
            )
            .is_err()
    );
    assert!(
        cipher
            .decrypt(
                &nonce,
                Payload {
                    msg: &out,
                    aad: b"other"
                }
            )
            .is_err()
    );
}

#[test]
fn argon2id_rfc9106_section_5_3() {
    // The RFC vector uses a secret and associated data; our construction uses
    // neither, so this checks the crate, not the construction.
    let params = ParamsBuilder::new()
        .m_cost(32)
        .t_cost(3)
        .p_cost(4)
        .output_len(32)
        .data(AssociatedData::new(&[0x04; 12]).unwrap())
        .build()
        .unwrap();
    let secret = [0x03u8; 8];
    let ctx =
        Argon2::new_with_secret(&secret, Algorithm::Argon2id, Version::V0x13, params).unwrap();
    let mut out = [0u8; 32];
    ctx.hash_password_into(&[0x01; 32], &[0x02; 16], &mut out)
        .unwrap();
    assert_eq!(
        hex(&out),
        "0d640df58d78766c08c037a34a8b53c9d01ef0452d75b65eb52520e96b01e659"
    );
}

#[test]
fn sha256_fips_180_2() {
    assert_eq!(
        hex(&shellpp_core::crypto::sha256(b"abc")),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert_eq!(
        hex(&shellpp_core::crypto::sha256(
            b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"
        )),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
}
