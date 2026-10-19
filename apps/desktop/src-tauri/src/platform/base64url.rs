//! Base64url without padding (RFC 4648, section 5) for the 32-byte device
//! key in the OS keychain: 43 characters (docs/storage-format.md, section 2.3).
//!
//! This is a text encoding, not cryptography. Outputs that hold key material
//! are returned in zeroizing containers.

use secrecy::zeroize::Zeroizing;

const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

/// Encoded length of a 32-byte key.
pub const KEY_TEXT_LEN: usize = 43;

/// Encodes without padding.
pub fn encode(bytes: &[u8]) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::with_capacity(bytes.len().div_ceil(3) * 4));
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk[0]);
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        let chars = chunk.len() + 1;
        for i in 0..chars {
            let index = ((n >> (18 - 6 * i)) & 0x3f) as usize;
            out.push(char::from(ALPHABET[index]));
        }
    }
    out
}

fn value(c: u8) -> Option<u32> {
    let v = match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'-' => 62,
        b'_' => 63,
        _ => return None,
    };
    Some(u32::from(v))
}

/// Decodes unpadded base64url. Refuses padding, other alphabets, an
/// impossible length (`len % 4 == 1`) and non-zero trailing bits.
pub fn decode(text: &str) -> Option<Zeroizing<Vec<u8>>> {
    let bytes = text.as_bytes();
    if bytes.len() % 4 == 1 {
        return None;
    }
    let mut out = Zeroizing::new(Vec::with_capacity(bytes.len() / 4 * 3 + 2));
    for chunk in bytes.chunks(4) {
        let mut n: u32 = 0;
        for (i, &c) in chunk.iter().enumerate() {
            n |= value(c)? << (18 - 6 * i);
        }
        let produced = chunk.len() - 1;
        let all = [(n >> 16) as u8, (n >> 8) as u8, n as u8];
        // Bits beyond the produced bytes must be zero (canonical encoding).
        if all[produced..].iter().any(|&b| b != 0) && produced < 3 {
            return None;
        }
        out.extend_from_slice(&all[..produced]);
    }
    Some(out)
}

/// Decodes a 32-byte key (exactly 43 characters).
pub fn decode_key(text: &str) -> Option<Zeroizing<[u8; 32]>> {
    if text.len() != KEY_TEXT_LEN {
        return None;
    }
    let bytes = decode(text)?;
    let mut key = Zeroizing::new([0u8; 32]);
    if bytes.len() != 32 {
        return None;
    }
    key.copy_from_slice(&bytes);
    Some(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rfc4648_vectors_without_padding() {
        for (plain, encoded) in [
            ("", ""),
            ("f", "Zg"),
            ("fo", "Zm8"),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg"),
            ("fooba", "Zm9vYmE"),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(encode(plain.as_bytes()).as_str(), encoded);
            assert_eq!(decode(encoded).unwrap().as_slice(), plain.as_bytes());
        }
    }

    #[test]
    fn url_safe_alphabet() {
        assert_eq!(encode(&[0xfb, 0xff]).as_str(), "-_8");
        assert_eq!(decode("-_8").unwrap().as_slice(), &[0xfb, 0xff]);
    }

    #[test]
    fn refuses_bad_input() {
        for text in ["Zg==", "Z", "Zh", "Zm9+", "Zm9/", "Zm 9", "Zm9vY"] {
            assert!(decode(text).is_none(), "{text}");
        }
    }

    #[test]
    fn key_round_trip_is_43_chars() {
        let key: [u8; 32] = core::array::from_fn(|i| (i * 7 + 3) as u8);
        let text = encode(&key);
        assert_eq!(text.len(), KEY_TEXT_LEN);
        assert_eq!(*decode_key(&text).unwrap(), key);
        assert!(decode_key(&text[..42]).is_none());
        assert!(decode_key(&format!("{}A", text.as_str())).is_none());
    }
}
