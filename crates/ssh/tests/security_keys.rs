//! FIDO2 security keys (D143) over a fake authenticator: a software key
//! that signs exactly what a real authenticator signs (`SHA-256(application)
//! || flags || counter || SHA-256(data)`), with Ed25519 and with ECDSA P-256
//! (a DER signature, as CTAP2 and WebAuthn return it). The SSH signature is
//! checked by ssh-key's verifier and by the test server's sign-in; a wrong
//! credential, a cancelled touch and a missing authenticator are refused.
//! Every key is generated at run time.

mod support;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};

use russh::keys::signature::Signer as _;
use russh::keys::ssh_key::private::{self, KeypairData};
use russh::keys::ssh_key::public::{self as public_key, EcdsaPublicKey};
use russh::keys::ssh_key::{Algorithm, EcdsaCurve, LineEnding, PrivateKey};
use sha2::{Digest as _, Sha256};
use shellpp_ssh::client::{AuthMethod, AuthPlan, KeyCredential, connect};
use shellpp_ssh::sk::{
    Assertion, Authenticator, SignRequest, SkError, USER_PRESENCE_REQUIRED, sign_blocking,
};
use support::{TestInteraction, key_user, secret, spec, start_server};

/// What the fake authenticator does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Behavior {
    Sign,
    Cancel,
}

/// A software authenticator holding one credential.
struct Fake {
    inner: PrivateKey,
    key_handle: Vec<u8>,
    behavior: Behavior,
    counter: AtomicU32,
    signed: AtomicUsize,
}

/// `mpint r, mpint s` of an SSH ECDSA signature as DER.
fn ssh_ecdsa_to_der(mut blob: &[u8]) -> Vec<u8> {
    let mut integers = Vec::new();
    for _ in 0..2 {
        let len = u32::from_be_bytes(blob[..4].try_into().unwrap()) as usize;
        let value = &blob[4..4 + len];
        blob = &blob[4 + len..];
        integers.push(0x02);
        integers.push(len as u8);
        integers.extend_from_slice(value);
    }
    let mut der = vec![0x30, integers.len() as u8];
    der.extend(integers);
    der
}

impl Authenticator for Fake {
    fn sign(&self, request: &SignRequest) -> Result<Assertion, SkError> {
        if self.behavior == Behavior::Cancel {
            return Err(SkError::Cancelled);
        }
        if request.key_handle != self.key_handle {
            return Err(SkError::UnknownCredential);
        }
        let flags = USER_PRESENCE_REQUIRED;
        let counter = self.counter.fetch_add(1, Ordering::SeqCst) + 1;
        let mut signed = Sha256::digest(request.application.as_bytes()).to_vec();
        signed.push(flags);
        signed.extend_from_slice(&counter.to_be_bytes());
        signed.extend_from_slice(&Sha256::digest(&request.data));
        let signature = self.inner.key_data().try_sign(&signed).unwrap();
        let signature = match self.inner.algorithm() {
            Algorithm::Ed25519 => signature.as_bytes().to_vec(),
            _ => ssh_ecdsa_to_der(signature.as_bytes()),
        };
        self.signed.fetch_add(1, Ordering::SeqCst);
        Ok(Assertion {
            flags,
            counter,
            signature,
        })
    }
}

/// A security key over a fresh software key: (its private key file, the
/// fake authenticator that holds it).
fn security_key(algorithm: Algorithm, behavior: Behavior) -> (PrivateKey, Arc<Fake>) {
    let inner = PrivateKey::random(&mut rand::rng(), algorithm).unwrap();
    let key_handle: Vec<u8> = (0..48).map(|_| rand::random::<u8>()).collect();
    let data = match inner.public_key().key_data() {
        public_key::KeyData::Ed25519(public) => KeypairData::SkEd25519(
            private::SkEd25519::new(
                public_key::SkEd25519::new(*public, "ssh:"),
                USER_PRESENCE_REQUIRED,
                key_handle.clone(),
            )
            .unwrap(),
        ),
        public_key::KeyData::Ecdsa(EcdsaPublicKey::NistP256(point)) => {
            KeypairData::SkEcdsaSha2NistP256(
                private::SkEcdsaSha2NistP256::new(
                    public_key::SkEcdsaSha2NistP256::new(*point, "ssh:"),
                    USER_PRESENCE_REQUIRED,
                    key_handle.clone(),
                )
                .unwrap(),
            )
        }
        _ => unreachable!(),
    };
    let key = PrivateKey::new(data, "security key").unwrap();
    let fake = Arc::new(Fake {
        inner,
        key_handle,
        behavior,
        counter: AtomicU32::new(0),
        signed: AtomicUsize::new(0),
    });
    (key, fake)
}

const ED25519: Algorithm = Algorithm::Ed25519;
const P256: Algorithm = Algorithm::Ecdsa {
    curve: EcdsaCurve::NistP256,
};

#[test]
fn signatures_of_both_kinds_verify() {
    for algorithm in [ED25519, P256] {
        let (key, fake) = security_key(algorithm.clone(), Behavior::Sign);
        // sign_blocking checks the signature with the key's public key.
        let signature = sign_blocking(fake.as_ref(), &key, b"session data").unwrap();
        assert_eq!(signature.algorithm(), key.algorithm());
        assert_eq!(fake.signed.load(Ordering::SeqCst), 1);
        // Another key's authenticator: refused, never sent.
        let (_, other) = security_key(algorithm, Behavior::Sign);
        assert_eq!(
            sign_blocking(other.as_ref(), &key, b"session data").unwrap_err(),
            SkError::UnknownCredential
        );
    }
}

#[test]
fn a_wrong_signature_or_a_cancelled_touch_is_refused() {
    let (key, _) = security_key(ED25519, Behavior::Sign);
    // An authenticator that answers with another key's signature for the
    // right credential.
    struct Liar(Arc<Fake>, Vec<u8>);
    impl Authenticator for Liar {
        fn sign(&self, request: &SignRequest) -> Result<Assertion, SkError> {
            let mut request = request.clone();
            request.key_handle = self.1.clone();
            self.0.sign(&request)
        }
    }
    let (_, other) = security_key(ED25519, Behavior::Sign);
    let liar = Liar(other.clone(), other.key_handle.clone());
    assert_eq!(
        sign_blocking(&liar, &key, b"data").unwrap_err(),
        SkError::Invalid
    );
    let (key, cancel) = security_key(P256, Behavior::Cancel);
    assert_eq!(
        sign_blocking(cancel.as_ref(), &key, b"data").unwrap_err(),
        SkError::Cancelled
    );
}

fn plan(key: &PrivateKey, authenticator: Option<Arc<Fake>>) -> AuthPlan {
    AuthPlan {
        key: Some(KeyCredential {
            label: "Security key".to_owned(),
            private_key: secret(key.to_openssh(LineEnding::LF).unwrap().as_str()),
            passphrase: None,
        }),
        authenticator: authenticator.map(|fake| fake as Arc<dyn Authenticator>),
        ..AuthPlan::default()
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_security_key_signs_in() {
    for algorithm in [ED25519, P256] {
        let (key, fake) = security_key(algorithm, Behavior::Sign);
        let public = key.public_key().to_openssh().unwrap();
        let server = start_server(vec![key_user("erin", &public)]).await;
        let connection = connect(
            spec(server.addr(), Some("erin"), plan(&key, Some(fake.clone()))),
            TestInteraction::accepting().as_dyn(),
        )
        .await
        .expect("security key sign-in");
        assert_eq!(connection.auth().method, AuthMethod::PublicKey);
        assert_eq!(fake.signed.load(Ordering::SeqCst), 1);
        connection.disconnect().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn without_an_authenticator_or_a_touch_it_does_not_sign_in() {
    let (key, cancel) = security_key(ED25519, Behavior::Cancel);
    let public = key.public_key().to_openssh().unwrap();
    let server = start_server(vec![key_user("frank", &public)]).await;
    // No authenticator: the key is unusable and nothing else is offered.
    assert!(
        connect(
            spec(server.addr(), Some("frank"), plan(&key, None)),
            TestInteraction::accepting().as_dyn(),
        )
        .await
        .is_err()
    );
    // A cancelled touch ends the attempt.
    assert!(
        connect(
            spec(server.addr(), Some("frank"), plan(&key, Some(cancel))),
            TestInteraction::accepting().as_dyn(),
        )
        .await
        .is_err()
    );
}
