//! Session integration tests (owner's stage 2a, step 3): SSH sessions through
//! the app's session layer (`shellpp_desktop::sessions::SessionManager`, the
//! same manager the session tabs use) against a real server, chosen
//! automatically (`servers`: OpenSSH in Docker when a Docker daemon runs,
//! otherwise the russh test server; `SHELLPP_TEST_SSHD=openssh|russh`
//! forces one). Every test prints the server it used.
//!
//! Scenarios: password, key file, keyboard-interactive, a wrong password
//! (asked again, then the server gives up), the first connection (TOFU:
//! accept and save, connect once, cancel), a changed host key (blocked,
//! cancel keeps the saved key, replace), a dropped connection with automatic
//! reconnection and without it, exit status 0 and non-zero.
//!
//! Every test uses a temporary profile (in-memory secret store), a key
//! directory with keys generated at run time and no agent, and its own
//! server; nothing touches the real `~/.ssh`, the real agent or a real host.
//! The terminal is a recording stand-in for the terminal view: the tests
//! look at the session's output bytes.

mod servers;

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpStream};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedReceiver;
use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use servers::{ServerKind, TestSshd};
use shellpp_core::Core;
use shellpp_core::service::dto::{
    RecordSaveInput, RecordType, RecordView, SettingsSetInput, VaultKind,
};
use shellpp_desktop::sessions::transport::BoxFuture;
use shellpp_desktop::sessions::{
    AgentSource, HostKeyChoice, KeySources, ManagerOptions, OpenRequest, PromptKind, PromptReply,
    SessionError, SessionEvent, SessionHandle, SessionManager, SessionState, SessionTarget,
    SessionTerminal, SessionView, TerminalClosed,
};
use shellpp_desktop::testing::TestProfile;
use shellpp_ssh::FailureKind;
use uuid::Uuid;

/// Connecting, prompts and reconnects on a busy CI machine.
const WAIT: Duration = Duration::from_secs(45);

/// The terminal stand-in: records the output, keeps the session handle.
#[derive(Default)]
struct RecordingTerminal {
    output: Mutex<Vec<u8>>,
    session: Mutex<Option<SessionHandle>>,
}

impl RecordingTerminal {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.output.lock().unwrap()).into_owned()
    }

    fn len(&self) -> usize {
        self.output.lock().unwrap().len()
    }
}

impl SessionTerminal for RecordingTerminal {
    fn feed<'a>(&'a self, bytes: &'a [u8]) -> BoxFuture<'a, Result<(), TerminalClosed>> {
        Box::pin(async move {
            self.output.lock().unwrap().extend_from_slice(bytes);
            Ok(())
        })
    }

    fn attach(&self, session: SessionHandle) {
        *self.session.lock().unwrap() = Some(session);
    }

    fn inject(&self, bytes: Vec<u8>) {
        if let Some(session) = self.session.lock().unwrap().clone() {
            let _ = session.write(&bytes);
        }
    }
}

/// One open session as a test sees it.
struct Session {
    handle: SessionHandle,
    events: UnboundedReceiver<SessionEvent>,
    terminal: Arc<RecordingTerminal>,
    view: SessionView,
    /// Every state the session went through, in order.
    states: Vec<SessionState>,
    /// Every prompt kind shown, in order.
    prompts: Vec<&'static str>,
}

impl Session {
    /// Applies every event received so far.
    fn drain(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            let view = event.view().clone();
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
    }

    /// Waits until `condition` holds for the current view.
    fn wait(&mut self, what: &str, condition: impl Fn(&SessionView) -> bool) -> &SessionView {
        let deadline = Instant::now() + WAIT;
        loop {
            self.drain();
            if condition(&self.view) {
                return &self.view;
            }
            assert!(
                Instant::now() < deadline,
                "timeout waiting for {what}; view: {:?}; states: {:?}; prompts: {:?}",
                self.view,
                self.states,
                self.prompts
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn wait_state(&mut self, state: SessionState) -> &SessionView {
        self.wait(&format!("{state:?}"), |view| view.state == state)
    }

    /// Waits for a prompt of `kind` and returns its id and kind.
    fn wait_prompt(&mut self, kind: &str) -> (u64, PromptKind) {
        let view = self.wait(&format!("a {kind} prompt"), |view| {
            view.prompt.as_ref().is_some_and(|p| p.kind.name() == kind)
                || matches!(view.state, SessionState::Failed | SessionState::Closed)
        });
        let prompt = view
            .prompt
            .clone()
            .unwrap_or_else(|| panic!("no {kind} prompt; view: {view:?}"));
        (prompt.id, prompt.kind)
    }

    fn reply(&mut self, id: u64, reply: PromptReply) {
        self.handle
            .reply(id, reply)
            .expect("the prompt takes the reply");
    }

    fn reply_text(&mut self, id: u64, value: &SecretString) {
        self.reply(
            id,
            PromptReply::Text {
                value: value.clone(),
                save: false,
            },
        );
    }

    /// Accepts and saves an unknown host key; returns its fingerprint.
    fn accept_host_key(&mut self) -> String {
        let (id, kind) = self.wait_prompt("host_key_unknown");
        let PromptKind::HostKeyUnknown { key } = kind else {
            unreachable!()
        };
        self.reply(id, PromptReply::HostKey(HostKeyChoice::AcceptAndSave));
        key.fingerprint
    }

    /// Types a line.
    fn type_line(&mut self, line: &str) {
        let deadline = Instant::now() + WAIT;
        let bytes = format!("{line}\r");
        loop {
            match self.handle.write(bytes.as_bytes()) {
                Ok(()) => return,
                Err(error) if Instant::now() < deadline => {
                    let _ = error;
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(error) => panic!("input not taken: {error}"),
            }
        }
    }

    /// Waits until the output (from byte `from` on) contains `needle`.
    fn wait_output(&mut self, from: usize, needle: &str) {
        let deadline = Instant::now() + WAIT;
        loop {
            let text = self.terminal.text();
            if text.get(from..).is_some_and(|rest| rest.contains(needle)) {
                return;
            }
            self.drain();
            assert!(
                Instant::now() < deadline,
                "timeout waiting for {needle:?} in the output; state {:?}",
                self.view.state
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Runs the marker command and waits for its output.
    fn run_marker(&mut self, server: &TestSshd) {
        let from = self.terminal.len();
        let commands = server.commands();
        self.type_line(commands.marker);
        self.wait_output(from, commands.marker_output);
    }
}

/// A temporary profile, the session manager over its core, and the server.
/// Fields drop in order: sessions, the manager, the core, the profile.
struct Harness {
    manager: SessionManager,
    core: Arc<Core>,
    _events: shellpp_desktop::platform::events::EventReceiver,
    vault_id: Uuid,
    server: TestSshd,
    _profile: TestProfile,
}

impl Harness {
    fn new() -> Self {
        let server = TestSshd::start();
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
        let manager = SessionManager::start(
            core.clone(),
            ManagerOptions::with_keys(KeySources {
                key_dir: Some(server.key_dir.path().to_path_buf()),
                agent: AgentSource::None,
            }),
        )
        .expect("session manager");
        Self {
            manager,
            core,
            _events: opened.events,
            vault_id,
            server,
            _profile: profile,
        }
    }

    /// A saved host at `addr` for `user`, with a saved password if given.
    fn host(&self, addr: SocketAddr, user: &str, password: Option<&SecretString>) -> Uuid {
        let data = json!({
            "label": format!("test {user}"),
            "address": addr.ip().to_string(),
            "ssh": {"port": addr.port(), "username": user},
        });
        let Value::Object(data) = data else {
            unreachable!()
        };
        let mut secrets = BTreeMap::new();
        if let Some(password) = password {
            secrets.insert("ssh.password".to_owned(), Some(password.clone()));
        }
        let view = self
            .core
            .record_save(RecordSaveInput {
                vault_id: self.vault_id,
                record_id: None,
                record_type: RecordType::Host,
                data,
                secrets,
            })
            .expect("host saved");
        match view {
            RecordView::Host(view) => view.record_id,
            RecordView::Group(_) => unreachable!(),
        }
    }

    fn open(&self, record_id: Uuid) -> Session {
        let terminal = Arc::new(RecordingTerminal::default());
        let opened = self
            .manager
            .open(
                OpenRequest::Ssh(SessionTarget::Host {
                    vault_id: self.vault_id,
                    record_id,
                }),
                (100, 30),
                terminal.clone(),
            )
            .expect("the session opens");
        let view = opened.handle.view();
        assert!(
            terminal.session.lock().unwrap().is_some(),
            "the terminal is attached before output flows"
        );
        Session {
            handle: opened.handle,
            events: opened.events,
            terminal,
            states: vec![view.state],
            prompts: Vec::new(),
            view,
        }
    }

    fn set_autoreconnect(&self, on: bool) {
        let mut values = BTreeMap::new();
        values.insert("terminal.autoreconnect".to_owned(), json!(on));
        self.core
            .settings_set(SettingsSetInput {
                values,
                reset: Vec::new(),
            })
            .expect("setting saved");
    }

    /// Fingerprints saved in known hosts.
    fn known_fingerprints(&self) -> Vec<String> {
        self.core
            .known_hosts_list()
            .expect("known hosts")
            .into_iter()
            .map(|known| known.fingerprint)
            .collect()
    }

    fn password_host(&self) -> Uuid {
        let user = self.server.password_user.clone();
        self.host(self.server.addr, &user.name, Some(&user.secret))
    }
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.manager.shutdown();
    }
}

/// Reads the server's identification line.
fn banner(addr: SocketAddr) -> String {
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(10)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .expect("timeout");
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while line.len() < 255 {
        match stream.read(&mut byte) {
            Ok(1) if byte[0] == b'\n' => break,
            Ok(1) => line.push(byte[0]),
            _ => break,
        }
    }
    let _ = stream.write_all(b"SSH-2.0-shellpp_probe\r\n");
    String::from_utf8_lossy(&line).trim_end().to_owned()
}

/// The provider itself: both addresses answer as SSH servers, the key
/// directory holds the generated key pair, and the server-side faults the
/// session tests rely on work.
#[test]
fn test_server_starts_and_its_faults_work() {
    let server = TestSshd::start();
    assert!(server.addr.ip().is_loopback());
    assert!(banner(server.addr).starts_with("SSH-2.0-"));
    assert!(banner(server.ki_addr).starts_with("SSH-2.0-"));

    let private = std::fs::read_to_string(server.key_dir.path().join("id_ed25519")).unwrap();
    assert!(private.starts_with("-----BEGIN OPENSSH PRIVATE KEY-----"));
    let public = std::fs::read_to_string(server.key_dir.path().join("id_ed25519.pub")).unwrap();
    assert!(public.starts_with("ssh-ed25519 "));

    let before = server.host_key_fingerprints();
    assert!(!before.is_empty());
    assert!(before.iter().all(|fp| fp.starts_with("SHA256:")));
    server.rotate_host_key();
    let after = server.host_key_fingerprints();
    assert!(!after.is_empty());
    assert!(
        after.iter().all(|fp| !before.contains(fp)),
        "every host key is new after the rotation"
    );
    assert!(banner(server.addr).starts_with("SSH-2.0-"));

    // Dropping is safe at any time and the server keeps accepting.
    let _ = server.drop_connections();
    assert!(banner(server.addr).starts_with("SSH-2.0-"));
    if server.kind == ServerKind::Russh {
        assert!(server.connection_count().is_some());
    }
}

/// Password: the first connection asks for the host key (TOFU, accepted and
/// saved), the saved password signs in without a prompt, commands run,
/// `exit 0` closes the session (the tab closes, UI 4.6) and the connection
/// log has the session (host, user).
#[test]
fn password_session_runs_and_exit_zero_closes() {
    let harness = Harness::new();
    let host = harness.password_host();
    let mut session = harness.open(host);
    let fingerprint = session.accept_host_key();
    assert!(
        harness
            .server
            .host_key_fingerprints()
            .contains(&fingerprint),
        "the dialog shows the server's key"
    );
    session.wait_state(SessionState::Connected);
    assert_eq!(session.prompts, ["host_key_unknown"], "no password prompt");
    assert_eq!(
        session.view.username.as_deref(),
        Some(harness.server.password_user.name.as_str())
    );
    session.run_marker(&harness.server);

    session.type_line("exit 0");
    session.wait_state(SessionState::Closed);
    assert!(!session.states.contains(&SessionState::Dropped));

    let recent = harness
        .core
        .recent_connections()
        .expect("recent connections");
    assert!(
        recent
            .iter()
            .any(|row| row.host.as_ref().is_some_and(|h| h.record_id == host)
                && row.username.as_deref() == Some(harness.server.password_user.name.as_str())),
        "the connection log has the session"
    );
}

/// Key file: `id_ed25519` of the injected key directory (never `~/.ssh`)
/// signs in without any prompt besides the host key.
#[test]
fn key_file_session() {
    let harness = Harness::new();
    let user = harness.server.key_user.clone();
    let host = harness.host(harness.server.addr, &user, None);
    let mut session = harness.open(host);
    session.accept_host_key();
    session.wait_state(SessionState::Connected);
    assert_eq!(session.prompts, ["host_key_unknown"]);
    session.run_marker(&harness.server);
    session.handle.close();
    session.wait_state(SessionState::Closed);
}

/// Keyboard-interactive: the server's question is asked as a prompt, one at
/// a time, masked (echo off); the answer signs in.
#[test]
fn keyboard_interactive_session() {
    let harness = Harness::new();
    let user = harness.server.ki_user.clone();
    let host = harness.host(harness.server.ki_addr, &user.name, None);
    let mut session = harness.open(host);
    session.accept_host_key();
    let (id, kind) = session.wait_prompt("keyboard_interactive");
    let PromptKind::KeyboardInteractive {
        prompt,
        echo,
        index,
        count,
        error,
        ..
    } = &kind
    else {
        unreachable!()
    };
    assert_eq!(prompt.trim(), servers::KI_PROMPT.trim());
    assert!(!echo);
    assert!(kind.is_secret(), "the answer is masked");
    assert_eq!(*count, 1, "one question");
    assert!(*index <= 1, "the first question");
    assert!(!error);
    session.reply_text(id, &user.secret);
    session.wait_state(SessionState::Connected);
    session.run_marker(&harness.server);
    session.handle.close();
    session.wait_state(SessionState::Closed);
}

/// A wrong saved password: the password is asked again with an error, the
/// right one signs in (UI 4.8: try again until the server closes).
#[test]
fn wrong_saved_password_is_asked_again() {
    let harness = Harness::new();
    let user = harness.server.password_user.clone();
    let wrong = SecretString::from(format!("{}-wrong", user.secret.expose_secret()));
    let host = harness.host(harness.server.addr, &user.name, Some(&wrong));
    let mut session = harness.open(host);
    session.accept_host_key();
    let (id, kind) = session.wait_prompt("password");
    let PromptKind::Password { error, .. } = kind else {
        unreachable!()
    };
    assert!(error, "the form says the password was wrong");
    session.reply_text(id, &user.secret);
    session.wait_state(SessionState::Connected);
    session.run_marker(&harness.server);
    session.handle.close();
}

/// Wrong passwords until the server gives up: the session fails with a
/// clear reason in the tab (no panic, no automatic retry).
#[test]
fn wrong_passwords_fail_with_a_clear_reason() {
    let harness = Harness::new();
    let user = harness.server.password_user.clone();
    let wrong = SecretString::from(format!("{}-wrong", user.secret.expose_secret()));
    let host = harness.host(harness.server.addr, &user.name, Some(&wrong));
    let mut session = harness.open(host);
    session.accept_host_key();
    for _ in 0..20 {
        let view = session.wait("a password prompt or the failure", |view| {
            view.prompt
                .as_ref()
                .is_some_and(|p| p.kind.name() == "password")
                || view.state == SessionState::Failed
        });
        if view.state == SessionState::Failed {
            break;
        }
        let id = view.prompt.as_ref().map(|p| p.id).unwrap();
        session.reply_text(id, &wrong);
    }
    let view = session.wait_state(SessionState::Failed).clone();
    assert!(
        matches!(
            view.error,
            Some(SessionError::Ssh(
                FailureKind::AuthFailed | FailureKind::Disconnected
            ))
        ),
        "{view:?}"
    );
    assert!(view.message().is_some_and(|text| !text.is_empty()));
    assert!(view.reconnect.is_none(), "a first attempt is never retried");
}

/// First connection (UI 10.2): "Connect once" does not save the key (the
/// next connection asks again), "Cancel" fails the attempt and saves
/// nothing, "Accept and save" saves it (the next connection does not ask).
#[test]
fn first_connection_dialog_choices() {
    let harness = Harness::new();
    let host = harness.password_host();

    let mut once = harness.open(host);
    let (id, _) = once.wait_prompt("host_key_unknown");
    once.reply(id, PromptReply::HostKey(HostKeyChoice::ConnectOnce));
    once.wait_state(SessionState::Connected);
    once.handle.close();
    once.wait_state(SessionState::Closed);
    assert!(harness.known_fingerprints().is_empty());

    let mut cancelled = harness.open(host);
    let (id, _) = cancelled.wait_prompt("host_key_unknown");
    cancelled.reply(id, PromptReply::HostKey(HostKeyChoice::Cancel));
    let view = cancelled.wait_state(SessionState::Failed).clone();
    assert!(view.message().is_some());
    assert!(harness.known_fingerprints().is_empty());

    let mut saved = harness.open(host);
    let fingerprint = saved.accept_host_key();
    saved.wait_state(SessionState::Connected);
    saved.handle.close();
    assert_eq!(harness.known_fingerprints(), [fingerprint]);

    let mut again = harness.open(host);
    again.wait_state(SessionState::Connected);
    assert!(again.prompts.is_empty(), "a saved key is not asked again");
    again.handle.close();
}

/// A changed host key (UI 10.2): the connection is blocked with both
/// fingerprints; "Cancel" keeps the saved key; only "Replace saved key"
/// continues and replaces it.
#[test]
fn changed_host_key_blocks_until_replaced() {
    let harness = Harness::new();
    let host = harness.password_host();
    let mut first = harness.open(host);
    let old = first.accept_host_key();
    first.wait_state(SessionState::Connected);
    first.handle.close();
    first.wait_state(SessionState::Closed);

    harness.server.rotate_host_key();

    let mut blocked = harness.open(host);
    let (id, kind) = blocked.wait_prompt("host_key_changed");
    let PromptKind::HostKeyChanged {
        key,
        old_fingerprint,
        ..
    } = kind
    else {
        unreachable!()
    };
    assert_eq!(old_fingerprint, old);
    assert_ne!(key.fingerprint, old);
    assert!(
        harness
            .server
            .host_key_fingerprints()
            .contains(&key.fingerprint)
    );
    // Only "Replace" or "Cancel" fit this dialog.
    assert!(
        blocked
            .handle
            .reply(id, PromptReply::HostKey(HostKeyChoice::AcceptAndSave))
            .is_err()
    );
    blocked.reply(id, PromptReply::HostKey(HostKeyChoice::Cancel));
    blocked.wait_state(SessionState::Failed);
    assert_eq!(harness.known_fingerprints(), std::slice::from_ref(&old));

    let mut replaced = harness.open(host);
    let (id, kind) = replaced.wait_prompt("host_key_changed");
    let PromptKind::HostKeyChanged { key, .. } = kind else {
        unreachable!()
    };
    replaced.reply(id, PromptReply::HostKey(HostKeyChoice::Replace));
    replaced.wait_state(SessionState::Connected);
    replaced.run_marker(&harness.server);
    replaced.handle.close();
    assert_eq!(
        harness.known_fingerprints(),
        std::slice::from_ref(&key.fingerprint)
    );

    let mut after = harness.open(host);
    after.wait_state(SessionState::Connected);
    assert!(after.prompts.is_empty());
    after.handle.close();
}

/// A dropped connection with Autoreconnect on (the default): the session
/// shows the reason and the countdown, keeps its output and connects again
/// by itself; the new shell works.
#[test]
fn dropped_connection_reconnects_automatically() {
    let harness = Harness::new();
    let host = harness.password_host();
    let mut session = harness.open(host);
    session.accept_host_key();
    session.wait_state(SessionState::Connected);
    session.run_marker(&harness.server);
    let output_before = session.terminal.len();

    assert!(harness.server.drop_connections() > 0);
    let view = session
        .wait("the drop with the reconnect countdown", |view| {
            view.state == SessionState::Dropped && view.reconnect.is_some()
        })
        .clone();
    let error = view.error.expect("a drop reason");
    assert!(error.is_transport(), "{error:?}");
    assert!(view.message().is_some_and(|text| !text.is_empty()));
    let reconnect = view
        .reconnect
        .expect("automatic reconnection is on by default");
    assert_eq!(reconnect.attempt, 1);

    let view = session.wait_state(SessionState::Connected).clone();
    assert!(view.error.is_none() && view.reconnect.is_none());
    assert!(
        session.terminal.len() >= output_before,
        "the output is kept"
    );
    assert_eq!(
        session.prompts,
        ["host_key_unknown"],
        "a reconnect asks nothing (the key is saved)"
    );
    session.run_marker(&harness.server);
    session.handle.close();
    session.wait_state(SessionState::Closed);
}

/// A dropped connection with Autoreconnect off: the session stays dropped
/// with the reason and no countdown; "Reconnect" connects again.
#[test]
fn dropped_connection_without_autoreconnect_waits_for_reconnect() {
    let harness = Harness::new();
    harness.set_autoreconnect(false);
    let host = harness.password_host();
    let mut session = harness.open(host);
    session.accept_host_key();
    session.wait_state(SessionState::Connected);

    assert!(harness.server.drop_connections() > 0);
    let view = session.wait_state(SessionState::Dropped).clone();
    assert!(view.reconnect.is_none());
    // Longer than the first automatic delay (1 s): nothing happens.
    std::thread::sleep(Duration::from_millis(2500));
    session.drain();
    assert_eq!(session.view.state, SessionState::Dropped);

    session.handle.reconnect();
    session.wait_state(SessionState::Connected);
    session.run_marker(&harness.server);
    session.handle.close();
}

/// A non-zero exit status ends the session without a reconnect (UI 4.6):
/// the output stays, "Session ended with code 3".
#[test]
fn non_zero_exit_ends_without_reconnect() {
    let harness = Harness::new();
    let host = harness.password_host();
    let mut session = harness.open(host);
    session.accept_host_key();
    session.wait_state(SessionState::Connected);
    session.type_line("exit 3");
    let view = session.wait_state(SessionState::Ended).clone();
    assert_eq!(view.exit_status, Some(3));
    assert!(view.reconnect.is_none());
    assert!(view.message().is_some_and(|text| text.contains('3')));
    std::thread::sleep(Duration::from_millis(1500));
    session.drain();
    assert_eq!(session.view.state, SessionState::Ended);
    assert!(!session.states.contains(&SessionState::Dropped));
}

/// An SFTP session (the SFTP tab, stage 4, item 1): the same first
/// connection dialog as a terminal, then the `sftp` subsystem instead of a
/// shell. OpenSSH lists the home folder through it; the russh test server
/// has no SFTP and the session fails with a reason instead of hanging.
#[test]
fn sftp_session_lists_the_home_folder() {
    let harness = Harness::new();
    let host = harness.password_host();
    let opened = harness
        .manager
        .open_sftp(SessionTarget::Host {
            vault_id: harness.vault_id,
            record_id: host,
        })
        .expect("the sftp session opens");
    let view = opened.handle.view();
    assert_eq!(view.kind, shellpp_core::service::dto::SessionKind::Sftp);
    let terminal = Arc::new(RecordingTerminal::default());
    let mut session = Session {
        handle: opened.handle,
        events: opened.events,
        terminal,
        states: vec![view.state],
        prompts: Vec::new(),
        view,
    };
    session.accept_host_key();
    if harness.server.kind == ServerKind::Russh {
        let view = session.wait_state(SessionState::Failed);
        assert!(view.error.is_some(), "a reason is shown");
        assert!(session.handle.sftp().is_none());
        return;
    }
    session.wait_state(SessionState::Connected);
    let sftp = session.handle.sftp().expect("the sftp channel");
    let listed = session.handle.spawn(async move {
        let home = sftp.canonicalize(".").await?;
        sftp.list(&home).await.map(|entries| (home, entries))
    });
    let (home, entries) = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(listed)
        .expect("joined")
        .expect("listed");
    assert!(home.starts_with('/'), "{home}");
    assert!(
        entries
            .iter()
            .all(|entry| entry.name != "." && entry.name != "..")
    );
    session.handle.close();
    session.wait_state(SessionState::Closed);
    assert!(session.handle.sftp().is_none());
}

/// Export to host (UI 7.3; stage 4, item 8): a connection without a tab
/// accepts only a host key Known Hosts holds, so the export fails before
/// the first connection; after it, OpenSSH gets the key once (the second
/// export finds it). The key is generated at run time.
#[test]
fn export_to_host_appends_the_key_once() {
    let harness = Harness::new();
    let host = harness.password_host();
    let target = shellpp_core::service::dto::RecordRef {
        vault_id: harness.vault_id,
        record_id: host,
    };
    let text = shellpp_core::keys::generate::generate(
        shellpp_core::keys::generate::KeyKind::Ed25519,
        "export@test",
        None,
    )
    .expect("key");
    let line = ssh_key::PrivateKey::from_openssh(text.as_bytes())
        .expect("parsed")
        .public_key()
        .to_openssh()
        .expect("public key line");
    let runtime = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap();
    let before = runtime.block_on(harness.manager.export_key(vec![target], line.clone()));
    assert_eq!(
        before,
        vec![(target, Err(shellpp_ssh::FailureKind::HostKeyRejected))],
        "an unknown host key is never accepted without a dialog"
    );
    // The first connection saves the host key.
    let mut session = harness.open(host);
    session.accept_host_key();
    session.wait_state(SessionState::Connected);
    session.handle.close();
    if harness.server.kind == ServerKind::Russh {
        // The russh test server runs no shell commands.
        return;
    }
    let first = runtime.block_on(harness.manager.export_key(vec![target], line.clone()));
    assert_eq!(first, vec![(target, Ok(shellpp_ssh::ExportOutcome::Added))]);
    let second = runtime.block_on(harness.manager.export_key(vec![target], line));
    assert_eq!(
        second,
        vec![(target, Ok(shellpp_ssh::ExportOutcome::AlreadyPresent))]
    );
}

/// The SFTP tab's operations on a host (stage 4 tests: choose a host,
/// upload a folder, rename, change permissions, delete), through the
/// channel of an SFTP session as the panels use it. OpenSSH only (the
/// russh test server has no SFTP).
#[test]
fn sftp_session_uploads_renames_changes_permissions_and_deletes() {
    let harness = Harness::new();
    if harness.server.kind == ServerKind::Russh {
        return;
    }
    let host = harness.password_host();
    let opened = harness
        .manager
        .open_sftp(SessionTarget::Host {
            vault_id: harness.vault_id,
            record_id: host,
        })
        .expect("opens");
    let view = opened.handle.view();
    let mut session = Session {
        handle: opened.handle,
        events: opened.events,
        terminal: Arc::new(RecordingTerminal::default()),
        states: vec![view.state],
        prompts: Vec::new(),
        view,
    };
    session.accept_host_key();
    session.wait_state(SessionState::Connected);
    let sftp = session.handle.sftp().expect("channel");
    let local = tempfile::tempdir().expect("temp");
    let folder = local.path().join("site");
    std::fs::create_dir_all(folder.join("css")).unwrap();
    std::fs::write(folder.join("index.html"), b"<html>").unwrap();
    std::fs::write(folder.join("css").join("a.css"), b"body{}").unwrap();
    let work = session.handle.spawn(async move {
        let home = sftp.canonicalize(".").await?;
        let remote = shellpp_ssh::sftp::join(&home, &format!("tab-{}", std::process::id()));
        let mut progress = |_: u64| {};
        let summary =
            shellpp_ssh::transfer::upload_tree(&sftp, &folder, &remote, &mut progress).await?;
        let renamed = format!("{remote}-renamed");
        sftp.rename(&remote, &renamed).await?;
        let changed = sftp.set_permissions_tree(&renamed, 0o750).await?;
        let mode = sftp
            .stat(&shellpp_ssh::sftp::join(&renamed, "index.html"))
            .await?
            .permissions;
        let removed = sftp.remove_tree(&renamed).await?;
        let gone = sftp.stat(&renamed).await.is_err();
        Ok::<_, shellpp_ssh::SftpError>((summary.files, changed, mode, removed, gone))
    });
    let (files, changed, mode, removed, gone) = tokio::runtime::Builder::new_current_thread()
        .build()
        .unwrap()
        .block_on(work)
        .expect("joined")
        .expect("the operations");
    assert_eq!(files, 2);
    assert!(changed >= 3, "{changed}");
    assert_eq!(mode.map(|mode| mode & 0o777), Some(0o750));
    assert!(removed >= 3, "{removed}");
    assert!(gone);
    session.handle.close();
}
