//! The system SSH agent (D3, D5; docs/architecture.md, section 5.1): the
//! agent behind `SSH_AUTH_SOCK` (a Unix socket, or a named pipe path on
//! Windows), the Windows OpenSSH agent pipe `\\.\pipe\openssh-ssh-agent` and
//! Pageant. Used for public key authentication with the agent's keys and as
//! the target of agent forwarding. The built-in agent is stage 3.
//!
//! Agent forwarding (docs/security.md, sections 11.2 and 11.4): a channel
//! `auth-agent@openssh.com` opened by the server is served by
//! [`serve_forwarded_channel`] with a [`ForwardedAgent`]: the built-in
//! agent's keys (the Keychain's, signed here, never sent) and, when the
//! option is on, the system agent's (identity listings and signature
//! requests passed to a fresh connection to it). Every other request
//! (adding, removing or locking keys, extensions) gets a failure. The
//! server can ask for signatures while the session lasts but cannot extract
//! or change keys. With confirmation on, every signature waits for the
//! user's yes. RSA keys sign with SHA-2 only (the request's flags); a
//! request for a SHA-1 `ssh-rsa` signature is refused (docs/security.md,
//! 11.2).

use std::path::PathBuf;
use std::time::Duration;

use std::sync::Arc;

use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::{AgentClient, AgentStream};
use russh::keys::ssh_encoding::Encode as _;
use russh::keys::ssh_key::{HashAlg, PrivateKey};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::client::BoxFuture;
use crate::error::{FailureKind, SshError};

/// The private key type of [`AgentKeys`] (re-exported for hosts that build
/// a [`ForwardedAgent`]).
pub use russh::keys::ssh_key::PrivateKey as PrivateKeyArc;

/// The keys of the built-in agent, read at every request (so a lock, which
/// empties it, takes effect at once).
pub trait AgentKeys: Send + Sync {
    /// The decrypted keys to offer; empty while the app is locked.
    fn keys(&self) -> Vec<Arc<PrivateKey>>;
}

/// Asks the user before a forwarded signature (the option is off by
/// default). `fingerprint` is the key's `SHA256:` fingerprint.
pub trait ConfirmSignature: Send + Sync {
    fn confirm<'a>(&'a self, fingerprint: &'a str) -> BoxFuture<'a, bool>;
}

/// What a forwarded agent channel offers (UI 3.1 Agent Forwarding).
#[derive(Clone)]
pub struct ForwardedAgent {
    /// The built-in agent's keys (the Keychain's).
    pub keys: Arc<dyn AgentKeys>,
    /// Also the system agent's keys (option "forward the system agent").
    pub system: Option<SystemAgent>,
    /// Ask before every signature.
    pub confirm: Option<Arc<dyn ConfirmSignature>>,
}

impl std::fmt::Debug for ForwardedAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ForwardedAgent")
            .field("system", &self.system.is_some())
            .field("confirm", &self.confirm.is_some())
            .finish_non_exhaustive()
    }
}

/// No built-in keys (the system agent only, the stage 2 behavior).
struct NoKeys;

impl AgentKeys for NoKeys {
    fn keys(&self) -> Vec<Arc<PrivateKey>> {
        Vec::new()
    }
}

impl ForwardedAgent {
    /// The system agent alone.
    pub fn system_only(agent: SystemAgent) -> Self {
        Self {
            keys: Arc::new(NoKeys),
            system: Some(agent),
            confirm: None,
        }
    }
}

/// Where a system agent is reached.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentLocation {
    /// `SSH_AUTH_SOCK` as found at discovery time.
    Env(PathBuf),
    /// Windows: the OpenSSH agent named pipe.
    WindowsOpenSsh,
    /// Windows: Pageant.
    Pageant,
    /// An explicit socket or pipe path (tests start their own agent).
    Path(PathBuf),
}

/// A reachable system agent. Each operation opens its own connection to the
/// agent, so the value is cheap to clone and keeps no key material.
#[derive(Clone, Debug)]
pub struct SystemAgent {
    location: AgentLocation,
}

/// A connection to an agent, whatever its transport.
pub(crate) type AgentConnection = Box<dyn AgentStream + Send + Unpin + 'static>;

/// Time allowed to open an agent connection.
const AGENT_CONNECT_TIMEOUT: Duration = Duration::from_secs(3);
/// Time allowed for the reachability probe of [`SystemAgent::discover`].
const AGENT_PROBE_TIMEOUT: Duration = Duration::from_secs(3);
/// Largest agent message accepted (the limit OpenSSH and russh use).
const MAX_AGENT_FRAME: usize = 256 * 1024;
/// Time the system agent may take to answer a forwarded request (it may ask
/// its user to confirm a signature).
const AGENT_RELAY_TIMEOUT: Duration = Duration::from_secs(120);

/// The Windows OpenSSH agent pipe.
#[cfg(windows)]
const WINDOWS_OPENSSH_PIPE: &str = r"\\.\pipe\openssh-ssh-agent";

// Agent protocol message numbers (draft-miller-ssh-agent, section 5.1).
const SSH_AGENT_FAILURE: u8 = 5;
const SSH_AGENTC_REQUEST_IDENTITIES: u8 = 11;
const SSH_AGENT_IDENTITIES_ANSWER: u8 = 12;
const SSH_AGENTC_SIGN_REQUEST: u8 = 13;
const SSH_AGENT_SIGN_RESPONSE: u8 = 14;
// Signature flags (section 5.3.1).
const SSH_AGENT_RSA_SHA2_256: u32 = 2;
const SSH_AGENT_RSA_SHA2_512: u32 = 4;

impl SystemAgent {
    /// The first reachable agent: `SSH_AUTH_SOCK`; on Windows then the
    /// OpenSSH agent pipe, then Pageant. `None` when no agent answers.
    pub async fn discover() -> Option<SystemAgent> {
        Self::discover_from(candidate_locations()).await
    }

    /// The first of `candidates` that answers an identity listing within
    /// the probe timeout; `None` when none does. [`Self::discover`] with
    /// the system's locations; tests pass their own agent's.
    pub async fn discover_from(candidates: Vec<AgentLocation>) -> Option<SystemAgent> {
        for location in candidates {
            let agent = SystemAgent::at(location);
            match tokio::time::timeout(AGENT_PROBE_TIMEOUT, agent.identities_raw()).await {
                Ok(Ok(_)) => {
                    tracing::debug!(location = agent.location_kind(), "system agent found");
                    return Some(agent);
                }
                _ => {
                    tracing::debug!(
                        location = agent.location_kind(),
                        "system agent not reachable"
                    );
                }
            }
        }
        None
    }

    /// An agent at a known location (no reachability check).
    pub fn at(location: AgentLocation) -> SystemAgent {
        SystemAgent { location }
    }

    pub fn location(&self) -> &AgentLocation {
        &self.location
    }

    /// The agent's public keys as OpenSSH public key lines.
    pub async fn identities(&self) -> Result<Vec<String>, crate::SshError> {
        let identities = self.identities_raw().await?;
        let mut lines = Vec::with_capacity(identities.len());
        for identity in identities {
            let line = identity
                .public_key()
                .to_openssh()
                .map_err(|_| SshError::new(FailureKind::Protocol))?;
            lines.push(line);
        }
        Ok(lines)
    }

    /// The agent's identities (keys and certificates).
    pub(crate) async fn identities_raw(&self) -> Result<Vec<AgentIdentity>, SshError> {
        let mut client = self.client().await?;
        client
            .request_identities()
            .await
            .map_err(|_| SshError::new(FailureKind::Protocol))
    }

    /// A new agent protocol client on a new connection.
    pub(crate) async fn client(&self) -> Result<AgentClient<AgentConnection>, SshError> {
        Ok(AgentClient::connect(self.connection().await?))
    }

    /// A new raw connection to the agent.
    pub(crate) async fn connection(&self) -> Result<AgentConnection, SshError> {
        match tokio::time::timeout(AGENT_CONNECT_TIMEOUT, open(&self.location)).await {
            Ok(Ok(stream)) => Ok(stream),
            Ok(Err(kind)) => Err(SshError::new(kind)),
            Err(_) => Err(SshError::new(FailureKind::Timeout)),
        }
    }

    /// The kind of location, for logs (never the path).
    fn location_kind(&self) -> &'static str {
        match self.location {
            AgentLocation::Env(_) => "env",
            AgentLocation::WindowsOpenSsh => "windows_openssh",
            AgentLocation::Pageant => "pageant",
            AgentLocation::Path(_) => "path",
        }
    }
}

/// Locations [`SystemAgent::discover`] tries, in order.
fn candidate_locations() -> Vec<AgentLocation> {
    let mut locations = Vec::new();
    if let Some(value) = std::env::var_os("SSH_AUTH_SOCK")
        && !value.is_empty()
    {
        let path = PathBuf::from(value);
        if env_path_usable(&path) {
            locations.push(AgentLocation::Env(path));
        }
    }
    #[cfg(windows)]
    {
        locations.push(AgentLocation::WindowsOpenSsh);
        locations.push(AgentLocation::Pageant);
    }
    locations
}

/// On Windows only a named pipe path in `SSH_AUTH_SOCK` can be used natively
/// (MSYS and Cygwin agents publish emulated sockets that only their own
/// tools understand).
#[cfg(windows)]
fn env_path_usable(path: &std::path::Path) -> bool {
    let text = path.to_string_lossy().replace('/', "\\");
    text.to_ascii_lowercase().starts_with(r"\\.\pipe\")
}

#[cfg(not(windows))]
fn env_path_usable(_path: &std::path::Path) -> bool {
    true
}

async fn open(location: &AgentLocation) -> Result<AgentConnection, FailureKind> {
    match location {
        AgentLocation::Env(path) | AgentLocation::Path(path) => open_path(path).await,
        AgentLocation::WindowsOpenSsh => open_windows_openssh().await,
        AgentLocation::Pageant => open_pageant().await,
    }
}

#[cfg(unix)]
async fn open_path(path: &std::path::Path) -> Result<AgentConnection, FailureKind> {
    let stream = tokio::net::UnixStream::connect(path)
        .await
        .map_err(|error| crate::transport::io_failure(&error))?;
    Ok(Box::new(stream))
}

#[cfg(windows)]
async fn open_path(path: &std::path::Path) -> Result<AgentConnection, FailureKind> {
    open_named_pipe(path.as_os_str()).await
}

#[cfg(not(any(unix, windows)))]
async fn open_path(_path: &std::path::Path) -> Result<AgentConnection, FailureKind> {
    Err(FailureKind::Network)
}

#[cfg(windows)]
async fn open_named_pipe(name: &std::ffi::OsStr) -> Result<AgentConnection, FailureKind> {
    use tokio::net::windows::named_pipe::ClientOptions;
    /// `ERROR_PIPE_BUSY`: every instance is in use; retry shortly.
    const ERROR_PIPE_BUSY: i32 = 231;
    let mut attempts = 0u32;
    loop {
        match ClientOptions::new().open(name) {
            Ok(client) => return Ok(Box::new(client)),
            Err(error) if error.raw_os_error() == Some(ERROR_PIPE_BUSY) && attempts < 40 => {
                attempts += 1;
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Err(error) => return Err(crate::transport::io_failure(&error)),
        }
    }
}

#[cfg(windows)]
async fn open_windows_openssh() -> Result<AgentConnection, FailureKind> {
    open_named_pipe(std::ffi::OsStr::new(WINDOWS_OPENSSH_PIPE)).await
}

#[cfg(not(windows))]
async fn open_windows_openssh() -> Result<AgentConnection, FailureKind> {
    Err(FailureKind::Network)
}

#[cfg(windows)]
async fn open_pageant() -> Result<AgentConnection, FailureKind> {
    let client = AgentClient::connect_pageant()
        .await
        .map_err(|_| FailureKind::Network)?;
    Ok(client.into_inner())
}

#[cfg(not(windows))]
async fn open_pageant() -> Result<AgentConnection, FailureKind> {
    Err(FailureKind::Network)
}

/// Reads one agent frame (length prefix included in the result). `None` at
/// a clean end of stream or on a malformed or oversized frame.
async fn read_frame<R: AsyncRead + Unpin>(reader: &mut R) -> Option<Vec<u8>> {
    let mut len = [0u8; 4];
    reader.read_exact(&mut len).await.ok()?;
    let body_len = u32::from_be_bytes(len) as usize;
    if body_len == 0 || body_len > MAX_AGENT_FRAME {
        return None;
    }
    let mut frame = vec![0u8; 4 + body_len];
    frame[..4].copy_from_slice(&len);
    reader.read_exact(&mut frame[4..]).await.ok()?;
    Some(frame)
}

/// `SSH_AGENT_FAILURE` as a frame.
fn failure_frame() -> Vec<u8> {
    vec![0, 0, 0, 1, SSH_AGENT_FAILURE]
}

/// Whether a forwarded request may reach the system agent.
fn forwarded_request_allowed(frame: &[u8]) -> bool {
    matches!(
        frame.get(4),
        Some(&SSH_AGENTC_REQUEST_IDENTITIES) | Some(&SSH_AGENTC_SIGN_REQUEST)
    )
}

/// Reads an SSH `string` at `*pos` of `buf`.
fn read_string<'a>(buf: &'a [u8], pos: &mut usize) -> Option<&'a [u8]> {
    let len = u32::from_be_bytes(buf.get(*pos..*pos + 4)?.try_into().ok()?) as usize;
    let start = *pos + 4;
    let bytes = buf.get(start..start.checked_add(len)?)?;
    *pos = start + len;
    Some(bytes)
}

fn put_string(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u32).to_be_bytes());
    out.extend_from_slice(bytes);
}

/// A frame from its body (type byte and payload).
fn frame_of(body: Vec<u8>) -> Vec<u8> {
    let mut frame = (body.len() as u32).to_be_bytes().to_vec();
    frame.extend_from_slice(&body);
    frame
}

/// The entries (key blob, comment) of an identities answer frame.
fn identity_entries(frame: &[u8]) -> Option<Vec<(Vec<u8>, Vec<u8>)>> {
    if frame.get(4) != Some(&SSH_AGENT_IDENTITIES_ANSWER) {
        return None;
    }
    let count = u32::from_be_bytes(frame.get(5..9)?.try_into().ok()?) as usize;
    let mut pos = 9;
    let mut entries = Vec::with_capacity(count.min(64));
    for _ in 0..count {
        let blob = read_string(frame, &mut pos)?.to_vec();
        let comment = read_string(frame, &mut pos)?.to_vec();
        entries.push((blob, comment));
    }
    Some(entries)
}

/// The public key blob of a built-in key.
fn key_blob(key: &PrivateKey) -> Option<Vec<u8>> {
    key.public_key().to_bytes().ok()
}

/// Signs `data` with a built-in key for the request's `flags`: RSA with
/// SHA-512 or SHA-256 as asked, never SHA-1; other keys their own way.
fn sign_with(key: &PrivateKey, data: &[u8], flags: u32) -> Option<Vec<u8>> {
    use russh::keys::signature::Signer as _;
    let signature: russh::keys::ssh_key::Signature = match key.key_data() {
        russh::keys::ssh_key::private::KeypairData::Rsa(rsa) => {
            let hash = if flags & SSH_AGENT_RSA_SHA2_512 != 0 {
                HashAlg::Sha512
            } else if flags & SSH_AGENT_RSA_SHA2_256 != 0 {
                HashAlg::Sha256
            } else {
                tracing::debug!("forwarded ssh-rsa (SHA-1) signature refused");
                return None;
            };
            (rsa, Some(hash)).try_sign(data).ok()?
        }
        _ => key.try_sign(data).ok()?,
    };
    let mut blob = Vec::new();
    signature.encode(&mut blob).ok()?;
    Some(blob)
}

/// Serves one forwarded agent channel (any stream carrying the agent
/// protocol) until the server closes it. See the module documentation.
pub(crate) async fn serve_forwarded_channel<S>(mut channel: S, agent: ForwardedAgent)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut upstream: Option<AgentConnection> = None;
    let mut requests = 0u64;
    while let Some(frame) = read_frame(&mut channel).await {
        requests += 1;
        let reply = if !forwarded_request_allowed(&frame) {
            failure_frame()
        } else if frame.get(4) == Some(&SSH_AGENTC_REQUEST_IDENTITIES) {
            list_identities(&agent, &mut upstream, &frame).await
        } else {
            sign(&agent, &mut upstream, &frame).await
        };
        if channel.write_all(&reply).await.is_err() || channel.flush().await.is_err() {
            break;
        }
    }
    let _ = channel.shutdown().await;
    tracing::debug!(requests, "forwarded agent channel closed");
}

/// The identities answer: the built-in keys first, then (with the option)
/// the system agent's.
async fn list_identities(
    agent: &ForwardedAgent,
    upstream: &mut Option<AgentConnection>,
    frame: &[u8],
) -> Vec<u8> {
    let mut entries: Vec<(Vec<u8>, Vec<u8>)> = agent
        .keys
        .keys()
        .iter()
        .filter_map(|key| Some((key_blob(key)?, key.comment().as_bytes().to_vec())))
        .collect();
    if let Some(system) = &agent.system {
        match relay(system, upstream, frame)
            .await
            .as_deref()
            .and_then(identity_entries)
        {
            Some(more) => {
                for entry in more {
                    if !entries.iter().any(|(blob, _)| *blob == entry.0) {
                        entries.push(entry);
                    }
                }
            }
            None => *upstream = None,
        }
    }
    let mut body = vec![SSH_AGENT_IDENTITIES_ANSWER];
    body.extend_from_slice(&(entries.len() as u32).to_be_bytes());
    for (blob, comment) in &entries {
        put_string(&mut body, blob);
        put_string(&mut body, comment);
    }
    frame_of(body)
}

/// A signature: by a built-in key here, else (with the option) by the
/// system agent; the user confirms first when the option is on.
async fn sign(
    agent: &ForwardedAgent,
    upstream: &mut Option<AgentConnection>,
    frame: &[u8],
) -> Vec<u8> {
    // string key blob, string data, uint32 flags.
    let mut pos = 5;
    let Some(blob) = read_string(frame, &mut pos) else {
        return failure_frame();
    };
    let Some(data) = read_string(frame, &mut pos) else {
        return failure_frame();
    };
    let flags = frame
        .get(pos..pos + 4)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u32::from_be_bytes)
        .unwrap_or(0);
    let keys = agent.keys.keys();
    let own = keys
        .iter()
        .find(|key| key_blob(key).as_deref() == Some(blob));
    let fingerprint = match own {
        Some(key) => key.public_key().fingerprint(HashAlg::Sha256).to_string(),
        None => match russh::keys::ssh_key::PublicKey::from_bytes(blob) {
            Ok(public) => public.fingerprint(HashAlg::Sha256).to_string(),
            Err(_) => return failure_frame(),
        },
    };
    if own.is_none() && agent.system.is_none() {
        return failure_frame();
    }
    if let Some(confirm) = &agent.confirm
        && !confirm.confirm(&fingerprint).await
    {
        tracing::debug!("forwarded signature declined");
        return failure_frame();
    }
    match own {
        Some(key) => match sign_with(key, data, flags) {
            Some(signature) => {
                let mut body = vec![SSH_AGENT_SIGN_RESPONSE];
                put_string(&mut body, &signature);
                frame_of(body)
            }
            None => failure_frame(),
        },
        None => {
            let Some(system) = &agent.system else {
                return failure_frame();
            };
            match relay(system, upstream, frame).await {
                Some(reply) => reply,
                None => {
                    *upstream = None;
                    failure_frame()
                }
            }
        }
    }
}

/// Sends one request to the system agent (connecting on first use) and
/// returns its reply frame.
async fn relay(
    agent: &SystemAgent,
    upstream: &mut Option<AgentConnection>,
    frame: &[u8],
) -> Option<Vec<u8>> {
    if upstream.is_none() {
        *upstream = Some(agent.connection().await.ok()?);
    }
    let stream = upstream.as_mut()?;
    let exchange = async {
        stream.write_all(frame).await.ok()?;
        stream.flush().await.ok()?;
        read_frame(stream).await
    };
    tokio::time::timeout(AGENT_RELAY_TIMEOUT, exchange)
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_listing_and_signing_are_forwarded() {
        let frame = |kind: u8| vec![0, 0, 0, 1, kind];
        assert!(forwarded_request_allowed(&frame(
            SSH_AGENTC_REQUEST_IDENTITIES
        )));
        assert!(forwarded_request_allowed(&frame(SSH_AGENTC_SIGN_REQUEST)));
        // add identity, remove identity, remove all, lock, unlock, extension
        for kind in [17u8, 18, 19, 22, 23, 25, 27] {
            assert!(!forwarded_request_allowed(&frame(kind)), "type {kind}");
        }
    }

    #[tokio::test]
    async fn refused_requests_get_a_failure_and_the_channel_stays_usable() {
        let (mut server, client) = tokio::io::duplex(1024);
        // The agent location does not exist: allowed requests fail too, but
        // the channel keeps answering.
        let agent = SystemAgent::at(AgentLocation::Path(PathBuf::from(
            "shellpp-agent-that-does-not-exist",
        )));
        let task = tokio::spawn(serve_forwarded_channel(
            client,
            ForwardedAgent::system_only(agent),
        ));
        // remove all identities (19): refused without contacting the agent
        server.write_all(&[0, 0, 0, 1, 19]).await.expect("write");
        let reply = read_frame(&mut server).await.expect("reply");
        assert_eq!(reply, failure_frame());
        // request identities: the agent is unreachable -> no identities
        server.write_all(&[0, 0, 0, 1, 11]).await.expect("write");
        let reply = read_frame(&mut server).await.expect("reply");
        assert_eq!(reply, [0, 0, 0, 5, 12, 0, 0, 0, 0]);
        // a signature with an unknown key and an unreachable agent: failure
        let mut sign = vec![SSH_AGENTC_SIGN_REQUEST];
        put_string(&mut sign, b"not a key");
        put_string(&mut sign, b"data");
        sign.extend_from_slice(&0u32.to_be_bytes());
        server.write_all(&frame_of(sign)).await.expect("write");
        let reply = read_frame(&mut server).await.expect("reply");
        assert_eq!(reply, failure_frame());
        drop(server);
        task.await.expect("task");
    }

    struct Keys(Vec<Arc<PrivateKey>>);

    impl AgentKeys for Keys {
        fn keys(&self) -> Vec<Arc<PrivateKey>> {
            self.0.clone()
        }
    }

    struct Answer(bool, std::sync::Mutex<Vec<String>>);

    impl ConfirmSignature for Answer {
        fn confirm<'a>(&'a self, fingerprint: &'a str) -> BoxFuture<'a, bool> {
            self.1.lock().unwrap().push(fingerprint.to_owned());
            let yes = self.0;
            Box::pin(async move { yes })
        }
    }

    fn ed25519(comment: &str) -> Arc<PrivateKey> {
        use secrecy::ExposeSecret as _;
        let generated =
            shellpp_test_sshd::generate_key(shellpp_test_sshd::KeyType::Ed25519, comment, None)
                .expect("key");
        Arc::new(
            PrivateKey::from_openssh(generated.private_openssh.expose_secret().as_bytes())
                .expect("parsed"),
        )
    }

    async fn ask(server: &mut tokio::io::DuplexStream, body: Vec<u8>) -> Vec<u8> {
        server.write_all(&frame_of(body)).await.expect("write");
        read_frame(server).await.expect("reply")
    }

    fn sign_request(blob: &[u8], data: &[u8]) -> Vec<u8> {
        let mut body = vec![SSH_AGENTC_SIGN_REQUEST];
        put_string(&mut body, blob);
        put_string(&mut body, data);
        body.extend_from_slice(&0u32.to_be_bytes());
        body
    }

    #[tokio::test]
    async fn the_built_in_agent_lists_and_signs_with_its_keys() {
        use russh::keys::signature::Verifier as _;
        let key = ed25519("work laptop");
        let blob = key_blob(&key).unwrap();
        let (mut server, client) = tokio::io::duplex(64 * 1024);
        let agent = ForwardedAgent {
            keys: Arc::new(Keys(vec![key.clone()])),
            system: None,
            confirm: None,
        };
        let task = tokio::spawn(serve_forwarded_channel(client, agent));
        let reply = ask(&mut server, vec![SSH_AGENTC_REQUEST_IDENTITIES]).await;
        assert_eq!(
            identity_entries(&reply).unwrap(),
            vec![(blob.clone(), b"work laptop".to_vec())]
        );
        let reply = ask(&mut server, sign_request(&blob, b"session data")).await;
        assert_eq!(reply[4], SSH_AGENT_SIGN_RESPONSE);
        let mut pos = 5;
        let encoded = read_string(&reply, &mut pos).unwrap();
        let signature = russh::keys::ssh_key::Signature::try_from(encoded).expect("signature");
        key.public_key()
            .key_data()
            .verify(b"session data", &signature)
            .expect("the signature verifies");
        // A key it does not hold, without the system agent: failure.
        let other = key_blob(&ed25519("other")).unwrap();
        assert_eq!(
            ask(&mut server, sign_request(&other, b"x")).await,
            failure_frame()
        );
        drop(server);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn every_signature_can_wait_for_a_yes() {
        let key = ed25519("k");
        let blob = key_blob(&key).unwrap();
        for yes in [false, true] {
            let (mut server, client) = tokio::io::duplex(64 * 1024);
            let answer = Arc::new(Answer(yes, std::sync::Mutex::new(Vec::new())));
            let agent = ForwardedAgent {
                keys: Arc::new(Keys(vec![key.clone()])),
                system: None,
                confirm: Some(answer.clone()),
            };
            let task = tokio::spawn(serve_forwarded_channel(client, agent));
            let reply = ask(&mut server, sign_request(&blob, b"data")).await;
            assert_eq!(reply[4] == SSH_AGENT_SIGN_RESPONSE, yes);
            assert_eq!(
                *answer.1.lock().unwrap(),
                [key.public_key().fingerprint(HashAlg::Sha256).to_string()],
                "asked once, with the fingerprint"
            );
            drop(server);
            task.await.unwrap();
        }
    }

    #[tokio::test]
    async fn a_locked_app_offers_no_keys() {
        // The keys are read at each request: an emptied agent lists none.
        let (mut server, client) = tokio::io::duplex(1024);
        let agent = ForwardedAgent {
            keys: Arc::new(Keys(Vec::new())),
            system: None,
            confirm: None,
        };
        let task = tokio::spawn(serve_forwarded_channel(client, agent));
        let reply = ask(&mut server, vec![SSH_AGENTC_REQUEST_IDENTITIES]).await;
        assert_eq!(identity_entries(&reply).unwrap(), Vec::new());
        drop(server);
        task.await.unwrap();
    }

    #[tokio::test]
    async fn oversized_frames_end_the_channel() {
        let (mut server, client) = tokio::io::duplex(1024);
        let agent = SystemAgent::at(AgentLocation::Path(PathBuf::from("unused")));
        let task = tokio::spawn(serve_forwarded_channel(
            client,
            ForwardedAgent::system_only(agent),
        ));
        server
            .write_all(&(MAX_AGENT_FRAME as u32 + 1).to_be_bytes())
            .await
            .expect("write");
        task.await.expect("task ends");
    }
}
