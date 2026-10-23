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
    Assertion, Attested, Authenticator, CreateRequest, SignRequest, SkError,
    USER_PRESENCE_REQUIRED, USER_VERIFICATION_REQUIRED, key_file, sign_blocking,
};
use support::{TestInteraction, key_user, secret, spec, start_server};

/// What the fake authenticator does.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Behavior {
    Sign,
    Cancel,
    /// No device (or a PIN key): not ready, never asked.
    NotReady,
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
    fn ready(&self, _flags: u8) -> bool {
        self.behavior != Behavior::NotReady
    }

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

    /// The credential this fake holds, as an authenticator reports a new
    /// one: authenticator data with the AT flag, a zero AAGUID, the
    /// credential id and the COSE key (CBOR).
    fn create(&self, request: &CreateRequest) -> Result<Attested, SkError> {
        let mut cose = Vec::new();
        match self.inner.public_key().key_data() {
            public_key::KeyData::Ed25519(public) => {
                // {1: 1 (OKP), 3: -8 (EdDSA), -1: 6 (Ed25519), -2: x}
                cose.extend([0xa4, 0x01, 0x01, 0x03, 0x27, 0x20, 0x06, 0x21, 0x58, 0x20]);
                cose.extend_from_slice(public.as_ref());
            }
            public_key::KeyData::Ecdsa(EcdsaPublicKey::NistP256(point)) => {
                // {1: 2 (EC2), 3: -7 (ES256), -1: 1 (P-256), -2: x, -3: y}
                let bytes = point.as_bytes();
                cose.extend([0xa5, 0x01, 0x02, 0x03, 0x26, 0x20, 0x01, 0x21, 0x58, 0x20]);
                cose.extend_from_slice(&bytes[1..33]);
                cose.extend([0x22, 0x58, 0x20]);
                cose.extend_from_slice(&bytes[33..65]);
            }
            _ => unreachable!(),
        }
        let mut auth_data = Sha256::digest(request.application.as_bytes()).to_vec();
        auth_data.push(0x41);
        auth_data.extend_from_slice(&0u32.to_be_bytes());
        auth_data.extend_from_slice(&[0; 16]);
        auth_data.extend_from_slice(&(self.key_handle.len() as u16).to_be_bytes());
        auth_data.extend_from_slice(&self.key_handle);
        auth_data.extend(cose);
        Ok(Attested {
            key_handle: self.key_handle.clone(),
            auth_data,
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

/// A key made on the authenticator (D143): its file holds the credential's
/// public key from the COSE key, the application, the flags and the key
/// handle, and signs in.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_made_on_the_authenticator_signs_in() {
    for algorithm in [ED25519, P256] {
        let (reference, fake) = security_key(algorithm, Behavior::Sign);
        let request = CreateRequest {
            application: "ssh:".to_owned(),
            user_verification: false,
        };
        let attested = fake.create(&request).unwrap();
        let made = key_file(&request, &attested, "made here").unwrap();
        assert_eq!(
            made.public_key().key_data(),
            reference.public_key().key_data()
        );
        assert_eq!(made.comment().as_str_lossy(), "made here");
        // The OpenSSH text round-trips and signs.
        let text = made.to_openssh(LineEnding::LF).unwrap();
        let reread = PrivateKey::from_openssh(text.as_str()).unwrap();
        sign_blocking(fake.as_ref(), &reread, b"data").unwrap();
        let server = start_server(vec![key_user(
            "gina",
            &made.public_key().to_openssh().unwrap(),
        )])
        .await;
        let connection = connect(
            spec(server.addr(), Some("gina"), plan(&made, Some(fake.clone()))),
            TestInteraction::accepting().as_dyn(),
        )
        .await
        .expect("sign-in with the made key");
        connection.disconnect().await;
    }
    // A PIN key gets the user verification flag.
    let (_, fake) = security_key(ED25519, Behavior::Sign);
    let request = CreateRequest {
        application: "ssh:".to_owned(),
        user_verification: true,
    };
    let made = key_file(&request, &fake.create(&request).unwrap(), "pin").unwrap();
    let KeypairData::SkEd25519(sk) = made.key_data() else {
        panic!("not an sk key");
    };
    assert_eq!(
        sk.flags(),
        USER_PRESENCE_REQUIRED | USER_VERIFICATION_REQUIRED
    );
    // Authenticator data without the credential, or a broken COSE key.
    let mut attested = fake.create(&request).unwrap();
    attested.auth_data[32] = 0x01;
    assert_eq!(
        key_file(&request, &attested, "x").unwrap_err(),
        SkError::Invalid
    );
    let mut attested = fake.create(&request).unwrap();
    attested.auth_data.truncate(attested.auth_data.len() - 5);
    assert_eq!(
        key_file(&request, &attested, "x").unwrap_err(),
        SkError::Invalid
    );
}

/// An authenticator that is not ready (no device, a PIN key) is never
/// asked: the key is skipped before it is offered and the password signs
/// in (D143).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_key_the_authenticator_cannot_sign_now_is_skipped() {
    let (key, absent) = security_key(ED25519, Behavior::NotReady);
    let password = support::random_secret("pw");
    let public = key.public_key().to_openssh().unwrap();
    let server = start_server(vec![
        key_user("ivan", &public).with_password(secret(&password)),
    ])
    .await;
    let mut auth = plan(&key, Some(absent.clone()));
    auth.password = Some(secret(&password));
    let connection = connect(
        spec(server.addr(), Some("ivan"), auth),
        TestInteraction::accepting().as_dyn(),
    )
    .await
    .expect("password after the skipped security key");
    assert_eq!(connection.auth().method, AuthMethod::Password);
    assert_eq!(absent.signed.load(Ordering::SeqCst), 0);
    connection.disconnect().await;
}
