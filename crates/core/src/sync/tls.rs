//! Certificate trust for a sync server (docs/sync-protocol.md, section 2.1;
//! D93): the system's trust (the OS verifier), a certificate the user pinned
//! (its SHA-256 must match; host name and dates are still checked) or a CA
//! the user added for this one origin (it counts together with the system
//! roots). A refused certificate is described for the trust dialog; only an
//! unknown issuer can be trusted, never an expired certificate or one for
//! another name.

use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{CertificateError, DigitallySignedStruct, Error as TlsError, SignatureScheme};
use sha2::{Digest, Sha256};
use x509_cert::der::Decode as _;

/// How the server's certificate is trusted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TrustMode {
    /// The system root store (and the OS's checks).
    System,
    /// The end-entity certificate must have this SHA-256 (DER).
    Pinned([u8; 32]),
    /// A CA the user added for this origin, beside the system roots.
    Ca(Vec<u8>),
}

/// What the trust dialog shows about a certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertInfo {
    pub subject: String,
    pub issuer: String,
    /// RFC 3339 UTC.
    pub not_before: String,
    pub not_after: String,
    pub sha256: [u8; 32],
    pub der: Vec<u8>,
}

/// Why a handshake refused the server's certificate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TlsProblem {
    /// The issuer is unknown (self-signed, a private CA): the user may pin
    /// the certificate or add its CA.
    UnknownIssuer(CertInfo),
    /// Expired or not yet valid: refused, pinned or not.
    Expired(CertInfo),
    /// Not issued for the origin's host name: refused, pinned or not.
    WrongName(CertInfo),
    /// Another certificate than the pinned one: sync stops until the user
    /// confirms it (`system_trusts`: the system would trust the new one).
    PinChanged {
        pinned: [u8; 32],
        new: CertInfo,
        system_trusts: bool,
    },
    /// Any other TLS failure (a reason for logs and the error text).
    Other(String),
}

/// The SHA-256 fingerprint as `openssl x509 -fingerprint -sha256` prints
/// it: 32 uppercase hex pairs separated by colons.
pub fn fingerprint_text(sha256: &[u8; 32]) -> String {
    sha256
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<Vec<_>>()
        .join(":")
}

pub fn sha256(der: &[u8]) -> [u8; 32] {
    Sha256::digest(der).into()
}

/// Reads a certificate for the dialog; `None` when it is not X.509 DER.
pub fn cert_info(der: &[u8]) -> Option<CertInfo> {
    let cert = x509_cert::Certificate::from_der(der).ok()?;
    let tbs = &cert.tbs_certificate;
    let time = |t: &x509_cert::time::Time| {
        time::OffsetDateTime::from_unix_timestamp(t.to_unix_duration().as_secs() as i64)
            .ok()
            .and_then(|t| {
                t.format(&time::format_description::well_known::Rfc3339)
                    .ok()
            })
            .unwrap_or_default()
    };
    Some(CertInfo {
        subject: tbs.subject.to_string(),
        issuer: tbs.issuer.to_string(),
        not_before: time(&tbs.validity.not_before),
        not_after: time(&tbs.validity.not_after),
        sha256: sha256(der),
        der: der.to_vec(),
    })
}

/// Reads a PEM or DER CA certificate file into DER.
pub fn ca_der_from_file(bytes: &[u8]) -> Option<Vec<u8>> {
    let text = std::str::from_utf8(bytes).ok();
    let der = match text.filter(|text| text.contains("-----BEGIN CERTIFICATE-----")) {
        Some(text) => {
            let body: String = text
                .split("-----BEGIN CERTIFICATE-----")
                .nth(1)?
                .split("-----END CERTIFICATE-----")
                .next()?
                .chars()
                .filter(|c| !c.is_whitespace())
                .collect();
            data_encoding::BASE64.decode(body.as_bytes()).ok()?
        }
        None => bytes.to_vec(),
    };
    cert_info(&der).map(|_| der)
}

/// The ring provider every sync connection uses (D4).
pub fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Shared between the verifier and the caller of a request: the last
/// problem the verifier found.
pub type Observation = Arc<Mutex<Option<TlsProblem>>>;

/// The verifier of one sync origin.
#[derive(Debug)]
pub struct TrustVerifier {
    mode: TrustMode,
    system: Arc<dyn ServerCertVerifier>,
    provider: Arc<CryptoProvider>,
    observed: Observation,
}

impl TrustVerifier {
    pub fn new(mode: TrustMode, observed: Observation) -> Result<Self, TlsError> {
        let provider = provider();
        let system: Arc<dyn ServerCertVerifier> = match &mode {
            TrustMode::Ca(der) => {
                Arc::new(rustls_platform_verifier::Verifier::new_with_extra_roots(
                    [CertificateDer::from(der.clone())],
                    provider.clone(),
                )?)
            }
            _ => Arc::new(rustls_platform_verifier::Verifier::new(provider.clone())?),
        };
        Ok(Self {
            mode,
            system,
            provider,
            observed,
        })
    }

    fn note(&self, problem: TlsProblem) {
        *self
            .observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(problem);
    }

    /// Classifies a refusal of the system (or CA) verifier for the dialog.
    fn refused(&self, error: &TlsError, info: Option<CertInfo>) {
        let problem = match (error, info) {
            (TlsError::InvalidCertificate(CertificateError::UnknownIssuer), Some(info)) => {
                TlsProblem::UnknownIssuer(info)
            }
            (
                TlsError::InvalidCertificate(
                    CertificateError::Expired
                    | CertificateError::ExpiredContext { .. }
                    | CertificateError::NotValidYet
                    | CertificateError::NotValidYetContext { .. },
                ),
                Some(info),
            ) => TlsProblem::Expired(info),
            (
                TlsError::InvalidCertificate(
                    CertificateError::NotValidForName
                    | CertificateError::NotValidForNameContext { .. },
                ),
                Some(info),
            ) => TlsProblem::WrongName(info),
            (error, _) => TlsProblem::Other(error.to_string()),
        };
        self.note(problem);
    }

    /// Host name and validity dates of a pinned certificate.
    fn check_pinned(
        &self,
        end_entity: &CertificateDer<'_>,
        server_name: &ServerName<'_>,
        now: UnixTime,
        info: CertInfo,
    ) -> Result<(), TlsError> {
        let cert = webpki::EndEntityCert::try_from(end_entity)
            .map_err(|_| TlsError::InvalidCertificate(CertificateError::BadEncoding))?;
        if cert.verify_is_valid_for_subject_name(server_name).is_err() {
            self.note(TlsProblem::WrongName(info));
            return Err(TlsError::InvalidCertificate(
                CertificateError::NotValidForName,
            ));
        }
        let parsed = x509_cert::Certificate::from_der(end_entity.as_ref())
            .map_err(|_| TlsError::InvalidCertificate(CertificateError::BadEncoding))?;
        let validity = &parsed.tbs_certificate.validity;
        let now = now.as_secs();
        if now < validity.not_before.to_unix_duration().as_secs()
            || now > validity.not_after.to_unix_duration().as_secs()
        {
            self.note(TlsProblem::Expired(info));
            return Err(TlsError::InvalidCertificate(CertificateError::Expired));
        }
        Ok(())
    }
}

impl ServerCertVerifier for TrustVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let info = cert_info(end_entity.as_ref());
        match &self.mode {
            TrustMode::System | TrustMode::Ca(_) => self
                .system
                .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
                .inspect_err(|error| self.refused(error, info)),
            TrustMode::Pinned(pin) => {
                let presented = sha256(end_entity.as_ref());
                let Some(info) = info else {
                    return Err(TlsError::InvalidCertificate(CertificateError::BadEncoding));
                };
                if presented != *pin {
                    let system_trusts = self
                        .system
                        .verify_server_cert(
                            end_entity,
                            intermediates,
                            server_name,
                            ocsp_response,
                            now,
                        )
                        .is_ok();
                    self.note(TlsProblem::PinChanged {
                        pinned: *pin,
                        new: info,
                        system_trusts,
                    });
                    return Err(TlsError::General(
                        "the pinned certificate changed".to_owned(),
                    ));
                }
                self.check_pinned(end_entity, server_name, now, info)?;
                Ok(ServerCertVerified::assertion())
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The rustls client configuration of one origin.
pub fn client_config(
    mode: TrustMode,
    observed: Observation,
) -> Result<rustls::ClientConfig, TlsError> {
    let verifier = Arc::new(TrustVerifier::new(mode, observed)?);
    Ok(rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()?
        .dangerous()
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_print_as_openssl_does() {
        let mut hash = [0u8; 32];
        hash[0] = 0xab;
        hash[31] = 0x01;
        let text = fingerprint_text(&hash);
        assert!(text.starts_with("AB:00:"));
        assert!(text.ends_with(":01"));
        assert_eq!(text.len(), 32 * 3 - 1);
    }

    #[test]
    fn garbage_is_no_certificate() {
        assert!(cert_info(b"not a certificate").is_none());
        assert!(
            ca_der_from_file(b"-----BEGIN CERTIFICATE-----\nAAAA\n-----END CERTIFICATE-----\n")
                .is_none()
        );
    }
}
