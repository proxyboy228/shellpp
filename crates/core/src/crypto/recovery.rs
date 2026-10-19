//! The recovery key text format (docs/security.md, section 5.8;
//! docs/storage-format.md, section 14.4).
//!
//! 32 key bytes ‖ the first 2 bytes of SHA-256(key) = 34 bytes = 272 bits,
//! written most significant bit first as 55 Crockford base32 characters (the
//! last character carries 2 data bits and 3 zero bits) and shown as `RK1-`
//! followed by 11 groups of 5 characters separated by `-`.

use secrecy::SecretString;
use zeroize::Zeroizing;

use super::CryptoError;
use super::keys::Key32;
use super::random::{ct_eq, sha256};

/// Crockford base32 alphabet (no I, L, O, U).
pub const CROCKFORD_ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
/// Prefix of the displayed recovery key.
pub const RECOVERY_KEY_PREFIX: &str = "RK1-";
/// Number of base32 characters after the prefix (hyphens not counted).
pub const RECOVERY_KEY_CHARS: usize = 55;
/// Length of the displayed text: prefix, 55 characters and 10 hyphens.
pub const RECOVERY_KEY_TEXT_LEN: usize = 4 + RECOVERY_KEY_CHARS + 10;

const DATA_LEN: usize = 34;

/// Formats a recovery key as `RK1-XXXXX-XXXXX-...` (11 groups of 5).
pub fn encode_recovery_key(key: &Key32) -> SecretString {
    let mut data = Zeroizing::new([0u8; DATA_LEN]);
    data[..32].copy_from_slice(key.expose());
    data[32..].copy_from_slice(&sha256(key.expose())[..2]);

    let mut symbols = Zeroizing::new([0u8; RECOVERY_KEY_CHARS]);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut n = 0;
    for &byte in data.iter() {
        acc = (acc << 8) | u32::from(byte);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            symbols[n] = ((acc >> bits) & 0x1F) as u8;
            n += 1;
        }
        acc &= (1 << bits) - 1;
    }
    // 272 = 54 * 5 + 2: the last symbol holds 2 data bits and 3 zero bits.
    debug_assert_eq!((n, bits), (RECOVERY_KEY_CHARS - 1, 2));
    symbols[n] = ((acc << (5 - bits)) & 0x1F) as u8;

    // Exact capacity, so turning the String into the boxed secret never reallocates.
    let mut text = String::with_capacity(RECOVERY_KEY_TEXT_LEN);
    text.push_str(RECOVERY_KEY_PREFIX);
    for (i, &s) in symbols.iter().enumerate() {
        if i > 0 && i % 5 == 0 {
            text.push('-');
        }
        text.push(char::from(CROCKFORD_ALPHABET[usize::from(s)]));
    }
    debug_assert_eq!(text.len(), RECOVERY_KEY_TEXT_LEN);
    SecretString::from(text)
}

/// Parses a recovery key text. Ignores case, ASCII whitespace, hyphens and an
/// `RK1` prefix; maps `I` and `L` to `1` and `O` to `0`; requires 55 base32
/// characters and checks the 2-byte SHA-256 checksum. The 3 bits after the
/// 272 data bits are ignored, as in the reference script of
/// docs/storage-format.md, section 14.3. Any problem is
/// [`CryptoError::RecoveryKeyFormat`].
pub fn decode_recovery_key(text: &str) -> Result<Key32, CryptoError> {
    let mut chars = Zeroizing::new(Vec::with_capacity(text.len()));
    for c in text.chars() {
        if c == '-' || c.is_ascii_whitespace() {
            continue;
        }
        if !c.is_ascii() {
            return Err(CryptoError::RecoveryKeyFormat);
        }
        let c = match c.to_ascii_uppercase() {
            'I' | 'L' => b'1',
            'O' => b'0',
            other => other as u8,
        };
        chars.push(c);
    }
    let body: &[u8] = if chars.len() == RECOVERY_KEY_CHARS + 3 && chars.starts_with(b"RK1") {
        &chars[3..]
    } else {
        &chars[..]
    };
    if body.len() != RECOVERY_KEY_CHARS {
        return Err(CryptoError::RecoveryKeyFormat);
    }

    let mut data = Zeroizing::new([0u8; DATA_LEN]);
    let mut acc: u32 = 0;
    let mut bits = 0u32;
    let mut n = 0;
    for &c in body {
        let value = CROCKFORD_ALPHABET
            .iter()
            .position(|&a| a == c)
            .ok_or(CryptoError::RecoveryKeyFormat)?;
        acc = (acc << 5) | value as u32;
        bits += 5;
        if bits >= 8 {
            bits -= 8;
            if n < DATA_LEN {
                data[n] = ((acc >> bits) & 0xFF) as u8;
                n += 1;
            }
        }
        acc &= (1 << bits) - 1;
    }
    debug_assert_eq!(n, DATA_LEN);

    let key = Key32::from_slice(&data[..32]).ok_or(CryptoError::RecoveryKeyFormat)?;
    if !ct_eq(&sha256(key.expose())[..2], &data[32..]) {
        return Err(CryptoError::RecoveryKeyFormat);
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn format_and_round_trip() {
        for _ in 0..32 {
            let key = Key32::generate().unwrap();
            let text = encode_recovery_key(&key);
            let s = text.expose_secret();
            assert_eq!(s.len(), RECOVERY_KEY_TEXT_LEN);
            assert!(s.starts_with("RK1-"));
            let groups: Vec<&str> = s[4..].split('-').collect();
            assert_eq!(groups.len(), 11);
            assert!(groups.iter().all(|g| g.len() == 5));
            assert!(
                s[4..]
                    .bytes()
                    .all(|b| b == b'-' || CROCKFORD_ALPHABET.contains(&b))
            );
            assert!(decode_recovery_key(s).unwrap().ct_eq(&key));
        }
    }

    #[test]
    fn lenient_input() {
        let key = Key32::from_bytes([0x3C; 32]);
        let text = encode_recovery_key(&key);
        let s = text.expose_secret().to_owned();
        let lower = s.to_lowercase();
        let bare = s.replace('-', "");
        let spaced = s[4..].replace('-', " ");
        let no_prefix = s[4..].to_owned();
        let with_newline = format!("  {s}\n");
        for variant in [&s, &lower, &bare, &spaced, &no_prefix, &with_newline] {
            assert!(
                decode_recovery_key(variant).unwrap().ct_eq(&key),
                "{variant}"
            );
        }
        // I/L -> 1 and O -> 0.
        let confusable = no_prefix.replace('1', "l").replace('0', "O");
        assert!(decode_recovery_key(&confusable).unwrap().ct_eq(&key));
    }

    #[test]
    fn malformed_input() {
        let key = Key32::from_bytes([0x11; 32]);
        let s = encode_recovery_key(&key).expose_secret().to_owned();
        let bare = s[4..].replace('-', "");
        let cases = [
            String::new(),
            "RK1-".to_owned(),
            bare[..54].to_owned(),
            format!("{bare}0"),
            format!("RK2{bare}"),
            format!("{}U", &bare[..54]),
            format!("{}\u{00e9}", &bare[..54]),
        ];
        for case in &cases {
            assert!(
                matches!(
                    decode_recovery_key(case),
                    Err(CryptoError::RecoveryKeyFormat)
                ),
                "{case}"
            );
        }
        // A changed data character breaks the checksum.
        let mut chars: Vec<u8> = bare.clone().into_bytes();
        chars[0] = if chars[0] == b'0' { b'1' } else { b'0' };
        let changed = String::from_utf8(chars).unwrap();
        assert!(matches!(
            decode_recovery_key(&changed),
            Err(CryptoError::RecoveryKeyFormat)
        ));
    }
}
