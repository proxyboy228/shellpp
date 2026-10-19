//! Known-answer and bounds tests of the master key derivation
//! (docs/security.md, section 5.4; docs/storage-format.md, section 6.5).
//!
//! KDF parameters are never weakened in tests: the only Argon2id run here
//! uses the desktop preset (the crypto crates are built with opt-level 3).

mod kat_support;

use secrecy::SecretString;
use shellpp_core::crypto::{
    CryptoError, KdfParams, PlatformClass, argon2_salt, derive_kek, derive_master_key,
};

use kat_support::{aead, array, hex, text};

#[test]
fn master_key_at_the_desktop_preset() {
    let v = aead();
    let m = &v["master_key"];
    let params = KdfParams {
        alg: text(m, "alg").to_owned(),
        v: m["v"].as_u64().unwrap() as u32,
        m_kib: m["m_kib"].as_u64().unwrap() as u32,
        t: m["t"].as_u64().unwrap() as u32,
        p: m["p"].as_u64().unwrap() as u32,
    };
    assert_eq!(params, KdfParams::preset(PlatformClass::Desktop));
    let kdf_salt: [u8; 16] = array(m, "kdf_salt");
    assert!(m["kdf_email"].is_null());
    let salt = argon2_salt(None, &kdf_salt);
    assert_eq!(hex(&salt), text(m, "argon2_salt"));

    // The stored password is in decomposed form; NFC must happen before Argon2id.
    let password = SecretString::from(text(m, "password"));
    assert!(text(m, "password").contains('\u{0308}'));
    let master = derive_master_key(&password, &params, &salt, PlatformClass::Desktop).unwrap();
    assert_eq!(hex(master.expose()), text(m, "master_key"));
    // And the KEK of that master key is a plain HKDF step.
    assert_eq!(derive_kek(&master).expose().len(), 32);
}

fn p(m_kib: u32, t: u32, p: u32) -> KdfParams {
    KdfParams {
        alg: "argon2id".into(),
        v: 1,
        m_kib,
        t,
        p,
    }
}

#[test]
fn parameters_outside_the_bounds_are_refused_before_the_kdf_runs() {
    let password = SecretString::from("kat bounds password");
    let salt = [0u8; 32];
    let refused: Vec<(KdfParams, PlatformClass)> = vec![
        // Floor.
        (p(32_767, 4, 4), PlatformClass::Desktop),
        (p(65_536, 2, 4), PlatformClass::Desktop),
        (p(32_768, 3, 4), PlatformClass::Desktop),
        (p(65_536, 3, 0), PlatformClass::Desktop),
        (p(65_536, 3, 17), PlatformClass::Desktop),
        (p(8, 1, 1), PlatformClass::Desktop),
        // Ceilings.
        (p(1_048_577, 3, 4), PlatformClass::Desktop),
        (p(65_536, 17, 4), PlatformClass::Desktop),
        (p(262_145, 3, 2), PlatformClass::Mobile),
        (p(1_048_576, 3, 4), PlatformClass::Mobile),
        (p(32_768, 17, 2), PlatformClass::Mobile),
        // Unknown algorithm or scheme version.
        (
            KdfParams {
                alg: "argon2i".into(),
                ..p(65_536, 3, 4)
            },
            PlatformClass::Desktop,
        ),
        (
            KdfParams {
                v: 2,
                ..p(65_536, 3, 4)
            },
            PlatformClass::Desktop,
        ),
    ];
    for (params, platform) in refused {
        assert!(
            matches!(params.check(platform), Err(CryptoError::KdfParams(_))),
            "{params:?}"
        );
        assert!(
            matches!(
                derive_master_key(&password, &params, &salt, platform),
                Err(CryptoError::KdfParams(_))
            ),
            "{params:?}"
        );
    }
}

#[test]
fn stored_parameters_round_trip_and_are_checked_after_parsing() {
    for platform in [PlatformClass::Desktop, PlatformClass::Mobile] {
        let preset = KdfParams::preset(platform);
        let parsed = KdfParams::from_json(&preset.to_json()).unwrap();
        assert_eq!(parsed, preset);
        parsed.check(PlatformClass::Desktop).unwrap();
        parsed.check(PlatformClass::Mobile).unwrap();
    }
    // A database or server answer with weak parameters parses but is refused.
    let weak =
        KdfParams::from_json(r#"{"alg":"argon2id","v":1,"m_kib":1024,"t":1,"p":1}"#).unwrap();
    assert!(weak.check(PlatformClass::Desktop).is_err());
    // A desktop-only cost is refused on mobile.
    let heavy =
        KdfParams::from_json(r#"{"alg":"argon2id","v":1,"m_kib":524288,"t":3,"p":4}"#).unwrap();
    heavy.check(PlatformClass::Desktop).unwrap();
    assert!(heavy.check(PlatformClass::Mobile).is_err());
}
