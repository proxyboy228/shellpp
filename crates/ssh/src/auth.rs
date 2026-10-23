//! Authentication (stage 2 plan, decision 23; docs/data-model.md, section
//! 8.6; night decision D36): a missing username is asked first, then, while
//! the server offers the method: the effective key (decrypted with its saved
//! passphrase or a `Passphrase` prompt), the saved password, the system
//! agent's keys, the user's key files (offered by their public key;
//! decrypted, with a `FileKeyPassphrase` prompt when needed, only after the
//! server accepted the key), the password when none is saved (prompted),
//! keyboard-interactive (one prompt per question). A refused saved password
//! is asked again at once.
//!
//! At most [`MAX_PUBLIC_KEY_OFFERS`] public keys are offered in all: servers
//! close the connection after `MaxAuthTries` failures (6 by default in
//! OpenSSH), so the password and keyboard-interactive keep their turn. A cancelled prompt ends the attempt with
//! `Cancelled`. A public key already offered (the same key in the agent and
//! in a file) is not offered again.
//!
//! Partial success (several required methods) is followed: after a method
//! succeeds partially, the remaining methods that were not tried yet run.

use std::sync::Arc;
use std::time::Duration;

use russh::client::{AuthResult, Handle, KeyboardInteractiveAuthResponse};
use russh::keys::agent::AgentIdentity;
use russh::keys::ssh_encoding::Encode as _;
use russh::keys::ssh_key::private::KeypairData;
use russh::keys::ssh_key::{Algorithm, HashAlg, Kdf, PrivateKey, PublicKey, Signature};
use russh::keys::{PrivateKeyWithHashAlg, decode_secret_key};
use russh::{MethodKind, MethodSet};
use secrecy::{ExposeSecret, SecretString};

use crate::agent::SystemAgent;
use crate::client::{
    AuthMethod, AuthOutcome, AuthPlan, FileKey, Interaction, KeyCredential, Prompt, PromptAnswer,
    PromptedSecrets,
};
use crate::error::FailureKind;
use crate::handler::ClientHandler;

/// Most public keys offered in one authentication: the effective key, the
/// agent's keys and the key files together (D36). Each refused key counts
/// against the server's `MaxAuthTries` (OpenSSH: 6); four leave room for
/// the password and keyboard-interactive.
pub(crate) const MAX_PUBLIC_KEY_OFFERS: usize = 4;
/// Time allowed for an agent to sign (it may ask its user for confirmation).
const AGENT_SIGN_TIMEOUT: Duration = Duration::from_secs(120);
/// Time allowed for the server's answer to a password or to
/// keyboard-interactive answers. Servers with push or out-of-band second
/// factors (a PAM module waiting for an approval on a phone) hold this
/// answer until the user approves, typically up to a minute; the user can
/// cancel the attempt at any time, and a lost transport fails it at once.
pub(crate) const AUTH_REPLY_TIMEOUT: Duration = Duration::from_secs(300);
/// Most questions accepted in one keyboard-interactive round; a server that
/// sends more is treated as broken (`Protocol`).
pub(crate) const MAX_KI_QUESTIONS: usize = 32;
/// Longest server texts of a keyboard-interactive round passed on, in characters.
pub(crate) const MAX_KI_NAME_CHARS: usize = 256;
pub(crate) const MAX_KI_PROMPT_CHARS: usize = 256;
pub(crate) const MAX_KI_INSTRUCTION_CHARS: usize = 2048;
/// Bound on keyboard-interactive rounds that need no answer (servers may
/// send empty information requests; never loop on them forever).
const MAX_EMPTY_KI_ROUNDS: u32 = 8;

/// Why authentication did not succeed.
#[derive(Debug)]
pub(crate) enum AuthError {
    /// The transport closed before the server answered anything (the server
    /// may have given up while the user decided about the host key).
    LostBeforeFirstReply,
    Failed(FailureKind),
}

impl From<FailureKind> for AuthError {
    fn from(kind: FailureKind) -> Self {
        Self::Failed(kind)
    }
}

/// The inputs of one authentication.
pub(crate) struct AuthContext<'a> {
    pub(crate) plan: &'a AuthPlan,
    pub(crate) agent: Option<&'a SystemAgent>,
    pub(crate) interaction: &'a dyn Interaction,
    /// Server round-trip timeout.
    pub(crate) timeout: Duration,
    /// The host allows legacy algorithms (D80): an RSA key may sign with
    /// SHA-1 (`ssh-rsa`) for a server that accepts nothing else.
    pub(crate) legacy: bool,
}

/// A successful authentication with the secrets typed at prompts.
pub(crate) struct Authenticated {
    pub(crate) outcome: AuthOutcome,
    pub(crate) prompted: PromptedSecrets,
}

/// What one method did.
enum Step {
    Success(Authenticated),
    /// The method is done (refused, unusable or not applicable); continue.
    Next,
}

/// Progress over the methods of one authentication.
struct Progress {
    username: String,
    methods: MethodSet,
    tried_key: bool,
    tried_agent: bool,
    tried_file_keys: bool,
    tried_password: bool,
    tried_keyboard_interactive: bool,
    /// The effective key could not be decoded.
    key_unusable: bool,
    /// The server refused at least one credential.
    refused_any: bool,
    /// Public keys already offered (the effective key, agent keys, key files).
    offered: Vec<PublicKey>,
}

impl Progress {
    fn offers(&self, kind: MethodKind) -> bool {
        self.methods.contains(&kind)
    }

    fn was_offered(&self, key: &PublicKey) -> bool {
        self.offered
            .iter()
            .any(|offered| offered.key_data() == key.key_data())
    }

    /// The server answered a failure. Returns the error to stop with when the
    /// transport is gone.
    fn refused(
        &mut self,
        handle: &Handle<ClientHandler>,
        remaining: MethodSet,
    ) -> Result<(), AuthError> {
        if remaining.is_empty() && handle.is_closed() {
            return Err(self.closed());
        }
        self.refused_any = true;
        self.methods = remaining;
        Ok(())
    }

    /// The error when the transport closed during authentication: after a
    /// refusal the server most likely gave up on wrong credentials.
    fn closed(&self) -> AuthError {
        if self.refused_any {
            AuthError::Failed(FailureKind::AuthFailed)
        } else {
            AuthError::Failed(FailureKind::Disconnected)
        }
    }

    fn outcome(&self, method: AuthMethod) -> Authenticated {
        Authenticated {
            outcome: AuthOutcome {
                username: self.username.clone(),
                method,
            },
            prompted: PromptedSecrets::default(),
        }
    }
}

async fn with_timeout<T, F>(timeout: Duration, future: F) -> Result<T, AuthError>
where
    F: std::future::Future<Output = T>,
{
    tokio::time::timeout(timeout, future)
        .await
        .map_err(|_| AuthError::Failed(FailureKind::Timeout))
}

/// Authenticates on an established transport. `username` is the spec's
/// username or the one prompted in an earlier attempt; a prompted one is
/// stored back so that a retry does not ask again.
pub(crate) async fn authenticate(
    handle: &mut Handle<ClientHandler>,
    context: &AuthContext<'_>,
    username: &mut Option<String>,
) -> Result<Authenticated, AuthError> {
    let user = match username.clone() {
        Some(user) => user,
        None => {
            let user = ask_username(context.interaction).await?;
            *username = Some(user.clone());
            user
        }
    };

    // `none` reveals the methods the server offers.
    let first = with_timeout(context.timeout, handle.authenticate_none(user.clone()))
        .await?
        .map_err(|_| {
            if handle.is_closed() {
                AuthError::LostBeforeFirstReply
            } else {
                AuthError::Failed(FailureKind::Protocol)
            }
        })?;
    let methods = match first {
        AuthResult::Success => {
            tracing::debug!(method = "none", "authenticated");
            return Ok(Authenticated {
                outcome: AuthOutcome {
                    username: user,
                    method: AuthMethod::None,
                },
                prompted: PromptedSecrets::default(),
            });
        }
        AuthResult::Failure {
            remaining_methods, ..
        } => {
            if remaining_methods.is_empty() && handle.is_closed() {
                return Err(AuthError::LostBeforeFirstReply);
            }
            remaining_methods
        }
    };

    let mut progress = Progress {
        username: user,
        methods,
        tried_key: false,
        tried_agent: false,
        tried_file_keys: false,
        tried_password: false,
        tried_keyboard_interactive: false,
        key_unusable: false,
        refused_any: false,
        offered: Vec::new(),
    };

    loop {
        let step = if let Some(key) = context
            .plan
            .key
            .as_ref()
            .filter(|_| !progress.tried_key && progress.offers(MethodKind::PublicKey))
        {
            progress.tried_key = true;
            try_key(handle, context, &mut progress, key).await?
        } else if context.plan.password.is_some()
            && !progress.tried_password
            && progress.offers(MethodKind::Password)
        {
            // A saved password comes before the agent and the key files (D36).
            progress.tried_password = true;
            try_password(handle, context, &mut progress).await?
        } else if let Some(agent) = context.agent.filter(|_| {
            context.plan.use_agent
                && !progress.tried_agent
                && progress.offers(MethodKind::PublicKey)
        }) {
            progress.tried_agent = true;
            try_agent(handle, context, &mut progress, agent).await?
        } else if !progress.tried_file_keys
            && !context.plan.file_keys.is_empty()
            && progress.offers(MethodKind::PublicKey)
        {
            progress.tried_file_keys = true;
            try_file_keys(handle, context, &mut progress).await?
        } else if !progress.tried_password && progress.offers(MethodKind::Password) {
            progress.tried_password = true;
            try_password(handle, context, &mut progress).await?
        } else if context.plan.keyboard_interactive
            && !progress.tried_keyboard_interactive
            && progress.offers(MethodKind::KeyboardInteractive)
        {
            progress.tried_keyboard_interactive = true;
            try_keyboard_interactive(handle, context, &mut progress).await?
        } else {
            break;
        };
        if let Step::Success(authenticated) = step {
            tracing::debug!(method = ?authenticated.outcome.method, "authenticated");
            return Ok(authenticated);
        }
    }

    if progress.key_unusable {
        Err(AuthError::Failed(FailureKind::KeyUnusable))
    } else {
        Err(AuthError::Failed(FailureKind::AuthFailed))
    }
}

async fn ask_username(interaction: &dyn Interaction) -> Result<String, AuthError> {
    loop {
        let answer = interaction
            .prompt(Prompt::Username)
            .await
            .ok_or(AuthError::Failed(FailureKind::Cancelled))?;
        let user = answer.value.expose_secret().trim().to_owned();
        if !user.is_empty() {
            return Ok(user);
        }
    }
}

/// The RSA signature hash for this server: SHA-512 or SHA-256 from
/// `server-sig-algs`, SHA-256 when the server sent no list. A server that
/// accepts only `ssh-rsa` (SHA-1) gets SHA-1 (`Ok(None)`) only when the host
/// allows legacy algorithms (D80; never otherwise, decision 22), as does a
/// server without a list then (OpenSSH before 7.2 knows no `rsa-sha2-*`).
async fn rsa_hash(
    handle: &Handle<ClientHandler>,
    timeout: Duration,
    legacy: bool,
) -> Result<Option<HashAlg>, ()> {
    match tokio::time::timeout(timeout, handle.best_supported_rsa_hash()).await {
        Ok(Ok(Some(Some(hash)))) => Ok(Some(hash)),
        Ok(Ok(Some(None))) if legacy => Ok(None),
        Ok(Ok(Some(None))) => Err(()),
        _ if legacy => Ok(None),
        _ => Ok(Some(HashAlg::Sha256)),
    }
}

fn is_rsa(algorithm: &Algorithm) -> bool {
    matches!(algorithm, Algorithm::Rsa { .. })
}

/// The signature hash for `algorithm`; `Err(())` when the key cannot be used
/// without SHA-1.
async fn hash_for(
    handle: &Handle<ClientHandler>,
    algorithm: &Algorithm,
    context: &AuthContext<'_>,
) -> Result<Option<HashAlg>, ()> {
    if !is_rsa(algorithm) {
        return Ok(None);
    }
    rsa_hash(handle, context.timeout, context.legacy).await
}

async fn try_key(
    handle: &mut Handle<ClientHandler>,
    context: &AuthContext<'_>,
    progress: &mut Progress,
    credential: &KeyCredential,
) -> Result<Step, AuthError> {
    let (key, prompted_passphrase) = match open_key(
        credential,
        context.plan.can_save_passphrase,
        context.interaction,
    )
    .await?
    {
        Some(opened) => opened,
        None => {
            tracing::debug!("effective key unusable");
            progress.key_unusable = true;
            return Ok(Step::Next);
        }
    };
    // A security key (FIDO2) signs on its authenticator (D143): without one
    // it is unusable here and the next method runs.
    if matches!(
        key.algorithm(),
        Algorithm::SkEd25519 | Algorithm::SkEcdsaSha2NistP256
    ) {
        let Some(authenticator) = context.plan.authenticator.clone() else {
            tracing::debug!("security key skipped: no authenticator");
            progress.key_unusable = true;
            return Ok(Step::Next);
        };
        return try_security_key(handle, context, progress, key, authenticator).await;
    }
    progress.offered.push(key.public_key().clone());
    let Ok(hash) = hash_for(handle, &key.algorithm(), context).await else {
        tracing::debug!("rsa key skipped: the server accepts only sha-1 signatures");
        return Ok(Step::Next);
    };
    let key = PrivateKeyWithHashAlg::new(Arc::new(key), hash);
    let result = with_timeout(
        context.timeout,
        handle.authenticate_publickey(progress.username.clone(), key),
    )
    .await?
    .map_err(|_| progress.closed())?;
    match result {
        AuthResult::Success => {
            let mut authenticated = progress.outcome(AuthMethod::PublicKey);
            authenticated.prompted.passphrase = prompted_passphrase;
            Ok(Step::Success(authenticated))
        }
        AuthResult::Failure {
            remaining_methods, ..
        } => {
            tracing::debug!(method = "publickey", "refused");
            progress.refused(handle, remaining_methods)?;
            Ok(Step::Next)
        }
    }
}

/// Offers a security key and signs with its authenticator once the server
/// accepted it (the user touches the key then).
async fn try_security_key(
    handle: &mut Handle<ClientHandler>,
    context: &AuthContext<'_>,
    progress: &mut Progress,
    key: PrivateKey,
    authenticator: Arc<dyn crate::sk::Authenticator>,
) -> Result<Step, AuthError> {
    use crate::sk::{SkError, SkSignError, SkSigner};
    let public = key.public_key().clone();
    progress.offered.push(public.clone());
    let mut signer = SkSigner {
        key: Arc::new(key),
        authenticator,
    };
    // The touch (or PIN) waits for the user: the long limit of answers.
    let result = tokio::time::timeout(
        context.timeout.max(AUTH_REPLY_TIMEOUT),
        handle.authenticate_publickey_with(progress.username.clone(), public, None, &mut signer),
    )
    .await
    .map_err(|_| AuthError::Failed(FailureKind::Timeout))?;
    match result {
        Ok(AuthResult::Success) => Ok(Step::Success(progress.outcome(AuthMethod::PublicKey))),
        Ok(AuthResult::Failure {
            remaining_methods, ..
        }) => {
            tracing::debug!(method = "publickey", "security key refused");
            progress.refused(handle, remaining_methods)?;
            Ok(Step::Next)
        }
        Err(SkSignError::Send) => Err(progress.closed()),
        Err(SkSignError::Sk(SkError::Cancelled)) => Err(AuthError::Failed(FailureKind::Cancelled)),
        // The server waits for a signature that cannot be made: this
        // attempt cannot go on.
        Err(SkSignError::Sk(error)) => {
            tracing::debug!(%error, "security key did not sign");
            Err(AuthError::Failed(FailureKind::KeyUnusable))
        }
    }
}

/// Why a key text did not open.
enum KeyDecode {
    Encrypted,
    Invalid,
}

/// Most bcrypt-pbkdf rounds of an encrypted OpenSSH key that are run (the
/// same bound as the core's key import). The rounds come from the key file
/// and decryption cannot be interrupted: a crafted key could tie up a
/// thread for days, so such a key is unusable instead.
pub(crate) const MAX_KDF_ROUNDS: u32 = 1024;

/// `false` for an OpenSSH key whose bcrypt-pbkdf rounds exceed [`MAX_KDF_ROUNDS`].
fn kdf_allowed(text: &str) -> bool {
    match PrivateKey::from_openssh(text) {
        Ok(key) => match key.kdf() {
            Kdf::Bcrypt { rounds, .. } => *rounds <= MAX_KDF_ROUNDS,
            _ => true,
        },
        Err(_) => true,
    }
}

fn decode(text: &SecretString, passphrase: Option<&SecretString>) -> Result<PrivateKey, KeyDecode> {
    let text = text.expose_secret();
    if passphrase.is_some() && !kdf_allowed(text) {
        tracing::debug!("key refused: too many kdf rounds");
        return Err(KeyDecode::Invalid);
    }
    match decode_secret_key(text, passphrase.map(|p| p.expose_secret())) {
        Ok(key) => Ok(key),
        Err(russh::keys::Error::KeyIsEncrypted) => Err(KeyDecode::Encrypted),
        Err(_) if passphrase.is_none() && looks_encrypted(text) => Err(KeyDecode::Encrypted),
        Err(_) if passphrase.is_some() => Err(KeyDecode::Encrypted),
        Err(_) => Err(KeyDecode::Invalid),
    }
}

/// Encrypted PEM and PuTTY keys (OpenSSH keys report encryption themselves).
fn looks_encrypted(text: &str) -> bool {
    text.contains("-----BEGIN ENCRYPTED PRIVATE KEY-----")
        || text.contains("Proc-Type: 4,ENCRYPTED")
        || text
            .lines()
            .any(|line| line.starts_with("Encryption:") && !line.trim_end().ends_with("none"))
}

/// Decodes on a blocking thread: decryption runs the key's KDF.
async fn decode_blocking(
    text: &SecretString,
    passphrase: Option<&SecretString>,
) -> Result<PrivateKey, KeyDecode> {
    if !kdf_allowed(text.expose_secret()) {
        tracing::debug!("key refused: too many kdf rounds");
        return Err(KeyDecode::Invalid);
    }
    let text = text.clone();
    let passphrase = passphrase.cloned();
    tokio::task::spawn_blocking(move || decode(&text, passphrase.as_ref()))
        .await
        .unwrap_or(Err(KeyDecode::Invalid))
}

/// Opens the key: plain, with the saved passphrase, or with prompted ones
/// until one works or the prompt is cancelled. `None`: the key text cannot
/// be decoded at all.
async fn open_key(
    credential: &KeyCredential,
    can_save: bool,
    interaction: &dyn Interaction,
) -> Result<Option<(PrivateKey, Option<PromptAnswer>)>, AuthError> {
    match decode_blocking(&credential.private_key, None).await {
        Ok(key) => return Ok(Some((key, None))),
        Err(KeyDecode::Invalid) => return Ok(None),
        Err(KeyDecode::Encrypted) => {}
    }
    let mut error = false;
    if let Some(saved) = credential.passphrase.as_ref() {
        match decode_blocking(&credential.private_key, Some(saved)).await {
            Ok(key) => return Ok(Some((key, None))),
            Err(_) => error = true,
        }
    }
    loop {
        let answer = interaction
            .prompt(Prompt::Passphrase {
                key_label: credential.label.clone(),
                error,
                can_save,
            })
            .await
            .ok_or(AuthError::Failed(FailureKind::Cancelled))?;
        match decode_blocking(&credential.private_key, Some(&answer.value)).await {
            Ok(key) => return Ok(Some((key, Some(answer)))),
            Err(_) => error = true,
        }
    }
}

async fn try_agent(
    handle: &mut Handle<ClientHandler>,
    context: &AuthContext<'_>,
    progress: &mut Progress,
    agent: &SystemAgent,
) -> Result<Step, AuthError> {
    let identities = match tokio::time::timeout(context.timeout, agent.identities_raw()).await {
        Ok(Ok(identities)) => identities,
        _ => {
            tracing::debug!("system agent not reachable");
            return Ok(Step::Next);
        }
    };
    for identity in identities {
        // Certificates are stage 3.
        let AgentIdentity::PublicKey { key, .. } = identity else {
            continue;
        };
        if progress.was_offered(&key) {
            continue;
        }
        if !progress.offers(MethodKind::PublicKey) {
            break;
        }
        if progress.offered.len() >= MAX_PUBLIC_KEY_OFFERS {
            tracing::debug!("public key limit reached: agent keys skipped");
            break;
        }
        progress.offered.push(key.clone());
        let Ok(hash) = hash_for(handle, &key.algorithm(), context).await else {
            continue;
        };
        let Ok(mut client) = agent.client().await else {
            return Ok(Step::Next);
        };
        let result = tokio::time::timeout(
            AGENT_SIGN_TIMEOUT,
            handle.authenticate_publickey_with(progress.username.clone(), key, hash, &mut client),
        )
        .await
        .map_err(|_| AuthError::Failed(FailureKind::Timeout))?;
        match result {
            Ok(AuthResult::Success) => {
                return Ok(Step::Success(progress.outcome(AuthMethod::Agent)));
            }
            Ok(AuthResult::Failure {
                remaining_methods,
                partial_success,
            }) => {
                tracing::debug!(method = "agent", "refused");
                progress.refused(handle, remaining_methods)?;
                if partial_success {
                    return Ok(Step::Next);
                }
            }
            Err(russh::AgentAuthError::Send(_)) => return Err(progress.closed()),
            Err(russh::AgentAuthError::Key(_)) => {
                // The agent refused to sign (for example a declined
                // confirmation); try its next key.
                tracing::debug!("agent signature refused");
                if handle.is_closed() {
                    return Err(progress.closed());
                }
            }
        }
    }
    Ok(Step::Next)
}

/// The public key of a key file without decrypting it: from the OpenSSH
/// private key text (which carries it in the clear), else from the `.pub`
/// line, else from the decoded text of an unencrypted legacy key. `None`
/// for an encrypted legacy key without a `.pub` file (it is skipped).
async fn file_public_key(file: &FileKey) -> Option<PublicKey> {
    if let Ok(key) = PrivateKey::from_openssh(file.private_key.expose_secret()) {
        return Some(key.public_key().clone());
    }
    if let Some(key) = file
        .public_key
        .as_deref()
        .and_then(|line| PublicKey::from_openssh(line.trim()).ok())
    {
        return Some(key);
    }
    decode_blocking(&file.private_key, None)
        .await
        .ok()
        .map(|key| key.public_key().clone())
}

/// Why a key file did not sign after the server accepted its public key.
#[derive(Debug)]
enum FileSignError {
    /// The transport is gone.
    Send,
    /// The passphrase prompt was cancelled.
    Cancelled,
    /// The text could not be decoded, or it is not the key that was offered.
    Unusable,
}

impl From<russh::SendError> for FileSignError {
    fn from(_: russh::SendError) -> Self {
        Self::Send
    }
}

/// Signs for one key file once the server accepted its public key: opens the
/// key (plain, with the remembered passphrase, or with prompted ones until
/// one works or the prompt is cancelled) and signs.
struct FileKeySigner<'a> {
    file: &'a FileKey,
    public: &'a PublicKey,
    interaction: &'a dyn Interaction,
    /// The passphrase typed at the prompt that opened the key.
    prompted: Option<PromptAnswer>,
}

impl FileKeySigner<'_> {
    async fn open(&mut self) -> Result<PrivateKey, FileSignError> {
        match decode_blocking(&self.file.private_key, None).await {
            Ok(key) => return Ok(key),
            Err(KeyDecode::Invalid) => return Err(FileSignError::Unusable),
            Err(KeyDecode::Encrypted) => {}
        }
        let mut error = false;
        if let Some(remembered) = self.file.passphrase.as_ref() {
            match decode_blocking(&self.file.private_key, Some(remembered)).await {
                Ok(key) => return Ok(key),
                Err(KeyDecode::Invalid) => return Err(FileSignError::Unusable),
                Err(KeyDecode::Encrypted) => error = true,
            }
        }
        loop {
            let answer = self
                .interaction
                .prompt(Prompt::FileKeyPassphrase {
                    key_label: self.file.label.clone(),
                    error,
                })
                .await
                .ok_or(FileSignError::Cancelled)?;
            match decode_blocking(&self.file.private_key, Some(&answer.value)).await {
                Ok(key) => {
                    self.prompted = Some(answer);
                    return Ok(key);
                }
                Err(KeyDecode::Invalid) => return Err(FileSignError::Unusable),
                Err(KeyDecode::Encrypted) => error = true,
            }
        }
    }
}

/// The SSH signature blob of `data` (algorithm name and signature).
fn signature_blob(key: &PrivateKey, hash: Option<HashAlg>, data: &[u8]) -> Option<Vec<u8>> {
    use russh::keys::signature::Signer as _;
    let signature: Signature = match key.key_data() {
        KeypairData::Rsa(rsa) => (rsa, hash).try_sign(data).ok()?,
        other => other.try_sign(data).ok()?,
    };
    let mut blob = Vec::new();
    signature.encode(&mut blob).ok()?;
    Some(blob)
}

impl russh::Signer for FileKeySigner<'_> {
    type Error = FileSignError;

    async fn auth_sign(
        &mut self,
        _key: &AgentIdentity,
        hash_alg: Option<HashAlg>,
        mut to_sign: Vec<u8>,
    ) -> Result<Vec<u8>, Self::Error> {
        let key = self.open().await?;
        if key.public_key().key_data() != self.public.key_data() {
            tracing::debug!("key file does not match its public key");
            return Err(FileSignError::Unusable);
        }
        let blob = signature_blob(&key, hash_alg, &to_sign).ok_or(FileSignError::Unusable)?;
        blob.encode(&mut to_sign)
            .map_err(|_| FileSignError::Unusable)?;
        Ok(to_sign)
    }
}

async fn try_file_keys(
    handle: &mut Handle<ClientHandler>,
    context: &AuthContext<'_>,
    progress: &mut Progress,
) -> Result<Step, AuthError> {
    for (index, file) in context.plan.file_keys.iter().enumerate() {
        if !progress.offers(MethodKind::PublicKey) {
            break;
        }
        let Some(public) = file_public_key(file).await else {
            tracing::debug!("key file skipped: no public key");
            continue;
        };
        if progress.was_offered(&public) {
            continue;
        }
        if progress.offered.len() >= MAX_PUBLIC_KEY_OFFERS {
            tracing::debug!("public key limit reached: key files skipped");
            break;
        }
        progress.offered.push(public.clone());
        let Ok(hash) = hash_for(handle, &public.algorithm(), context).await else {
            tracing::debug!("rsa key file skipped: the server accepts only sha-1 signatures");
            continue;
        };
        let mut signer = FileKeySigner {
            file,
            public: &public,
            interaction: context.interaction,
            prompted: None,
        };
        // The passphrase prompt runs inside this call: the limit is the
        // long one of answers that wait for the user.
        let result = tokio::time::timeout(
            context.timeout.max(AUTH_REPLY_TIMEOUT),
            handle.authenticate_publickey_with(
                progress.username.clone(),
                public.clone(),
                hash,
                &mut signer,
            ),
        )
        .await
        .map_err(|_| AuthError::Failed(FailureKind::Timeout))?;
        match result {
            Ok(AuthResult::Success) => {
                let mut authenticated = progress.outcome(AuthMethod::PublicKey);
                authenticated.prompted.file_passphrase =
                    signer.prompted.take().map(|answer| (index, answer));
                return Ok(Step::Success(authenticated));
            }
            Ok(AuthResult::Failure {
                remaining_methods,
                partial_success,
            }) => {
                tracing::debug!(method = "publickey", "key file refused");
                progress.refused(handle, remaining_methods)?;
                if partial_success {
                    return Ok(Step::Next);
                }
            }
            Err(FileSignError::Send) => return Err(progress.closed()),
            Err(FileSignError::Cancelled) => {
                return Err(AuthError::Failed(FailureKind::Cancelled));
            }
            // The server waits for a signature that cannot be made: this
            // attempt cannot go on.
            Err(FileSignError::Unusable) => {
                return Err(AuthError::Failed(FailureKind::KeyUnusable));
            }
        }
    }
    Ok(Step::Next)
}

async fn try_password(
    handle: &mut Handle<ClientHandler>,
    context: &AuthContext<'_>,
    progress: &mut Progress,
) -> Result<Step, AuthError> {
    let mut saved = context.plan.password.clone();
    let mut error = false;
    loop {
        let (value, prompted) = match saved.take() {
            Some(saved) => (saved, None),
            None => {
                let answer = context
                    .interaction
                    .prompt(Prompt::Password {
                        username: progress.username.clone(),
                        error,
                        can_save: context.plan.can_save_password,
                    })
                    .await
                    .ok_or(AuthError::Failed(FailureKind::Cancelled))?;
                (answer.value.clone(), Some(answer))
            }
        };
        let result = with_timeout(
            context.timeout.max(AUTH_REPLY_TIMEOUT),
            handle.authenticate_password(progress.username.clone(), value.expose_secret()),
        )
        .await?
        .map_err(|_| progress.closed())?;
        drop(value);
        match result {
            AuthResult::Success => {
                let mut authenticated = progress.outcome(AuthMethod::Password);
                authenticated.prompted.password = prompted;
                return Ok(Step::Success(authenticated));
            }
            AuthResult::Failure {
                remaining_methods,
                partial_success,
            } => {
                tracing::debug!(method = "password", "refused");
                progress.refused(handle, remaining_methods)?;
                if partial_success || !progress.offers(MethodKind::Password) {
                    return Ok(Step::Next);
                }
                error = true;
            }
        }
    }
}

/// The questions of one keyboard-interactive round, to recognize a repeated
/// round (the server asking the same questions again after wrong answers).
#[derive(PartialEq, Eq)]
struct RoundSignature {
    name: String,
    instruction: String,
    prompts: Vec<(String, bool)>,
}

async fn try_keyboard_interactive(
    handle: &mut Handle<ClientHandler>,
    context: &AuthContext<'_>,
    progress: &mut Progress,
) -> Result<Step, AuthError> {
    let mut error = false;
    loop {
        let mut answered_any = false;
        let mut empty_rounds = 0u32;
        let mut previous: Option<RoundSignature> = None;
        let mut response = with_timeout(
            context.timeout,
            handle.authenticate_keyboard_interactive_start(progress.username.clone(), None),
        )
        .await?
        .map_err(|_| progress.closed())?;
        loop {
            match response {
                KeyboardInteractiveAuthResponse::Success => {
                    return Ok(Step::Success(
                        progress.outcome(AuthMethod::KeyboardInteractive),
                    ));
                }
                KeyboardInteractiveAuthResponse::Failure {
                    remaining_methods,
                    partial_success,
                } => {
                    tracing::debug!(method = "keyboard-interactive", "refused");
                    progress.refused(handle, remaining_methods)?;
                    if partial_success
                        || !answered_any
                        || !progress.offers(MethodKind::KeyboardInteractive)
                    {
                        return Ok(Step::Next);
                    }
                    error = true;
                    break;
                }
                KeyboardInteractiveAuthResponse::InfoRequest {
                    name,
                    instructions,
                    prompts,
                } => {
                    if prompts.len() > MAX_KI_QUESTIONS {
                        tracing::debug!(
                            questions = prompts.len(),
                            "keyboard-interactive round with too many questions"
                        );
                        return Err(AuthError::Failed(FailureKind::Protocol));
                    }
                    let name = server_text(&name, MAX_KI_NAME_CHARS, false);
                    let instructions = server_text(&instructions, MAX_KI_INSTRUCTION_CHARS, true);
                    let answers = if prompts.is_empty() {
                        empty_rounds += 1;
                        if empty_rounds > MAX_EMPTY_KI_ROUNDS {
                            return Ok(Step::Next);
                        }
                        Vec::new()
                    } else {
                        let signature = RoundSignature {
                            name: name.clone(),
                            instruction: instructions.clone(),
                            prompts: prompts
                                .iter()
                                .map(|p| {
                                    (server_text(&p.prompt, MAX_KI_PROMPT_CHARS, false), p.echo)
                                })
                                .collect(),
                        };
                        if previous.as_ref() == Some(&signature) {
                            error = true;
                        }
                        let count = u32::try_from(prompts.len()).unwrap_or(u32::MAX);
                        let mut answers = Vec::with_capacity(prompts.len());
                        for (index, (prompt, echo)) in signature.prompts.iter().enumerate() {
                            let answer = context
                                .interaction
                                .prompt(Prompt::KeyboardInteractive {
                                    name: name.clone(),
                                    instruction: instructions.clone(),
                                    prompt: prompt.clone(),
                                    echo: *echo,
                                    index: u32::try_from(index).unwrap_or(u32::MAX),
                                    count,
                                    error,
                                })
                                .await
                                .ok_or(AuthError::Failed(FailureKind::Cancelled))?;
                            answers.push(answer.value.expose_secret().to_owned());
                        }
                        answered_any = true;
                        error = false;
                        previous = Some(signature);
                        answers
                    };
                    response = with_timeout(
                        context.timeout.max(AUTH_REPLY_TIMEOUT),
                        handle.authenticate_keyboard_interactive_respond(answers),
                    )
                    .await?
                    .map_err(|_| progress.closed())?;
                }
            }
        }
    }
}

/// Server text shown to the user (keyboard-interactive name, instruction
/// and prompts): C0 and C1 control characters (except LF when `keep_lf`;
/// a tab becomes a space), bidi controls and invisible format characters
/// are removed, and the text is cut at `max` characters.
pub(crate) fn server_text(text: &str, max: usize, keep_lf: bool) -> String {
    text.chars()
        .filter_map(|c| match c {
            '\n' if keep_lf => Some('\n'),
            '\t' => Some(' '),
            c if c.is_control() || is_invisible(c) => None,
            c => Some(c),
        })
        .take(max)
        .collect()
}

/// Bidi controls and zero-width or invisible formatting characters.
fn is_invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{061C}'
            | '\u{180E}'
            | '\u{200B}'..='\u{200F}'
            | '\u{2028}'..='\u{202E}'
            | '\u{2060}'..='\u{2069}'
            | '\u{FEFF}'
            | '\u{FFF9}'..='\u{FFFB}'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_texts_are_cleaned_and_bounded() {
        assert_eq!(
            server_text("Code\u{1b}[31m: \u{202E}evil\u{200B}\r\n", 256, false),
            "Code[31m: evil"
        );
        assert_eq!(
            server_text("line one\nline\ttwo\u{85}", 2048, true),
            "line one\nline two"
        );
        assert_eq!(server_text(&"x".repeat(300), 256, false).len(), 256);
        assert_eq!(server_text("\u{65e5}\u{672c}", 1, false), "\u{65e5}");
    }

    #[test]
    fn keys_with_too_many_kdf_rounds_are_not_decrypted() {
        // Not a key at all: left to the decoder.
        assert!(kdf_allowed("not a key"));
        let text = SecretString::from("not a key".to_owned());
        assert!(matches!(
            decode(&text, Some(&SecretString::from("p".to_owned()))),
            Err(KeyDecode::Encrypted | KeyDecode::Invalid)
        ));
    }

    #[test]
    fn encrypted_pem_and_ppk_are_recognized() {
        assert!(looks_encrypted(
            "-----BEGIN ENCRYPTED PRIVATE KEY-----\nAAAA\n-----END ENCRYPTED PRIVATE KEY-----\n"
        ));
        assert!(looks_encrypted(
            "-----BEGIN RSA PRIVATE KEY-----\nProc-Type: 4,ENCRYPTED\n"
        ));
        assert!(looks_encrypted(
            "PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: aes256-cbc\n"
        ));
        assert!(!looks_encrypted(
            "PuTTY-User-Key-File-3: ssh-ed25519\nEncryption: none\n"
        ));
        assert!(!looks_encrypted("-----BEGIN OPENSSH PRIVATE KEY-----\n"));
    }

    #[test]
    fn garbage_is_invalid_not_encrypted() {
        let text = SecretString::from("not a key".to_owned());
        assert!(matches!(decode(&text, None), Err(KeyDecode::Invalid)));
    }
}
