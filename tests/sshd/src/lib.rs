//! `shellpp-test-sshd`: the russh SSH test server of docs/architecture.md,
//! section 9.3 (stage 2 scope; SFTP and forwarding join in stages 4 and 5).
//!
//! Used two ways:
//! - as a library by Rust tests (`crates/ssh/tests`, `apps/desktop/src-tauri/tests`):
//!   [`TestServer::start`] with a [`ServerConfig`];
//! - as the `shellpp-test-sshd` binary by the UI e2e harness
//!   (`apps/desktop/e2e/support/sshd.ts`): `serve --config <file>` and
//!   `keygen` (CLI contract in the stage 2 plan, section 4.1, and in
//!   `src/main.rs`).
//!
//! Everything secret (host keys, user keys, passwords, keyboard-interactive
//! answers) is generated or passed in at run time; nothing is checked in
//! (CONTRIBUTING, "Secrets and test data"). The server listens on loopback
//! only and is never shipped.
//!
//! The shell contract (prompt, line discipline, commands) is documented in
//! [`shell`]; server behavior (authentication rules, events, rotation, drops)
//! in [`server`].
//!
//! Owner: S1. The public API is the S1/S2/S5/S8 contract of the stage 2 plan,
//! section 4.1. Extensions beyond the plan: [`TestServer::freeze_connections`],
//! [`TestServer::connection_count`], [`TestServer::host_public_key`], the
//! `UserConfig`/`KiQuestion` constructors and the `freeze` CLI command.

mod conn;

pub mod config;
pub mod keygen;
pub mod server;
pub mod shell;

pub use config::{ExecAnswer, HostKeySource, KiQuestion, ServerConfig, UserConfig};
pub use keygen::{GeneratedKey, KeyType, generate_key};
pub use server::{ServerEvent, TestServer};

/// Errors of the test server. Messages never contain passwords or keys.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid configuration: {0}")]
    Config(String),
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("ssh error: {0}")]
    Ssh(String),
    #[error("key error: {0}")]
    Key(String),
}
