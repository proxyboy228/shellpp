//! Session layer tests: the state machine of SSH sessions and local
//! terminals against the russh test server (`shellpp-test-sshd`) in process
//! and the real core in a temporary profile (in-memory secret store).
//!
//! - `ssh`: connecting, host key decisions (TOFU, connect once, changed,
//!   revoked), prompts (password, keyboard-interactive, username, key file
//!   passphrases), TERM, environment, charset and resize, the lock;
//! - `ends`: exit 0, non-zero exit, transport drop, keepalive timeout,
//!   automatic reconnection with backoff and its cancellation, first-attempt
//!   failures, the connection log, flow control;
//! - `local`: the local terminal (the machine's default shell).
//!
//! Keys and passwords are generated at run time; key files live in a
//! temporary directory and no agent is used, so nothing touches the real
//! `~/.ssh`, the real agent or a real host. Servers listen on 127.0.0.1.

mod background;
mod ends;
mod history;
mod local;
mod ssh;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt as _;
use futures::channel::mpsc::UnboundedReceiver;
use secrecy::SecretString;
use serde_json::{Value, json};
use shellpp_core::Core;
use shellpp_core::error::CoreError;
use shellpp_core::os_detect::DetectedOs;
use shellpp_core::service::dto::{
    OsSource, RecordRef, RecordSaveInput, RecordType, RecordView, SessionTarget, SettingsSetInput,
    SettingsValues, VaultKind,
};
use shellpp_core::service::host_api::{
    ConnectionEndReason, ConnectionLogStart, HostKeyCheck, HostKeyStatus, SessionConfig,
};
use shellpp_ssh::FailureKind;
use shellpp_test_sshd::{ServerConfig, TestServer, UserConfig};
use tokio::sync::watch;
use uuid::Uuid;

use super::*;
use crate::state::OpenedCore;
use crate::testing::TestProfile;

/// Longest wait for one step (connecting, a prompt, a reconnect).
pub(super) const WAIT: Duration = Duration::from_secs(30);

pub(super) fn secret(value: &str) -> SecretString {
    SecretString::from(value.to_owned())
}

/// A random secret for passwords and answers.
pub(super) fn random_secret(prefix: &str) -> String {
    let suffix = crate::profile::random_suffix().expect("random");
    format!("{prefix}-{suffix}")
}

/// Starts the russh test server with these users.
/// Polls `condition` until it holds or [`WAIT`] passes.
pub(super) async fn eventually_async<F, Fut>(mut condition: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        if condition().await {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub(super) async fn start_server(users: Vec<UserConfig>) -> TestServer {
    start_with(ServerConfig::new(users)).await
}

pub(super) async fn start_with(config: ServerConfig) -> TestServer {
    TestServer::start(config).await.expect("test server starts")
}

/// A connection log call seen by [`RecordingCore`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LogCall {
    Start {
        log_id: Uuid,
        host: Option<RecordRef>,
        username: Option<String>,
    },
    End {
        log_id: Uuid,
        reason: ConnectionEndReason,
        exit_status: Option<u32>,
    },
}

/// The real core, with the connection log calls recorded.
pub(super) struct RecordingCore {
    pub core: Arc<Core>,
    pub log: Mutex<Vec<LogCall>>,
    /// `session_config` answers `Locked` (the core locked before the
    /// session layer heard of it), counting the refusals.
    pub config_locked: std::sync::atomic::AtomicBool,
    pub config_refusals: AtomicUsize,
    /// Lines that reached `history_record`.
    pub history_calls: AtomicUsize,
    /// Reads of `record_history_now`.
    pub history_flag_reads: AtomicUsize,
}

impl RecordingCore {
    pub fn log(&self) -> Vec<LogCall> {
        self.log.lock().unwrap().clone()
    }

    pub fn starts(&self) -> Vec<Uuid> {
        self.log()
            .into_iter()
            .filter_map(|call| match call {
                LogCall::Start { log_id, .. } => Some(log_id),
                LogCall::End { .. } => None,
            })
            .collect()
    }

    pub fn end_of(&self, log_id: Uuid) -> Option<(ConnectionEndReason, Option<u32>)> {
        self.log().into_iter().find_map(|call| match call {
            LogCall::End {
                log_id: id,
                reason,
                exit_status,
            } if id == log_id => Some((reason, exit_status)),
            _ => None,
        })
    }
}

impl SessionCore for RecordingCore {
    fn is_unlocked(&self) -> bool {
        self.core.is_unlocked()
    }

    fn recording_for(
        &self,
        host: Option<shellpp_core::service::dto::RecordRef>,
        log_id: Uuid,
        size: (u16, u16),
    ) -> Option<shellpp_core::recording::RecordingWriter> {
        SessionCore::recording_for(&*self.core, host, log_id, size)
    }

    fn agent_keys(&self) -> Result<Vec<shellpp_core::service::host_api::SessionKey>, CoreError> {
        self.core.agent_keys()
    }

    fn forwarding_rules(
        &self,
    ) -> Result<Vec<shellpp_core::service::snippets::ForwardingRuleView>, CoreError> {
        self.core.forwarding_rules_personal()
    }

    fn settings(&self) -> Result<SettingsValues, CoreError> {
        SessionCore::settings(&*self.core)
    }

    fn session_config(&self, target: &SessionTarget) -> Result<SessionConfig, CoreError> {
        if self.config_locked.load(Ordering::SeqCst) {
            self.config_refusals.fetch_add(1, Ordering::SeqCst);
            return Err(CoreError::Locked);
        }
        self.core.session_config(target)
    }

    fn known_host_check(&self, check: &HostKeyCheck) -> Result<HostKeyStatus, CoreError> {
        self.core.known_host_check(check)
    }

    fn known_host_accept(&self, check: &HostKeyCheck) -> Result<(), CoreError> {
        SessionCore::known_host_accept(&*self.core, check)
    }

    fn known_host_replace(&self, record_id: Uuid, check: &HostKeyCheck) -> Result<(), CoreError> {
        SessionCore::known_host_replace(&*self.core, record_id, check)
    }

    fn save_prompted_password(
        &self,
        host: RecordRef,
        password: SecretString,
    ) -> Result<(), CoreError> {
        self.core.save_prompted_password(host, password)
    }

    fn save_key_passphrase(
        &self,
        key: RecordRef,
        passphrase: SecretString,
    ) -> Result<(), CoreError> {
        self.core.save_key_passphrase(key, passphrase)
    }

    fn host_os_detected(
        &self,
        host: RecordRef,
        os: DetectedOs,
        source: OsSource,
    ) -> Result<bool, CoreError> {
        self.core.host_os_detected(host, os, source)
    }

    fn connection_log_start(&self, entry: ConnectionLogStart) -> Result<Uuid, CoreError> {
        let host = entry.host;
        let username = entry.username.clone();
        let log_id = self.core.connection_log_start(entry)?;
        self.log.lock().unwrap().push(LogCall::Start {
            log_id,
            host,
            username,
        });
        Ok(log_id)
    }

    fn connection_log_end(
        &self,
        log_id: Uuid,
        reason: ConnectionEndReason,
        exit_status: Option<u32>,
    ) {
        self.core.connection_log_end(log_id, reason, exit_status);
        self.log.lock().unwrap().push(LogCall::End {
            log_id,
            reason,
            exit_status,
        });
    }

    fn history_record(
        &self,
        input: shellpp_core::service::host_api::HistoryRecordInput,
    ) -> Result<shellpp_core::service::host_api::HistoryDecision, CoreError> {
        self.history_calls.fetch_add(1, Ordering::SeqCst);
        self.core.history_record(input)
    }

    fn record_history_now(&self, host: RecordRef) -> bool {
        let on = SessionCore::record_history_now(&*self.core, host);
        self.history_flag_reads.fetch_add(1, Ordering::SeqCst);
        on
    }
}

/// The terminal stand-in: records the output, keeps the session handle,
/// answers the cursor position query of ConPTY, and can hold its feed
/// (a terminal that is behind, for flow control).
pub(super) struct TestTerminal {
    output: Mutex<Vec<u8>>,
    handle: Mutex<Option<SessionHandle>>,
    open: watch::Sender<bool>,
    feeds: AtomicUsize,
    /// What the session injected (a startup snippet).
    injected: Mutex<Vec<u8>>,
}

impl Default for TestTerminal {
    fn default() -> Self {
        Self {
            output: Mutex::new(Vec::new()),
            handle: Mutex::new(None),
            open: watch::Sender::new(true),
            feeds: AtomicUsize::new(0),
            injected: Mutex::new(Vec::new()),
        }
    }
}

impl TestTerminal {
    /// The input the session injected so far.
    pub fn injected(&self) -> Vec<u8> {
        self.injected.lock().unwrap().clone()
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.output.lock().unwrap()).into_owned()
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.output.lock().unwrap().clone()
    }

    pub fn len(&self) -> usize {
        self.output.lock().unwrap().len()
    }

    /// Feed calls so far (including one waiting at the gate).
    pub fn feeds(&self) -> usize {
        self.feeds.load(Ordering::SeqCst)
    }

    /// Holds (`false`) or lets through (`true`) the output.
    pub fn set_open(&self, open: bool) {
        self.open.send_replace(open);
    }

    pub fn attached(&self) -> Option<SessionHandle> {
        self.handle.lock().unwrap().clone()
    }
}

impl SessionTerminal for TestTerminal {
    fn feed<'a>(&'a self, bytes: &'a [u8]) -> BoxFuture<'a, Result<(), TerminalClosed>> {
        Box::pin(async move {
            self.feeds.fetch_add(1, Ordering::SeqCst);
            let mut open = self.open.subscribe();
            let _ = open.wait_for(|open| *open).await;
            self.output.lock().unwrap().extend_from_slice(bytes);
            // ConPTY waits for the answer to its cursor position query.
            if bytes.windows(4).any(|window| window == b"\x1b[6n")
                && let Some(handle) = self.attached()
            {
                let _ = handle.write(b"\x1b[1;1R");
            }
            Ok(())
        })
    }

    fn attach(&self, session: SessionHandle) {
        *self.handle.lock().unwrap() = Some(session);
    }

    fn inject(&self, bytes: Vec<u8>) {
        self.injected.lock().unwrap().extend_from_slice(&bytes);
        if let Some(handle) = self.attached() {
            let _ = handle.write(&bytes);
        }
    }
}

/// One open session as a test sees it.
pub(super) struct TestSession {
    pub handle: SessionHandle,
    pub events: UnboundedReceiver<SessionEvent>,
    pub terminal: Arc<TestTerminal>,
    pub view: SessionView,
    /// Every state the session went through, in order (repeats collapsed).
    pub states: Vec<SessionState>,
    /// Every prompt shown, by kind name, in order.
    pub prompts: Vec<&'static str>,
    /// The id of the last prompt answered (a newer one is waited for).
    pub answered: u64,
}

impl TestSession {
    fn apply(&mut self, view: SessionView) {
        if self.states.last() != Some(&view.state) {
            self.states.push(view.state);
        }
        if let Some(prompt) = &view.prompt
            && self
                .view
                .prompt
                .as_ref()
                .is_none_or(|shown| shown.id != prompt.id)
        {
            self.prompts.push(prompt.kind.name());
        }
        self.view = view;
    }

    /// Takes every event received so far.
    pub fn drain(&mut self) {
        while let Ok(SessionEvent::Changed(view)) = self.events.try_recv() {
            self.apply(view);
        }
    }

    /// Waits until `condition` holds for the current view.
    pub async fn until(
        &mut self,
        what: &str,
        condition: impl Fn(&SessionView) -> bool,
    ) -> SessionView {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            if condition(&self.view) {
                return self.view.clone();
            }
            match tokio::time::timeout_at(deadline, self.events.next()).await {
                Ok(Some(SessionEvent::Changed(view))) => self.apply(view),
                Ok(None) => panic!(
                    "events ended while waiting for {what}; view {:?}; states {:?}",
                    self.view, self.states
                ),
                Err(_) => panic!(
                    "timeout waiting for {what}; view {:?}; states {:?}; prompts {:?}",
                    self.view, self.states, self.prompts
                ),
            }
        }
    }

    pub async fn state(&mut self, state: SessionState) -> SessionView {
        self.until(&format!("{state:?}"), |view| view.state == state)
            .await
    }

    /// Waits for a prompt named `name`; returns its id and kind.
    pub async fn prompt(&mut self, name: &str) -> (u64, PromptKind) {
        let answered = self.answered;
        let view = self
            .until(&format!("a {name} prompt"), |view| {
                view.prompt
                    .as_ref()
                    .is_some_and(|p| p.kind.name() == name && p.id > answered)
            })
            .await;
        let prompt = view.prompt.expect("prompt");
        (prompt.id, prompt.kind)
    }

    pub fn reply(&mut self, id: u64, reply: PromptReply) {
        self.answered = self.answered.max(id);
        self.handle
            .reply(id, reply)
            .expect("the prompt takes the reply");
    }

    /// Answers the next `name` prompt with `value`.
    pub async fn answer(&mut self, name: &str, value: &str, save: bool) -> PromptKind {
        let (id, kind) = self.prompt(name).await;
        self.reply(
            id,
            PromptReply::Text {
                value: secret(value),
                save,
            },
        );
        kind
    }

    /// Accepts and saves an unknown host key; returns its fingerprint.
    pub async fn accept_key(&mut self) -> String {
        let (id, kind) = self.prompt("host_key_unknown").await;
        let PromptKind::HostKeyUnknown { key } = kind else {
            unreachable!("named host_key_unknown")
        };
        self.reply(id, PromptReply::HostKey(HostKeyChoice::AcceptAndSave));
        key.fingerprint
    }

    /// Types a line (Enter is CR).
    pub fn type_line(&self, line: &str) {
        self.handle
            .write(format!("{line}\r").as_bytes())
            .expect("input taken");
    }

    /// Waits until the output from byte `from` on contains `needle`.
    pub async fn output_from(&mut self, from: usize, needle: &str) {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let bytes = self.terminal.bytes();
            if bytes.len() >= from && String::from_utf8_lossy(&bytes[from..]).contains(needle) {
                return;
            }
            self.drain();
            assert!(
                tokio::time::Instant::now() < deadline,
                "timeout waiting for {needle:?} in the output; view {:?}",
                self.view
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    pub async fn output(&mut self, needle: &str) {
        self.output_from(0, needle).await;
    }

    /// Waits until the terminal got any output.
    pub async fn any_output(&mut self) {
        let deadline = tokio::time::Instant::now() + WAIT;
        while self.terminal.len() == 0 {
            self.drain();
            assert!(
                tokio::time::Instant::now() < deadline,
                "no output; view {:?}",
                self.view
            );
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    /// Runs `echo <marker>` and waits for the marker's output line.
    pub async fn echo_round_trip(&mut self, marker: &str) {
        let from = self.terminal.len();
        self.type_line(&format!("echo {marker}"));
        self.output_from(from, &format!("{marker}\r\n")).await;
    }
}

/// The real core in a temporary profile, the session manager on the test's
/// runtime and a temporary key directory.
pub(super) struct Harness {
    pub manager: SessionManager,
    pub calls: Arc<RecordingCore>,
    pub core: Arc<Core>,
    pub vault_id: Uuid,
    pub key_dir: tempfile::TempDir,
    _opened: OpenedCore,
    _profile: TestProfile,
}

impl Harness {
    /// No key files are offered unless a test writes some into `key_dir`.
    pub fn new() -> Self {
        Self::with(|_| {})
    }

    pub fn with(configure: impl FnOnce(&mut ManagerOptions)) -> Self {
        let mut profile = TestProfile::new();
        let opened = profile.open_unlocked();
        let core = opened.core.clone();
        let vault_id = core
            .vaults()
            .expect("vaults")
            .into_iter()
            .find(|vault| vault.kind == VaultKind::Personal)
            .expect("personal vault")
            .vault_id;
        let key_dir = tempfile::tempdir().expect("key directory");
        let calls = Arc::new(RecordingCore {
            core: core.clone(),
            log: Mutex::new(Vec::new()),
            config_locked: Default::default(),
            config_refusals: AtomicUsize::new(0),
            history_calls: AtomicUsize::new(0),
            history_flag_reads: AtomicUsize::new(0),
        });
        let mut options = ManagerOptions::with_keys(KeySources {
            key_dir: Some(key_dir.path().to_path_buf()),
            agent: AgentSource::None,
        });
        options.runtime = Some(tokio::runtime::Handle::current());
        configure(&mut options);
        let manager =
            SessionManager::start(calls.clone() as Arc<dyn SessionCore>, options).expect("start");
        Self {
            manager,
            calls,
            core,
            vault_id,
            key_dir,
            _opened: opened,
            _profile: profile,
        }
    }

    /// Saves a host at 127.0.0.1 on `port` with `extra` data members and an
    /// optional saved password; returns its target.
    pub fn host(&self, port: u16, extra: Value, password: Option<&str>) -> SessionTarget {
        let mut data = json!({
            "address": "127.0.0.1",
            "label": "test host",
            "ssh": {"port": port}
        });
        let object = data.as_object_mut().expect("object");
        if let Some(extra) = extra.as_object() {
            for (name, value) in extra {
                if name == "ssh" {
                    let ssh = object
                        .get_mut("ssh")
                        .and_then(Value::as_object_mut)
                        .expect("ssh");
                    for (member, value) in value.as_object().expect("ssh object") {
                        ssh.insert(member.clone(), value.clone());
                    }
                } else {
                    object.insert(name.clone(), value.clone());
                }
            }
        }
        let mut secrets = std::collections::BTreeMap::new();
        if let Some(password) = password {
            secrets.insert("ssh.password".to_owned(), Some(secret(password)));
        }
        let view = self
            .core
            .record_save(RecordSaveInput {
                vault_id: self.vault_id,
                record_id: None,
                record_type: RecordType::Host,
                data: object.clone(),
                secrets,
            })
            .expect("host saved");
        let RecordView::Host(host) = view else {
            panic!("expected a host");
        };
        SessionTarget::Host {
            vault_id: self.vault_id,
            record_id: host.record_id,
        }
    }

    /// A saved host for `user` with a saved password.
    pub fn password_host(&self, server: &TestServer, user: &str, password: &str) -> SessionTarget {
        self.host(
            server.port(),
            json!({"ssh": {"username": user}}),
            Some(password),
        )
    }

    pub fn set(&self, key: &str, value: Value) {
        let mut values = std::collections::BTreeMap::new();
        values.insert(key.to_owned(), value);
        self.core
            .settings_set(SettingsSetInput {
                values,
                reset: Vec::new(),
            })
            .expect("setting stored");
    }

    pub fn open_with(
        &self,
        request: OpenRequest,
        terminal: Arc<TestTerminal>,
    ) -> Result<TestSession, SessionError> {
        let opened = self.manager.open(
            request,
            (80, 24),
            terminal.clone() as Arc<dyn SessionTerminal>,
        )?;
        let mut session = TestSession {
            view: opened.handle.view(),
            handle: opened.handle,
            events: opened.events,
            terminal,
            states: Vec::new(),
            prompts: Vec::new(),
            answered: 0,
        };
        session.drain();
        Ok(session)
    }

    pub fn open(&self, target: SessionTarget) -> TestSession {
        self.open_with(OpenRequest::Ssh(target), Arc::new(TestTerminal::default()))
            .expect("session opens")
    }

    /// Opens a session to `target`, accepting the host key when asked, and
    /// waits until it is connected.
    pub async fn connect(&self, target: SessionTarget, first: bool) -> TestSession {
        let mut session = self.open(target);
        if first {
            session.accept_key().await;
        }
        session.state(SessionState::Connected).await;
        session
    }
}
