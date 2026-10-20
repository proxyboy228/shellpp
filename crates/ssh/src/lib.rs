//! Shell++ SSH client (docs/architecture.md, sections 2.4 and 4.2): direct TCP
//! connections in stage 2 (jump hosts and proxies join in stage 5), host key
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
pub mod client;
pub mod error;
mod handler;
pub mod reconnect;
mod transport;

pub use agent::{AgentLocation, SystemAgent};
pub use authorized_keys::{ExportOutcome, export_public_key};
pub use client::{
    AuthMethod, AuthOutcome, AuthPlan, BoxFuture, ConnectSpec, ConnectStep, Connection, ExecOutput,
    FileKey, HostKeyDecision, HostKeyInfo, Interaction, KeyCredential, Prompt, PromptAnswer,
    PromptedSecrets, SessionEnd, Shell, ShellEvent, ShellRequest, ShellWriter, connect,
};
pub use error::{FailureKind, SshError};
pub use reconnect::{Backoff, RECONNECT_DELAYS_S, RECONNECT_STEADY_S};
