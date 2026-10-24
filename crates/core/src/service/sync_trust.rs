//! The certificate trust of the sync server's origin (docs/sync-protocol.md,
//! section 2.1; UI 12.2, 13.2, 13.5; D93, D187): a trust choice is checked
//! against the server before it is stored, so the dialogs never keep a pin,
//! a CA or "the system's trust" under which the server does not answer; the
//! current trust for Account & Sync; and the root set that stands for the
//! system's trust (tests replace it, never the machine's store).
//!
//! A confirmed change of the trust changes only the `server_tls` row of the
//! origin: the account, its tokens and every local record stay, so sync goes
//! on without signing out.

use std::sync::Arc;

use serde::Serialize;

use super::Core;
use super::sync_ops::{ServerCheck, TrustChoice, trust_of};
use crate::crypto::KdfParams;
use crate::error::CoreError;
use crate::sync::tls::{self, SystemRoots};
use crate::sync::{HttpError, TrustMode, normalize_origin};

/// KDF parameters a sign-in refused (D189): the server's are weaker than
/// the ones this bound device holds for the account.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct WeakerKdf {
    pub local: KdfParams,
    pub server: KdfParams,
}

/// `server` costs an attacker less than `local`: less memory, or less
/// memory times passes (the lanes change neither).
pub(super) fn kdf_weaker(server: &KdfParams, local: &KdfParams) -> bool {
    server.m_kib < local.m_kib
        || u64::from(server.m_kib) * u64::from(server.t)
            < u64::from(local.m_kib) * u64::from(local.t)
}

/// The server's TOTP URI when the QR code may carry it (SP 5.5; D188): the
/// `otpauth://totp/` scheme and type, printable ASCII without spaces, at most
/// 1024 bytes, and a `secret` parameter equal to the secret sent beside it.
/// Anything else (a web link, another secret) is not shown at all.
pub(super) fn checked_otpauth(uri: &str, secret: &str) -> Option<String> {
    const MAX: usize = 1024;
    let prefix = "otpauth://totp/";
    if uri.len() > MAX
        || !uri.is_char_boundary(prefix.len())
        || !uri[..prefix.len()].eq_ignore_ascii_case(prefix)
        || !uri.bytes().all(|byte| byte.is_ascii_graphic())
    {
        return None;
    }
    let normal = |text: &str| text.trim_end_matches('=').to_ascii_uppercase();
    let base32 = |text: &str| {
        !text.is_empty()
            && text
                .bytes()
                .all(|byte| matches!(byte.to_ascii_uppercase(), b'A'..=b'Z' | b'2'..=b'7' | b'='))
    };
    if !base32(secret) {
        return None;
    }
    let (_, query) = uri.split_once('?')?;
    let query = query.split('#').next().unwrap_or_default();
    let mut secrets = query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .filter(|(name, _)| name.eq_ignore_ascii_case("secret"));
    let (_, given) = secrets.next()?;
    if secrets.next().is_some() || normal(given) != normal(secret) {
        return None;
    }
    Some(uri.to_owned())
}

/// How the sync server's certificate is trusted (the row of UI 12.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub enum ServerTrust {
    /// The system's root store: no row for the origin.
    System,
    /// A pinned certificate: its SHA-256.
    Pinned { sha256: [u8; 32] },
    /// A CA the user added for this origin: its name (the subject's common
    /// name, else the subject) and SHA-256.
    Ca { name: String, sha256: [u8; 32] },
}

/// The common name of an RFC 4514 subject (its escapes undone), else the
/// whole subject.
fn common_name(subject: &str) -> String {
    // Split at the commas that are not escaped, undoing `\X` and `\hh`.
    let mut parts: Vec<Vec<u8>> = vec![Vec::new()];
    let bytes = subject.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let part = parts.last_mut().expect("one part at least");
        match bytes[i] {
            b'\\' if i + 1 < bytes.len() => {
                let hex = bytes
                    .get(i + 1..i + 3)
                    .and_then(|pair| std::str::from_utf8(pair).ok())
                    .and_then(|pair| u8::from_str_radix(pair, 16).ok());
                match hex {
                    Some(byte) => {
                        part.push(byte);
                        i += 3;
                    }
                    None => {
                        part.push(bytes[i + 1]);
                        i += 2;
                    }
                }
            }
            b',' => {
                parts.push(Vec::new());
                i += 1;
            }
            byte => {
                part.push(byte);
                i += 1;
            }
        }
    }
    parts
        .into_iter()
        .map(|part| String::from_utf8_lossy(&part).trim().to_owned())
        .find_map(|part| part.strip_prefix("CN=").map(str::to_owned))
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| subject.to_owned())
}

impl Core {
    /// The root set of the system's trust for sync connections.
    pub(super) fn system_roots(&self) -> SystemRoots {
        self.sync_roots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Replaces the system's roots of sync connections by `roots` (DER):
    /// tests stand a throwaway CA for a publicly trusted one without
    /// touching the machine's certificate store.
    #[cfg(any(test, feature = "test-hooks"))]
    pub fn sync_set_system_roots_for_test(&self, roots: Vec<Vec<u8>>) {
        let roots = roots
            .into_iter()
            .map(rustls::pki_types::CertificateDer::from)
            .collect();
        *self
            .sync_roots
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) =
            SystemRoots::Fixed(Arc::new(roots));
    }

    /// The trust dialogs' choice (SP 2.1): asks `GET /v1/server` under it
    /// and stores it only when the server answers. Otherwise nothing is
    /// stored and the answer is the certificate problem under that choice:
    /// a CA the server's certificate is not issued by (still an unknown
    /// issuer), a certificate for another name or out of its dates (refused
    /// with the reason, pinned or not), or another certificate than the one
    /// confirmed. Signing in, tokens and records are not touched: a bound
    /// device goes on syncing with the new trust.
    pub fn sync_server_trust_verified(
        &self,
        origin: &str,
        choice: TrustChoice,
    ) -> Result<ServerCheck, CoreError> {
        let origin = normalize_origin(origin).map_err(HttpError::from)?;
        let mode = match &choice {
            TrustChoice::System => TrustMode::System,
            TrustChoice::Pin(sha256) => TrustMode::Pinned(*sha256),
            TrustChoice::Ca(der) => {
                if tls::cert_info(der).is_none() {
                    return Err(CoreError::Sync(HttpError::Refused("not_a_certificate")));
                }
                TrustMode::Ca(der.clone())
            }
        };
        let api = self.api_with(&origin, mode)?;
        let check = Self::server_check_with(&api, origin.clone())?;
        if matches!(check, ServerCheck::Ready { .. }) {
            self.sync_server_trust(&origin, choice)?;
            let mut guard = self.inner();
            self.emit(
                &mut guard,
                crate::service::dto::CoreEventKind::SyncChanged {},
            );
        }
        Ok(check)
    }

    /// The weaker KDF parameters the last sign-in refused (D189), for the
    /// dialog that asks to go on with them.
    pub fn sync_weaker_kdf(&self) -> Option<WeakerKdf> {
        self.inner().sync.weaker_kdf.clone()
    }

    /// The stored trust of `origin` (UI 12.2).
    pub fn sync_server_trust_info(&self, origin: &str) -> Result<ServerTrust, CoreError> {
        let origin = normalize_origin(origin).map_err(HttpError::from)?;
        let mode = trust_of(&self.inner().store, &origin)?;
        Ok(match mode {
            TrustMode::System => ServerTrust::System,
            TrustMode::Pinned(sha256) => ServerTrust::Pinned { sha256 },
            TrustMode::Ca(der) => ServerTrust::Ca {
                name: tls::cert_info(&der)
                    .map(|info| common_name(&info.subject))
                    .unwrap_or_default(),
                sha256: tls::sha256(&der),
            },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kdf(m_kib: u32, t: u32, p: u32) -> KdfParams {
        KdfParams {
            m_kib,
            t,
            p,
            ..KdfParams::preset(crate::crypto::PlatformClass::Desktop)
        }
    }

    #[test]
    fn the_qr_code_carries_only_an_otpauth_uri_of_the_secret() {
        let secret = "JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP";
        let good = "otpauth://totp/Shell%2B%2B:alice%40example.com?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&issuer=Shell%2B%2B";
        assert_eq!(checked_otpauth(good, secret).as_deref(), Some(good));
        assert!(checked_otpauth(&good.replace("otpauth://", "OTPAUTH://"), secret).is_some());
        for bad in [
            "https://evil.example/otpauth?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
            "javascript:alert(1)",
            "otpauth://hotp/x?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&counter=1",
            "otpauth://totp/x?secret=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA",
            "otpauth://totp/x",
            "otpauth://totp/x?issuer=a",
            "otpauth://totp/x y?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
            "otpauth://totp/x?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP&secret=AAAA",
            "otpauth://totp/\u{202e}?secret=JBSWY3DPEHPK3PXPJBSWY3DPEHPK3PXP",
        ] {
            assert!(checked_otpauth(bad, secret).is_none(), "{bad}");
        }
        let long = format!("{good}&x={}", "a".repeat(1024));
        assert!(checked_otpauth(&long, secret).is_none());
        assert!(checked_otpauth(good, "not base32!").is_none());
    }

    #[test]
    fn weaker_means_less_memory_or_less_work() {
        let desktop = kdf(65_536, 3, 4);
        assert!(
            kdf_weaker(&kdf(32_768, 4, 2), &desktop),
            "the mobile preset"
        );
        assert!(kdf_weaker(&kdf(49_152, 3, 4), &desktop));
        assert!(!kdf_weaker(&desktop, &desktop));
        assert!(!kdf_weaker(&kdf(65_536, 3, 1), &desktop), "lanes only");
        assert!(!kdf_weaker(&kdf(131_072, 3, 4), &desktop));
        assert!(
            kdf_weaker(&kdf(98_304, 3, 4), &kdf(65_536, 5, 4)),
            "less work"
        );
    }

    #[test]
    fn a_ca_is_named_by_its_common_name() {
        assert_eq!(
            common_name("CN=Caddy Local Authority - 2026 ECC Root"),
            "Caddy Local Authority - 2026 ECC Root"
        );
        assert_eq!(common_name("O=Home,CN=Home CA"), "Home CA");
        assert_eq!(common_name("O=Home"), "O=Home");
        assert_eq!(common_name("CN=Shell\\+\\+ test CA"), "Shell++ test CA");
        assert_eq!(common_name("CN=A\\, B,O=C"), "A, B");
        assert_eq!(common_name("CN=caf\\C3\\A9"), "café");
    }
}
