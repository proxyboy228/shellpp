//! Shell++ SSH client (docs/architecture.md, sections 2.4 and 4.2): direct TCP
//! connections, routes through jump hosts and an HTTP or SOCKS5 proxy
//! (stage 5), port forwarding ([`forward`]), host key
//! verification through a callback, authentication (public key, system agent,
//! password, keyboard-interactive), a shell channel with a PTY, an exec
//! channel for OS detection, keepalive, the reconnect backoff, the system
//! agent (`SSH_AUTH_SOCK`, the Windows OpenSSH agent pipe, Pageant) and agent
//! forwarding.
//!
//! The crate knows nothing about records, settings or windows: the desktop
//! host resolves a [`ConnectSpec`] from the core and implements
//! [`Interaction`] (host key decisions, prompts, connection steps).
//!
//! Security rules (docs/security.md, sections 9.1 and 11):
//! - the host key is decided before any authentication data is sent;
//! - SHA-1 key exchange and `ssh-rsa` (SHA-1) signatures are not offered;
//! - server-opened channels other than `auth-agent@openssh.com` (and that
//!   one only after agent forwarding was requested on that connection) are
//!   rejected; forwarded agent channels pass only identity listings and
//!   signature requests;
//! - nothing here logs addresses, usernames, secrets or terminal data, and
//!   `Debug` of every public type omits them.
//!
//! Owner: S2. The public API is the S2/S5 contract written by the lead
//! (stage 2 plan, section 4.2).

#![forbid(unsafe_code)]

pub mod agent;
mod algorithms;
mod auth;
pub mod authorized_keys;
pub mod batch;
pub mod client;
pub mod error;
pub mod forward;
mod handler;
pub mod local_names;
pub mod open_with;
pub mod proxy;
pub mod reconnect;
pub mod sftp;
pub mod transfer;
pub mod transfer_queue;
mod transport;

pub use agent::{AgentKeys, AgentLocation, ConfirmSignature, ForwardedAgent, SystemAgent};
pub use algorithms::supported as supported_algorithms;
pub use authorized_keys::{ExportOutcome, export_public_key};
pub use client::{
    AuthMethod, AuthOutcome, AuthPlan, BoxFuture, ConnectSpec, ConnectStep, Connection, ExecOutput,
    FileKey, HostKeyDecision, HostKeyInfo, Interaction, JumpHost, KeyCredential, Prompt,
    PromptAnswer, PromptedSecrets, SessionEnd, Shell, ShellEvent, ShellRequest, ShellWriter,
    connect,
};
pub use error::{AlgorithmKind, AlgorithmMismatch, FailureKind, FailurePlace, SshError};
pub use forward::{
    Bound, Forward, ForwardEnd, ForwardError, ForwardKind, ForwardSpec, HostForwarder, RuleStatus,
};
pub use proxy::{ProxyKind, ProxySpec};
pub use reconnect::{Backoff, RECONNECT_DELAYS_S, RECONNECT_STEADY_S};
pub use sftp::{EntryKind, RemoteEntry, Sftp, SftpError};
/// Name resolution and TCP connection with the failures of an SSH
/// connection, for the app's other TCP transports (Telnet, D87).
pub use transport::{connect_any, resolve};
