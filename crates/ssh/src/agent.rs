//! The system SSH agent (D3, D5; docs/architecture.md, section 5.1): the
//! agent behind `SSH_AUTH_SOCK` (a Unix socket, or a named pipe path on
//! Windows), the Windows OpenSSH agent pipe `\\.\pipe\openssh-ssh-agent` and
//! Pageant. Used for public key authentication with the agent's keys and as
//! the target of agent forwarding. The built-in agent is stage 3.
//!
//! Agent forwarding (docs/security.md, sections 11.2 and 11.4): a channel
//! `auth-agent@openssh.com` opened by the server is served by
//! [`serve_forwarded_channel`], which passes only identity listings and
//! signature requests to a fresh connection to the system agent and answers
//! every other request (adding, removing or locking keys, extensions) with
//! a failure. The server can ask for signatures while the session lasts but
//! cannot extract or change keys.

use std::path::PathBuf;
use std::time::Duration;

use russh::keys::agent::AgentIdentity;
use russh::keys::agent::client::{AgentClient, AgentStream};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

use crate::error::{FailureKind, SshError};

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
const SSH_AGENTC_SIGN_REQUEST: u8 = 13;

impl SystemAgent {
    /// The first reachable agent: `SSH_AUTH_SOCK`; on Windows then the
    /// OpenSSH agent pipe, then Pageant. `None` when no agent answers.
    pub async fn discover() -> Option<SystemAgent> {
        for location in candidate_locations() {
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

/// Serves one forwarded agent channel (any stream carrying the agent
/// protocol) until the server closes it. See the module documentation.
pub(crate) async fn serve_forwarded_channel<S>(mut channel: S, agent: SystemAgent)
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let mut upstream: Option<AgentConnection> = None;
    let mut requests = 0u64;
    while let Some(frame) = read_frame(&mut channel).await {
        requests += 1;
        let reply = if forwarded_request_allowed(&frame) {
            match relay(&agent, &mut upstream, &frame).await {
                Some(reply) => reply,
                None => {
                    upstream = None;
                    failure_frame()
                }
            }
        } else {
            failure_frame()
        };
        if channel.write_all(&reply).await.is_err() || channel.flush().await.is_err() {
            break;
        }
    }
    let _ = channel.shutdown().await;
    tracing::debug!(requests, "forwarded agent channel closed");
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
        let task = tokio::spawn(serve_forwarded_channel(client, agent));
        // remove all identities (19): refused without contacting the agent
        server.write_all(&[0, 0, 0, 1, 19]).await.expect("write");
        let reply = read_frame(&mut server).await.expect("reply");
        assert_eq!(reply, failure_frame());
        // request identities: the agent is unreachable -> failure
        server.write_all(&[0, 0, 0, 1, 11]).await.expect("write");
        let reply = read_frame(&mut server).await.expect("reply");
        assert_eq!(reply, failure_frame());
        drop(server);
        task.await.expect("task");
    }

    #[tokio::test]
    async fn oversized_frames_end_the_channel() {
        let (mut server, client) = tokio::io::duplex(1024);
        let agent = SystemAgent::at(AgentLocation::Path(PathBuf::from("unused")));
        let task = tokio::spawn(serve_forwarded_channel(client, agent));
        server
            .write_all(&(MAX_AGENT_FRAME as u32 + 1).to_be_bytes())
            .await
            .expect("write");
        task.await.expect("task ends");
    }
}
