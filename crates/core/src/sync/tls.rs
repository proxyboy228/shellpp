//! Certificate trust for a sync server (docs/sync-protocol.md, section 2.1;
//! D93): the system's trust (the OS verifier), a certificate the user pinned
//! (its SHA-256 must match; host name and dates are still checked) or a CA
//! the user added for this one origin (it counts together with the system
//! roots). A refused certificate is described for the trust dialog; only an
//! unknown issuer can be trusted, never an expired certificate or one for
//! another name.
//!
//! The system's trust is the OS verifier, or for tests a fixed set of roots
//! ([`SystemRoots::Fixed`]): no test changes the machine's certificate
//! store, and the tests of the trust choices do not read it either. An
//! added CA lives only in the verifier of its origin; nothing here writes
//! to the OS store.

use std::sync::{Arc, Mutex};

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, DigitallySignedStruct, Error as TlsError, RootCertStore, SignatureScheme,
};
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

/// What stands for "the system's trust".
#[derive(Clone, Debug, Default)]
pub enum SystemRoots {
    /// The OS verifier and its root store (the app).
    #[default]
    Platform,
    /// These roots only, checked by webpki (tests: a throwaway CA stands for
    /// a publicly trusted one, and the machine's store is never involved).
    Fixed(Arc<Vec<CertificateDer<'static>>>),
}

/// A verifier that trusts nothing: a fixed set without roots.
#[derive(Debug)]
struct NoRoots(Arc<CryptoProvider>);

impl ServerCertVerifier for NoRoots {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        Err(TlsError::InvalidCertificate(
            CertificateError::UnknownIssuer,
        ))
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
            &self.0.signature_verification_algorithms,
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
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// An added CA beside a verifier of the system's trust that takes no extra
/// roots: on Android the platform verifier asks the system's trust manager
/// and cannot be given one (`docs/android-feasibility.md`, 2). The chain is
/// checked against the CA alone first (webpki, as [`SystemRoots::Fixed`]
/// does), and the system is asked when the CA does not vouch for it.
#[derive(Debug)]
struct CaBesideSystem {
    ca: Arc<WebPkiServerVerifier>,
    system: Arc<dyn ServerCertVerifier>,
    provider: Arc<CryptoProvider>,
}

impl CaBesideSystem {
    #[cfg_attr(not(any(target_os = "android", test)), allow(dead_code))]
    fn new(
        ca: CertificateDer<'static>,
        system: Arc<dyn ServerCertVerifier>,
        provider: &Arc<CryptoProvider>,
    ) -> Result<Self, TlsError> {
        let mut store = RootCertStore::empty();
        store.add(ca)?;
        let ca = WebPkiServerVerifier::builder_with_provider(Arc::new(store), provider.clone())
            .build()
            .map_err(|error| TlsError::General(error.to_string()))?;
        Ok(Self {
            ca,
            system,
            provider: provider.clone(),
        })
    }
}

impl ServerCertVerifier for CaBesideSystem {
    /// The CA's answer when it vouches for the chain; else the system's. When
    /// both refuse, the CA's reason wins unless it is only "unknown issuer"
    /// (the chain does not lead to the CA): a certificate the CA issued for
    /// another name or out of its dates is refused for that, as webpki
    /// refuses it with both roots in one store.
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, TlsError> {
        let by_ca =
            self.ca
                .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now);
        let Err(ca_error) = by_ca else {
            return by_ca;
        };
        self.system
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
            .map_err(|system_error| match ca_error {
                TlsError::InvalidCertificate(CertificateError::UnknownIssuer) => system_error,
                ca_error => ca_error,
            })
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

/// The platform verifier with an added CA: on Windows, Linux and macOS the
/// CA joins the system's roots in the platform verifier itself.
#[cfg(not(target_os = "android"))]
fn platform_with_ca(
    ca: CertificateDer<'static>,
    provider: &Arc<CryptoProvider>,
) -> Result<Arc<dyn ServerCertVerifier>, TlsError> {
    Ok(Arc::new(
        rustls_platform_verifier::Verifier::new_with_extra_roots([ca], provider.clone())?,
    ))
}

/// The platform verifier with an added CA: on Android the platform verifier
/// takes no extra roots, so the CA is checked beside it ([`CaBesideSystem`]).
#[cfg(target_os = "android")]
fn platform_with_ca(
    ca: CertificateDer<'static>,
    provider: &Arc<CryptoProvider>,
) -> Result<Arc<dyn ServerCertVerifier>, TlsError> {
    let system = Arc::new(rustls_platform_verifier::Verifier::new(provider.clone())?);
    Ok(Arc::new(CaBesideSystem::new(ca, system, provider)?))
}

/// The chain verifier of `roots`, plus `extra` (an added CA) when given.
fn chain_verifier(
    roots: &SystemRoots,
    extra: Option<&[u8]>,
    provider: &Arc<CryptoProvider>,
) -> Result<Arc<dyn ServerCertVerifier>, TlsError> {
    let extra = extra.map(|der| CertificateDer::from(der.to_vec()));
    Ok(match roots {
        SystemRoots::Platform => match extra {
            Some(ca) => platform_with_ca(ca, provider)?,
            None => Arc::new(rustls_platform_verifier::Verifier::new(provider.clone())?),
        },
        SystemRoots::Fixed(list) => {
            let mut store = RootCertStore::empty();
            for cert in list.iter().cloned().chain(extra) {
                store.add(cert)?;
            }
            if store.is_empty() {
                return Ok(Arc::new(NoRoots(provider.clone())));
            }
            WebPkiServerVerifier::builder_with_provider(Arc::new(store), provider.clone())
                .build()
                .map_err(|error| TlsError::General(error.to_string()))?
        }
    })
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
    pub fn new(
        mode: TrustMode,
        roots: &SystemRoots,
        observed: Observation,
    ) -> Result<Self, TlsError> {
        let provider = provider();
        let extra = match &mode {
            TrustMode::Ca(der) => Some(der.as_slice()),
            _ => None,
        };
        let system = chain_verifier(roots, extra, &provider)?;
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
    /// An unknown issuer is offered for trust only when the certificate's
    /// name and dates hold (D187): webpki reports the issuer first, and a
    /// certificate for another name or out of its dates is refused, pinned
    /// or not.
    fn refused(
        &self,
        error: &TlsError,
        info: Option<CertInfo>,
        end_entity: &CertificateDer<'_>,
        server_name: &ServerName<'_>,
        now: UnixTime,
    ) {
        let problem = match (error, info) {
            (TlsError::InvalidCertificate(CertificateError::UnknownIssuer), Some(info)) => {
                match name_and_dates(end_entity, server_name, now) {
                    Ok(()) => TlsProblem::UnknownIssuer(info),
                    Err(refusal) => refusal.problem(info),
                }
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
        name_and_dates(end_entity, server_name, now).map_err(|refusal| {
            let error = refusal.error();
            if !matches!(refusal, Refusal::Encoding) {
                self.note(refusal.problem(info));
            }
            error
        })
    }
}

/// Why a certificate's own name or dates refuse it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Refusal {
    Encoding,
    WrongName,
    Dates,
}

impl Refusal {
    fn problem(self, info: CertInfo) -> TlsProblem {
        match self {
            Refusal::WrongName => TlsProblem::WrongName(info),
            Refusal::Dates => TlsProblem::Expired(info),
            Refusal::Encoding => TlsProblem::Other("the certificate does not parse".to_owned()),
        }
    }

    fn error(self) -> TlsError {
        TlsError::InvalidCertificate(match self {
            Refusal::Encoding => CertificateError::BadEncoding,
            Refusal::WrongName => CertificateError::NotValidForName,
            Refusal::Dates => CertificateError::Expired,
        })
    }
}

/// The host name and the validity dates of an end-entity certificate,
/// whatever its issuer.
fn name_and_dates(
    end_entity: &CertificateDer<'_>,
    server_name: &ServerName<'_>,
    now: UnixTime,
) -> Result<(), Refusal> {
    let cert = webpki::EndEntityCert::try_from(end_entity).map_err(|_| Refusal::Encoding)?;
    if cert.verify_is_valid_for_subject_name(server_name).is_err() {
        return Err(Refusal::WrongName);
    }
    let parsed =
        x509_cert::Certificate::from_der(end_entity.as_ref()).map_err(|_| Refusal::Encoding)?;
    let validity = &parsed.tbs_certificate.validity;
    let now = now.as_secs();
    if now < validity.not_before.to_unix_duration().as_secs()
        || now > validity.not_after.to_unix_duration().as_secs()
    {
        return Err(Refusal::Dates);
    }
    Ok(())
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
                .inspect_err(|error| self.refused(error, info, end_entity, server_name, now)),
            TrustMode::Pinned(pin) => {
                let presented = sha256(end_entity.as_ref());
                let Some(info) = info else {
                    return Err(TlsError::InvalidCertificate(CertificateError::BadEncoding));
                };
                if presented != *pin {
                    // A new certificate for another name or out of its
                    // dates is refused with that reason: never offered.
                    if let Err(refusal) = name_and_dates(end_entity, server_name, now) {
                        let error = refusal.error();
                        self.note(refusal.problem(info));
                        return Err(error);
                    }
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
    roots: &SystemRoots,
    observed: Observation,
) -> Result<rustls::ClientConfig, TlsError> {
    let verifier = Arc::new(TrustVerifier::new(mode, roots, observed)?);
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

    mod ca_beside_system {
        use rcgen::{BasicConstraints, CertificateParams, DnType, IsCa, Issuer, KeyPair};

        use super::*;

        fn ca(name: &str) -> (CertificateDer<'static>, Issuer<'static, KeyPair>) {
            let key = KeyPair::generate().expect("key");
            let mut params = CertificateParams::new(Vec::<String>::new()).expect("params");
            params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
            params.distinguished_name.push(DnType::CommonName, name);
            let cert = params.self_signed(&key).expect("ca");
            (cert.der().clone(), Issuer::new(params, key))
        }

        /// A certificate for `name` from `issuer`, valid from `from` to `to`
        /// days from now.
        fn leaf(
            name: &str,
            issuer: &Issuer<'static, KeyPair>,
            (from, to): (i64, i64),
        ) -> CertificateDer<'static> {
            let key = KeyPair::generate().expect("key");
            let mut params = CertificateParams::new(vec![name.to_owned()]).expect("params");
            params.distinguished_name.push(DnType::CommonName, name);
            let now = time::OffsetDateTime::now_utc();
            params.not_before = now + time::Duration::days(from);
            params.not_after = now + time::Duration::days(to);
            params
                .signed_by(&key, issuer)
                .expect("signed")
                .der()
                .clone()
        }

        /// The CA beside a "system" whose only root is `public` (webpki, as
        /// the platform verifier stands in no test).
        fn verifier(
            added: &CertificateDer<'static>,
            public: &CertificateDer<'static>,
        ) -> CaBesideSystem {
            let provider = provider();
            let system = chain_verifier(
                &SystemRoots::Fixed(Arc::new(vec![public.clone()])),
                None,
                &provider,
            )
            .expect("system");
            CaBesideSystem::new(added.clone(), system, &provider).expect("verifier")
        }

        fn verify(verifier: &CaBesideSystem, cert: &CertificateDer<'_>) -> Result<(), TlsError> {
            let name = ServerName::try_from("sync.example.test").expect("name");
            verifier
                .verify_server_cert(cert, &[], &name, &[], UnixTime::now())
                .map(|_| ())
        }

        fn refused(error: CertificateError) -> Result<(), TlsError> {
            Err(TlsError::InvalidCertificate(error))
        }

        /// The Android way of an added CA (D187, run 13): a certificate of
        /// the added CA and one the system trusts both pass; one of neither
        /// is an unknown issuer (the trust dialog's case); one the added CA
        /// issued for another name or out of its dates is refused for that
        /// reason, as with both roots in one store.
        #[test]
        fn an_added_ca_counts_beside_the_systems_trust() {
            let (added, added_issuer) = ca("added CA");
            let (public, public_issuer) = ca("public CA");
            let (_, stranger) = ca("unknown CA");
            let verifier = verifier(&added, &public);
            let name = "sync.example.test";
            assert_eq!(
                verify(&verifier, &leaf(name, &added_issuer, (-1, 30))),
                Ok(())
            );
            assert_eq!(
                verify(&verifier, &leaf(name, &public_issuer, (-1, 30))),
                Ok(())
            );
            assert_eq!(
                verify(&verifier, &leaf(name, &stranger, (-1, 30))),
                refused(CertificateError::UnknownIssuer)
            );
            let expired = verify(&verifier, &leaf(name, &added_issuer, (-30, -1)));
            assert!(
                matches!(
                    expired,
                    Err(TlsError::InvalidCertificate(
                        CertificateError::Expired | CertificateError::ExpiredContext { .. }
                    ))
                ),
                "{expired:?}"
            );
            let other_name = verify(
                &verifier,
                &leaf("other.example.test", &added_issuer, (-1, 30)),
            );
            assert!(
                matches!(
                    other_name,
                    Err(TlsError::InvalidCertificate(
                        CertificateError::NotValidForName
                            | CertificateError::NotValidForNameContext { .. }
                    ))
                ),
                "{other_name:?}"
            );
            // A certificate of the system's CA out of its dates: the system's
            // reason, not the added CA's "unknown issuer".
            let public_expired = verify(&verifier, &leaf(name, &public_issuer, (-30, -1)));
            assert!(
                matches!(
                    public_expired,
                    Err(TlsError::InvalidCertificate(
                        CertificateError::Expired | CertificateError::ExpiredContext { .. }
                    ))
                ),
                "{public_expired:?}"
            );
        }
    }
}
