//! The task of one SSH session (stage 2 plan, sections 4.5 and 5.4;
//! docs/architecture.md, section 4.2): connection attempts with steps, the
//! host key decision against known hosts, prompts, saving prompted secrets,
//! the running shell, OS detection, the connection log, the end of the
//! shell and the reconnect loop (UI 4.6, D18).
//!
//! ```text
//! attempt -> connected -> exit 0 -> closed
//!                      -> exit n / signal -> ended  --Reconnect--> attempt (Retry)
//!                      -> drop -> dropped -> (autoreconnect) wait 1, 2, 4 ... 60 s -> attempt (Auto)
//! attempt (First/Retry) fails -> failed --Retry--> attempt (Retry)
//! attempt (Auto) fails -> dropped -> next delay (transport failures only;
//!                         any other failure stops the automatic attempts)
//! ```

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use secrecy::SecretString;
use shellpp_core::os_detect::{EXEC_TIMEOUT_SECS, MAX_OUTPUT_BYTES};
use shellpp_core::service::dto::{
    ConnectStep as UiStep, HostKeyDecision as UiDecision, HostKeyInfo as UiKey, OsSource,
    PromptKind, PromptReply, ReconnectInfo, RecordRef, SessionError, SessionState, SessionTarget,
};
use shellpp_core::service::host_api::{
    ConnectionEndReason, ConnectionLogStart, HostKeyCheck, HostKeyStatus, SessionConfig,
};
use shellpp_ssh::{
    AuthPlan, Backoff, BoxFuture, ConnectSpec, ConnectStep, FailureKind, HostKeyDecision,
    HostKeyInfo, Interaction, KeyCredential, Prompt, PromptAnswer, ShellRequest, SshError,
};
use tokio::sync::{mpsc, watch};

use super::backend::SessionSettings;
use super::charset::{self, InputEncoder, OutputDecoder};
use super::transport::{AuthResult, RemoteConnection, TermEnd, TermParts, closed_while_deciding};
use super::{
    AGENT_DISCOVERY_TIMEOUT, CLOSE_TIMEOUT, CONNECT_TIMEOUT, Control, PumpEnd, Session, Shared,
    ask, core_session_error, error_code, host_os, pump, ssh_session_error,
};

/// Why an attempt runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AttemptKind {
    /// Right after `session_open`.
    First,
    /// Retry (failed) or Reconnect (ended): a failure shows `failed`.
    Retry,
    /// An automatic reconnect attempt of a dropped session.
    Auto,
    /// "Reconnect now" on a dropped session: a failure restarts the backoff.
    Manual,
}

impl AttemptKind {
    fn is_reconnect(self) -> bool {
        matches!(self, Self::Auto | Self::Manual)
    }
}

/// What the task does next.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Next {
    Attempt(AttemptKind),
    /// Ended, failed or dropped without automatic attempts: wait for a command.
    Idle,
    /// Dropped: schedule the next automatic attempt.
    Schedule,
    /// An automatic attempt was aborted by a lock: run it again at unlock.
    Resume,
    Exit,
}

enum Outcome {
    Connected(Established),
    Failed {
        error: SessionError,
        host_key: Option<UiKey>,
        retry_once: bool,
    },
    Cancelled,
    Closed,
    Locked,
}

struct Established {
    connection: Arc<dyn RemoteConnection>,
    parts: TermParts,
    username: String,
    /// The size the shell was opened with.
    size: (u16, u16),
}

/// What the task remembers about the connection being established.
#[derive(Clone)]
struct ConnInfo {
    host: Option<RecordRef>,
    title: String,
    address: String,
    port: u16,
    encoding: &'static encoding_rs::Encoding,
    os_detection_script: Option<String>,
}

/// Where prompted secrets may be saved, and the server they are kept for.
#[derive(Clone)]
struct SaveTargets {
    host: Option<RecordRef>,
    can_save_password: bool,
    key: Option<(RecordRef, bool)>,
    username_missing: bool,
    address: String,
    port: u16,
}

struct ShellSetup {
    term: String,
    env: Vec<(String, SecretString)>,
    agent_forwarding: bool,
}

pub(crate) struct SshTask {
    shared: Arc<Shared>,
    session: Arc<Session>,
    ctrl: mpsc::UnboundedReceiver<Control>,
    target: SessionTarget,
    config: Option<SessionConfig>,
    settings: SessionSettings,
    backoff: Backoff,
    os_checked: bool,
    unlocked: watch::Receiver<bool>,
    /// The error of the last drop, shown while automatic attempts are stopped.
    last_drop: SessionError,
    /// The connection being established (between `connect_once` and `connected`).
    info: Option<ConnInfo>,
    /// A shell ran in this session before: the next one starts with a
    /// terminal mode reset.
    reconnected: bool,
}

impl SshTask {
    pub(crate) fn new(
        shared: Arc<Shared>,
        session: Arc<Session>,
        ctrl: mpsc::UnboundedReceiver<Control>,
        config: SessionConfig,
        settings: SessionSettings,
    ) -> Self {
        let target = session
            .info()
            .target
            .expect("an SSH session always has a target");
        let unlocked = shared.unlocked();
        Self {
            shared,
            session,
            ctrl,
            target,
            config: Some(config),
            settings,
            backoff: Backoff::new(),
            os_checked: false,
            unlocked,
            last_drop: disconnected(),
            info: None,
            reconnected: false,
        }
    }

    pub(crate) async fn run(mut self) {
        let mut next = Next::Attempt(AttemptKind::First);
        loop {
            next = match next {
                Next::Attempt(kind) => self.attempt(kind).await,
                Next::Idle => self.idle().await,
                Next::Schedule => {
                    let delay = self.backoff.next_delay();
                    let attempt = self.backoff.attempt();
                    self.wait_for_attempt(attempt, Some(delay)).await
                }
                Next::Resume => {
                    let attempt = self.backoff.attempt().max(1);
                    self.wait_for_attempt(attempt, None).await
                }
                Next::Exit => break,
            };
        }
        tracing::debug!(session = %self.session.id, "session task ended");
    }

    fn update(&self, change: impl FnOnce(&mut super::SessionMut)) {
        self.session.update(&*self.shared.events, change);
    }

    async fn attempt(&mut self, kind: AttemptKind) -> Next {
        let started = Instant::now();
        let reconnect = kind.is_reconnect().then(|| ReconnectInfo {
            attempt: self.backoff.attempt().max(1),
            next_in_ms: None,
            waiting_for_unlock: false,
            in_progress: true,
        });
        self.update(|st| {
            let info = &mut st.info;
            info.state = SessionState::Connecting;
            info.step = Some(UiStep::Resolving);
            info.error = None;
            info.host_key = None;
            info.exit_status = None;
            info.prompt = None;
            info.reconnect = reconnect;
        });
        tracing::info!(session = %self.session.id, ?kind, "connection attempt");
        let mut retried = false;
        loop {
            match self.connect_once().await {
                Outcome::Connected(established) => {
                    tracing::info!(
                        session = %self.session.id,
                        ms = started.elapsed().as_millis() as u64,
                        "connected"
                    );
                    return self.connected(established).await;
                }
                Outcome::Failed {
                    error,
                    host_key,
                    retry_once,
                } => {
                    if retry_once && !retried {
                        retried = true;
                        tracing::info!(
                            session = %self.session.id,
                            "closed during the host key decision; retrying once"
                        );
                        continue;
                    }
                    tracing::info!(
                        session = %self.session.id,
                        code = %error.code,
                        reason = ?error.reason,
                        "attempt failed"
                    );
                    return self.failed(kind, error, host_key).await;
                }
                Outcome::Cancelled => return self.stopped(kind, error_code("cancelled")),
                Outcome::Closed => return Next::Exit,
                Outcome::Locked => {
                    tracing::info!(session = %self.session.id, "attempt aborted by lock");
                    return if kind.is_reconnect() {
                        Next::Resume
                    } else {
                        self.stopped(kind, error_code("cancelled"))
                    };
                }
            }
        }
    }

    /// One connection: settings, agent, TCP, host key, authentication, shell.
    async fn connect_once(&mut self) -> Outcome {
        let config = match self.config.take() {
            Some(config) => {
                let encoding = charset::encoding_for(&config.charset);
                self.session.lock().encoder = InputEncoder::new(encoding);
                config
            }
            None => match self.resolve_config().await {
                Ok(config) => config,
                Err(error) => {
                    return Outcome::Failed {
                        error,
                        host_key: None,
                        retry_once: false,
                    };
                }
            },
        };
        let kept = self.session.lock().kept.clone();
        // `resolve_config` dropped values kept for another server; still
        // check, so a typed secret never goes anywhere else.
        let kept = if kept.usable_for(&config.address, config.port, config.username.as_deref()) {
            kept
        } else {
            super::KeptCredentials::default()
        };
        let username = config.username.clone().or(kept.username.clone());
        let info = ConnInfo {
            host: config.host,
            title: config.title.clone(),
            address: config.address.clone(),
            port: config.port,
            encoding: charset::encoding_for(&config.charset),
            os_detection_script: config.os_detection_script.clone(),
        };
        let saves = SaveTargets {
            host: config.host,
            can_save_password: config.can_save_password && config.host.is_some(),
            key: config
                .key
                .as_ref()
                .map(|key| (key.key, key.can_save_passphrase)),
            username_missing: config.username.is_none(),
            address: config.address.clone(),
            port: config.port,
        };
        let kept_passphrase = kept
            .passphrase
            .clone()
            .filter(|_| kept.passphrase_key == config.key.as_ref().map(|key| key.key));
        let spec = ConnectSpec {
            address: config.address.clone(),
            port: config.port,
            username,
            auth: AuthPlan {
                key: config.key.as_ref().map(|key| KeyCredential {
                    label: key.label.clone(),
                    private_key: key.private_key.clone(),
                    passphrase: key.passphrase.clone().or(kept_passphrase.clone()),
                }),
                use_agent: false,
                password: config.password.clone().or(kept.password.clone()),
                keyboard_interactive: true,
                can_save_password: saves.can_save_password,
                can_save_passphrase: saves.key.is_some_and(|(_, can_save)| can_save),
            },
            agent: None,
            keepalive: (config.keepalive_s > 0)
                .then(|| Duration::from_secs(u64::from(config.keepalive_s))),
            connect_timeout: CONNECT_TIMEOUT,
            preferred_host_key_types: config.known_key_types.clone(),
        };
        let shell = ShellSetup {
            term: config.term.clone(),
            env: config.env.clone(),
            agent_forwarding: config.agent_forwarding,
        };
        drop(config);

        let interaction = Arc::new(SessionInteraction::new(
            self.shared.clone(),
            self.session.clone(),
            info.address.clone(),
            info.port,
        ));
        let establish = establish(
            self.shared.clone(),
            self.session.clone(),
            spec,
            interaction.clone(),
            saves,
            shell,
        );
        tokio::pin!(establish);
        // Biased: a lock cancels the pending prompt after it flips the lock
        // state, so an attempt that fails because of that cancel is seen as
        // aborted by the lock, not as cancelled by the user.
        let result = loop {
            tokio::select! {
                biased;
                control = self.ctrl.recv() => match control {
                    Some(Control::Reconnect) => {}
                    Some(Control::Cancel) => return Outcome::Cancelled,
                    Some(Control::Close) | None => return Outcome::Closed,
                },
                () = wait_locked(&mut self.unlocked) => break None,
                result = &mut establish => break Some(result),
            }
        };
        let Some(result) = result else {
            return Outcome::Locked;
        };
        match result {
            Ok(established) => {
                self.info = Some(info);
                Outcome::Connected(established)
            }
            Err(error) => {
                if let Some(key) = interaction.take_revoked() {
                    return Outcome::Failed {
                        error: error_code("host_key_revoked"),
                        host_key: Some(key),
                        retry_once: false,
                    };
                }
                if let Some(failure) = interaction.take_failure() {
                    return Outcome::Failed {
                        error: failure,
                        host_key: None,
                        retry_once: false,
                    };
                }
                Outcome::Failed {
                    error: ssh_session_error(error),
                    host_key: None,
                    retry_once: interaction.decided() && closed_while_deciding(error.kind),
                }
            }
        }
    }

    /// Settings of a later attempt (the host may have changed since).
    async fn resolve_config(&mut self) -> Result<SessionConfig, SessionError> {
        let target = self.target.clone();
        let config = match self
            .shared
            .blocking(move |core| core.session_config(&target))
            .await
        {
            Some(Ok(config)) => config,
            Some(Err(error)) => return Err(core_session_error(&error)),
            None => return Err(error_code("internal")),
        };
        let encoding = charset::encoding_for(&config.charset);
        let session_id = self.session.id;
        self.update(|st| {
            st.host = config.host;
            st.encoder = InputEncoder::new(encoding);
            if !st
                .kept
                .usable_for(&config.address, config.port, config.username.as_deref())
            {
                // The host now points to another server or user: what was
                // typed for the previous one is not sent there.
                st.kept = super::KeptCredentials::default();
                tracing::info!(session = %session_id, "kept credentials dropped: the host changed");
            }
            let username = config.username.clone().or(st.kept.username.clone());
            let info = &mut st.info;
            info.title = config.title.clone();
            info.address = Some(config.address.clone());
            info.port = Some(config.port);
            info.username = username;
            info.terminal_theme = config.terminal_theme.clone();
            info.backspace = config.backspace;
            info.record_history = config.record_history;
            if config.os.is_some() {
                info.os = config.os.clone();
            }
        });
        Ok(config)
    }

    async fn connected(&mut self, established: Established) -> Next {
        let Established {
            connection,
            parts,
            username,
            size,
        } = established;
        let info = self.info.take().expect("connection info of the attempt");
        self.backoff.reset();
        if self.reconnected {
            // The pane still shows the previous shell's screen and modes.
            super::reset_terminal_modes(&self.session);
        }
        self.reconnected = true;
        let mut input = None;
        self.update(|st| {
            input = Some(super::open_input(st, size));
            let info = &mut st.info;
            info.state = SessionState::Connected;
            info.step = None;
            info.prompt = None;
            info.error = None;
            info.host_key = None;
            info.exit_status = None;
            info.reconnect = None;
            info.username = Some(username.clone());
        });

        // Connection log (plan decision 10).
        let entry = ConnectionLogStart {
            host: info.host,
            host_label: info.host.map(|_| info.title.clone()),
            address: info.address.clone(),
            port: info.port,
            username: Some(username),
            os: self.session.info().os,
        };
        let log_id = self
            .shared
            .blocking(move |core| core.connection_log_start(entry))
            .await
            .and_then(|result| match result {
                Ok(log_id) => Some(log_id),
                Err(error) => {
                    tracing::warn!(code = error.code(), "connection log not written");
                    None
                }
            });
        {
            let mut st = self.session.lock();
            if st.closed {
                drop(st);
                self.shared
                    .end_log(log_id, ConnectionEndReason::Closed, None);
            } else {
                st.log_id = log_id;
            }
        }

        // OS detection after the first successful connection (UI 12.4).
        if !self.os_checked {
            self.os_checked = true;
            if self.settings.os_detection_enabled {
                drop(self.shared.rt.spawn(detect_os(
                    self.shared.clone(),
                    self.session.clone(),
                    connection.clone(),
                    info.host,
                    info.os_detection_script.clone(),
                )));
            }
        }

        let decoder = OutputDecoder::new(info.encoding);
        // `None` only when the session was closed meanwhile (no update ran).
        let input = input.unwrap_or_else(super::InputReceiver::closed);
        let end = pump(
            &self.shared,
            &self.session,
            &mut self.ctrl,
            parts,
            decoder,
            input,
        )
        .await;
        let log_id = self.session.lock().log_id.take();
        drop(self.shared.rt.spawn(async move {
            let _ = tokio::time::timeout(CLOSE_TIMEOUT, connection.disconnect()).await;
        }));
        tracing::info!(session = %self.session.id, ?end, "shell ended");
        match end {
            PumpEnd::Closed => {
                self.shared
                    .end_log(log_id, ConnectionEndReason::Closed, None);
                Next::Exit
            }
            PumpEnd::Ended(TermEnd::ExitStatus(0)) => {
                self.shared
                    .end_log(log_id, ConnectionEndReason::Closed, Some(0));
                let late = self.shared.close_session(&self.session);
                self.shared
                    .end_log(late, ConnectionEndReason::Closed, Some(0));
                Next::Exit
            }
            PumpEnd::Ended(TermEnd::ExitStatus(status)) => {
                self.ended(log_id, status);
                Next::Idle
            }
            PumpEnd::Ended(TermEnd::ExitSignal | TermEnd::ExitUnknown) => {
                self.ended(log_id, 255);
                Next::Idle
            }
            PumpEnd::Ended(TermEnd::Dropped(kind)) => {
                self.shared
                    .end_log(log_id, ConnectionEndReason::Dropped, None);
                self.dropped(kind).await
            }
        }
    }

    fn ended(&self, log_id: Option<uuid::Uuid>, status: u32) {
        self.shared
            .end_log(log_id, ConnectionEndReason::ExitStatus, Some(status));
        self.update(|st| {
            let info = &mut st.info;
            info.state = SessionState::Ended;
            info.exit_status = Some(status);
            info.error = None;
            info.reconnect = None;
        });
    }

    /// The shell dropped: `kind` is the reason shown (`disconnected`,
    /// `network` or `timeout`).
    async fn dropped(&mut self, kind: FailureKind) -> Next {
        self.last_drop = ssh_session_error(SshError::new(match kind {
            FailureKind::Network | FailureKind::Timeout => kind,
            _ => FailureKind::Disconnected,
        }));
        let error = self.last_drop.clone();
        self.update(|st| {
            let info = &mut st.info;
            info.state = SessionState::Dropped;
            info.error = Some(error);
            info.reconnect = None;
        });
        if self.autoreconnect().await {
            Next::Schedule
        } else {
            Next::Idle
        }
    }

    async fn autoreconnect(&self) -> bool {
        self.shared
            .blocking(|core| SessionSettings::read(core).autoreconnect)
            .await
            .unwrap_or(true)
    }

    async fn failed(
        &mut self,
        kind: AttemptKind,
        error: SessionError,
        host_key: Option<UiKey>,
    ) -> Next {
        let revoked = error.code == "host_key_revoked";
        if !kind.is_reconnect() || revoked {
            self.update(|st| {
                let info = &mut st.info;
                info.state = SessionState::Failed;
                info.step = None;
                info.prompt = None;
                info.error = Some(error);
                info.host_key = host_key;
                info.reconnect = None;
            });
            return Next::Idle;
        }
        if stopped_by_user(&error) {
            return self.stopped(kind, error);
        }
        if !retry_automatically(&error) {
            // Wrong or unusable credentials, a refused channel, a host that
            // is gone: trying again every minute changes nothing (and trips
            // intrusion detection on the server). Automatic attempts stop
            // with this error; Reconnect stays available.
            tracing::info!(session = %self.session.id, code = %error.code, "automatic attempts stopped");
            self.last_drop = error.clone();
            self.update(|st| {
                let info = &mut st.info;
                info.state = SessionState::Dropped;
                info.step = None;
                info.prompt = None;
                info.error = Some(error);
                info.reconnect = None;
            });
            return Next::Idle;
        }
        self.last_drop = error.clone();
        self.update(|st| {
            let info = &mut st.info;
            info.state = SessionState::Dropped;
            info.step = None;
            info.prompt = None;
            info.error = Some(error);
            info.reconnect = None;
        });
        if kind == AttemptKind::Manual {
            self.backoff.reset();
        }
        if self.autoreconnect().await {
            Next::Schedule
        } else {
            Next::Idle
        }
    }

    /// The attempt was cancelled (Cancel, a cancelled prompt or dialog, a lock).
    fn stopped(&mut self, kind: AttemptKind, error: SessionError) -> Next {
        if kind.is_reconnect() {
            // Automatic attempts stop; the drop reason stays visible.
            let last = self.last_drop.clone();
            self.update(|st| {
                let info = &mut st.info;
                info.state = SessionState::Dropped;
                info.step = None;
                info.prompt = None;
                info.error = Some(last);
                info.reconnect = None;
            });
        } else {
            self.update(|st| {
                let info = &mut st.info;
                info.state = SessionState::Failed;
                info.step = None;
                info.prompt = None;
                info.error = Some(error);
                info.reconnect = None;
            });
        }
        Next::Idle
    }

    async fn idle(&mut self) -> Next {
        loop {
            match self.ctrl.recv().await {
                Some(Control::Reconnect) => {
                    let state = self.session.info().state;
                    return Next::Attempt(if state == SessionState::Dropped {
                        AttemptKind::Manual
                    } else {
                        AttemptKind::Retry
                    });
                }
                Some(Control::Cancel) => {}
                Some(Control::Close) | None => return Next::Exit,
            }
        }
    }

    /// Waits for automatic attempt `attempt`: `delay` (countdown) while
    /// unlocked; while locked until unlock, then at once.
    async fn wait_for_attempt(&mut self, attempt: u32, delay: Option<Duration>) -> Next {
        loop {
            // The drop reason stays visible while waiting (an attempt that
            // a lock aborted cleared it).
            let reason = self.last_drop.clone();
            if !*self.unlocked.borrow() {
                self.update(|st| {
                    st.info.state = SessionState::Dropped;
                    st.info.step = None;
                    st.info.prompt = None;
                    st.info.error = Some(reason);
                    st.info.reconnect = Some(ReconnectInfo {
                        attempt,
                        next_in_ms: None,
                        waiting_for_unlock: true,
                        in_progress: false,
                    });
                });
                return tokio::select! {
                    control = self.ctrl.recv() => match control {
                        Some(Control::Cancel) => self.stop_attempts(),
                        Some(Control::Reconnect) => Next::Attempt(AttemptKind::Manual),
                        Some(Control::Close) | None => Next::Exit,
                    },
                    () = wait_unlocked(&mut self.unlocked) => Next::Attempt(AttemptKind::Auto),
                };
            }
            let Some(delay) = delay else {
                return Next::Attempt(AttemptKind::Auto);
            };
            self.update(|st| {
                st.info.state = SessionState::Dropped;
                st.info.step = None;
                st.info.prompt = None;
                st.info.error = Some(reason);
                st.info.reconnect = Some(ReconnectInfo {
                    attempt,
                    next_in_ms: Some(u64::try_from(delay.as_millis()).unwrap_or(u64::MAX)),
                    waiting_for_unlock: false,
                    in_progress: false,
                });
            });
            tokio::select! {
                () = tokio::time::sleep(delay) => return Next::Attempt(AttemptKind::Auto),
                control = self.ctrl.recv() => match control {
                    Some(Control::Cancel) => return self.stop_attempts(),
                    Some(Control::Reconnect) => return Next::Attempt(AttemptKind::Manual),
                    Some(Control::Close) | None => return Next::Exit,
                },
                () = wait_locked(&mut self.unlocked) => {}
            }
        }
    }

    fn stop_attempts(&mut self) -> Next {
        tracing::info!(session = %self.session.id, "automatic attempts stopped");
        let reason = self.last_drop.clone();
        self.update(|st| {
            st.info.state = SessionState::Dropped;
            st.info.step = None;
            st.info.error = Some(reason);
            st.info.reconnect = None;
        });
        Next::Idle
    }
}

/// Failures an automatic reconnect attempt retries (UI 4.6): the server
/// could not be reached or the connection was lost. Authentication, key,
/// channel and protocol failures, and core errors (a removed or invalid
/// host), stop the automatic attempts.
fn retry_automatically(error: &SessionError) -> bool {
    error.code == "ssh_error"
        && matches!(
            error.reason.as_deref(),
            Some("not_found" | "refused" | "timeout" | "network" | "disconnected")
        )
}

fn disconnected() -> SessionError {
    SessionError {
        code: "ssh_error".to_owned(),
        reason: Some("disconnected".to_owned()),
    }
}

/// A cancel by the user (prompt or host key dialog) stops automatic attempts.
fn stopped_by_user(error: &SessionError) -> bool {
    error.code == "cancelled"
        || (error.code == "ssh_error" && error.reason.as_deref() == Some("host_key_rejected"))
}

/// Resolves when the app is locked.
async fn wait_locked(unlocked: &mut watch::Receiver<bool>) {
    if unlocked.wait_for(|unlocked| !*unlocked).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Resolves when the app is unlocked.
async fn wait_unlocked(unlocked: &mut watch::Receiver<bool>) {
    if unlocked.wait_for(|unlocked| *unlocked).await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// Agent discovery, connect, saving prompted secrets, shell.
async fn establish(
    shared: Arc<Shared>,
    session: Arc<Session>,
    mut spec: ConnectSpec,
    interaction: Arc<SessionInteraction>,
    saves: SaveTargets,
    shell: ShellSetup,
) -> Result<Established, SshError> {
    let agent = tokio::time::timeout(AGENT_DISCOVERY_TIMEOUT, shared.connector.discover_agent())
        .await
        .ok()
        .flatten();
    spec.auth.use_agent = agent.is_some();
    spec.agent = agent;
    let connection = shared
        .connector
        .connect(spec, interaction.clone() as Arc<dyn Interaction>)
        .await?;
    let username = keep_credentials(&shared, &session, saves, connection.take_auth()).await;
    session.update(&*shared.events, |st| {
        if st.info.state == SessionState::Connecting {
            st.info.step = Some(UiStep::OpeningSession);
        }
    });
    let (cols, rows) = session.lock().size;
    let parts = connection
        .open_shell(ShellRequest {
            term: shell.term,
            cols: u32::from(cols),
            rows: u32::from(rows),
            env: shell.env,
            agent_forwarding: shell.agent_forwarding,
        })
        .await?;
    Ok(Established {
        connection,
        parts,
        username,
        size: (cols, rows),
    })
}

/// Saves prompted secrets with "Save" checked and keeps them for reconnects
/// while unlocked (plan decision 23), bound to the server and user that
/// accepted them. Returns the username used.
async fn keep_credentials(
    shared: &Arc<Shared>,
    session: &Arc<Session>,
    saves: SaveTargets,
    auth: AuthResult,
) -> String {
    let AuthResult {
        username,
        prompted_password,
        prompted_passphrase,
    } = auth;
    if let Some(password) = &prompted_password
        && password.save
        && saves.can_save_password
        && let Some(host) = saves.host
    {
        let value = password.value.clone();
        match shared
            .blocking(move |core| core.save_prompted_password(host, value))
            .await
        {
            Some(Ok(())) => tracing::info!(session = %session.id, "prompted password saved"),
            Some(Err(error)) => {
                tracing::warn!(session = %session.id, code = error.code(), "password not saved");
            }
            None => {}
        }
    }
    if let Some(passphrase) = &prompted_passphrase
        && passphrase.save
        && let Some((key, true)) = saves.key
    {
        let value = passphrase.value.clone();
        match shared
            .blocking(move |core| core.save_key_passphrase(key, value))
            .await
        {
            Some(Ok(())) => tracing::info!(session = %session.id, "prompted passphrase saved"),
            Some(Err(error)) => {
                tracing::warn!(session = %session.id, code = error.code(), "passphrase not saved");
            }
            None => {}
        }
    }
    // The lock state is read under the session mutex: a lock flips it before
    // `on_lock` takes the same mutex, so either the values are stored before
    // `on_lock` wipes them, or the lock is seen here and nothing is stored.
    let mut st = session.lock();
    let unlocked = shared.is_unlocked();
    let bound = (
        saves.address.trim().to_lowercase(),
        saves.port,
        username.clone(),
    );
    if st.kept.bound.as_ref() != Some(&bound) {
        st.kept = super::KeptCredentials {
            bound: Some(bound),
            ..super::KeptCredentials::default()
        };
    }
    if unlocked {
        if let Some(password) = prompted_password {
            st.kept.password = Some(password.value);
        }
        if let Some(passphrase) = prompted_passphrase {
            st.kept.passphrase = Some(passphrase.value);
            st.kept.passphrase_key = saves.key.map(|(key, _)| key);
        }
    }
    if saves.username_missing {
        st.kept.username = Some(username.clone());
    }
    username
}

/// OS detection (UI 12.4; plan decision 24): the banner, else the script in
/// a separate exec channel for a saved host without `os`. Never touches the
/// shell and is never recorded.
async fn detect_os(
    shared: Arc<Shared>,
    session: Arc<Session>,
    connection: Arc<dyn RemoteConnection>,
    host: Option<RecordRef>,
    script: Option<String>,
) {
    let ident = connection.server_ident();
    let banner = shared
        .blocking(move |core| core.os_from_banner(&ident))
        .await
        .flatten();
    let (os, source) = match banner {
        Some(os) => (os, OsSource::Banner),
        None => {
            let (Some(script), Some(_)) = (script, host) else {
                return;
            };
            let timeout = Duration::from_secs(EXEC_TIMEOUT_SECS);
            let output = tokio::time::timeout(
                timeout + Duration::from_secs(1),
                connection.exec(script, timeout, MAX_OUTPUT_BYTES),
            )
            .await;
            let stdout = match output {
                Ok(Ok(output)) => output.stdout,
                Ok(Err(error)) => {
                    tracing::debug!(session = %session.id, reason = error.kind.reason(), "os detection failed");
                    return;
                }
                Err(_) => {
                    tracing::debug!(session = %session.id, "os detection timed out");
                    return;
                }
            };
            let text = String::from_utf8_lossy(&stdout).into_owned();
            match shared
                .blocking(move |core| core.os_from_script(&text))
                .await
                .flatten()
            {
                Some(os) => (os, OsSource::Script),
                None => return,
            }
        }
    };
    if let Some(host) = host {
        let detected = os.clone();
        match shared
            .blocking(move |core| core.host_os_detected(host, detected, source))
            .await
        {
            Some(Ok(written)) => tracing::debug!(session = %session.id, written, "os detected"),
            Some(Err(error)) => {
                tracing::debug!(session = %session.id, code = error.code(), "os not stored");
            }
            None => {}
        }
    }
    let os = host_os(&os, source);
    session.update(&*shared.events, |st| st.info.os = Some(os));
}

/// The [`Interaction`] of one attempt: steps, host key decision, prompts.
pub(crate) struct SessionInteraction {
    shared: Arc<Shared>,
    session: Arc<Session>,
    address: String,
    port: u16,
    revoked: Mutex<Option<UiKey>>,
    failure: Mutex<Option<SessionError>>,
    decided: AtomicBool,
}

impl SessionInteraction {
    fn new(shared: Arc<Shared>, session: Arc<Session>, address: String, port: u16) -> Self {
        Self {
            shared,
            session,
            address,
            port,
            revoked: Mutex::new(None),
            failure: Mutex::new(None),
            decided: AtomicBool::new(false),
        }
    }

    fn take_revoked(&self) -> Option<UiKey> {
        self.revoked
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    fn take_failure(&self) -> Option<SessionError> {
        self.failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    fn fail(&self, error: SessionError) {
        *self
            .failure
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(error);
    }

    /// The user accepted or replaced the host key in a dialog.
    fn decided(&self) -> bool {
        self.decided.load(Ordering::SeqCst)
    }

    async fn decide(&self, key: &HostKeyInfo) -> HostKeyDecision {
        let check = HostKeyCheck {
            address: self.address.clone(),
            port: self.port,
            key_type: key.key_type.clone(),
            public_key: key.public_key_base64.clone(),
        };
        let shown = UiKey {
            address: self.address.clone(),
            port: self.port,
            key_type: key.key_type.clone(),
            fingerprint: key.fingerprint.clone(),
        };
        let lookup = check.clone();
        let status = match self
            .shared
            .blocking(move |core| core.known_host_check(&lookup))
            .await
        {
            Some(Ok(status)) => status,
            Some(Err(error)) => {
                self.fail(core_session_error(&error));
                return HostKeyDecision::Reject;
            }
            None => {
                self.fail(error_code("internal"));
                return HostKeyDecision::Reject;
            }
        };
        match status {
            HostKeyStatus::Match { .. } => HostKeyDecision::Accept,
            HostKeyStatus::Revoked { .. } => {
                tracing::warn!(session = %self.session.id, "revoked host key presented");
                *self
                    .revoked
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(shown);
                HostKeyDecision::Reject
            }
            HostKeyStatus::Unknown => {
                let once_keys = self.session.lock().once_keys.clone();
                if once_keys.iter().any(|(key_type, blob)| {
                    *key_type == check.key_type && *blob == check.public_key
                }) {
                    return HostKeyDecision::Accept;
                }
                if !once_keys.is_empty() {
                    // This session trusted another key once: a different key
                    // now is a changed key, not a first connection (UI 10.2).
                    return self.once_key_changed(&check, shown, &once_keys).await;
                }
                let reply = ask(
                    &self.shared,
                    &self.session,
                    PromptKind::HostKeyUnknown { key: shown },
                )
                .await;
                match reply {
                    Some(PromptReply::HostKey {
                        decision: UiDecision::AcceptSave,
                    }) => {
                        let save = check.clone();
                        match self
                            .shared
                            .blocking(move |core| core.known_host_accept(&save))
                            .await
                        {
                            Some(Ok(())) => {
                                self.decided.store(true, Ordering::SeqCst);
                                HostKeyDecision::Accept
                            }
                            Some(Err(error)) => {
                                self.fail(core_session_error(&error));
                                HostKeyDecision::Reject
                            }
                            None => {
                                self.fail(error_code("internal"));
                                HostKeyDecision::Reject
                            }
                        }
                    }
                    Some(PromptReply::HostKey {
                        decision: UiDecision::ConnectOnce,
                    }) => {
                        self.session
                            .lock()
                            .once_keys
                            .push((check.key_type.clone(), check.public_key.clone()));
                        self.decided.store(true, Ordering::SeqCst);
                        HostKeyDecision::Accept
                    }
                    _ => HostKeyDecision::Reject,
                }
            }
            HostKeyStatus::Changed {
                record_id,
                old_key_type,
                old_fingerprint,
            } => {
                let reply = ask(
                    &self.shared,
                    &self.session,
                    PromptKind::HostKeyChanged {
                        key: shown,
                        old_key_type,
                        old_fingerprint,
                    },
                )
                .await;
                match reply {
                    Some(PromptReply::HostKey {
                        decision: UiDecision::Replace,
                    }) => {
                        let save = check.clone();
                        match self
                            .shared
                            .blocking(move |core| core.known_host_replace(record_id, &save))
                            .await
                        {
                            Some(Ok(())) => {
                                self.decided.store(true, Ordering::SeqCst);
                                HostKeyDecision::Accept
                            }
                            Some(Err(error)) => {
                                self.fail(core_session_error(&error));
                                HostKeyDecision::Reject
                            }
                            None => {
                                self.fail(error_code("internal"));
                                HostKeyDecision::Reject
                            }
                        }
                    }
                    _ => HostKeyDecision::Reject,
                }
            }
        }
    }
}

impl SessionInteraction {
    /// The presented key differs from the key this session accepted with
    /// "Connect once": the changed-key warning with the old fingerprint.
    /// "Replace" trusts the new key for this session only (nothing is
    /// saved, there is no record); anything else rejects it.
    async fn once_key_changed(
        &self,
        check: &HostKeyCheck,
        shown: UiKey,
        once_keys: &[(String, String)],
    ) -> HostKeyDecision {
        let (old_key_type, old_blob) = once_keys
            .iter()
            .find(|(key_type, _)| *key_type == check.key_type)
            .unwrap_or(&once_keys[0])
            .clone();
        let old_fingerprint =
            shellpp_core::known_hosts::fingerprint_sha256(&old_blob).unwrap_or_default();
        tracing::warn!(session = %self.session.id, "host key differs from the one accepted once");
        let reply = ask(
            &self.shared,
            &self.session,
            PromptKind::HostKeyChanged {
                key: shown,
                old_key_type,
                old_fingerprint,
            },
        )
        .await;
        match reply {
            Some(PromptReply::HostKey {
                decision: UiDecision::Replace,
            }) => {
                let mut st = self.session.lock();
                st.once_keys.clear();
                st.once_keys
                    .push((check.key_type.clone(), check.public_key.clone()));
                drop(st);
                self.decided.store(true, Ordering::SeqCst);
                HostKeyDecision::Accept
            }
            _ => HostKeyDecision::Reject,
        }
    }
}

/// Maps an SSH prompt to the UI prompt and whether "Save" may apply.
fn prompt_kind(prompt: Prompt) -> (PromptKind, bool) {
    match prompt {
        Prompt::Username => (PromptKind::Username, false),
        Prompt::Password {
            username,
            error,
            can_save,
        } => (
            PromptKind::Password {
                username: Some(username),
                error,
                can_save,
            },
            can_save,
        ),
        Prompt::Passphrase {
            key_label,
            error,
            can_save,
        } => (
            PromptKind::Passphrase {
                key_label: Some(key_label),
                error,
                can_save,
            },
            can_save,
        ),
        Prompt::KeyboardInteractive {
            name,
            instruction,
            prompt,
            echo,
            index,
            count,
            error,
        } => (
            PromptKind::KeyboardInteractive {
                name,
                instruction,
                prompt,
                echo,
                index,
                count,
                error,
            },
            false,
        ),
    }
}

impl Interaction for SessionInteraction {
    fn step(&self, step: ConnectStep) {
        let step = match step {
            ConnectStep::Resolving => UiStep::Resolving,
            ConnectStep::Connecting => UiStep::Connecting,
            ConnectStep::HostKey => UiStep::HostKey,
            ConnectStep::Authenticating => UiStep::Authenticating,
        };
        let current = self.session.lock().info.step;
        if current != Some(step) {
            self.session.update(&*self.shared.events, |st| {
                if st.info.state == SessionState::Connecting {
                    st.info.step = Some(step);
                }
            });
        }
    }

    fn verify_host_key<'a>(&'a self, key: &'a HostKeyInfo) -> BoxFuture<'a, HostKeyDecision> {
        Box::pin(self.decide(key))
    }

    fn prompt<'a>(&'a self, prompt: Prompt) -> BoxFuture<'a, Option<PromptAnswer>> {
        Box::pin(async move {
            let (kind, can_save) = prompt_kind(prompt);
            match ask(&self.shared, &self.session, kind).await {
                Some(PromptReply::Text { value, save }) => Some(PromptAnswer {
                    value,
                    save: save && can_save,
                }),
                _ => None,
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prompts_map_to_ui_prompts() {
        assert_eq!(prompt_kind(Prompt::Username), (PromptKind::Username, false));
        let (kind, can_save) = prompt_kind(Prompt::Password {
            username: "u".into(),
            error: true,
            can_save: true,
        });
        assert!(can_save);
        assert_eq!(
            kind,
            PromptKind::Password {
                username: Some("u".into()),
                error: true,
                can_save: true
            }
        );
        let (kind, can_save) = prompt_kind(Prompt::KeyboardInteractive {
            name: "n".into(),
            instruction: "i".into(),
            prompt: "Code: ".into(),
            echo: false,
            index: 1,
            count: 2,
            error: false,
        });
        assert!(!can_save);
        assert!(matches!(
            kind,
            PromptKind::KeyboardInteractive {
                echo: false,
                index: 1,
                count: 2,
                ..
            }
        ));
    }

    #[test]
    fn only_transport_failures_are_retried_automatically() {
        let ssh = |reason: &str| SessionError {
            code: "ssh_error".into(),
            reason: Some(reason.into()),
        };
        for reason in ["not_found", "refused", "timeout", "network", "disconnected"] {
            assert!(retry_automatically(&ssh(reason)), "{reason}");
        }
        for reason in [
            "auth_failed",
            "key_unusable",
            "channel_failed",
            "protocol",
            "host_key_rejected",
        ] {
            assert!(!retry_automatically(&ssh(reason)), "{reason}");
        }
        for code in ["not_found", "validation_failed", "internal", "locked"] {
            assert!(!retry_automatically(&error_code(code)), "{code}");
        }
    }

    #[test]
    fn user_cancels_stop_automatic_attempts() {
        assert!(stopped_by_user(&error_code("cancelled")));
        assert!(stopped_by_user(&SessionError {
            code: "ssh_error".into(),
            reason: Some("host_key_rejected".into())
        }));
        assert!(!stopped_by_user(&disconnected()));
        assert!(!stopped_by_user(&SessionError {
            code: "ssh_error".into(),
            reason: Some("refused".into())
        }));
    }
}
