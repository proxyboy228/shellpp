//! Session manager tests (stage 2 plan, section 4.5 "Tests"): the state
//! machine against an in-memory core, a scripted SSH connector with a fake
//! shell and a fake local backend. No Tauri runtime: events and output go to
//! collectors. The tests against the real SSH client and `shellpp-test-sshd`
//! are in `tests/sessions_sshd.rs`.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use shellpp_core::error::CoreError;
use shellpp_core::os_detect::DetectedOs;
use shellpp_core::service::dto::{
    Backspace, ConnectStep, HistoryCandidate, HostKeyDecision as UiDecision, OsFamily, OsSource,
    PromptKind, PromptReply, RecordRef, SessionInfo, SessionInjectInput, SessionKind,
    SessionOpenInput, SessionPromptReplyInput, SessionResizeInput, SessionState, SessionTarget,
    SessionWriteInput, SettingsValues,
};
use shellpp_core::service::host_api::{
    ConnectionEndReason, ConnectionLogStart, HistoryDecision, HistoryDropReason,
    HistoryRecordInput, HostKeyCheck, HostKeyStatus, SessionConfig,
};
use shellpp_ssh::{
    BoxFuture, ConnectSpec, ExecOutput, FailureKind, HostKeyDecision, HostKeyInfo, Interaction,
    Prompt, ShellRequest, SshError, SystemAgent,
};
use tokio::sync::mpsc;
use uuid::Uuid;

use super::output::tests::Collector;
use super::transport::{
    AuthResult, Connector, PromptedSecret, RemoteConnection, TermEnd, TermEvent, TermEvents,
    TermInput, TermParts,
};
use super::*;

const WINDOW: &str = "w-1";
const OTHER_WINDOW: &str = "w-2";
const WAIT: Duration = Duration::from_secs(10);

fn host_ref() -> RecordRef {
    RecordRef {
        vault_id: Uuid::parse_str("0199f6a1-b2c3-7d4e-8f00-000000000001").unwrap(),
        record_id: Uuid::parse_str("0199f6a1-b2c3-7d4e-8f00-000000000002").unwrap(),
    }
}

fn host_target() -> SessionTarget {
    let host = host_ref();
    SessionTarget::Host {
        vault_id: host.vault_id,
        record_id: host.record_id,
    }
}

fn quick_target() -> SessionTarget {
    SessionTarget::Quick {
        address: "192.0.2.10".into(),
        port: Some(2222),
        username: None,
    }
}

fn key(n: u8) -> HostKeyInfo {
    HostKeyInfo {
        key_type: "ssh-ed25519".into(),
        public_key_base64: format!("AAAAkey{n}"),
        fingerprint: format!("SHA256:key{n}"),
    }
}

/// A runtime-generated secret (CONTRIBUTING: no fixed passwords).
fn secret(tag: &str) -> String {
    format!("{tag}-{}", Uuid::now_v7().simple())
}

// ---------------------------------------------------------------------------
// Fake core
// ---------------------------------------------------------------------------

type ConfigFn = Arc<dyn Fn(&SessionTarget) -> Result<SessionConfig, CoreError> + Send + Sync>;
type OsWrite = (RecordRef, OsFamily, Option<String>, OsSource);

struct KnownEntry {
    id: Uuid,
    address: String,
    port: u16,
    key_type: String,
    blob: String,
    revoked: bool,
}

struct FakeCore {
    unlocked: AtomicBool,
    settings: Mutex<SettingsValues>,
    config: Mutex<ConfigFn>,
    known: Mutex<Vec<KnownEntry>>,
    saved_passwords: Mutex<Vec<(RecordRef, String)>>,
    saved_passphrases: Mutex<Vec<(RecordRef, String)>>,
    history: Mutex<Vec<(String, bool, Option<RecordRef>)>>,
    /// What `record_history_now` answers.
    record_now: AtomicBool,
    logs: Mutex<Vec<String>>,
    os_written: Mutex<Vec<OsWrite>>,
}

fn base_config(target: &SessionTarget) -> SessionConfig {
    let (host, address, port, username) = match target {
        SessionTarget::Host { .. } => (
            Some(host_ref()),
            "192.0.2.5".to_owned(),
            22,
            Some("alice".to_owned()),
        ),
        SessionTarget::Quick {
            address,
            port,
            username,
        } => (None, address.clone(), port.unwrap_or(22), username.clone()),
    };
    SessionConfig {
        host,
        title: host.map_or_else(|| address.clone(), |_| "web".to_owned()),
        address,
        port,
        username,
        password: None,
        key: None,
        keepalive_s: 30,
        term: "xterm-256color".into(),
        agent_forwarding: false,
        env: Vec::new(),
        charset: "utf-8".into(),
        backspace: Backspace::Default,
        terminal_theme: json!({"builtin": "graphite"}),
        record_history: true,
        os_detection_script: None,
        os: None,
        known_key_types: Vec::new(),
        can_save_password: host.is_some(),
    }
}

impl FakeCore {
    fn new() -> Self {
        Self {
            unlocked: AtomicBool::new(true),
            settings: Mutex::new(SettingsValues::new()),
            config: Mutex::new(Arc::new(|target| Ok(base_config(target)))),
            known: Mutex::new(Vec::new()),
            saved_passwords: Mutex::new(Vec::new()),
            saved_passphrases: Mutex::new(Vec::new()),
            history: Mutex::new(Vec::new()),
            record_now: AtomicBool::new(true),
            logs: Mutex::new(Vec::new()),
            os_written: Mutex::new(Vec::new()),
        }
    }

    fn set_config(&self, config: impl Fn(&SessionTarget) -> SessionConfig + Send + Sync + 'static) {
        *self.config.lock().unwrap() = Arc::new(move |target| Ok(config(target)));
    }

    fn add_known(&self, address: &str, port: u16, key: &HostKeyInfo, revoked: bool) {
        self.known.lock().unwrap().push(KnownEntry {
            id: Uuid::now_v7(),
            address: address.to_owned(),
            port,
            key_type: key.key_type.clone(),
            blob: key.public_key_base64.clone(),
            revoked,
        });
    }

    fn known_count(&self) -> usize {
        self.known.lock().unwrap().len()
    }

    fn logs(&self) -> Vec<String> {
        self.logs.lock().unwrap().clone()
    }
}

impl SessionCore for FakeCore {
    fn is_unlocked(&self) -> bool {
        self.unlocked.load(Ordering::SeqCst)
    }

    fn settings(&self) -> Result<SettingsValues, CoreError> {
        Ok(self.settings.lock().unwrap().clone())
    }

    fn session_config(&self, target: &SessionTarget) -> Result<SessionConfig, CoreError> {
        if !self.is_unlocked() {
            return Err(CoreError::Locked);
        }
        let config = self.config.lock().unwrap().clone();
        config(target)
    }

    fn known_host_check(&self, check: &HostKeyCheck) -> Result<HostKeyStatus, CoreError> {
        if !self.is_unlocked() {
            return Err(CoreError::Locked);
        }
        let known = self.known.lock().unwrap();
        let same_key =
            |entry: &KnownEntry| entry.key_type == check.key_type && entry.blob == check.public_key;
        if let Some(entry) = known.iter().find(|entry| entry.revoked && same_key(entry)) {
            return Ok(HostKeyStatus::Revoked {
                record_id: entry.id,
            });
        }
        let matching: Vec<&KnownEntry> = known
            .iter()
            .filter(|entry| entry.address == check.address && entry.port == check.port)
            .collect();
        if let Some(entry) = matching.iter().find(|entry| same_key(entry)) {
            return Ok(HostKeyStatus::Match {
                record_id: entry.id,
            });
        }
        match matching.first() {
            Some(entry) => Ok(HostKeyStatus::Changed {
                record_id: entry.id,
                old_key_type: entry.key_type.clone(),
                old_fingerprint: format!("SHA256:{}", entry.blob.trim_start_matches("AAAA")),
            }),
            None => Ok(HostKeyStatus::Unknown),
        }
    }

    fn known_host_accept(&self, check: &HostKeyCheck) -> Result<(), CoreError> {
        self.known.lock().unwrap().push(KnownEntry {
            id: Uuid::now_v7(),
            address: check.address.clone(),
            port: check.port,
            key_type: check.key_type.clone(),
            blob: check.public_key.clone(),
            revoked: false,
        });
        Ok(())
    }

    fn known_host_replace(&self, record_id: Uuid, check: &HostKeyCheck) -> Result<(), CoreError> {
        let mut known = self.known.lock().unwrap();
        let entry = known
            .iter_mut()
            .find(|entry| entry.id == record_id)
            .ok_or(CoreError::NotFound)?;
        entry.key_type = check.key_type.clone();
        entry.blob = check.public_key.clone();
        Ok(())
    }

    fn save_prompted_password(
        &self,
        host: RecordRef,
        password: SecretString,
    ) -> Result<(), CoreError> {
        self.saved_passwords
            .lock()
            .unwrap()
            .push((host, password.expose_secret().to_owned()));
        Ok(())
    }

    fn save_key_passphrase(
        &self,
        key: RecordRef,
        passphrase: SecretString,
    ) -> Result<(), CoreError> {
        self.saved_passphrases
            .lock()
            .unwrap()
            .push((key, passphrase.expose_secret().to_owned()));
        Ok(())
    }

    fn os_from_banner(&self, ident: &str) -> Option<DetectedOs> {
        ident.contains("Ubuntu").then(|| DetectedOs {
            family: OsFamily::Linux,
            distro: Some("ubuntu".into()),
            version: None,
        })
    }

    fn os_from_script(&self, stdout: &str) -> Option<DetectedOs> {
        (stdout.starts_with("Linux") && stdout.contains("ID=debian")).then(|| DetectedOs {
            family: OsFamily::Linux,
            distro: Some("debian".into()),
            version: Some("12".into()),
        })
    }

    fn host_os_detected(
        &self,
        host: RecordRef,
        os: DetectedOs,
        source: OsSource,
    ) -> Result<bool, CoreError> {
        self.os_written
            .lock()
            .unwrap()
            .push((host, os.family, os.distro, source));
        Ok(true)
    }

    fn history_record(&self, input: HistoryRecordInput) -> Result<HistoryDecision, CoreError> {
        self.history.lock().unwrap().push((
            input.candidate.command.clone(),
            input.injected,
            input.host,
        ));
        Ok(if input.injected {
            HistoryDecision::Dropped(HistoryDropReason::Injected)
        } else {
            HistoryDecision::Recorded
        })
    }

    fn record_history_now(&self, _host: Option<RecordRef>) -> Result<bool, CoreError> {
        if !self.is_unlocked() {
            return Err(CoreError::Locked);
        }
        Ok(self.record_now.load(Ordering::SeqCst))
    }

    fn connection_log_start(&self, entry: ConnectionLogStart) -> Result<Uuid, CoreError> {
        let id = Uuid::now_v7();
        self.logs
            .lock()
            .unwrap()
            .push(format!("start host={}", entry.host.is_some()));
        Ok(id)
    }

    fn connection_log_end(
        &self,
        _log_id: Uuid,
        reason: ConnectionEndReason,
        exit_status: Option<u32>,
    ) {
        self.logs
            .lock()
            .unwrap()
            .push(format!("end {reason:?} {exit_status:?}"));
    }
}

// ---------------------------------------------------------------------------
// Fake shell
// ---------------------------------------------------------------------------

/// The server side of a fake shell: what it received, and a way to send output.
#[derive(Clone)]
struct ShellHandle {
    tx: mpsc::UnboundedSender<TermEvent>,
    received: Arc<Mutex<Vec<u8>>>,
    resizes: Arc<Mutex<Vec<(u16, u16)>>>,
    request: (String, u32, u32),
    /// The input side was closed (the session ended the shell).
    closed: Arc<AtomicBool>,
}

impl ShellHandle {
    fn received(&self) -> Vec<u8> {
        self.received.lock().unwrap().clone()
    }

    fn send(&self, data: &[u8]) {
        let _ = self.tx.send(TermEvent::Data(data.to_vec()));
    }
}

struct ChanEvents(mpsc::UnboundedReceiver<TermEvent>);

impl TermEvents for ChanEvents {
    fn next(&mut self) -> BoxFuture<'_, TermEvent> {
        Box::pin(async move {
            self.0
                .recv()
                .await
                .unwrap_or(TermEvent::End(TermEnd::Dropped(FailureKind::Disconnected)))
        })
    }
}

/// Runs lines like the test server's shell: `echo <text>`, `exit <n>`,
/// `signal`, `drop`. Typed input is not echoed (keeps assertions exact).
struct LineShell {
    tx: mpsc::UnboundedSender<TermEvent>,
    received: Arc<Mutex<Vec<u8>>>,
    resizes: Arc<Mutex<Vec<(u16, u16)>>>,
    line: Mutex<Vec<u8>>,
    /// The remote side stopped reading: writes never complete.
    stalled: bool,
    closed: Arc<AtomicBool>,
}

impl TermInput for LineShell {
    fn write(&self, data: Vec<u8>) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            if self.stalled {
                std::future::pending::<()>().await;
            }
            self.received.lock().unwrap().extend_from_slice(&data);
            for byte in data {
                if byte != b'\r' {
                    self.line.lock().unwrap().push(byte);
                    continue;
                }
                let line = std::mem::take(&mut *self.line.lock().unwrap());
                let event = if let Some(text) = line.strip_prefix(b"echo ") {
                    let mut out = text.to_vec();
                    out.extend_from_slice(b"\r\n");
                    TermEvent::Data(out)
                } else if let Some(code) = line.strip_prefix(b"exit ") {
                    let code = std::str::from_utf8(code).unwrap().parse().unwrap();
                    TermEvent::End(TermEnd::ExitStatus(code))
                } else if line == b"signal" {
                    TermEvent::End(TermEnd::ExitSignal)
                } else if line == b"drop" {
                    TermEvent::End(TermEnd::Dropped(FailureKind::Disconnected))
                } else if line == b"timeout" {
                    TermEvent::End(TermEnd::Dropped(FailureKind::Timeout))
                } else {
                    continue;
                };
                let _ = self.tx.send(event);
            }
            true
        })
    }

    fn resize(&self, cols: u16, rows: u16) -> BoxFuture<'_, bool> {
        Box::pin(async move {
            self.resizes.lock().unwrap().push((cols, rows));
            true
        })
    }

    fn close(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.closed.store(true, Ordering::SeqCst);
            let _ = self.tx.send(TermEvent::End(TermEnd::ExitStatus(0)));
        })
    }
}

fn fake_shell(motd: &[u8], request: (String, u32, u32)) -> (TermParts, ShellHandle) {
    fake_shell_with(motd, request, false)
}

fn fake_shell_with(
    motd: &[u8],
    request: (String, u32, u32),
    stalled: bool,
) -> (TermParts, ShellHandle) {
    let (tx, rx) = mpsc::unbounded_channel();
    let received = Arc::new(Mutex::new(Vec::new()));
    let resizes = Arc::new(Mutex::new(Vec::new()));
    let closed = Arc::new(AtomicBool::new(false));
    if !motd.is_empty() {
        let _ = tx.send(TermEvent::Data(motd.to_vec()));
    }
    let handle = ShellHandle {
        tx: tx.clone(),
        received: received.clone(),
        resizes: resizes.clone(),
        request,
        closed: closed.clone(),
    };
    let parts = TermParts {
        events: Box::new(ChanEvents(rx)),
        input: Arc::new(LineShell {
            tx,
            received,
            resizes,
            line: Mutex::new(Vec::new()),
            stalled,
            closed,
        }),
    };
    (parts, handle)
}

// ---------------------------------------------------------------------------
// Fake connector
// ---------------------------------------------------------------------------

#[derive(Clone)]
struct Behavior {
    refuse: bool,
    hang: bool,
    key: HostKeyInfo,
    password: Option<String>,
    ident: String,
    motd: Vec<u8>,
    exec_stdout: Option<String>,
    /// The first attempt fails with `Disconnected` after the host key decision.
    close_during_decision: bool,
    /// Every attempt fails with this after the host key decision.
    fail_with: Option<FailureKind>,
    /// Opening the shell takes this long.
    shell_delay: Option<Duration>,
    /// Shells never take input.
    stall_input: bool,
}

impl Default for Behavior {
    fn default() -> Self {
        Self {
            refuse: false,
            hang: false,
            key: key(1),
            password: None,
            ident: "SSH-2.0-fake".into(),
            motd: b"Welcome\r\n$ ".to_vec(),
            exec_stdout: None,
            close_during_decision: false,
            fail_with: None,
            shell_delay: None,
            stall_input: false,
        }
    }
}

struct SpecSeen {
    username: Option<String>,
    password: Option<String>,
    preferred: Vec<String>,
    keepalive: Option<Duration>,
}

#[derive(Default)]
struct FakeConnector {
    behavior: Mutex<Behavior>,
    attempts: AtomicUsize,
    specs: Mutex<Vec<SpecSeen>>,
    shells: Arc<Mutex<Vec<ShellHandle>>>,
    execs: Arc<AtomicUsize>,
}

impl FakeConnector {
    fn set(&self, change: impl FnOnce(&mut Behavior)) {
        change(&mut self.behavior.lock().unwrap());
    }

    fn shell(&self, index: usize) -> ShellHandle {
        self.shells.lock().unwrap()[index].clone()
    }

    fn shell_count(&self) -> usize {
        self.shells.lock().unwrap().len()
    }
}

struct FakeConnection {
    behavior: Behavior,
    username: String,
    prompted: Mutex<Option<(String, bool)>>,
    shells: Arc<Mutex<Vec<ShellHandle>>>,
    execs: Arc<AtomicUsize>,
}

impl RemoteConnection for FakeConnection {
    fn server_ident(&self) -> String {
        self.behavior.ident.clone()
    }

    fn take_auth(&self) -> AuthResult {
        AuthResult {
            username: self.username.clone(),
            prompted_password: self.prompted.lock().unwrap().take().map(|(value, save)| {
                PromptedSecret {
                    value: SecretString::from(value),
                    save,
                }
            }),
            prompted_passphrase: None,
        }
    }

    fn open_shell(&self, request: ShellRequest) -> BoxFuture<'_, Result<TermParts, SshError>> {
        Box::pin(async move {
            if let Some(delay) = self.behavior.shell_delay {
                tokio::time::sleep(delay).await;
            }
            let (parts, handle) = fake_shell_with(
                &self.behavior.motd,
                (request.term, request.cols, request.rows),
                self.behavior.stall_input,
            );
            self.shells.lock().unwrap().push(handle);
            Ok(parts)
        })
    }

    fn exec(
        &self,
        _command: String,
        _timeout: Duration,
        _max_output: usize,
    ) -> BoxFuture<'_, Result<ExecOutput, SshError>> {
        Box::pin(async move {
            self.execs.fetch_add(1, Ordering::SeqCst);
            match &self.behavior.exec_stdout {
                Some(stdout) => Ok(ExecOutput {
                    stdout: stdout.as_bytes().to_vec(),
                    exit_status: Some(0),
                }),
                None => Err(SshError::new(FailureKind::ChannelFailed)),
            }
        })
    }

    fn disconnect(&self) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}

impl Connector for FakeConnector {
    fn connect(
        &self,
        spec: ConnectSpec,
        interaction: Arc<dyn Interaction>,
    ) -> BoxFuture<'static, Result<Arc<dyn RemoteConnection>, SshError>> {
        let attempt = self.attempts.fetch_add(1, Ordering::SeqCst);
        let behavior = self.behavior.lock().unwrap().clone();
        self.specs.lock().unwrap().push(SpecSeen {
            username: spec.username.clone(),
            password: spec
                .auth
                .password
                .as_ref()
                .map(|password| password.expose_secret().to_owned()),
            preferred: spec.preferred_host_key_types.clone(),
            keepalive: spec.keepalive,
        });
        let shells = self.shells.clone();
        let execs = self.execs.clone();
        Box::pin(async move {
            interaction.step(shellpp_ssh::ConnectStep::Resolving);
            interaction.step(shellpp_ssh::ConnectStep::Connecting);
            if behavior.hang {
                std::future::pending::<()>().await;
            }
            if behavior.refuse {
                return Err(SshError::new(FailureKind::Refused));
            }
            interaction.step(shellpp_ssh::ConnectStep::HostKey);
            if interaction.verify_host_key(&behavior.key).await == HostKeyDecision::Reject {
                return Err(SshError::new(FailureKind::HostKeyRejected));
            }
            if behavior.close_during_decision && attempt == 0 {
                return Err(SshError::new(FailureKind::Disconnected));
            }
            if let Some(kind) = behavior.fail_with {
                return Err(SshError::new(kind));
            }
            interaction.step(shellpp_ssh::ConnectStep::Authenticating);
            let username = match spec.username.clone() {
                Some(username) => username,
                None => match interaction.prompt(Prompt::Username).await {
                    Some(answer) => answer.value.expose_secret().to_owned(),
                    None => return Err(SshError::new(FailureKind::Cancelled)),
                },
            };
            let mut prompted = None;
            if let Some(expected) = &behavior.password {
                let saved_ok = spec
                    .auth
                    .password
                    .as_ref()
                    .is_some_and(|password| password.expose_secret() == expected);
                if !saved_ok {
                    let mut error = spec.auth.password.is_some();
                    let mut tries = 0;
                    loop {
                        tries += 1;
                        if tries > 3 {
                            return Err(SshError::new(FailureKind::AuthFailed));
                        }
                        let answer = interaction
                            .prompt(Prompt::Password {
                                username: username.clone(),
                                error,
                                can_save: spec.auth.can_save_password,
                            })
                            .await;
                        let Some(answer) = answer else {
                            return Err(SshError::new(FailureKind::Cancelled));
                        };
                        if answer.value.expose_secret() == expected {
                            prompted = Some((answer.value.expose_secret().to_owned(), answer.save));
                            break;
                        }
                        error = true;
                    }
                }
            }
            Ok(Arc::new(FakeConnection {
                behavior,
                username,
                prompted: Mutex::new(prompted),
                shells,
                execs,
            }) as Arc<dyn RemoteConnection>)
        })
    }

    fn discover_agent(&self) -> BoxFuture<'static, Option<SystemAgent>> {
        Box::pin(async { None })
    }
}

// ---------------------------------------------------------------------------
// Event collector and harness
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Events {
    log: Mutex<Vec<SessionInfo>>,
}

impl SessionEvents for Events {
    fn session_changed(&self, info: &SessionInfo) {
        self.log.lock().unwrap().push(info.clone());
    }
}

impl Events {
    fn of(&self, id: Uuid) -> Vec<SessionInfo> {
        self.log
            .lock()
            .unwrap()
            .iter()
            .filter(|info| info.session_id == id)
            .cloned()
            .collect()
    }

    fn latest(&self, id: Uuid) -> Option<SessionInfo> {
        self.of(id).last().cloned()
    }

    /// Revs strictly increase per session; nothing follows `closed`.
    fn assert_ordered(&self, id: Uuid) {
        let infos = self.of(id);
        for pair in infos.windows(2) {
            assert!(pair[1].rev > pair[0].rev, "revs must increase");
            assert_ne!(pair[0].state, SessionState::Closed, "event after closed");
        }
        for info in &infos {
            assert_eq!(info.window, WINDOW);
        }
    }

    /// Number of transitions into `connected`.
    fn connections(&self, id: Uuid) -> usize {
        let infos = self.of(id);
        infos
            .iter()
            .enumerate()
            .filter(|(index, info)| {
                info.state == SessionState::Connected
                    && (*index == 0 || infos[index - 1].state != SessionState::Connected)
            })
            .count()
    }

    /// Whether an automatic or manual reconnect attempt was shown running.
    fn saw_attempt_in_progress(&self, id: Uuid) -> bool {
        self.of(id).iter().any(|info| {
            info.state == SessionState::Connecting
                && info.reconnect.as_ref().is_some_and(|r| r.in_progress)
        })
    }

    fn prompts(&self, id: Uuid, kind: &str) -> usize {
        let mut ids: Vec<u64> = self
            .of(id)
            .iter()
            .filter_map(|info| info.prompt.as_ref())
            .filter(|prompt| prompt_kind_name(&prompt.kind) == kind)
            .map(|prompt| prompt.prompt_id)
            .collect();
        ids.dedup();
        ids.len()
    }
}

fn prompt_kind_name(kind: &PromptKind) -> &'static str {
    match kind {
        PromptKind::HostKeyUnknown { .. } => "host_key_unknown",
        PromptKind::HostKeyChanged { .. } => "host_key_changed",
        PromptKind::Username => "username",
        PromptKind::Password { .. } => "password",
        PromptKind::Passphrase { .. } => "passphrase",
        PromptKind::KeyboardInteractive { .. } => "keyboard_interactive",
    }
}

#[cfg(desktop)]
mod fake_local {
    use super::*;
    use shellpp_term_local::{LocalError, ShellInfo, ShellSpec};

    #[derive(Default)]
    pub(super) struct FakeLocal {
        pub(super) fail_spawn: AtomicBool,
        pub(super) shells: Arc<Mutex<Vec<ShellHandle>>>,
    }

    impl super::local::LocalBackend for FakeLocal {
        fn shells(&self, _refresh: bool) -> Vec<ShellInfo> {
            vec![ShellInfo {
                id: "powershell".into(),
                label: "PowerShell".into(),
                program: "powershell.exe".into(),
                args: vec!["-NoLogo".into()],
            }]
        }

        fn default_id(&self, _shells: &[ShellInfo]) -> Option<String> {
            Some("powershell".into())
        }

        fn resolve(
            &self,
            requested: Option<&str>,
            _setting: Option<&str>,
            shells: &[ShellInfo],
        ) -> Result<ShellSpec, LocalError> {
            let id = requested.unwrap_or("powershell");
            let shell = shells
                .iter()
                .find(|shell| shell.id == id)
                .ok_or(LocalError::NoShell)?;
            Ok(ShellSpec {
                id: Some(shell.id.clone()),
                label: shell.label.clone(),
                program: shell.program.clone(),
                args: shell.args.clone(),
            })
        }

        fn spawn(
            &self,
            _spec: &ShellSpec,
            cols: u16,
            rows: u16,
            term: &str,
        ) -> Result<TermParts, LocalError> {
            if self.fail_spawn.load(Ordering::SeqCst) {
                return Err(LocalError::Spawn);
            }
            let (parts, handle) =
                fake_shell(b"PS> ", (term.to_owned(), u32::from(cols), u32::from(rows)));
            self.shells.lock().unwrap().push(handle);
            Ok(parts)
        }
    }
}

struct Harness {
    manager: SessionManager,
    core: Arc<FakeCore>,
    connector: Arc<FakeConnector>,
    events: Arc<Events>,
    #[cfg(desktop)]
    local: Arc<fake_local::FakeLocal>,
}

fn harness() -> Harness {
    let core = Arc::new(FakeCore::new());
    let connector = Arc::new(FakeConnector::default());
    let events = Arc::new(Events::default());
    #[cfg(desktop)]
    let local = Arc::new(fake_local::FakeLocal::default());
    let manager = SessionManager::new(ManagerParts {
        events: events.clone(),
        core: core.clone(),
        connector: connector.clone(),
        #[cfg(desktop)]
        local: local.clone(),
        runtime: tokio::runtime::Handle::current(),
        unlocked: true,
    });
    Harness {
        manager,
        core,
        connector,
        events,
        #[cfg(desktop)]
        local,
    }
}

impl Harness {
    async fn open(&self, target: SessionTarget) -> SessionInfo {
        self.manager
            .open_ssh(
                WINDOW,
                SessionOpenInput {
                    target,
                    cols: 80,
                    rows: 24,
                },
            )
            .await
            .expect("session_open")
    }

    async fn wait(
        &self,
        id: Uuid,
        what: &str,
        ready: impl Fn(&SessionInfo) -> bool,
    ) -> SessionInfo {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            if let Some(info) = self.events.latest(id)
                && ready(&info)
            {
                return info;
            }
            if tokio::time::Instant::now() > deadline {
                let states: Vec<String> = self
                    .events
                    .of(id)
                    .iter()
                    .map(|info| format!("{:?}/{:?}/{:?}", info.state, info.step, info.error))
                    .collect();
                panic!("timed out waiting for {what}; events: {states:?}");
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// Waits until the session connected `n` times in total.
    async fn wait_connections(&self, id: Uuid, n: usize) -> SessionInfo {
        let deadline = tokio::time::Instant::now() + WAIT;
        while self.events.connections(id) < n {
            assert!(
                tokio::time::Instant::now() < deadline,
                "timed out waiting for connection {n}"
            );
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
        self.wait_state(id, SessionState::Connected).await
    }

    async fn wait_state(&self, id: Uuid, state: SessionState) -> SessionInfo {
        self.wait(id, &format!("{state:?}"), |info| info.state == state)
            .await
    }

    async fn wait_prompt(&self, id: Uuid, kind: &str) -> (u64, PromptKind) {
        let info = self
            .wait(id, kind, |info| {
                info.prompt
                    .as_ref()
                    .is_some_and(|prompt| prompt_kind_name(&prompt.kind) == kind)
            })
            .await;
        let prompt = info.prompt.unwrap();
        (prompt.prompt_id, prompt.kind)
    }

    fn reply(&self, id: Uuid, prompt_id: u64, reply: PromptReply) -> Result<(), CmdError> {
        self.manager.prompt_reply(
            WINDOW,
            SessionPromptReplyInput {
                session_id: id,
                prompt_id,
                reply,
            },
        )
    }

    fn decide(&self, id: Uuid, prompt_id: u64, decision: UiDecision) {
        self.reply(id, prompt_id, PromptReply::HostKey { decision })
            .expect("host key reply");
    }

    fn text(&self, id: Uuid, prompt_id: u64, value: &str, save: bool) {
        self.reply(
            id,
            prompt_id,
            PromptReply::Text {
                value: SecretString::from(value.to_owned()),
                save,
            },
        )
        .expect("text reply");
    }

    fn write(&self, id: Uuid, data: &str) {
        self.manager
            .write(
                WINDOW,
                SessionWriteInput {
                    session_id: id,
                    data: data.to_owned(),
                    binary: false,
                    history: None,
                },
            )
            .expect("session_write");
    }

    fn attach(&self, id: Uuid) -> Collector {
        let collector = Collector::default();
        self.manager
            .attach(WINDOW, id, Box::new(collector.clone()))
            .expect("session_attach");
        collector
    }

    /// Opens a quick-connect session and accepts the unknown host key.
    async fn connected_quick(&self) -> Uuid {
        let id = self.open(quick_target()).await.session_id;
        let (prompt, _) = self.wait_prompt(id, "host_key_unknown").await;
        self.decide(id, prompt, UiDecision::AcceptSave);
        let (prompt, _) = self.wait_prompt(id, "username").await;
        self.text(id, prompt, "alice", false);
        self.wait_state(id, SessionState::Connected).await;
        id
    }

    /// Opens a saved-host session whose key is known.
    async fn connected_host(&self) -> Uuid {
        let behavior_key = self.connector.behavior.lock().unwrap().key.clone();
        self.core.add_known("192.0.2.5", 22, &behavior_key, false);
        let id = self.open(host_target()).await.session_id;
        self.wait_state(id, SessionState::Connected).await;
        id
    }
}

async fn wait_bytes(collector: &Collector, needle: &[u8]) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let bytes = collector.bytes();
        if bytes.windows(needle.len()).any(|window| window == needle) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "output never contained {:?}; got {:?}",
            String::from_utf8_lossy(needle),
            String::from_utf8_lossy(&bytes)
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn wait_received(shell: &ShellHandle, needle: &[u8]) {
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let bytes = shell.received();
        if bytes.windows(needle.len()).any(|window| window == needle) {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "shell never received {needle:?}; got {bytes:?}"
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

async fn wait_for(what: &str, ready: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !ready() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

fn code(result: Result<impl Sized, CmdError>) -> &'static str {
    match result {
        Ok(_) => "ok",
        Err(error) => error.code,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn tofu_accept_save_connects_and_streams_output() {
    let h = harness();
    let opened = h.open(quick_target()).await;
    assert_eq!(opened.state, SessionState::Connecting);
    assert_eq!(opened.step, Some(ConnectStep::Resolving));
    assert_eq!(opened.kind, SessionKind::Ssh);
    assert_eq!(opened.window, WINDOW);
    assert_eq!(opened.rev, 1);
    let id = opened.session_id;

    let (prompt, kind) = h.wait_prompt(id, "host_key_unknown").await;
    let PromptKind::HostKeyUnknown { key: shown } = kind else {
        unreachable!()
    };
    assert_eq!(shown.fingerprint, "SHA256:key1");
    assert_eq!(shown.address, "192.0.2.10");
    assert_eq!(shown.port, 2222);
    // A text reply does not answer a host key prompt; a wrong id is not_found.
    assert_eq!(
        code(h.reply(
            id,
            prompt,
            PromptReply::Text {
                value: SecretString::from("x".to_owned()),
                save: false
            }
        )),
        "validation_failed"
    );
    assert_eq!(
        code(h.reply(
            id,
            prompt,
            PromptReply::HostKey {
                decision: UiDecision::Replace
            }
        )),
        "validation_failed"
    );
    assert_eq!(
        code(h.reply(id, prompt + 7, PromptReply::Cancel)),
        "not_found"
    );
    h.decide(id, prompt, UiDecision::AcceptSave);
    wait_for("saved key", || h.core.known_count() == 1).await;

    // Quick connect without a username: asked.
    let (prompt, _) = h.wait_prompt(id, "username").await;
    h.text(id, prompt, "alice", false);
    let info = h.wait_state(id, SessionState::Connected).await;
    assert_eq!(info.username.as_deref(), Some("alice"));
    assert_eq!(info.prompt, None);

    // Steps were reported in order.
    let steps: Vec<ConnectStep> = h
        .events
        .of(id)
        .iter()
        .filter_map(|info| info.step)
        .collect();
    let mut seen = steps.clone();
    seen.dedup();
    assert_eq!(
        seen,
        vec![
            ConnectStep::Resolving,
            ConnectStep::Connecting,
            ConnectStep::HostKey,
            ConnectStep::Authenticating,
            ConnectStep::OpeningSession
        ]
    );

    // The MOTD arrived before the attach: replayed.
    let output = h.attach(id);
    wait_bytes(&output, b"Welcome\r\n$ ").await;
    h.write(id, "echo hi\r");
    wait_bytes(&output, b"hi\r\n").await;
    let shell = h.connector.shell(0);
    assert_eq!(shell.request, ("xterm-256color".to_owned(), 80, 24));
    wait_for("log start", || !h.core.logs().is_empty()).await;
    assert_eq!(h.core.logs(), vec!["start host=false".to_owned()]);
    h.events.assert_ordered(id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn replay_before_attach_and_reattach() {
    let h = harness();
    let id = h.connected_host().await;
    let shell = h.connector.shell(0);
    shell.send(b"while nobody watched\r\n");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let first = h.attach(id);
    wait_bytes(&first, b"Welcome\r\n$ while nobody watched\r\n").await;
    // A window reload attaches again: the replay repeats, the old channel stops.
    let second = h.attach(id);
    wait_bytes(&second, b"Welcome\r\n$ while nobody watched\r\n").await;
    shell.send(b"live");
    wait_bytes(&second, b"live").await;
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(!first.bytes().ends_with(b"live"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn exit_zero_closes_and_forgets_the_session() {
    let h = harness();
    let id = h.connected_host().await;
    assert_eq!(h.manager.active_count(), 1);
    h.write(id, "exit 0\r");
    h.wait_state(id, SessionState::Closed).await;
    assert!(h.manager.sessions_of(WINDOW).is_empty());
    assert_eq!(h.manager.active_count(), 0);
    wait_for("log end", || h.core.logs().len() == 2).await;
    assert_eq!(h.core.logs()[1], "end Closed Some(0)");
    assert_eq!(code(h.manager.cancel(WINDOW, id)), "not_found");
    h.events.assert_ordered(id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn non_zero_exit_ends_and_reconnect_retries() {
    let h = harness();
    let id = h.connected_host().await;
    h.write(id, "exit 3\r");
    let info = h.wait_state(id, SessionState::Ended).await;
    assert_eq!(info.exit_status, Some(3));
    assert_eq!(info.reconnect, None);
    assert_eq!(h.manager.active_count(), 0);
    wait_for("log end", || h.core.logs().len() == 2).await;
    assert_eq!(h.core.logs()[1], "end ExitStatus Some(3)");
    // No automatic attempt.
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert_eq!(h.connector.attempts.load(Ordering::SeqCst), 1);
    // Reconnect in the same tab.
    h.manager.reconnect(WINDOW, id).expect("reconnect");
    let info = h.wait_state(id, SessionState::Connected).await;
    assert_eq!(info.exit_status, None);
    assert_eq!(h.connector.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(code(h.manager.reconnect(WINDOW, id)), "wrong_state");
    // An exit signal is shown like a non-zero code.
    h.write(id, "signal\r");
    let info = h.wait_state(id, SessionState::Ended).await;
    assert_eq!(info.exit_status, Some(255));
    h.events.assert_ordered(id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn drop_reconnects_automatically() {
    let h = harness();
    let id = h.connected_host().await;
    let output = h.attach(id);
    h.write(id, "echo before\r");
    wait_bytes(&output, b"before\r\n").await;
    h.write(id, "drop\r");
    let info = h
        .wait(id, "scheduled attempt", |info| {
            info.state == SessionState::Dropped && info.reconnect.is_some()
        })
        .await;
    let error = info.error.unwrap();
    assert_eq!(error.code, "ssh_error");
    assert_eq!(error.reason.as_deref(), Some("disconnected"));
    let reconnect = info.reconnect.unwrap();
    assert_eq!(reconnect.attempt, 1);
    assert_eq!(reconnect.next_in_ms, Some(1000));
    assert!(!reconnect.waiting_for_unlock);
    assert!(!reconnect.in_progress);
    assert_eq!(h.manager.active_count(), 1);
    wait_for("log end", || {
        h.core.logs().contains(&"end Dropped None".to_owned())
    })
    .await;

    h.wait_connections(id, 2).await;
    assert!(h.events.saw_attempt_in_progress(id));
    assert_eq!(h.connector.shell_count(), 2);
    // The old output stays; the new shell's output follows.
    wait_bytes(&output, b"before\r\n").await;
    h.write(id, "echo after\r");
    wait_bytes(&output, b"after\r\n").await;
    assert!(h.connector.shell(1).received().ends_with(b"echo after\r"));
    h.events.assert_ordered(id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_automatic_attempts_back_off_and_cancel_stops_them() {
    let h = harness();
    let id = h.connected_host().await;
    h.connector.set(|b| b.refuse = true);
    h.write(id, "drop\r");
    // Attempt 1 after 1 s fails (refused), attempt 2 is scheduled after 2 s.
    let info = h
        .wait(id, "attempt 2 scheduled", |info| {
            info.reconnect
                .as_ref()
                .is_some_and(|r| r.attempt == 2 && r.next_in_ms.is_some())
        })
        .await;
    assert_eq!(info.state, SessionState::Dropped);
    assert_eq!(info.reconnect.unwrap().next_in_ms, Some(2000));
    assert_eq!(info.error.unwrap().reason.as_deref(), Some("refused"));
    h.manager.cancel(WINDOW, id).unwrap();
    let info = h
        .wait(id, "attempts stopped", |info| info.reconnect.is_none())
        .await;
    assert_eq!(info.state, SessionState::Dropped);
    assert_eq!(h.manager.active_count(), 0);
    let attempts = h.connector.attempts.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(2300)).await;
    assert_eq!(h.connector.attempts.load(Ordering::SeqCst), attempts);
    // Reconnect now works once the server accepts again.
    h.connector.set(|b| b.refuse = false);
    h.manager.reconnect(WINDOW, id).unwrap();
    h.wait_state(id, SessionState::Connected).await;
    h.events.assert_ordered(id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn autoreconnect_off_leaves_the_session_dropped() {
    let h = harness();
    h.core
        .settings
        .lock()
        .unwrap()
        .insert("terminal.autoreconnect".into(), json!(false));
    let id = h.connected_host().await;
    h.write(id, "drop\r");
    let info = h.wait_state(id, SessionState::Dropped).await;
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert_eq!(h.events.latest(id).unwrap().reconnect, None);
    assert_eq!(info.reconnect, None);
    assert_eq!(h.connector.attempts.load(Ordering::SeqCst), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancel_and_close_during_connecting() {
    let h = harness();
    h.connector.set(|b| b.hang = true);
    let id = h.open(host_target()).await.session_id;
    h.wait(id, "connecting step", |info| {
        info.step == Some(ConnectStep::Connecting)
    })
    .await;
    assert_eq!(h.manager.active_count_of(WINDOW), 1);
    h.manager.cancel(WINDOW, id).unwrap();
    let info = h.wait_state(id, SessionState::Failed).await;
    assert_eq!(info.error.unwrap().code, "cancelled");
    assert_eq!(h.manager.active_count_of(WINDOW), 0);
    // Retry (still hanging), then close while connecting.
    h.manager.reconnect(WINDOW, id).unwrap();
    h.wait(id, "connecting again", |info| {
        info.state == SessionState::Connecting && info.step == Some(ConnectStep::Connecting)
    })
    .await;
    h.manager.close(WINDOW, id).unwrap();
    h.wait_state(id, SessionState::Closed).await;
    assert!(h.manager.sessions_of(WINDOW).is_empty());
    h.events.assert_ordered(id);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wrong_password_then_right_with_save() {
    let h = harness();
    let password = secret("pw");
    h.connector.set(|b| b.password = Some(password.clone()));
    let id = h.connected_host_with_prompts(&password).await;
    // Saved in the core for the host.
    let saved = h.core.saved_passwords.lock().unwrap().clone();
    assert_eq!(saved, vec![(host_ref(), password.clone())]);
    // Reconnects use the kept password: no prompt.
    let prompts = h.events.prompts(id, "password");
    h.write(id, "drop\r");
    h.wait_connections(id, 2).await;
    assert!(h.events.saw_attempt_in_progress(id));
    assert_eq!(h.events.prompts(id, "password"), prompts);
    let specs = h.connector.specs.lock().unwrap();
    assert_eq!(
        specs.last().unwrap().password.as_deref(),
        Some(password.as_str())
    );
}

impl Harness {
    async fn connected_host_with_prompts(&self, password: &str) -> Uuid {
        let behavior_key = self.connector.behavior.lock().unwrap().key.clone();
        self.core.add_known("192.0.2.5", 22, &behavior_key, false);
        let id = self.open(host_target()).await.session_id;
        let (prompt, kind) = self.wait_prompt(id, "password").await;
        assert_eq!(
            kind,
            PromptKind::Password {
                username: Some("alice".into()),
                error: false,
                can_save: true
            }
        );
        self.text(id, prompt, &secret("wrong"), true);
        let (second, kind) = self
            .wait(id, "second password prompt", |info| {
                info.prompt
                    .as_ref()
                    .is_some_and(|p| matches!(p.kind, PromptKind::Password { error: true, .. }))
            })
            .await
            .prompt
            .map(|p| (p.prompt_id, p.kind))
            .unwrap();
        assert_ne!(second, prompt);
        assert!(matches!(kind, PromptKind::Password { error: true, .. }));
        self.text(id, second, password, true);
        self.wait_state(id, SessionState::Connected).await;
        id
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quick_connect_password_is_never_saved() {
    let h = harness();
    let password = secret("pw");
    h.connector.set(|b| b.password = Some(password.clone()));
    h.core.add_known("192.0.2.10", 2222, &key(1), false);
    let id = h.open(quick_target()).await.session_id;
    let (prompt, _) = h.wait_prompt(id, "username").await;
    h.text(id, prompt, "bob", false);
    let (prompt, kind) = h.wait_prompt(id, "password").await;
    assert!(matches!(
        kind,
        PromptKind::Password {
            can_save: false,
            ..
        }
    ));
    // "save" without can_save is ignored.
    h.text(id, prompt, &password, true);
    h.wait_state(id, SessionState::Connected).await;
    assert!(h.core.saved_passwords.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cancelled_prompt_fails_the_attempt() {
    let h = harness();
    h.connector.set(|b| b.password = Some(secret("pw")));
    h.core.add_known("192.0.2.5", 22, &key(1), false);
    let id = h.open(host_target()).await.session_id;
    let (prompt, _) = h.wait_prompt(id, "password").await;
    h.reply(id, prompt, PromptReply::Cancel).unwrap();
    let info = h.wait_state(id, SessionState::Failed).await;
    assert_eq!(info.error.unwrap().code, "cancelled");
    assert_eq!(info.prompt, None);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn revoked_key_fails_without_a_prompt() {
    let h = harness();
    // Revoked under another host pattern: still revoked.
    h.core.add_known("198.51.100.7", 22, &key(1), true);
    let id = h.open(host_target()).await.session_id;
    let info = h.wait_state(id, SessionState::Failed).await;
    assert_eq!(info.error.as_ref().unwrap().code, "host_key_revoked");
    let shown = info.host_key.unwrap();
    assert_eq!(shown.fingerprint, "SHA256:key1");
    assert_eq!(shown.address, "192.0.2.5");
    assert_eq!(h.events.prompts(id, "host_key_unknown"), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn changed_key_is_blocked_until_replaced() {
    let h = harness();
    h.core.add_known("192.0.2.5", 22, &key(1), false);
    h.connector.set(|b| b.key = key(2));
    let id = h.open(host_target()).await.session_id;
    let (prompt, kind) = h.wait_prompt(id, "host_key_changed").await;
    let PromptKind::HostKeyChanged {
        key: shown,
        old_key_type,
        old_fingerprint,
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(shown.fingerprint, "SHA256:key2");
    assert_eq!(old_key_type, "ssh-ed25519");
    assert_eq!(old_fingerprint, "SHA256:key1");
    // "Accept and save" is not an answer to a changed key.
    assert_eq!(
        code(h.reply(
            id,
            prompt,
            PromptReply::HostKey {
                decision: UiDecision::AcceptSave
            }
        )),
        "validation_failed"
    );
    // Cancel: rejected.
    h.decide(id, prompt, UiDecision::Cancel);
    let info = h.wait_state(id, SessionState::Failed).await;
    assert_eq!(
        info.error.unwrap().reason.as_deref(),
        Some("host_key_rejected")
    );
    // Retry and replace.
    h.manager.reconnect(WINDOW, id).unwrap();
    let (prompt, _) = h.wait_prompt(id, "host_key_changed").await;
    h.decide(id, prompt, UiDecision::Replace);
    h.wait_state(id, SessionState::Connected).await;
    let known = h.core.known.lock().unwrap();
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].blob, "AAAAkey2");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn connect_once_is_remembered_for_the_session_only() {
    let h = harness();
    let id = h.open(host_target()).await.session_id;
    let (prompt, _) = h.wait_prompt(id, "host_key_unknown").await;
    h.decide(id, prompt, UiDecision::ConnectOnce);
    h.wait_state(id, SessionState::Connected).await;
    assert_eq!(h.core.known_count(), 0);
    // Its reconnects do not ask again.
    h.write(id, "drop\r");
    h.wait_connections(id, 2).await;
    assert!(h.events.saw_attempt_in_progress(id));
    assert_eq!(h.events.prompts(id, "host_key_unknown"), 1);
    // A new session asks again.
    let other = h.open(host_target()).await.session_id;
    h.wait_prompt(other, "host_key_unknown").await;
    h.manager.close(WINDOW, other).unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn server_closing_during_the_decision_retries_once() {
    let h = harness();
    h.connector.set(|b| b.close_during_decision = true);
    let id = h.open(host_target()).await.session_id;
    let (prompt, _) = h.wait_prompt(id, "host_key_unknown").await;
    h.decide(id, prompt, UiDecision::AcceptSave);
    h.wait_state(id, SessionState::Connected).await;
    assert_eq!(h.connector.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(h.events.prompts(id, "host_key_unknown"), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_records_typed_lines_but_not_injected_ones() {
    let h = harness();
    let id = h.connected_host().await;
    let candidate = |command: &str| HistoryCandidate {
        command: command.to_owned(),
        prompt_text: "$ ".to_owned(),
        typed_printable: true,
        alt_screen: false,
    };
    h.write(id, "ls -la");
    h.manager
        .write(
            WINDOW,
            SessionWriteInput {
                session_id: id,
                data: "\r".into(),
                binary: false,
                history: Some(candidate("ls -la")),
            },
        )
        .unwrap();
    // A history insert, then Enter typed by the user.
    h.manager
        .inject(
            WINDOW,
            SessionInjectInput {
                session_id: id,
                data: "git status".into(),
            },
        )
        .unwrap();
    h.manager
        .write(
            WINDOW,
            SessionWriteInput {
                session_id: id,
                data: "\r".into(),
                binary: false,
                history: Some(candidate("git status")),
            },
        )
        .unwrap();
    wait_for("two candidates", || {
        h.core.history.lock().unwrap().len() == 2
    })
    .await;
    let history = h.core.history.lock().unwrap().clone();
    assert_eq!(history[0], ("ls -la".to_owned(), false, Some(host_ref())));
    assert_eq!(
        history[1],
        ("git status".to_owned(), true, Some(host_ref()))
    );
    let shell = h.connector.shell(0);
    wait_received(&shell, b"ls -la\rgit status\r").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_is_not_sent_when_recording_is_off() {
    let h = harness();
    h.core.set_config(|target| SessionConfig {
        record_history: false,
        ..base_config(target)
    });
    let id = h.connected_host().await;
    assert!(!h.events.latest(id).unwrap().record_history);
    h.manager
        .write(
            WINDOW,
            SessionWriteInput {
                session_id: id,
                data: "pwd\r".into(),
                binary: false,
                history: Some(HistoryCandidate {
                    command: "pwd".into(),
                    prompt_text: String::new(),
                    typed_printable: true,
                    alt_screen: false,
                }),
            },
        )
        .unwrap();
    wait_received(&h.connector.shell(0), b"pwd\r").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(h.core.history.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn charset_round_trip_windows_1251() {
    let h = harness();
    h.core.set_config(|target| SessionConfig {
        charset: "windows-1251".into(),
        ..base_config(target)
    });
    // MOTD "Привет" in windows-1251.
    h.connector
        .set(|b| b.motd = vec![0xcf, 0xf0, 0xe8, 0xe2, 0xe5, 0xf2, b'\r', b'\n']);
    let id = h.connected_host().await;
    let output = h.attach(id);
    wait_bytes(
        &output,
        "\u{41f}\u{440}\u{438}\u{432}\u{435}\u{442}\r\n".as_bytes(),
    )
    .await;
    h.write(id, "echo \u{43c}\u{438}\u{440}\r");
    let shell = h.connector.shell(0);
    wait_received(
        &shell,
        &[b'e', b'c', b'h', b'o', b' ', 0xec, 0xe8, 0xf0, b'\r'],
    )
    .await;
    wait_bytes(&output, "\u{43c}\u{438}\u{440}\r\n".as_bytes()).await;
    // Binary input bypasses the encoder.
    h.manager
        .write(
            WINDOW,
            SessionWriteInput {
                session_id: id,
                data: "\u{1b}[M\u{ff}".into(),
                binary: true,
                history: None,
            },
        )
        .unwrap();
    wait_received(&shell, &[0x1b, b'[', b'M', 0xff]).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn other_windows_get_not_found() {
    let h = harness();
    let id = h.connected_host().await;
    let m = &h.manager;
    assert_eq!(
        code(m.write(
            OTHER_WINDOW,
            SessionWriteInput {
                session_id: id,
                data: "x".into(),
                binary: false,
                history: None
            }
        )),
        "not_found"
    );
    assert_eq!(
        code(m.inject(
            OTHER_WINDOW,
            SessionInjectInput {
                session_id: id,
                data: "x".into()
            }
        )),
        "not_found"
    );
    assert_eq!(
        code(m.resize(
            OTHER_WINDOW,
            SessionResizeInput {
                session_id: id,
                cols: 100,
                rows: 30
            }
        )),
        "not_found"
    );
    assert_eq!(
        code(m.attach(OTHER_WINDOW, id, Box::new(Collector::default()))),
        "not_found"
    );
    assert_eq!(code(m.cancel(OTHER_WINDOW, id)), "not_found");
    assert_eq!(code(m.close(OTHER_WINDOW, id)), "not_found");
    assert_eq!(code(m.reconnect(OTHER_WINDOW, id)), "not_found");
    assert_eq!(
        code(m.prompt_reply(
            OTHER_WINDOW,
            SessionPromptReplyInput {
                session_id: id,
                prompt_id: 1,
                reply: PromptReply::Cancel
            }
        )),
        "not_found"
    );
    assert!(m.sessions_of(OTHER_WINDOW).is_empty());
    assert_eq!(m.sessions_of(WINDOW).len(), 1);
    assert_eq!(m.active_count_of(OTHER_WINDOW), 0);
    assert_eq!(m.active_count_of(WINDOW), 1);
    // The owning window still works.
    h.write(id, "echo still\r");
    wait_received(&h.connector.shell(0), b"echo still\r").await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resize_is_sent_and_remembered() {
    let h = harness();
    let id = h.connected_host().await;
    let resize = |cols, rows| {
        h.manager.resize(
            WINDOW,
            SessionResizeInput {
                session_id: id,
                cols,
                rows,
            },
        )
    };
    resize(120, 40).unwrap();
    resize(120, 40).unwrap();
    assert_eq!(code(resize(0, 40)), "validation_failed");
    assert_eq!(code(resize(120, 1001)), "validation_failed");
    let shell = h.connector.shell(0);
    wait_for("resize", || !shell.resizes.lock().unwrap().is_empty()).await;
    assert_eq!(*shell.resizes.lock().unwrap(), vec![(120, 40)]);
    // The next shell starts with the remembered size.
    h.write(id, "drop\r");
    h.wait_connections(id, 2).await;
    assert!(h.events.saw_attempt_in_progress(id));
    assert_eq!(h.connector.shell(1).request.1, 120);
    assert_eq!(h.connector.shell(1).request.2, 40);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn argument_validation() {
    let h = harness();
    let open = |cols, rows| {
        h.manager.open_ssh(
            WINDOW,
            SessionOpenInput {
                target: host_target(),
                cols,
                rows,
            },
        )
    };
    let error = open(0, 24).await.unwrap_err();
    assert_eq!(
        serde_json::to_value(&error).unwrap(),
        json!({"code": "validation_failed", "details": {"errors": [{"field": "cols", "reason": "out_of_range"}]}})
    );
    let error = open(1001, 0).await.unwrap_err();
    assert_eq!(error.details["errors"].as_array().unwrap().len(), 2);
    // Core errors are command errors.
    *h.core.config.lock().unwrap() = Arc::new(|_| Err(CoreError::NotFound));
    assert_eq!(code(open(80, 24).await), "not_found");
    assert!(h.manager.sessions_of(WINDOW).is_empty());

    *h.core.config.lock().unwrap() = Arc::new(|target| Ok(base_config(target)));
    h.core.add_known("192.0.2.5", 22, &key(1), false);
    let id = open(80, 24).await.unwrap().session_id;
    h.wait_state(id, SessionState::Connected).await;
    let big = "a".repeat(MAX_WRITE_BYTES + 1);
    let write = |data: &str, binary| {
        h.manager.write(
            WINDOW,
            SessionWriteInput {
                session_id: id,
                data: data.to_owned(),
                binary,
                history: None,
            },
        )
    };
    assert_eq!(code(write(&big, false)), "validation_failed");
    assert_eq!(code(write("\u{100}", true)), "validation_failed");
    assert_eq!(code(write(&"a".repeat(MAX_WRITE_BYTES), false)), "ok");
    assert_eq!(
        code(h.manager.inject(
            WINDOW,
            SessionInjectInput {
                session_id: id,
                data: big
            }
        )),
        "validation_failed"
    );
    assert_eq!(code(h.manager.cancel(WINDOW, Uuid::now_v7())), "not_found");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn input_while_not_connected_is_dropped() {
    let h = harness();
    h.connector.set(|b| b.hang = true);
    let id = h.open(host_target()).await.session_id;
    h.write(id, "lost\r");
    h.manager.close(WINDOW, id).unwrap();
    assert_eq!(h.connector.shell_count(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lock_cancels_prompts_and_wipes_kept_credentials() {
    let h = harness();
    // A pending host key prompt is cancelled by the lock: the first attempt fails.
    let id = h.open(host_target()).await.session_id;
    h.wait_prompt(id, "host_key_unknown").await;
    h.core.unlocked.store(false, Ordering::SeqCst);
    h.manager.set_unlocked(false);
    let info = h.wait_state(id, SessionState::Failed).await;
    assert_eq!(info.error.unwrap().code, "cancelled");
    assert_eq!(info.prompt, None);
    // Commands need the unlocked state.
    assert_eq!(code(h.manager.reconnect(WINDOW, id)), "locked");
    assert_eq!(
        code(
            h.manager
                .open_ssh(
                    WINDOW,
                    SessionOpenInput {
                        target: host_target(),
                        cols: 80,
                        rows: 24
                    }
                )
                .await
        ),
        "locked"
    );
    h.core.unlocked.store(true, Ordering::SeqCst);
    h.manager.set_unlocked(true);
    h.manager.close(WINDOW, id).unwrap();

    // A prompted password is kept while unlocked and wiped on lock.
    let password = secret("pw");
    h.connector.set(|b| b.password = Some(password.clone()));
    let id = h.connected_host_with_prompts(&password).await;
    let session = h.manager.shared().sessions().get(&id).cloned().unwrap();
    assert!(session.lock().kept.password.is_some());
    h.core.unlocked.store(false, Ordering::SeqCst);
    h.manager.set_unlocked(false);
    assert!(session.lock().kept.password.is_none());
    // The running session keeps running while locked.
    assert_eq!(h.events.latest(id).unwrap().state, SessionState::Connected);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reconnect_waits_for_unlock() {
    let h = harness();
    let id = h.connected_host().await;
    h.core.unlocked.store(false, Ordering::SeqCst);
    h.manager.set_unlocked(false);
    h.write_locked(id, "drop\r");
    let info = h
        .wait(id, "waiting for unlock", |info| {
            info.reconnect
                .as_ref()
                .is_some_and(|r| r.waiting_for_unlock)
        })
        .await;
    assert_eq!(info.state, SessionState::Dropped);
    assert_eq!(info.reconnect.unwrap().next_in_ms, None);
    tokio::time::sleep(Duration::from_millis(1300)).await;
    assert_eq!(h.connector.attempts.load(Ordering::SeqCst), 1);
    // Unlock: the attempt starts at once.
    h.core.unlocked.store(true, Ordering::SeqCst);
    h.manager.set_unlocked(true);
    h.wait_state(id, SessionState::Connected).await;
    assert_eq!(h.connector.attempts.load(Ordering::SeqCst), 2);
}

impl Harness {
    /// Server-side drop while the app is locked (input commands are refused).
    fn write_locked(&self, id: Uuid, line: &str) {
        let _ = id;
        let shell = self.connector.shell(self.connector.shell_count() - 1);
        if line.starts_with("drop") {
            let _ = shell
                .tx
                .send(TermEvent::End(TermEnd::Dropped(FailureKind::Disconnected)));
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn os_detection_from_the_banner_and_the_script() {
    let h = harness();
    h.connector
        .set(|b| b.ident = "SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13".into());
    let id = h.connected_host().await;
    let info = h.wait(id, "os from banner", |info| info.os.is_some()).await;
    let os = info.os.unwrap();
    assert_eq!(os.family, OsFamily::Linux);
    assert_eq!(os.distro.as_deref(), Some("ubuntu"));
    assert_eq!(os.source, OsSource::Banner);
    assert_eq!(h.connector.execs.load(Ordering::SeqCst), 0);
    assert_eq!(
        h.core.os_written.lock().unwrap()[0],
        (
            host_ref(),
            OsFamily::Linux,
            Some("ubuntu".into()),
            OsSource::Banner
        )
    );
    h.manager.close(WINDOW, id).unwrap();

    // No OS in the banner: the script runs in an exec channel.
    let h = harness();
    h.core.set_config(|target| SessionConfig {
        os_detection_script: Some("uname -s".into()),
        ..base_config(target)
    });
    h.connector
        .set(|b| b.exec_stdout = Some("Linux\nID=debian\nVERSION_ID=12\n".into()));
    let id = h.connected_host().await;
    let info = h.wait(id, "os from script", |info| info.os.is_some()).await;
    let os = info.os.unwrap();
    assert_eq!(os.distro.as_deref(), Some("debian"));
    assert_eq!(os.source, OsSource::Script);
    assert_eq!(h.connector.execs.load(Ordering::SeqCst), 1);
    // Nothing went into the shell.
    assert!(h.connector.shell(0).received().is_empty());

    // Detection off: neither.
    let h = harness();
    h.core
        .settings
        .lock()
        .unwrap()
        .insert("terminal.os_detection_enabled".into(), json!(false));
    h.connector
        .set(|b| b.ident = "SSH-2.0-OpenSSH_9.6p1 Ubuntu-3ubuntu13".into());
    let id = h.connected_host().await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(h.events.latest(id).unwrap().os, None);
    assert!(h.core.os_written.lock().unwrap().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn window_close_and_exit_counts() {
    let h = harness();
    let first = h.connected_host().await;
    let second = h.connected_host().await;
    assert_eq!(h.manager.active_count(), 2);
    assert_eq!(h.manager.active_count_of(WINDOW), 2);
    let logs = h.manager.close_window(WINDOW);
    assert_eq!(logs.len(), 2);
    h.manager.end_logs_now(&logs);
    assert_eq!(h.events.latest(first).unwrap().state, SessionState::Closed);
    assert_eq!(h.events.latest(second).unwrap().state, SessionState::Closed);
    assert_eq!(h.manager.active_count(), 0);
    let ends = h
        .core
        .logs()
        .iter()
        .filter(|log| log.starts_with("end Closed"))
        .count();
    assert_eq!(ends, 2);
    // Nothing is ended twice.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let ends_later = h
        .core
        .logs()
        .iter()
        .filter(|log| log.starts_with("end"))
        .count();
    assert_eq!(ends_later, 2);
    // The closed window gets no new sessions; Exit closes those of the others.
    let h = harness();
    let third = h.connected_host().await;
    assert_eq!(h.manager.close_all().len(), 1);
    assert_eq!(h.events.latest(third).unwrap().state, SessionState::Closed);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn later_attempts_use_fresh_settings() {
    let h = harness();
    let id = h.connected_host().await;
    let first_keepalive = h.connector.specs.lock().unwrap()[0].keepalive;
    assert_eq!(first_keepalive, Some(Duration::from_secs(30)));
    h.core.set_config(|target| SessionConfig {
        title: "renamed".into(),
        keepalive_s: 0,
        known_key_types: vec!["ssh-ed25519".into()],
        ..base_config(target)
    });
    h.write(id, "drop\r");
    let info = h.wait_connections(id, 2).await;
    assert!(h.events.saw_attempt_in_progress(id));
    assert_eq!(info.title, "renamed");
    let specs = h.connector.specs.lock().unwrap();
    assert_eq!(specs[1].keepalive, None);
    assert_eq!(specs[1].preferred, vec!["ssh-ed25519".to_owned()]);
    assert_eq!(specs[1].username.as_deref(), Some("alice"));
}

#[cfg(desktop)]
mod local_sessions {
    use super::*;
    use shellpp_core::service::dto::LocalOpenInput;

    async fn open_local(h: &Harness, shell_id: Option<&str>) -> Result<SessionInfo, CmdError> {
        h.manager
            .open_local(
                WINDOW,
                LocalOpenInput {
                    shell_id: shell_id.map(str::to_owned),
                    cols: 100,
                    rows: 30,
                },
            )
            .await
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_terminal_lifecycle() {
        let h = harness();
        let info = open_local(&h, None).await.unwrap();
        assert_eq!(info.kind, SessionKind::Local);
        assert_eq!(info.state, SessionState::Connected);
        assert_eq!(info.title, "PowerShell");
        assert_eq!(info.shell_id.as_deref(), Some("powershell"));
        assert_eq!(info.target, None);
        assert_eq!(info.terminal_theme, json!({"builtin": "graphite"}));
        let id = info.session_id;
        let output = h.attach(id);
        wait_bytes(&output, b"PS> ").await;
        h.write(id, "echo local\r");
        wait_bytes(&output, b"local\r\n").await;
        let shell = h.local.shells.lock().unwrap()[0].clone();
        assert_eq!(shell.request, ("xterm-256color".to_owned(), 100, 30));

        h.write(id, "exit 3\r");
        let info = h.wait_state(id, SessionState::Ended).await;
        assert_eq!(info.exit_status, Some(3));
        h.manager.reconnect(WINDOW, id).unwrap();
        h.wait_state(id, SessionState::Connected).await;
        h.write(id, "exit 0\r");
        h.wait_state(id, SessionState::Closed).await;
        // No connection log for local terminals.
        assert!(h.core.logs().is_empty());
        h.events.assert_ordered(id);

        let shells = h.manager.local_shells().await.unwrap();
        assert_eq!(shells.shells.len(), 1);
        assert_eq!(shells.default_id.as_deref(), Some("powershell"));
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_errors() {
        let h = harness();
        assert_eq!(
            serde_json::to_value(open_local(&h, Some("zsh")).await.unwrap_err()).unwrap(),
            json!({"code": "validation_failed", "details": {"errors": [{"field": "shell_id", "reason": "invalid"}]}})
        );
        h.local.fail_spawn.store(true, Ordering::SeqCst);
        let info = open_local(&h, None).await.unwrap();
        assert_eq!(info.state, SessionState::Failed);
        assert_eq!(info.error.unwrap().code, "unavailable");
        h.local.fail_spawn.store(false, Ordering::SeqCst);
        h.manager.reconnect(WINDOW, info.session_id).unwrap();
        h.wait_state(info.session_id, SessionState::Connected).await;
        h.core.unlocked.store(false, Ordering::SeqCst);
        h.manager.set_unlocked(false);
        assert_eq!(code(open_local(&h, None).await), "locked");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn local_history_has_no_host() {
        let h = harness();
        let id = open_local(&h, None).await.unwrap().session_id;
        h.manager
            .write(
                WINDOW,
                SessionWriteInput {
                    session_id: id,
                    data: "dir\r".into(),
                    binary: false,
                    history: Some(HistoryCandidate {
                        command: "dir".into(),
                        prompt_text: "PS> ".into(),
                        typed_printable: true,
                        alt_screen: false,
                    }),
                },
            )
            .unwrap();
        wait_for("candidate", || h.core.history.lock().unwrap().len() == 1).await;
        assert_eq!(
            h.core.history.lock().unwrap()[0],
            ("dir".to_owned(), false, None)
        );
    }
}

#[test]
fn prompt_reply_kinds() {
    let text = PromptReply::Text {
        value: SecretString::from("v".to_owned()),
        save: false,
    };
    let host_key = |decision| PromptReply::HostKey { decision };
    assert!(Expect::Text.accepts(&text));
    assert!(Expect::Text.accepts(&PromptReply::Cancel));
    assert!(!Expect::Text.accepts(&host_key(UiDecision::AcceptSave)));
    assert!(Expect::HostKeyUnknown.accepts(&host_key(UiDecision::AcceptSave)));
    assert!(Expect::HostKeyUnknown.accepts(&host_key(UiDecision::ConnectOnce)));
    assert!(Expect::HostKeyUnknown.accepts(&host_key(UiDecision::Cancel)));
    assert!(!Expect::HostKeyUnknown.accepts(&host_key(UiDecision::Replace)));
    assert!(!Expect::HostKeyUnknown.accepts(&text));
    assert!(Expect::HostKeyChanged.accepts(&host_key(UiDecision::Replace)));
    assert!(!Expect::HostKeyChanged.accepts(&host_key(UiDecision::AcceptSave)));
    assert!(!Expect::HostKeyChanged.accepts(&host_key(UiDecision::ConnectOnce)));
}

#[test]
fn session_errors() {
    assert_eq!(
        ssh_session_error(SshError::new(FailureKind::Cancelled)),
        error_code("cancelled")
    );
    let refused = ssh_session_error(SshError::new(FailureKind::Refused));
    assert_eq!(refused.code, "ssh_error");
    assert_eq!(refused.reason.as_deref(), Some("refused"));
    assert_eq!(
        core_session_error(&CoreError::NotFound),
        error_code("not_found")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn quick_connect_keeps_the_typed_username_for_reconnects() {
    let h = harness();
    let id = h.connected_quick().await;
    assert_eq!(h.events.prompts(id, "username"), 1);
    h.write(id, "drop\r");
    let info = h.wait_connections(id, 2).await;
    assert!(h.events.saw_attempt_in_progress(id));
    assert_eq!(info.username.as_deref(), Some("alice"));
    assert_eq!(h.events.prompts(id, "username"), 1);
    assert_eq!(h.events.prompts(id, "host_key_unknown"), 1);
    let specs = h.connector.specs.lock().unwrap();
    assert_eq!(specs[0].username, None);
    assert_eq!(specs[1].username.as_deref(), Some("alice"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_page_reload_detaches_output_until_the_next_attach() {
    let h = harness();
    let id = h.connected_host().await;
    let before = h.attach(id);
    wait_bytes(&before, b"Welcome").await;
    h.manager.detach_window(OTHER_WINDOW);
    h.manager.detach_window(WINDOW);
    let shell = h.connector.shell(0);
    shell.send(b"during reload");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!before.bytes().ends_with(b"during reload"));
    let after = h.attach(id);
    wait_bytes(&after, b"Welcome\r\n$ during reload").await;
}

// ---------------------------------------------------------------------------
// Review fixes
// ---------------------------------------------------------------------------

/// A session opened for a window that is gone (the open was in flight when
/// it closed) is never started.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_closed_window_gets_no_new_sessions() {
    let h = harness();
    let first = h.connected_host().await;
    let logs = h.manager.close_window(WINDOW);
    h.manager.end_logs_now(&logs);
    assert_eq!(h.events.latest(first).unwrap().state, SessionState::Closed);
    let attempts = h.connector.attempts.load(Ordering::SeqCst);
    let opened = h
        .manager
        .open_ssh(
            WINDOW,
            SessionOpenInput {
                target: host_target(),
                cols: 80,
                rows: 24,
            },
        )
        .await;
    assert_eq!(code(opened), "not_found");
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(h.connector.attempts.load(Ordering::SeqCst), attempts);
    assert!(h.manager.sessions_of(WINDOW).is_empty());
    assert_eq!(h.manager.active_count(), 0);
    // Other windows are not affected.
    let other = h
        .manager
        .open_ssh(
            OTHER_WINDOW,
            SessionOpenInput {
                target: host_target(),
                cols: 80,
                rows: 24,
            },
        )
        .await
        .expect("another window opens");
    assert_eq!(other.window, OTHER_WINDOW);
    h.manager.close(OTHER_WINDOW, other.session_id).unwrap();
}

#[cfg(desktop)]
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_local_terminal_for_a_closed_window_is_ended() {
    use shellpp_core::service::dto::LocalOpenInput;
    let h = harness();
    h.manager.close_window(WINDOW);
    let opened = h
        .manager
        .open_local(
            WINDOW,
            LocalOpenInput {
                shell_id: None,
                cols: 80,
                rows: 24,
            },
        )
        .await;
    assert_eq!(code(opened), "not_found");
    // The shell that started was ended.
    let shell = h.local.shells.lock().unwrap()[0].clone();
    assert!(shell.closed.load(Ordering::SeqCst));
    assert!(h.manager.sessions_of(WINDOW).is_empty());
}

/// Automatic attempts retry transport failures only: a refused login
/// stops them at once.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn automatic_attempts_stop_on_non_transport_failures() {
    let h = harness();
    let id = h.connected_host().await;
    h.connector
        .set(|b| b.fail_with = Some(FailureKind::AuthFailed));
    h.write(id, "drop\r");
    let info = h
        .wait(id, "auth failure shown", |info| {
            info.state == SessionState::Dropped
                && info
                    .error
                    .as_ref()
                    .is_some_and(|e| e.reason.as_deref() == Some("auth_failed"))
        })
        .await;
    assert_eq!(info.reconnect, None);
    assert_eq!(h.connector.attempts.load(Ordering::SeqCst), 2);
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(h.connector.attempts.load(Ordering::SeqCst), 2);
    assert_eq!(h.manager.active_count(), 0);
    // A removed host (a core error) stops them too.
    h.connector.set(|b| b.fail_with = None);
    h.manager.reconnect(WINDOW, id).unwrap();
    h.wait_connections(id, 2).await;
    *h.core.config.lock().unwrap() = Arc::new(|_| Err(CoreError::NotFound));
    h.write(id, "drop\r");
    let info = h
        .wait(id, "not found shown", |info| {
            info.state == SessionState::Dropped
                && info.error.as_ref().is_some_and(|e| e.code == "not_found")
        })
        .await;
    assert_eq!(info.reconnect, None);
    let attempts = h.connector.attempts.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_millis(2500)).await;
    assert_eq!(h.connector.attempts.load(Ordering::SeqCst), attempts);
    h.events.assert_ordered(id);
}

/// A reconnect of a "Connect once" session that meets another key shows
/// the changed-key warning, not the first-connection dialog.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_new_key_after_connect_once_is_a_changed_key() {
    let h = harness();
    let id = h.open(host_target()).await.session_id;
    let (prompt, _) = h.wait_prompt(id, "host_key_unknown").await;
    h.decide(id, prompt, UiDecision::ConnectOnce);
    h.wait_state(id, SessionState::Connected).await;
    h.connector.set(|b| b.key = key(2));
    h.write(id, "drop\r");
    let (prompt, kind) = h.wait_prompt(id, "host_key_changed").await;
    let PromptKind::HostKeyChanged {
        key: shown,
        old_key_type,
        old_fingerprint,
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(shown.fingerprint, "SHA256:key2");
    assert_eq!(old_key_type, "ssh-ed25519");
    assert_eq!(
        Some(old_fingerprint),
        shellpp_core::known_hosts::fingerprint_sha256("AAAAkey1")
    );
    assert_eq!(h.events.prompts(id, "host_key_unknown"), 1);
    // Cancel rejects; automatic attempts stop.
    h.decide(id, prompt, UiDecision::Cancel);
    let info = h
        .wait(id, "rejected", |info| {
            info.state == SessionState::Dropped && info.reconnect.is_none()
        })
        .await;
    assert_eq!(info.prompt, None);
    // Reconnect now and replace: trusted for this session only.
    h.manager.reconnect(WINDOW, id).unwrap();
    let (prompt, _) = h.wait_prompt(id, "host_key_changed").await;
    h.decide(id, prompt, UiDecision::Replace);
    h.wait_connections(id, 2).await;
    assert_eq!(h.core.known_count(), 0);
    h.write(id, "drop\r");
    h.wait_connections(id, 3).await;
    assert_eq!(h.events.prompts(id, "host_key_changed"), 2);
}

/// A password typed for one server is never sent to another one after the
/// host record changed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn kept_credentials_stay_with_their_server() {
    let h = harness();
    let password = secret("pw");
    h.connector.set(|b| b.password = Some(password.clone()));
    let id = h.connected_host_with_prompts(&password).await;
    let session = h.manager.shared().sessions().get(&id).cloned().unwrap();
    assert_eq!(
        session.lock().kept.bound,
        Some(("192.0.2.5".to_owned(), 22, "alice".to_owned()))
    );
    // The host now points to another address.
    h.core.set_config(|target| SessionConfig {
        address: "198.51.100.9".into(),
        ..base_config(target)
    });
    h.core.add_known("198.51.100.9", 22, &key(1), false);
    let prompts = h.events.prompts(id, "password");
    h.write(id, "drop\r");
    let (prompt, kind) = h.wait_prompt(id, "password").await;
    assert!(matches!(kind, PromptKind::Password { error: false, .. }));
    assert_eq!(h.events.prompts(id, "password"), prompts + 1);
    {
        let specs = h.connector.specs.lock().unwrap();
        assert_eq!(specs.last().unwrap().password, None);
    }
    assert!(session.lock().kept.password.is_none());
    h.text(id, prompt, &password, false);
    h.wait_connections(id, 2).await;
    assert_eq!(
        session.lock().kept.bound,
        Some(("198.51.100.9".to_owned(), 22, "alice".to_owned()))
    );
    // Another username on the same server drops them too.
    h.core.set_config(|target| SessionConfig {
        address: "198.51.100.9".into(),
        username: Some("bob".into()),
        ..base_config(target)
    });
    h.write(id, "drop\r");
    h.wait_prompt(id, "password").await;
    let specs = h.connector.specs.lock().unwrap();
    assert_eq!(specs.last().unwrap().password, None);
    assert_eq!(specs.last().unwrap().username.as_deref(), Some("bob"));
}

/// The pane of a reconnected session gets a terminal mode reset before the
/// new shell's output; the first shell does not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_reconnect_resets_terminal_modes() {
    let h = harness();
    let id = h.connected_host().await;
    let output = h.attach(id);
    wait_bytes(&output, b"Welcome\r\n$ ").await;
    assert!(
        !output
            .bytes()
            .windows(MODE_RESET.len())
            .any(|w| w == MODE_RESET)
    );
    h.connector.shell(0).send(b"\x1b[?1049h\x1b[?1000h");
    h.write(id, "drop\r");
    h.wait_connections(id, 2).await;
    let mut expected = MODE_RESET.to_vec();
    expected.extend_from_slice(b"Welcome\r\n$ ");
    wait_bytes(&output, &expected).await;
}

/// The reason of a drop is shown: a keepalive timeout is not a generic
/// disconnect, and the reason stays while an attempt waits for unlock.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_drop_reason_is_kept() {
    let h = harness();
    let id = h.connected_host().await;
    h.write(id, "timeout\r");
    let info = h
        .wait(id, "dropped", |info| {
            info.state == SessionState::Dropped && info.reconnect.is_some()
        })
        .await;
    assert_eq!(info.error.unwrap().reason.as_deref(), Some("timeout"));
    h.wait_connections(id, 2).await;
    // Locked during an automatic attempt: the reason comes back while waiting.
    h.connector.set(|b| b.hang = true);
    h.write(id, "drop\r");
    h.wait(id, "attempt running", |info| {
        info.state == SessionState::Connecting
    })
    .await;
    h.core.unlocked.store(false, Ordering::SeqCst);
    h.manager.set_unlocked(false);
    let info = h
        .wait(id, "waiting for unlock", |info| {
            info.reconnect
                .as_ref()
                .is_some_and(|r| r.waiting_for_unlock)
        })
        .await;
    assert_eq!(info.error.unwrap().reason.as_deref(), Some("disconnected"));
    h.core.unlocked.store(true, Ordering::SeqCst);
    h.manager.set_unlocked(true);
    h.manager.cancel(WINDOW, id).unwrap();
    let info = h
        .wait(id, "stopped", |info| {
            info.state == SessionState::Dropped && info.reconnect.is_none()
        })
        .await;
    assert_eq!(info.error.unwrap().reason.as_deref(), Some("disconnected"));
}

/// A resize that arrives while the shell is being opened reaches the new shell.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_resize_during_shell_setup_is_not_lost() {
    let h = harness();
    h.connector
        .set(|b| b.shell_delay = Some(Duration::from_millis(300)));
    h.core.add_known("192.0.2.5", 22, &key(1), false);
    let id = h.open(host_target()).await.session_id;
    h.wait(id, "opening the shell", |info| {
        info.step == Some(ConnectStep::OpeningSession)
    })
    .await;
    h.manager
        .resize(
            WINDOW,
            SessionResizeInput {
                session_id: id,
                cols: 132,
                rows: 43,
            },
        )
        .unwrap();
    h.wait_state(id, SessionState::Connected).await;
    let shell = h.connector.shell(0);
    assert_eq!(shell.request.1, 80);
    wait_for("resize", || !shell.resizes.lock().unwrap().is_empty()).await;
    assert_eq!(*shell.resizes.lock().unwrap(), vec![(132, 43)]);
}

/// Resize and attach are accepted while locked; input is not.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resize_and_attach_work_while_locked() {
    let h = harness();
    let id = h.connected_host().await;
    h.core.unlocked.store(false, Ordering::SeqCst);
    h.manager.set_unlocked(false);
    h.manager
        .resize(
            WINDOW,
            SessionResizeInput {
                session_id: id,
                cols: 100,
                rows: 50,
            },
        )
        .expect("resize while locked");
    let output = h.attach(id);
    wait_bytes(&output, b"Welcome").await;
    h.manager.ack(WINDOW, id, 10).expect("ack while locked");
    assert_eq!(code(h.manager.ack(OTHER_WINDOW, id, 10)), "not_found");
    assert_eq!(
        code(h.manager.write(
            WINDOW,
            SessionWriteInput {
                session_id: id,
                data: "x".into(),
                binary: false,
                history: None
            }
        )),
        "locked"
    );
    let shell = h.connector.shell(0);
    wait_for("resize", || !shell.resizes.lock().unwrap().is_empty()).await;
    assert_eq!(*shell.resizes.lock().unwrap(), vec![(100, 50)]);
}

/// Output is not read while the pane is behind with acknowledgements.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn output_waits_for_acknowledgements() {
    let h = harness();
    let id = h.connected_host().await;
    let output = h.attach(id);
    wait_bytes(&output, b"Welcome").await;
    let shell = h.connector.shell(0);
    let chunk = vec![b'z'; 64 * 1024];
    let total = 6 * 1024 * 1024;
    for _ in 0..total / chunk.len() {
        shell.send(&chunk);
    }
    // Reading stops a little above the pause mark.
    wait_for("paused", || {
        output.bytes().len() as u64 > output::PAUSE_BYTES
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let shown = output.bytes().len() as u64;
    assert!(shown < output::PAUSE_BYTES + 512 * 1024, "{shown}");
    // Acknowledgements let the rest through.
    let mut acked = 0u64;
    let deadline = tokio::time::Instant::now() + WAIT;
    while (output.bytes().len() as u64) < total as u64 {
        let received = output.bytes().len() as u64;
        h.manager.ack(WINDOW, id, received - acked).unwrap();
        acked = received;
        assert!(
            tokio::time::Instant::now() < deadline,
            "stuck at {received}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Input that waits for a terminal that stopped reading is bounded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn input_waiting_for_the_terminal_is_bounded() {
    let h = harness();
    h.connector.set(|b| b.stall_input = true);
    let id = h.connected_host().await;
    let chunk = "y".repeat(MAX_WRITE_BYTES);
    let write = || {
        h.manager.write(
            WINDOW,
            SessionWriteInput {
                session_id: id,
                data: chunk.clone(),
                binary: false,
                history: None,
            },
        )
    };
    for _ in 0..MAX_PENDING_INPUT / MAX_WRITE_BYTES {
        assert_eq!(code(write()), "ok");
    }
    assert_eq!(code(write()), "unavailable");
    assert_eq!(
        code(h.manager.inject(
            WINDOW,
            SessionInjectInput {
                session_id: id,
                data: chunk.clone()
            }
        )),
        "unavailable"
    );
}

/// History candidates of one session are recorded in the order they were typed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn history_is_recorded_in_order() {
    let h = harness();
    let id = h.connected_host().await;
    let commands: Vec<String> = (0..30).map(|n| format!("echo {n}")).collect();
    for command in &commands {
        h.manager
            .write(
                WINDOW,
                SessionWriteInput {
                    session_id: id,
                    data: format!("{command}\r"),
                    binary: false,
                    history: Some(HistoryCandidate {
                        command: command.clone(),
                        prompt_text: "$ ".into(),
                        typed_printable: true,
                        alt_screen: false,
                    }),
                },
            )
            .unwrap();
    }
    wait_for("all candidates", || {
        h.core.history.lock().unwrap().len() == commands.len()
    })
    .await;
    let recorded: Vec<String> = h
        .core
        .history
        .lock()
        .unwrap()
        .iter()
        .map(|(command, _, _)| command.clone())
        .collect();
    assert_eq!(recorded, commands);
}

/// Turning history recording off reaches sessions that are already open.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn record_history_follows_changes_while_open() {
    let h = harness();
    let id = h.connected_host().await;
    assert!(h.events.latest(id).unwrap().record_history);
    h.core.record_now.store(false, Ordering::SeqCst);
    h.manager.refresh_record_history();
    h.wait(id, "recording off", |info| !info.record_history)
        .await;
    h.manager
        .write(
            WINDOW,
            SessionWriteInput {
                session_id: id,
                data: "pwd\r".into(),
                binary: false,
                history: Some(HistoryCandidate {
                    command: "pwd".into(),
                    prompt_text: "$ ".into(),
                    typed_printable: true,
                    alt_screen: false,
                }),
            },
        )
        .unwrap();
    wait_received(&h.connector.shell(0), b"pwd\r").await;
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(h.core.history.lock().unwrap().is_empty());
    h.core.record_now.store(true, Ordering::SeqCst);
    h.manager.refresh_record_history();
    h.wait(id, "recording on", |info| info.record_history).await;
}
