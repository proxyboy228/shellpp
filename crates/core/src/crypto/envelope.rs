//! Record envelopes: padding, sealing and opening (docs/storage-format.md,
//! sections 5.2-5.4). The header codec and the size checks live in
//! `shellpp_sync_api::envelope`.
//!
//! - Key: `record_key[vault_id, key_epoch]` ([`super::derive_record_key`]).
//! - Nonce: envelope bytes `[48, 72)`, fresh from the OS CSPRNG for every seal.
//! - AAD: `"shellpp/v1/record"` ‖ envelope bytes `[0, 48)` (65 bytes).
//! - Plaintext: the payload JSON padded to a multiple of 256 bytes.

use shellpp_sync_api::envelope::{
    self as wire, EnvelopeHeader, FIXED_LEN, HEADER_LEN, NONCE_LEN, PAD_BLOCK,
};
use zeroize::Zeroizing;

use super::keys::Key32;
use super::{CryptoError, labels, random, xchacha};

/// Largest payload: 262 143 bytes (one byte of the 256 KiB is the `0x80` marker).
pub const MAX_PAYLOAD_LEN: usize = 262_143;
/// Length of the record AAD: label (17) plus header (48).
pub const RECORD_AAD_LEN: usize = 65;

const _: () = assert!(labels::RECORD.len() + HEADER_LEN == RECORD_AAD_LEN);

/// The padded length for a payload of `payload_len` bytes:
/// `256 * ceil((payload_len + 1) / 256)`.
pub fn padded_len(payload_len: usize) -> usize {
    (payload_len / PAD_BLOCK + 1) * PAD_BLOCK
}

/// ISO/IEC 7816-4 style padding: P ‖ `0x80` ‖ `0x00` × (L − len(P) − 1).
/// Refuses payloads above [`MAX_PAYLOAD_LEN`] with [`CryptoError::TooLarge`].
pub fn pad(payload: &[u8]) -> Result<Zeroizing<Vec<u8>>, CryptoError> {
    if payload.len() > MAX_PAYLOAD_LEN {
        return Err(CryptoError::TooLarge);
    }
    let len = padded_len(payload.len());
    let mut out = Zeroizing::new(Vec::with_capacity(len + xchacha::TAG_LEN));
    out.extend_from_slice(payload);
    out.push(0x80);
    out.resize(len, 0x00);
    Ok(out)
}

/// Strips trailing `0x00` bytes, then requires and strips one `0x80`.
pub fn unpad(padded: &[u8]) -> Result<&[u8], CryptoError> {
    let end = padded
        .iter()
        .rposition(|&b| b != 0x00)
        .ok_or(CryptoError::Malformed("bad padding"))?;
    if padded[end] != 0x80 {
        return Err(CryptoError::Malformed("bad padding"));
    }
    Ok(&padded[..end])
}

/// The AAD of a record: `"shellpp/v1/record"` ‖ header bytes.
pub fn record_aad(header_bytes: &[u8]) -> Vec<u8> {
    let mut aad = Vec::with_capacity(labels::RECORD.len() + header_bytes.len());
    aad.extend_from_slice(labels::RECORD);
    aad.extend_from_slice(header_bytes);
    aad
}

/// Seals a payload into a version 1 envelope with a fresh random nonce.
///
/// The header must be valid ([`EnvelopeHeader::validate`]); a payload above
/// [`MAX_PAYLOAD_LEN`] is refused with [`CryptoError::TooLarge`].
pub fn seal_envelope(
    record_key: &Key32,
    header: &EnvelopeHeader,
    payload_json: &[u8],
) -> Result<Vec<u8>, CryptoError> {
    let nonce: [u8; NONCE_LEN] = random::random_array()?;
    seal_envelope_with_nonce(record_key, header, payload_json, &nonce)
}

pub(crate) fn seal_envelope_with_nonce(
    record_key: &Key32,
    header: &EnvelopeHeader,
    payload_json: &[u8],
    nonce: &[u8; NONCE_LEN],
) -> Result<Vec<u8>, CryptoError> {
    header.validate().map_err(CryptoError::Envelope)?;
    let padded = pad(payload_json)?;
    let header_bytes = header.encode();
    let mut out = Vec::with_capacity(FIXED_LEN + padded.len() + xchacha::TAG_LEN);
    out.extend_from_slice(&header_bytes);
    out.extend_from_slice(nonce);
    xchacha::seal_append(
        record_key,
        nonce,
        &record_aad(&header_bytes),
        &padded,
        &mut out,
    )?;
    debug_assert!(wire::check_length(out.len()).is_ok());
    Ok(out)
}

/// Opens a version 1 envelope: checks 1-3 (version, length, header fields),
/// 5 (AEAD) and 6 (unpadding; the payload is UTF-8 JSON whose top level is an
/// object) of docs/storage-format.md, section 5.4. Returns the header and the
/// unpadded payload bytes.
///
/// Checks 4 (header against the row columns), 7 (payload type against the
/// class) and 8 (tombstone flag against `deleted`) are the caller's.
pub fn open_envelope(
    record_key: &Key32,
    envelope: &[u8],
) -> Result<(EnvelopeHeader, Zeroizing<Vec<u8>>), CryptoError> {
    let parts = wire::split(envelope).map_err(CryptoError::Envelope)?;
    let mut padded = xchacha::open(
        record_key,
        parts.nonce,
        &record_aad(parts.header_bytes),
        parts.ciphertext,
    )?;
    let payload_len = unpad(&padded)?.len();
    check_json_object(&padded[..payload_len])?;
    // Clear the padding area before shortening, so no bytes are left beyond `len`.
    padded[payload_len..].fill(0);
    padded.truncate(payload_len);
    Ok((parts.header, padded))
}

fn check_json_object(payload: &[u8]) -> Result<(), CryptoError> {
    let text =
        std::str::from_utf8(payload).map_err(|_| CryptoError::Malformed("payload is not UTF-8"))?;
    if !text.trim_start().starts_with('{') {
        return Err(CryptoError::Malformed("payload is not a JSON object"));
    }
    serde_json::from_str::<serde::de::IgnoredAny>(text)
        .map_err(|_| CryptoError::Malformed("payload is not valid JSON"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use shellpp_sync_api::classes;
    use shellpp_sync_api::envelope::EnvelopeError;

    fn header() -> EnvelopeHeader {
        EnvelopeHeader {
            class: classes::HOSTS,
            tombstone: false,
            vault_id: [1; 16],
            record_id: [2; 16],
            revision: 1,
            key_epoch: 1,
        }
    }

    #[test]
    fn padding_lengths() {
        assert_eq!(padded_len(0), 256);
        assert_eq!(padded_len(254), 256);
        assert_eq!(padded_len(255), 256);
        assert_eq!(padded_len(256), 512);
        assert_eq!(padded_len(MAX_PAYLOAD_LEN), 262_144);
        for len in [0usize, 1, 255, 256, 257, 511, 512, MAX_PAYLOAD_LEN] {
            let payload = vec![b'x'; len];
            let padded = pad(&payload).unwrap();
            assert_eq!(padded.len(), padded_len(len));
            assert_eq!(padded[len], 0x80);
            assert!(padded[len + 1..].iter().all(|&b| b == 0));
            assert_eq!(unpad(&padded).unwrap(), &payload[..]);
        }
        assert!(matches!(
            pad(&vec![0u8; MAX_PAYLOAD_LEN + 1]),
            Err(CryptoError::TooLarge)
        ));
    }

    #[test]
    fn payload_ending_in_marker_bytes_round_trips() {
        for payload in [&[0x80u8][..], &[0x00], &[0x80, 0x00], &[0x00, 0x80, 0x00]] {
            let padded = pad(payload).unwrap();
            assert_eq!(unpad(&padded).unwrap(), payload);
        }
    }

    #[test]
    fn bad_padding() {
        assert!(unpad(&[]).is_err());
        assert!(unpad(&[0u8; 256]).is_err());
        let mut p = vec![0u8; 256];
        p[10] = 0x81;
        assert!(unpad(&p).is_err());
        p[10] = 0x80;
        assert_eq!(unpad(&p).unwrap().len(), 10);
        let mut q = vec![b'a'; 256];
        q[255] = b'}';
        assert!(unpad(&q).is_err());
    }

    #[test]
    fn seal_open_round_trip() {
        let key = Key32::generate().unwrap();
        let payload = br#"{"v":1,"type":"host","data":{}}"#;
        let env = seal_envelope(&key, &header(), payload).unwrap();
        assert_eq!(env.len(), 344);
        let (h, p) = open_envelope(&key, &env).unwrap();
        assert_eq!(h, header());
        assert_eq!(&p[..], &payload[..]);

        let other = seal_envelope(&key, &header(), payload).unwrap();
        assert_ne!(env[48..72], other[48..72], "fresh nonce");
    }

    #[test]
    fn seal_refuses_invalid_headers_and_large_payloads() {
        let key = Key32::generate().unwrap();
        let bad = EnvelopeHeader {
            revision: 0,
            ..header()
        };
        assert!(matches!(
            seal_envelope(&key, &bad, b"{}"),
            Err(CryptoError::Envelope(EnvelopeError::ZeroRevision))
        ));
        let big = vec![b' '; MAX_PAYLOAD_LEN + 1];
        assert!(matches!(
            seal_envelope(&key, &header(), &big),
            Err(CryptoError::TooLarge)
        ));
    }

    #[test]
    fn payload_must_be_a_json_object() {
        let key = Key32::generate().unwrap();
        for bad in [
            &b"[]"[..],
            b"\"x\"",
            b"{",
            b"{\"a\":}",
            b"\xff\xfe",
            b"",
            b"{} x",
        ] {
            let env = seal_envelope(&key, &header(), bad).unwrap();
            assert!(
                matches!(open_envelope(&key, &env), Err(CryptoError::Malformed(_))),
                "{bad:?}"
            );
        }
    }
}
