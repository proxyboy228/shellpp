//! What the key panel shows of an OpenSSH user certificate (night plan,
//! stage 3 item 5; docs/data-model.md, section 9.2; UI 7.3 block 5): key id,
//! serial, principals, the validity window, the SHA256 fingerprint of the
//! certificate authority and the status (not yet valid, valid, expired; an
//! expired certificate is kept and marked). Derived, never stored.

use serde::Serialize;
use ssh_key::{Certificate, HashAlg};

use crate::model::time::format_unix_ms;

/// The `valid_before` of a certificate that never expires.
const FOREVER: u64 = u64::MAX;

/// Where a certificate stands at a moment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CertificateStatus {
    NotYetValid,
    Valid,
    Expired,
}

/// The parts of a certificate the panel shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CertificateInfo {
    pub key_id: String,
    pub serial: u64,
    /// Empty: valid for any principal.
    pub principals: Vec<String>,
    /// RFC 3339 UTC; `None` = from the beginning of time.
    pub valid_after: Option<String>,
    /// RFC 3339 UTC; `None` = never expires.
    pub valid_before: Option<String>,
    /// `SHA256:` fingerprint of the signing CA key.
    pub ca_fingerprint: String,
    pub status: CertificateStatus,
}

fn time_text(seconds: u64) -> Option<String> {
    seconds.checked_mul(1000).map(format_unix_ms)
}

/// The info of one OpenSSH certificate line at `now` (Unix seconds); `None`
/// when the line is not a certificate.
pub fn certificate_info(line: &str, now: u64) -> Option<CertificateInfo> {
    let certificate = Certificate::from_openssh(line.trim()).ok()?;
    let after = certificate.valid_after();
    let before = certificate.valid_before();
    let status = if now < after {
        CertificateStatus::NotYetValid
    } else if before != FOREVER && now >= before {
        CertificateStatus::Expired
    } else {
        CertificateStatus::Valid
    };
    Some(CertificateInfo {
        key_id: certificate.key_id().chars().take(256).collect(),
        serial: certificate.serial(),
        principals: certificate
            .valid_principals()
            .iter()
            .take(64)
            .map(|principal| principal.chars().take(256).collect())
            .collect(),
        valid_after: (after != 0).then(|| time_text(after)).flatten(),
        valid_before: (before != FOREVER).then(|| time_text(before)).flatten(),
        ca_fingerprint: certificate
            .signature_key()
            .fingerprint(HashAlg::Sha256)
            .to_string(),
        status,
    })
}
