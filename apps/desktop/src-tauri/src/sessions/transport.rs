//! The transports behind a session, as the session manager sees them: an
//! SSH connector (real: `shellpp-ssh`; tests: scripted fakes) and a running
//! terminal (an SSH shell channel or a local PTY) with ordered output events
//! and an input handle.

use std::sync::Arc;
use std::time::Duration;

use secrecy::SecretString;
use shellpp_ssh::{
    ConnectSpec, Connection, ExecOutput, FailureKind, Interaction, SessionEnd, Shell, ShellEvent,
    ShellRequest, ShellWriter, SshError, SystemAgent,
};

pub use shellpp_ssh::BoxFuture;

/// How a terminal ended, for both transports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TermEnd {
    /// Exit status (0 closes the tab, UI 4.6).
    ExitStatus(u32),
    /// SSH exit signal: shown like a non-zero status.
    ExitSignal,
    /// Transport error (`network`), keepalive timeout (`timeout`) or close
    /// without exit status (`disconnected`): the reason shown after the drop.
    Dropped(FailureKind),
    /// A local process ended without a known exit code.
    ExitUnknown,
}

/// Output events of a running terminal, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TermEvent {
    Data(Vec<u8>),
    /// No event follows.
    End(TermEnd),
}

/// The output side of a terminal. `next` is called from one task only.
pub trait TermEvents: Send + 'static {
    fn next(&mut self) -> BoxFuture<'_, TermEvent>;
}

/// The input side of a terminal. Calls are made in order from one task.
pub trait TermInput: Send + Sync + 'static {
    /// `false` when the terminal is gone.
    fn write(&self, data: Vec<u8>) -> BoxFuture<'_, bool>;
    fn resize(&self, cols: u16, rows: u16) -> BoxFuture<'_, bool>;
    /// Closes the input side (SSH: EOF and close; local: ends the process).
    fn close(&self) -> BoxFuture<'_, ()>;
}

/// A running terminal.
pub struct TermParts {
    pub events: Box<dyn TermEvents>,
    pub input: Arc<dyn TermInput>,
}

impl std::fmt::Debug for TermParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TermParts { .. }")
    }
}

/// A value typed at a prompt during authentication, with its "Save" flag.
pub struct PromptedSecret {
    pub value: SecretString,
    pub save: bool,
}

impl std::fmt::Debug for PromptedSecret {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PromptedSecret")
            .field("save", &self.save)
            .finish_non_exhaustive()
    }
}

/// What authentication used, with the secrets typed at prompts (moved out of
/// the connection: afterwards the session holds the only copy).
pub struct AuthResult {
    pub username: String,
    pub prompted_password: Option<PromptedSecret>,
    pub prompted_passphrase: Option<PromptedSecret>,
}

impl std::fmt::Debug for AuthResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthResult")
            .field("prompted_password", &self.prompted_password.is_some())
            .field("prompted_passphrase", &self.prompted_passphrase.is_some())
            .finish_non_exhaustive()
    }
}

/// An authenticated SSH connection.
pub trait RemoteConnection: Send + Sync + 'static {
    /// The server identification line (OS detection from the banner).
    fn server_ident(&self) -> String;
    /// The username and the prompted secrets; the secrets are handed out
    /// once (later calls return none).
    fn take_auth(&self) -> AuthResult;
    fn open_shell(&self, request: ShellRequest) -> BoxFuture<'_, Result<TermParts, SshError>>;
    /// A separate exec channel (OS detection), never the shell.
    fn exec(
        &self,
        command: String,
        timeout: Duration,
        max_output: usize,
    ) -> BoxFuture<'_, Result<ExecOutput, SshError>>;
    fn disconnect(&self) -> BoxFuture<'_, ()>;
}

/// Opens SSH connections.
pub trait Connector: Send + Sync + 'static {
    fn connect(
        &self,
        spec: ConnectSpec,
        interaction: Arc<dyn Interaction>,
    ) -> BoxFuture<'static, Result<Arc<dyn RemoteConnection>, SshError>>;
    /// The system agent, if one answers (`SSH_AUTH_SOCK`, the Windows
    /// OpenSSH agent pipe, Pageant).
    fn discover_agent(&self) -> BoxFuture<'static, Option<SystemAgent>>;
}

/// The real connector (`shellpp-ssh`).
#[derive(Clone, Copy, Debug)]
pub struct SshConnector {
    system_agent: bool,
}

impl SshConnector {
    /// `system_agent`: use the system agent (`SSH_AUTH_SOCK`, the Windows
    /// OpenSSH agent pipe, Pageant) for authentication and forwarding. e2e
    /// builds and tests turn it off so the developer's agent never takes
    /// part (agent tests are in `crates/ssh`, stage 2 plan, section 8.2).
    pub fn new(system_agent: bool) -> Self {
        Self { system_agent }
    }
}

impl Default for SshConnector {
    fn default() -> Self {
        Self::new(true)
    }
}

impl Connector for SshConnector {
    fn connect(
        &self,
        spec: ConnectSpec,
        interaction: Arc<dyn Interaction>,
    ) -> BoxFuture<'static, Result<Arc<dyn RemoteConnection>, SshError>> {
        Box::pin(async move {
            let connection = shellpp_ssh::connect(spec, interaction).await?;
            Ok(Arc::new(SshConnection(connection)) as Arc<dyn RemoteConnection>)
        })
    }

    fn discover_agent(&self) -> BoxFuture<'static, Option<SystemAgent>> {
        if !self.system_agent {
            return Box::pin(async { None });
        }
        Box::pin(SystemAgent::discover())
    }
}

struct SshConnection(Connection);

fn move_answer(answer: Option<shellpp_ssh::PromptAnswer>) -> Option<PromptedSecret> {
    answer.map(|answer| PromptedSecret {
        value: answer.value,
        save: answer.save,
    })
}

impl RemoteConnection for SshConnection {
    fn server_ident(&self) -> String {
        self.0.server_ident().to_owned()
    }

    fn take_auth(&self) -> AuthResult {
        let prompted = self.0.take_prompted_secrets();
        AuthResult {
            username: self.0.auth().username.clone(),
            prompted_password: move_answer(prompted.password),
            prompted_passphrase: move_answer(prompted.passphrase),
        }
    }

    fn open_shell(&self, request: ShellRequest) -> BoxFuture<'_, Result<TermParts, SshError>> {
        Box::pin(async move {
            let shell = self.0.open_shell(request).await?;
            let writer = shell.writer();
            Ok(TermParts {
                events: Box::new(SshEvents(shell)),
                input: Arc::new(SshInput(writer)),
            })
        })
    }

    fn exec(
        &self,
        command: String,
        timeout: Duration,
        max_output: usize,
    ) -> BoxFuture<'_, Result<ExecOutput, SshError>> {
        Box::pin(async move { self.0.exec(&command, timeout, max_output).await })
    }

    fn disconnect(&self) -> BoxFuture<'_, ()> {
        Box::pin(self.0.disconnect())
    }
}

struct SshEvents(Shell);

impl TermEvents for SshEvents {
    fn next(&mut self) -> BoxFuture<'_, TermEvent> {
        Box::pin(async move {
            match self.0.next().await {
                ShellEvent::Data(data) => TermEvent::Data(data),
                ShellEvent::End(end) => TermEvent::End(map_end(&end)),
            }
        })
    }
}

/// Maps an SSH shell end (UI 4.6).
pub fn map_end(end: &SessionEnd) -> TermEnd {
    match end {
        SessionEnd::ExitStatus(status) => TermEnd::ExitStatus(*status),
        SessionEnd::ExitSignal(_) => TermEnd::ExitSignal,
        SessionEnd::ClosedWithoutStatus => TermEnd::Dropped(FailureKind::Disconnected),
        SessionEnd::Dropped(kind) => TermEnd::Dropped(*kind),
    }
}

struct SshInput(ShellWriter);

impl TermInput for SshInput {
    fn write(&self, data: Vec<u8>) -> BoxFuture<'_, bool> {
        Box::pin(async move { self.0.write(&data).await.is_ok() })
    }

    fn resize(&self, cols: u16, rows: u16) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            self.0
                .resize(u32::from(cols), u32::from(rows))
                .await
                .is_ok()
        })
    }

    fn close(&self) -> BoxFuture<'_, ()> {
        Box::pin(self.0.close())
    }
}

/// `true` for failures after which a decided host key justifies one
/// immediate retry: the server closed the connection while the user decided
/// (docs/architecture.md, section 4.2, step 4).
pub fn closed_while_deciding(kind: FailureKind) -> bool {
    matches!(
        kind,
        FailureKind::Disconnected | FailureKind::Network | FailureKind::Protocol
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ends_map_to_the_ui_rules() {
        assert_eq!(map_end(&SessionEnd::ExitStatus(0)), TermEnd::ExitStatus(0));
        assert_eq!(map_end(&SessionEnd::ExitStatus(3)), TermEnd::ExitStatus(3));
        assert_eq!(
            map_end(&SessionEnd::ExitSignal("KILL".into())),
            TermEnd::ExitSignal
        );
        assert_eq!(
            map_end(&SessionEnd::ClosedWithoutStatus),
            TermEnd::Dropped(FailureKind::Disconnected)
        );
        assert_eq!(
            map_end(&SessionEnd::Dropped(FailureKind::Disconnected)),
            TermEnd::Dropped(FailureKind::Disconnected)
        );
        assert_eq!(
            map_end(&SessionEnd::Dropped(FailureKind::Timeout)),
            TermEnd::Dropped(FailureKind::Timeout)
        );
        assert_eq!(
            map_end(&SessionEnd::Dropped(FailureKind::Network)),
            TermEnd::Dropped(FailureKind::Network)
        );
    }

    #[test]
    fn retry_after_decision_only_for_transport_failures() {
        assert!(closed_while_deciding(FailureKind::Disconnected));
        assert!(closed_while_deciding(FailureKind::Network));
        assert!(!closed_while_deciding(FailureKind::AuthFailed));
        assert!(!closed_while_deciding(FailureKind::Cancelled));
        assert!(!closed_while_deciding(FailureKind::HostKeyRejected));
    }
}
