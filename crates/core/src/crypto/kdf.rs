//! Argon2id master key derivation, KDF parameter bounds, password and email
//! normalization (docs/security.md, section 5.4; docs/storage-format.md,
//! sections 3 and 6.5).

use argon2::{Algorithm, Argon2, Block, Params, Version};
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use unicode_normalization::UnicodeNormalization;
use zeroize::Zeroizing;

use super::keys::Key32;
use super::labels;
use super::random::sha256_parts;
use super::{CryptoError, PlatformClass};

/// The only supported KDF algorithm name.
pub const ALG_ARGON2ID: &str = "argon2id";
/// KDF scheme version 1: Argon2id, 32-byte output, HKDF labels `shellpp/v1/*`.
pub const KDF_VERSION_1: u32 = 1;
/// Minimum master password length in Unicode scalar values after NFC
/// (decision D29: 8, without composition rules).
pub const MIN_PASSWORD_CHARS: usize = 8;
/// Length of the stored random `kdf_salt`.
pub const KDF_SALT_LEN: usize = 16;

/// Floor: `m_kib >= 32768`.
pub const MIN_M_KIB: u32 = 32_768;
/// Floor: `t >= 3`.
pub const MIN_T: u32 = 3;
/// Floor: `m_kib * t >= 131072` (the cost of the mobile preset).
pub const MIN_COST: u64 = 131_072;
/// Floor: `p >= 1`.
pub const MIN_P: u32 = 1;
/// Ceiling: `p <= 16`.
pub const MAX_P: u32 = 16;
/// Ceiling on every platform: `t <= 16`.
pub const MAX_T: u32 = 16;
/// Desktop ceiling: `m_kib <= 1048576` (1 GiB).
pub const MAX_M_KIB_DESKTOP: u32 = 1_048_576;
/// Mobile ceiling: `m_kib <= 262144` (256 MiB).
pub const MAX_M_KIB_MOBILE: u32 = 262_144;

/// KDF parameters as stored in `local_auth.kdf`:
/// `{"alg":"argon2id","v":1,"m_kib":65536,"t":3,"p":4}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KdfParams {
    /// Algorithm name, always `argon2id` (Argon2 version 0x13).
    pub alg: String,
    /// KDF scheme version, always 1.
    pub v: u32,
    /// Memory in KiB.
    pub m_kib: u32,
    /// Iterations.
    pub t: u32,
    /// Lanes.
    pub p: u32,
}

impl KdfParams {
    /// The preset written when a password is created or changed on `platform`:
    /// desktop 65536 / 3 / 4, mobile 32768 / 4 / 2.
    pub fn preset(platform: PlatformClass) -> Self {
        let (m_kib, t, p) = match platform {
            PlatformClass::Desktop => (65_536, 3, 4),
            PlatformClass::Mobile => (32_768, 4, 2),
        };
        Self {
            alg: ALG_ARGON2ID.to_owned(),
            v: KDF_VERSION_1,
            m_kib,
            t,
            p,
        }
    }

    /// Refuses parameters outside the floor (`alg = argon2id`, `v = 1`,
    /// `m_kib >= 32768`, `t >= 3`, `m_kib * t >= 131072`, `1 <= p <= 16`) or
    /// above the ceiling of `platform` (desktop `m_kib <= 1048576`, mobile
    /// `m_kib <= 262144`, both `t <= 16`).
    pub fn check(&self, platform: PlatformClass) -> Result<(), CryptoError> {
        if self.alg != ALG_ARGON2ID {
            return Err(CryptoError::KdfParams("unknown algorithm"));
        }
        if self.v != KDF_VERSION_1 {
            return Err(CryptoError::KdfParams("unknown version"));
        }
        if self.m_kib < MIN_M_KIB {
            return Err(CryptoError::KdfParams("memory below the floor"));
        }
        if self.t < MIN_T {
            return Err(CryptoError::KdfParams("iterations below the floor"));
        }
        if u64::from(self.m_kib) * u64::from(self.t) < MIN_COST {
            return Err(CryptoError::KdfParams("cost below the floor"));
        }
        if self.p < MIN_P {
            return Err(CryptoError::KdfParams("lanes below the floor"));
        }
        if self.p > MAX_P {
            return Err(CryptoError::KdfParams("lanes above the ceiling"));
        }
        let max_m_kib = match platform {
            PlatformClass::Desktop => MAX_M_KIB_DESKTOP,
            PlatformClass::Mobile => MAX_M_KIB_MOBILE,
        };
        if self.m_kib > max_m_kib {
            return Err(CryptoError::KdfParams("memory above the ceiling"));
        }
        if self.t > MAX_T {
            return Err(CryptoError::KdfParams("iterations above the ceiling"));
        }
        Ok(())
    }

    /// Compact JSON with the members in the order `alg`, `v`, `m_kib`, `t`, `p`.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("KdfParams always serializes")
    }

    /// Parses the KDF JSON. Unknown members are ignored; the values are not
    /// checked here (call [`KdfParams::check`]).
    pub fn from_json(json: &str) -> Result<Self, CryptoError> {
        serde_json::from_str(json).map_err(|_| CryptoError::KdfParams("malformed JSON"))
    }
}

/// The password bytes fed to Argon2id: NFC normalization, then UTF-8. No trimming.
pub fn normalize_password(password: &SecretString) -> Zeroizing<Vec<u8>> {
    let text = password.expose_secret();
    // Two passes, so the output buffer is allocated once at its final size and
    // never reallocated (a reallocation would leave an unwiped copy behind).
    let len: usize = text.nfc().map(char::len_utf8).sum();
    let mut out = Zeroizing::new(Vec::with_capacity(len));
    let mut buf = [0u8; 4];
    for c in text.nfc() {
        out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
    buf.fill(0);
    out
}

/// True when the password has at least [`MIN_PASSWORD_CHARS`] Unicode scalar
/// values after NFC normalization.
pub fn password_length_ok(password: &SecretString) -> bool {
    password.expose_secret().nfc().count() >= MIN_PASSWORD_CHARS
}

/// The normalized form of an account email: trimmed, NFC, lowercase.
pub fn normalize_email(email: &str) -> String {
    email.trim().nfc().collect::<String>().to_lowercase()
}

/// `argon2_salt` = SHA-256(`"shellpp/v1/kdf-salt"` ‖ be16(len(e)) ‖ e ‖ kdf_salt),
/// where `e` is the UTF-8 of the stored (already normalized) `kdf_email`, empty
/// when there is none. Emails are at most 254 bytes; input longer than 65 535
/// bytes is not a valid email and only its first 65 535 bytes are used.
pub fn argon2_salt(kdf_email: Option<&str>, kdf_salt: &[u8; KDF_SALT_LEN]) -> [u8; 32] {
    let email = kdf_email.unwrap_or("").as_bytes();
    let len = u16::try_from(email.len()).unwrap_or(u16::MAX);
    sha256_parts(&[
        labels::KDF_SALT,
        &len.to_be_bytes(),
        &email[..usize::from(len)],
        kdf_salt,
    ])
}

/// Checks `params` against the bounds of `platform`, then derives the
/// 32-byte `master_key` with Argon2id version 0x13 (no secret, no associated
/// data) over the normalized password and `argon2_salt`.
///
/// This is slow by design (about 0.3-1 s at the desktop preset): never call it
/// while holding a lock other threads wait on.
///
/// The Argon2id working memory (`m_kib` KiB) is wiped before it is freed: its
/// last blocks give `master_key` without the password. `hash_password_into`
/// would free it unwiped, so the memory is allocated here.
pub fn derive_master_key(
    password: &SecretString,
    params: &KdfParams,
    argon2_salt: &[u8; 32],
    platform: PlatformClass,
) -> Result<Key32, CryptoError> {
    params.check(platform)?;
    let argon_params = Params::new(params.m_kib, params.t, params.p, Some(32))
        .map_err(|_| CryptoError::KdfParams("rejected by Argon2"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, argon_params);
    let block_count = argon.params().block_count();
    let mut blocks: Zeroizing<Vec<Block>> = Zeroizing::new(Vec::new());
    blocks
        .try_reserve_exact(block_count)
        .map_err(|_| CryptoError::Kdf)?;
    blocks.resize(block_count, Block::default());
    let pwd = normalize_password(password);
    let mut out = Key32::zeroed();
    argon
        .hash_password_into_with_memory(&pwd, argon2_salt, out.expose_mut(), blocks.as_mut_slice())
        .map_err(|_| CryptoError::Kdf)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(m_kib: u32, t: u32, p: u32) -> KdfParams {
        KdfParams {
            alg: ALG_ARGON2ID.into(),
            v: 1,
            m_kib,
            t,
            p,
        }
    }

    #[test]
    fn presets_and_json() {
        let desktop = KdfParams::preset(PlatformClass::Desktop);
        assert_eq!(
            desktop.to_json(),
            r#"{"alg":"argon2id","v":1,"m_kib":65536,"t":3,"p":4}"#
        );
        let mobile = KdfParams::preset(PlatformClass::Mobile);
        assert_eq!(
            mobile.to_json(),
            r#"{"alg":"argon2id","v":1,"m_kib":32768,"t":4,"p":2}"#
        );
        for platform in [PlatformClass::Desktop, PlatformClass::Mobile] {
            desktop.check(platform).unwrap();
            mobile.check(platform).unwrap();
        }
        assert_eq!(KdfParams::from_json(&desktop.to_json()).unwrap(), desktop);
        assert_eq!(
            KdfParams::from_json(r#"{"p":4,"t":3,"m_kib":65536,"v":1,"alg":"argon2id","x":0}"#)
                .unwrap(),
            desktop
        );
    }

    #[test]
    fn malformed_json_is_refused() {
        for json in [
            "",
            "{}",
            "[]",
            r#"{"alg":"argon2id","v":1,"m_kib":65536,"t":3}"#,
            r#"{"alg":"argon2id","v":1,"m_kib":-1,"t":3,"p":4}"#,
            r#"{"alg":"argon2id","v":1,"m_kib":4294967296,"t":3,"p":4}"#,
            r#"{"alg":"argon2id","v":1,"m_kib":65536.5,"t":3,"p":4}"#,
            r#"{"alg":1,"v":1,"m_kib":65536,"t":3,"p":4}"#,
        ] {
            assert!(
                matches!(KdfParams::from_json(json), Err(CryptoError::KdfParams(_))),
                "{json}"
            );
        }
    }

    #[test]
    fn floor_and_ceiling() {
        let d = PlatformClass::Desktop;
        let m = PlatformClass::Mobile;
        // Exactly at the floor.
        params(32_768, 4, 1).check(d).unwrap();
        params(43_691, 3, 16).check(d).unwrap();
        // Exactly at the ceilings.
        params(1_048_576, 16, 16).check(d).unwrap();
        params(262_144, 16, 16).check(m).unwrap();

        let refused = [
            (params(32_767, 5, 4), d),
            (params(65_536, 2, 4), d),
            (params(32_768, 3, 4), d), // cost 98 304 < 131 072
            (params(43_690, 3, 4), d), // cost 131 070
            (params(65_536, 3, 0), d),
            (params(65_536, 3, 17), d),
            (params(1_048_577, 3, 4), d),
            (params(65_536, 17, 4), d),
            (params(262_145, 3, 4), m),
            (params(65_536, 17, 4), m),
            (params(u32::MAX, u32::MAX, 4), d),
        ];
        for (p, platform) in refused {
            assert!(
                matches!(p.check(platform), Err(CryptoError::KdfParams(_))),
                "{p:?} on {platform:?}"
            );
        }
        let mut wrong_alg = KdfParams::preset(d);
        wrong_alg.alg = "argon2i".into();
        assert!(wrong_alg.check(d).is_err());
        let mut wrong_v = KdfParams::preset(d);
        wrong_v.v = 2;
        assert!(wrong_v.check(d).is_err());
        let mut zero_v = KdfParams::preset(d);
        zero_v.v = 0;
        assert!(zero_v.check(d).is_err());
    }

    #[test]
    fn derive_refuses_out_of_bounds_parameters_before_running() {
        let password = SecretString::from("correct horse battery");
        let salt = [0u8; 32];
        // These would be fast if they ran; the bounds must stop them first.
        for p in [params(8, 1, 1), params(32_768, 1, 1), params(1024, 3, 1)] {
            assert!(matches!(
                derive_master_key(&password, &p, &salt, PlatformClass::Desktop),
                Err(CryptoError::KdfParams(_))
            ));
        }
        // Above the mobile ceiling: refused on mobile without allocating.
        assert!(matches!(
            derive_master_key(
                &password,
                &params(1_048_576, 3, 4),
                &salt,
                PlatformClass::Mobile
            ),
            Err(CryptoError::KdfParams(_))
        ));
    }

    #[test]
    fn password_normalization() {
        // "e" + COMBINING ACUTE ACCENT -> U+00E9.
        let decomposed = SecretString::from("caf\u{0065}\u{0301} 12345");
        let composed = SecretString::from("caf\u{00e9} 12345");
        assert_eq!(
            &normalize_password(&decomposed)[..],
            "caf\u{00e9} 12345".as_bytes()
        );
        assert_eq!(
            normalize_password(&decomposed),
            normalize_password(&composed)
        );
        // No trimming.
        let spaced = SecretString::from("  pass  ");
        assert_eq!(&normalize_password(&spaced)[..], b"  pass  ");
    }

    #[test]
    fn password_length_counts_scalars_after_nfc() {
        assert!(!password_length_ok(&SecretString::from("1234567")));
        assert!(password_length_ok(&SecretString::from("12345678")));
        // 7 composed characters written with 14 code points: too short after NFC.
        let decomposed = "e\u{0301}".repeat(7);
        assert!(!password_length_ok(&SecretString::from(
            decomposed.as_str()
        )));
        // Eight emoji (4 UTF-8 bytes each) count as eight.
        assert!(password_length_ok(&SecretString::from(
            "\u{1F600}".repeat(8)
        )));
        assert!(!password_length_ok(&SecretString::from("")));
    }

    #[test]
    fn email_normalization() {
        assert_eq!(
            normalize_email("  Alice@Example.COM \n"),
            "alice@example.com"
        );
        assert_eq!(
            normalize_email("Jose\u{0301}@example.org"),
            "jos\u{00e9}@example.org"
        );
        assert_eq!(normalize_email(""), "");
    }

    #[test]
    fn argon2_salt_layout() {
        let kdf_salt = [0xA5u8; 16];
        let none = argon2_salt(None, &kdf_salt);
        let empty = argon2_salt(Some(""), &kdf_salt);
        assert_eq!(none, empty);
        let mut manual = Vec::new();
        manual.extend_from_slice(b"shellpp/v1/kdf-salt");
        manual.extend_from_slice(&[0, 17]);
        manual.extend_from_slice(b"alice@example.com");
        manual.extend_from_slice(&kdf_salt);
        assert_eq!(
            argon2_salt(Some("alice@example.com"), &kdf_salt),
            crate::crypto::sha256(&manual)
        );
        assert_ne!(argon2_salt(Some("alice@example.com"), &kdf_salt), none);
    }
}
