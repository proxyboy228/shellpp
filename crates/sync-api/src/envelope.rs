//! Record envelope header codec and size checks, version 1
//! (docs/storage-format.md, sections 5.1 and 5.4, checks 1 to 3).
//!
//! Layout of a version 1 envelope (big-endian integers):
//!
//! | Offset | Size | Field |
//! |---|---|---|
//! | 0 | 1 | `version` = `0x01` |
//! | 1 | 1 | `class` |
//! | 2 | 1 | `flags`: bit 0 = tombstone, bits 1-7 reserved (0) |
//! | 3 | 1 | `reserved` = `0x00` |
//! | 4 | 16 | `vault_id` |
//! | 20 | 16 | `record_id` |
//! | 36 | 8 | `revision` (>= 1) |
//! | 44 | 4 | `key_epoch` (>= 1) |
//! | 48 | 24 | `nonce` |
//! | 72 | L + 16 | ciphertext and tag, L a multiple of 256 |
//!
//! This module never decrypts anything: the AEAD step (check 5) and the
//! payload checks belong to the client core.
//!
//! Owner: W1.

use std::fmt;

use crate::classes;

/// The only envelope version defined so far.
pub const VERSION_1: u8 = 0x01;
/// Header length: bytes `[0, 48)`, also the AEAD associated data after the label.
pub const HEADER_LEN: usize = 48;
/// XChaCha20-Poly1305 nonce length.
pub const NONCE_LEN: usize = 24;
/// Poly1305 tag length.
pub const TAG_LEN: usize = 16;
/// Header plus nonce.
pub const FIXED_LEN: usize = HEADER_LEN + NONCE_LEN;
/// The padded plaintext length is a multiple of this block.
pub const PAD_BLOCK: usize = 256;
/// Smallest envelope: one padding block.
pub const MIN_ENVELOPE_LEN: usize = FIXED_LEN + PAD_BLOCK + TAG_LEN;
/// Largest envelope: 256 KiB of padded plaintext.
pub const MAX_ENVELOPE_LEN: usize = FIXED_LEN + 262_144 + TAG_LEN;
/// Bit 0 of the flags byte.
pub const FLAG_TOMBSTONE: u8 = 0x01;

const _: () = assert!(MIN_ENVELOPE_LEN == 344);
const _: () = assert!(MAX_ENVELOPE_LEN == 262_232);

/// The decoded header of a version 1 envelope.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvelopeHeader {
    /// Record class code (see [`crate::classes`]).
    pub class: u8,
    /// Flags bit 0.
    pub tombstone: bool,
    /// Vault UUID, 16 bytes in network order.
    pub vault_id: [u8; 16],
    /// Record UUID, 16 bytes in network order.
    pub record_id: [u8; 16],
    /// Record revision, at least 1.
    pub revision: u64,
    /// Vault key epoch the record is sealed with, at least 1.
    pub key_epoch: u32,
}

impl EnvelopeHeader {
    /// Encodes the header: version byte `0x01`, reserved byte `0x00`,
    /// big-endian integers. The fields are written as given; use
    /// [`EnvelopeHeader::validate`] to check them first.
    pub fn encode(&self) -> [u8; HEADER_LEN] {
        let mut out = [0u8; HEADER_LEN];
        out[0] = VERSION_1;
        out[1] = self.class;
        out[2] = if self.tombstone { FLAG_TOMBSTONE } else { 0 };
        out[3] = 0;
        out[4..20].copy_from_slice(&self.vault_id);
        out[20..36].copy_from_slice(&self.record_id);
        out[36..44].copy_from_slice(&self.revision.to_be_bytes());
        out[44..48].copy_from_slice(&self.key_epoch.to_be_bytes());
        out
    }

    /// Checks the field values a version 1 header can carry: a known class,
    /// `revision >= 1` and `key_epoch >= 1`.
    pub fn validate(&self) -> Result<(), EnvelopeError> {
        if !classes::is_known(self.class) {
            return Err(EnvelopeError::UnknownClass(self.class));
        }
        if self.revision == 0 {
            return Err(EnvelopeError::ZeroRevision);
        }
        if self.key_epoch == 0 {
            return Err(EnvelopeError::ZeroEpoch);
        }
        Ok(())
    }

    /// Decodes the first 48 bytes of `bytes` (longer input is allowed; the
    /// rest is ignored). Checks, in this order: the version
    /// ([`EnvelopeError::UnknownVersion`]), the length, a known class, flags
    /// bits 1-7 zero, the reserved byte zero, `revision >= 1`, `key_epoch >= 1`.
    pub fn decode(bytes: &[u8]) -> Result<Self, EnvelopeError> {
        let Some(&version) = bytes.first() else {
            return Err(EnvelopeError::BadLength(0));
        };
        if version != VERSION_1 {
            return Err(EnvelopeError::UnknownVersion(version));
        }
        if bytes.len() < HEADER_LEN {
            return Err(EnvelopeError::BadLength(bytes.len()));
        }
        let class = bytes[1];
        let flags = bytes[2];
        let reserved = bytes[3];
        if !classes::is_known(class) {
            return Err(EnvelopeError::UnknownClass(class));
        }
        if flags & !FLAG_TOMBSTONE != 0 {
            return Err(EnvelopeError::ReservedFlags);
        }
        if reserved != 0 {
            return Err(EnvelopeError::ReservedByte);
        }
        let mut vault_id = [0u8; 16];
        vault_id.copy_from_slice(&bytes[4..20]);
        let mut record_id = [0u8; 16];
        record_id.copy_from_slice(&bytes[20..36]);
        let mut revision = [0u8; 8];
        revision.copy_from_slice(&bytes[36..44]);
        let mut key_epoch = [0u8; 4];
        key_epoch.copy_from_slice(&bytes[44..48]);
        let header = Self {
            class,
            tombstone: flags & FLAG_TOMBSTONE != 0,
            vault_id,
            record_id,
            revision: u64::from_be_bytes(revision),
            key_epoch: u32::from_be_bytes(key_epoch),
        };
        if header.revision == 0 {
            return Err(EnvelopeError::ZeroRevision);
        }
        if header.key_epoch == 0 {
            return Err(EnvelopeError::ZeroEpoch);
        }
        Ok(header)
    }
}

/// Check 2 of storage-format 5.4: `len >= 344`, `len <= 262 232` and
/// `(len - 88) % 256 == 0`.
pub fn check_length(len: usize) -> Result<(), EnvelopeError> {
    if !(MIN_ENVELOPE_LEN..=MAX_ENVELOPE_LEN).contains(&len)
        || !(len - FIXED_LEN - TAG_LEN).is_multiple_of(PAD_BLOCK)
    {
        return Err(EnvelopeError::BadLength(len));
    }
    Ok(())
}

/// A version 1 envelope split into its parts after checks 1-3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EnvelopeParts<'a> {
    /// The decoded header.
    pub header: EnvelopeHeader,
    /// Bytes `[0, 48)`.
    pub header_bytes: &'a [u8],
    /// Bytes `[48, 72)`.
    pub nonce: &'a [u8],
    /// Bytes `[72, end)`: ciphertext followed by the 16-byte tag.
    pub ciphertext: &'a [u8],
}

/// Runs checks 1-3 of storage-format 5.4 in order (version first, so a newer
/// envelope reports [`EnvelopeError::UnknownVersion`] even when its length
/// differs) and splits the envelope.
pub fn split(envelope: &[u8]) -> Result<EnvelopeParts<'_>, EnvelopeError> {
    match envelope.first() {
        None => return Err(EnvelopeError::BadLength(0)),
        Some(&version) if version != VERSION_1 => {
            return Err(EnvelopeError::UnknownVersion(version));
        }
        Some(_) => {}
    }
    check_length(envelope.len())?;
    let header = EnvelopeHeader::decode(envelope)?;
    Ok(EnvelopeParts {
        header,
        header_bytes: &envelope[..HEADER_LEN],
        nonce: &envelope[HEADER_LEN..FIXED_LEN],
        ciphertext: &envelope[FIXED_LEN..],
    })
}

/// Why an envelope was refused. Carries only the numbers named here, never
/// envelope contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EnvelopeError {
    /// The version byte is not `0x01`: written by a newer app (keep, never rewrite).
    UnknownVersion(u8),
    /// The total length is outside the allowed range or not on a padding boundary.
    BadLength(usize),
    /// The class code is reserved.
    UnknownClass(u8),
    /// One of the flags bits 1-7 is set.
    ReservedFlags,
    /// The reserved byte is not zero.
    ReservedByte,
    /// The revision is 0.
    ZeroRevision,
    /// The key epoch is 0.
    ZeroEpoch,
}

impl fmt::Display for EnvelopeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownVersion(v) => write!(f, "unknown envelope version {v}"),
            Self::BadLength(len) => write!(f, "bad envelope length {len}"),
            Self::UnknownClass(c) => write!(f, "unknown record class {c}"),
            Self::ReservedFlags => f.write_str("reserved envelope flags set"),
            Self::ReservedByte => f.write_str("reserved envelope byte not zero"),
            Self::ZeroRevision => f.write_str("envelope revision is zero"),
            Self::ZeroEpoch => f.write_str("envelope key epoch is zero"),
        }
    }
}

impl std::error::Error for EnvelopeError {}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> EnvelopeHeader {
        EnvelopeHeader {
            class: classes::HOSTS,
            tombstone: false,
            vault_id: [0x11; 16],
            record_id: [0x22; 16],
            revision: 0x0102_0304_0506_0708,
            key_epoch: 0x0A0B_0C0D,
        }
    }

    fn envelope(header: &EnvelopeHeader, padded_len: usize) -> Vec<u8> {
        let mut out = header.encode().to_vec();
        out.resize(FIXED_LEN + padded_len + TAG_LEN, 0xAB);
        out
    }

    #[test]
    fn encode_layout_is_big_endian() {
        let bytes = sample().encode();
        let mut expected = vec![0x01, 0x02, 0x00, 0x00];
        expected.extend_from_slice(&[0x11; 16]);
        expected.extend_from_slice(&[0x22; 16]);
        expected.extend_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        expected.extend_from_slice(&[0x0A, 0x0B, 0x0C, 0x0D]);
        assert_eq!(bytes.to_vec(), expected);

        let tomb = EnvelopeHeader {
            tombstone: true,
            ..sample()
        };
        assert_eq!(tomb.encode()[2], FLAG_TOMBSTONE);
    }

    #[test]
    fn round_trip() {
        for tombstone in [false, true] {
            for class in classes::ALL {
                let header = EnvelopeHeader {
                    class,
                    tombstone,
                    revision: u64::MAX,
                    key_epoch: u32::MAX,
                    ..sample()
                };
                assert_eq!(header.validate(), Ok(()));
                assert_eq!(EnvelopeHeader::decode(&header.encode()), Ok(header));
                let env = envelope(&header, 256);
                let parts = split(&env).unwrap();
                assert_eq!(parts.header, header);
                assert_eq!(parts.header_bytes, &header.encode()[..]);
                assert_eq!(parts.nonce.len(), NONCE_LEN);
                assert_eq!(parts.ciphertext.len(), 256 + TAG_LEN);
            }
        }
    }

    #[test]
    fn decode_errors() {
        let good = sample().encode();

        let mut b = good;
        b[0] = 0x02;
        assert_eq!(
            EnvelopeHeader::decode(&b),
            Err(EnvelopeError::UnknownVersion(2))
        );
        b[0] = 0x00;
        assert_eq!(
            EnvelopeHeader::decode(&b),
            Err(EnvelopeError::UnknownVersion(0))
        );

        assert_eq!(
            EnvelopeHeader::decode(&good[..47]),
            Err(EnvelopeError::BadLength(47))
        );
        assert_eq!(
            EnvelopeHeader::decode(&[]),
            Err(EnvelopeError::BadLength(0))
        );

        for class in [0x00, 0x0B, 0x80, 0xFF] {
            let mut b = good;
            b[1] = class;
            assert_eq!(
                EnvelopeHeader::decode(&b),
                Err(EnvelopeError::UnknownClass(class))
            );
        }

        for bit in 1..8 {
            let mut b = good;
            b[2] = 1 << bit;
            assert_eq!(
                EnvelopeHeader::decode(&b),
                Err(EnvelopeError::ReservedFlags)
            );
            b[2] = (1 << bit) | FLAG_TOMBSTONE;
            assert_eq!(
                EnvelopeHeader::decode(&b),
                Err(EnvelopeError::ReservedFlags)
            );
        }

        let mut b = good;
        b[3] = 0x01;
        assert_eq!(EnvelopeHeader::decode(&b), Err(EnvelopeError::ReservedByte));

        let mut b = good;
        b[36..44].copy_from_slice(&[0; 8]);
        assert_eq!(EnvelopeHeader::decode(&b), Err(EnvelopeError::ZeroRevision));

        let mut b = good;
        b[44..48].copy_from_slice(&[0; 4]);
        assert_eq!(EnvelopeHeader::decode(&b), Err(EnvelopeError::ZeroEpoch));
    }

    #[test]
    fn validate_errors() {
        let h = EnvelopeHeader {
            class: 0,
            ..sample()
        };
        assert_eq!(h.validate(), Err(EnvelopeError::UnknownClass(0)));
        let h = EnvelopeHeader {
            revision: 0,
            ..sample()
        };
        assert_eq!(h.validate(), Err(EnvelopeError::ZeroRevision));
        let h = EnvelopeHeader {
            key_epoch: 0,
            ..sample()
        };
        assert_eq!(h.validate(), Err(EnvelopeError::ZeroEpoch));
    }

    #[test]
    fn length_boundaries() {
        assert_eq!(check_length(0), Err(EnvelopeError::BadLength(0)));
        assert_eq!(check_length(88), Err(EnvelopeError::BadLength(88)));
        assert_eq!(check_length(343), Err(EnvelopeError::BadLength(343)));
        assert_eq!(check_length(344), Ok(()));
        assert_eq!(check_length(345), Err(EnvelopeError::BadLength(345)));
        assert_eq!(check_length(599), Err(EnvelopeError::BadLength(599)));
        assert_eq!(check_length(600), Ok(()));
        assert_eq!(check_length(601), Err(EnvelopeError::BadLength(601)));
        assert_eq!(
            check_length(262_231),
            Err(EnvelopeError::BadLength(262_231))
        );
        assert_eq!(check_length(262_232), Ok(()));
        assert_eq!(
            check_length(262_233),
            Err(EnvelopeError::BadLength(262_233))
        );
        assert_eq!(
            check_length(262_488),
            Err(EnvelopeError::BadLength(262_488))
        );
        assert_eq!(
            check_length(usize::MAX),
            Err(EnvelopeError::BadLength(usize::MAX))
        );
    }

    #[test]
    fn split_checks_in_order() {
        let header = sample();
        assert!(split(&envelope(&header, 256)).is_ok());
        assert!(split(&envelope(&header, 262_144)).is_ok());

        assert_eq!(split(&[]), Err(EnvelopeError::BadLength(0)));

        // Version first: a newer envelope with any length reports its version.
        let mut newer = envelope(&header, 256);
        newer[0] = 0x02;
        newer.truncate(100);
        assert_eq!(split(&newer), Err(EnvelopeError::UnknownVersion(2)));
        assert_eq!(split(&[0x07]), Err(EnvelopeError::UnknownVersion(7)));

        // Length before the header fields.
        let mut short = envelope(&header, 256);
        short[1] = 0x00;
        short.pop();
        assert_eq!(split(&short), Err(EnvelopeError::BadLength(343)));
        assert_eq!(split(&short[..48]), Err(EnvelopeError::BadLength(48)));

        let mut bad_class = envelope(&header, 256);
        bad_class[1] = 0x0B;
        assert_eq!(split(&bad_class), Err(EnvelopeError::UnknownClass(0x0B)));

        let mut too_long = envelope(&header, 262_144);
        too_long.extend_from_slice(&[0; 256]);
        assert_eq!(
            split(&too_long),
            Err(EnvelopeError::BadLength(262_232 + 256))
        );
    }

    #[test]
    fn display_has_no_payload() {
        assert_eq!(
            EnvelopeError::UnknownVersion(2).to_string(),
            "unknown envelope version 2"
        );
        assert_eq!(
            EnvelopeError::BadLength(343).to_string(),
            "bad envelope length 343"
        );
        assert_eq!(
            EnvelopeError::UnknownClass(11).to_string(),
            "unknown record class 11"
        );
    }
}
