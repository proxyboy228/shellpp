//! Session manager against the real stack (stage 2 plan, section 4.5
//! "Tests"): `shellpp-core` (in a temporary data directory with the
//! in-memory platform), `shellpp-ssh` and the russh test server
//! `shellpp-test-sshd` on loopback, with collecting event and output sinks
//! instead of the Tauri runtime. The local terminal test starts the default
//! shell of this machine.
//!
//! Passwords and markers are generated at run time; no real hosts.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde_json::{Value, json};
use shellpp_core::platform::OsKind;
use shellpp_core::platform::memory::MemoryPlatform;
use shellpp_core::service::dto::{
    HistoryCandidate, HistoryListInput, HostKeyDecision, KnownHostAddInput, OsFamily, PromptKind,
    PromptReply, RecordRef, RecordSaveInput, RecordType, RecordView, SecretFieldRef, SessionInfo,
    SessionInjectInput, SessionOpenInput, SessionPromptReplyInput, SessionResizeInput,
    SessionState, SessionTarget, SessionWriteInput,
};
use shellpp_core::{Core, CoreConfig};
use shellpp_desktop::sessions::{
    ManagerParts, OutputSink, SessionEvents, SessionManager, SshConnector,
};
use shellpp_test_sshd::{ServerConfig, TestServer, UserConfig};
use tempfile::TempDir;
use uuid::Uuid;

const WINDOW: &str = "w-1";
const WAIT: Duration = Duration::from_secs(20);

fn random() -> String {
    Uuid::now_v7().simple().to_string()
}

// ---------------------------------------------------------------------------
// Sinks
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
}

#[derive(Clone, Default)]
struct Output(Arc<Mutex<Vec<u8>>>);

impl OutputSink for Output {
    fn send(&self, frame: Vec<u8>) -> bool {
        self.0.lock().unwrap().extend_from_slice(&frame);
        true
    }
}

impl Output {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }

    async fn wait_for(&self, needle: &str) {
        let deadline = tokio::time::Instant::now() + WAIT;
        while !self.text().contains(needle) {
            assert!(
                tokio::time::Instant::now() < deadline,
                "output never contained {needle:?}; got {:?}",
                self.text()
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

struct Stack {
    _dir: TempDir,
    core: Arc<Core>,
    vault_id: Uuid,
    events: Arc<Events>,
    manager: SessionManager,
}

fn stack() -> Stack {
    let dir = tempfile::tempdir().expect("temp dir");
    let memory = MemoryPlatform::new();
    let core = Core::open(
        CoreConfig {
            data_dir: dir.path().join("data"),
            app_version: "0.1.0".to_owned(),
        },
        memory.platform(OsKind::Windows, "TEST-PC"),
    )
    .expect("core opens");
    let password = SecretString::from(format!("Pw-{}-x", random()));
    core.first_run_setup(password).expect("first run");
    let vault_id = core.vaults().expect("vaults")[0].vault_id;
    let core = Arc::new(core);
    let events = Arc::new(Events::default());
    let manager = SessionManager::new(ManagerParts {
        events: events.clone(),
        core: core.clone(),
        connector: Arc::new(SshConnector::new(false)),
        #[cfg(desktop)]
        local: Arc::new(shellpp_desktop::sessions::local::PtyBackend::new()),
        runtime: tokio::runtime::Handle::current(),
        unlocked: true,
    });
    Stack {
        _dir: dir,
        core,
        vault_id,
        events,
        manager,
    }
}

impl Stack {
    /// A saved host for the test server.
    fn host(&self, port: u16, username: &str, password: Option<&str>) -> RecordRef {
        let data = json!({"label": "test server", "address": "127.0.0.1",
            "ssh": {"port": port, "username": username}});
        let secrets: BTreeMap<String, Option<SecretString>> = password
            .map(|password| {
                (
                    "ssh.password".to_owned(),
                    Some(SecretString::from(password.to_owned())),
                )
            })
            .into_iter()
            .collect();
        let view = self
            .core
            .record_save(RecordSaveInput {
                vault_id: self.vault_id,
                record_id: None,
                record_type: RecordType::Host,
                data: data.as_object().cloned().unwrap(),
                secrets,
            })
            .expect("host saved");
        RecordRef {
            vault_id: self.vault_id,
            record_id: view.record_id(),
        }
    }

    async fn open(&self, host: RecordRef) -> Uuid {
        self.manager
            .open_ssh(
                WINDOW,
                SessionOpenInput {
                    target: SessionTarget::Host {
                        vault_id: host.vault_id,
                        record_id: host.record_id,
                    },
                    cols: 80,
                    rows: 24,
                },
            )
            .await
            .expect("session_open")
            .session_id
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
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    async fn wait_state(&self, id: Uuid, state: SessionState) -> SessionInfo {
        self.wait(id, &format!("{state:?}"), |info| info.state == state)
            .await
    }

    async fn wait_prompt(&self, id: Uuid, matches: impl Fn(&PromptKind) -> bool) -> u64 {
        self.wait(id, "prompt", |info| {
            info.prompt
                .as_ref()
                .is_some_and(|prompt| matches(&prompt.kind))
        })
        .await
        .prompt
        .unwrap()
        .prompt_id
    }

    async fn wait_connections(&self, id: Uuid, n: usize) {
        let deadline = tokio::time::Instant::now() + WAIT;
        while self.events.connections(id) < n {
            assert!(
                tokio::time::Instant::now() < deadline,
                "connection {n} never happened"
            );
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn reply(&self, id: Uuid, prompt_id: u64, reply: PromptReply) {
        self.manager
            .prompt_reply(
                WINDOW,
                SessionPromptReplyInput {
                    session_id: id,
                    prompt_id,
                    reply,
                },
            )
            .expect("prompt reply");
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

    fn attach(&self, id: Uuid) -> Output {
        let output = Output::default();
        self.manager
            .attach(WINDOW, id, Box::new(output.clone()))
            .expect("session_attach");
        output
    }

    /// Opens a session to `host`, accepts the unknown key and waits for the prompt.
    async fn connect_accepting(&self, host: RecordRef) -> (Uuid, Output) {
        let id = self.open(host).await;
        let prompt = self
            .wait_prompt(id, |kind| matches!(kind, PromptKind::HostKeyUnknown { .. }))
            .await;
        self.reply(
            id,
            prompt,
            PromptReply::HostKey {
                decision: HostKeyDecision::AcceptSave,
            },
        );
        self.wait_state(id, SessionState::Connected).await;
        let output = self.attach(id);
        output.wait_for("@shellpp-test:~$ ").await;
        (id, output)
    }
}

async fn server(users: Vec<UserConfig>, configure: impl FnOnce(&mut ServerConfig)) -> TestServer {
    let mut config = ServerConfig::new(users);
    configure(&mut config);
    TestServer::start(config).await.expect("test server starts")
}

fn password_user(name: &str, password: &str) -> UserConfig {
    UserConfig::new(name).with_password(SecretString::from(password.to_owned()))
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn connect_tofu_echo_exit_codes_and_known_hosts() {
    let password = format!("pw-{}", random());
    let sshd = server(vec![password_user("alice", &password)], |_| {}).await;
    let s = stack();
    let host = s.host(sshd.port(), "alice", Some(&password));
    let id = s.open(host).await;
    let prompt = s
        .wait_prompt(id, |kind| matches!(kind, PromptKind::HostKeyUnknown { .. }))
        .await;
    let info = s.events.latest(id).unwrap();
    let Some(PromptKind::HostKeyUnknown { key }) = info.prompt.map(|p| p.kind) else {
        unreachable!()
    };
    assert_eq!(key.fingerprint, sshd.host_key_fingerprint());
    assert_eq!(key.key_type, sshd.host_key_type());
    assert_eq!(key.port, sshd.port());
    s.reply(
        id,
        prompt,
        PromptReply::HostKey {
            decision: HostKeyDecision::AcceptSave,
        },
    );
    let info = s.wait_state(id, SessionState::Connected).await;
    assert_eq!(info.username.as_deref(), Some("alice"));
    let output = s.attach(id);
    output.wait_for("Welcome to shellpp-test-sshd").await;
    output.wait_for("alice@shellpp-test:~$ ").await;
    let marker = format!("hello-{}", random());
    s.write(id, &format!("echo {marker}\r"));
    output.wait_for(&format!("{marker}\r\n")).await;
    s.write(id, "term\r");
    output.wait_for("xterm-256color").await;

    // Saved as a known host: the next connection does not ask.
    let known = s.core.known_hosts_list().expect("known hosts");
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].fingerprint, sshd.host_key_fingerprint());
    assert_eq!(known[0].host_ids, vec![host.record_id]);

    s.write(id, "exit 3\r");
    let info = s.wait_state(id, SessionState::Ended).await;
    assert_eq!(info.exit_status, Some(3));
    s.manager.reconnect(WINDOW, id).expect("reconnect");
    s.wait_connections(id, 2).await;
    output.wait_for("Welcome to shellpp-test-sshd").await;
    // The reconnect did not ask again (one prompt in total).
    let mut prompt_ids: Vec<u64> = s
        .events
        .of(id)
        .iter()
        .filter_map(|info| info.prompt.as_ref().map(|prompt| prompt.prompt_id))
        .collect();
    prompt_ids.dedup();
    assert_eq!(prompt_ids.len(), 1);
    s.write(id, "exit\r");
    s.wait_state(id, SessionState::Closed).await;
    assert!(s.manager.sessions_of(WINDOW).is_empty());

    // Recent connections come from the connection log.
    let recent = s.core.recent_connections().expect("recent");
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].host, Some(host));
    sshd.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn prompted_password_wrong_then_right_is_saved() {
    let password = format!("pw-{}", random());
    let sshd = server(vec![password_user("bob", &password)], |_| {}).await;
    let s = stack();
    let host = s.host(sshd.port(), "bob", None);
    let id = s.open(host).await;
    let prompt = s
        .wait_prompt(id, |kind| matches!(kind, PromptKind::HostKeyUnknown { .. }))
        .await;
    s.reply(
        id,
        prompt,
        PromptReply::HostKey {
            decision: HostKeyDecision::AcceptSave,
        },
    );
    let first = s
        .wait_prompt(id, |kind| {
            matches!(
                kind,
                PromptKind::Password {
                    error: false,
                    can_save: true,
                    ..
                }
            )
        })
        .await;
    s.reply(
        id,
        first,
        PromptReply::Text {
            value: SecretString::from(format!("wrong-{}", random())),
            save: true,
        },
    );
    let second = s
        .wait_prompt(id, |kind| {
            matches!(kind, PromptKind::Password { error: true, .. })
        })
        .await;
    assert_ne!(first, second);
    s.reply(
        id,
        second,
        PromptReply::Text {
            value: SecretString::from(password.clone()),
            save: true,
        },
    );
    s.wait_state(id, SessionState::Connected).await;
    let core = s.core.clone();
    let saved = tokio::task::spawn_blocking(move || {
        core.secret_reveal(SecretFieldRef {
            vault_id: host.vault_id,
            record_id: host.record_id,
            path: "ssh.password".into(),
        })
    })
    .await
    .unwrap()
    .expect("password saved");
    assert_eq!(saved.expose_secret(), password);
    s.manager.close(WINDOW, id).unwrap();
    sshd.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drop_reconnects_and_changed_key_needs_replace() {
    let password = format!("pw-{}", random());
    let sshd = server(vec![password_user("carol", &password)], |_| {}).await;
    let s = stack();
    let host = s.host(sshd.port(), "carol", Some(&password));
    let (id, output) = s.connect_accepting(host).await;

    // A drop: automatic reconnect after 1 s, output stays.
    assert!(sshd.drop_connections() >= 1);
    let info = s
        .wait(id, "dropped", |info| info.state == SessionState::Dropped)
        .await;
    assert_eq!(info.error.unwrap().reason.as_deref(), Some("disconnected"));
    s.wait_connections(id, 2).await;
    let marker = format!("after-{}", random());
    s.write(id, &format!("echo {marker}\r"));
    output.wait_for(&marker).await;

    // The server key changes: the automatic attempt asks, Replace continues.
    let old = sshd.host_key_fingerprint();
    sshd.rotate_host_key().await.expect("rotate");
    assert_ne!(sshd.host_key_fingerprint(), old);
    let prompt = s
        .wait_prompt(id, |kind| matches!(kind, PromptKind::HostKeyChanged { .. }))
        .await;
    let info = s.events.latest(id).unwrap();
    let Some(PromptKind::HostKeyChanged {
        key,
        old_fingerprint,
        ..
    }) = info.prompt.map(|p| p.kind)
    else {
        unreachable!()
    };
    assert_eq!(old_fingerprint, old);
    assert_eq!(key.fingerprint, sshd.host_key_fingerprint());
    s.reply(
        id,
        prompt,
        PromptReply::HostKey {
            decision: HostKeyDecision::Replace,
        },
    );
    s.wait_connections(id, 3).await;
    let known = s.core.known_hosts_list().unwrap();
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].fingerprint, sshd.host_key_fingerprint());
    s.manager.close(WINDOW, id).unwrap();
    sshd.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn revoked_key_is_blocked() {
    let password = format!("pw-{}", random());
    let sshd = server(vec![password_user("dave", &password)], |_| {}).await;
    let s = stack();
    let core = s.core.clone();
    let (key_type, blob) = (sshd.host_key_type(), sshd.host_key_base64());
    tokio::task::spawn_blocking(move || {
        core.known_host_add(KnownHostAddInput {
            host: "*.example.com".into(),
            port: None,
            key_type,
            public_key: blob,
            revoked: true,
            comment: None,
        })
    })
    .await
    .unwrap()
    .expect("seeded");
    let host = s.host(sshd.port(), "dave", Some(&password));
    let id = s.open(host).await;
    let info = s.wait_state(id, SessionState::Failed).await;
    assert_eq!(info.error.unwrap().code, "host_key_revoked");
    assert_eq!(
        info.host_key.unwrap().fingerprint,
        sshd.host_key_fingerprint()
    );
    // Nothing was sent before the decision: no authentication attempt.
    assert!(
        !sshd
            .events()
            .iter()
            .any(|event| matches!(event, shellpp_test_sshd::ServerEvent::Auth { .. }))
    );
    sshd.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn os_detection_history_and_pty_size() {
    let password = format!("pw-{}", random());
    let sshd = server(vec![password_user("erin", &password)], |config| {
        config.banner_comment = Some("Ubuntu-3ubuntu13".into());
    })
    .await;
    let s = stack();
    let host = s.host(sshd.port(), "erin", Some(&password));
    let (id, output) = s.connect_accepting(host).await;

    // OS from the banner, stored on the host.
    let info = s.wait(id, "os", |info| info.os.is_some()).await;
    let os = info.os.unwrap();
    assert_eq!(os.family, OsFamily::Linux);
    assert_eq!(os.distro.as_deref(), Some("ubuntu"));
    let core = s.core.clone();
    let deadline = tokio::time::Instant::now() + WAIT;
    loop {
        let record = core.record_get(host).expect("host");
        let RecordView::Host(view) = record else {
            unreachable!()
        };
        if view.data.get("os").is_some_and(Value::is_object) {
            assert_eq!(view.data["os"]["family"], "linux");
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "os not stored");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // Detection never ran a command: no exec request.
    assert!(
        !sshd
            .events()
            .iter()
            .any(|event| matches!(event, shellpp_test_sshd::ServerEvent::Exec { .. }))
    );

    // PTY size and resize.
    s.write(id, "size\r");
    output.wait_for("80x24").await;
    s.manager
        .resize(
            WINDOW,
            SessionResizeInput {
                session_id: id,
                cols: 100,
                rows: 30,
            },
        )
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    s.write(id, "size\r");
    output.wait_for("100x30").await;

    // History: a typed line is recorded, a line with injected input is not.
    let marker = format!("typed-{}", random());
    let candidate = |command: &str| HistoryCandidate {
        command: command.to_owned(),
        prompt_text: "erin@shellpp-test:~$ ".to_owned(),
        typed_printable: true,
        alt_screen: false,
    };
    s.manager
        .write(
            WINDOW,
            SessionWriteInput {
                session_id: id,
                data: format!("echo {marker}\r"),
                binary: false,
                history: Some(candidate(&format!("echo {marker}"))),
            },
        )
        .unwrap();
    output.wait_for(&format!("{marker}\r\n")).await;
    let injected = format!("injected-{}", random());
    s.manager
        .inject(
            WINDOW,
            SessionInjectInput {
                session_id: id,
                data: format!("echo {injected}"),
            },
        )
        .unwrap();
    s.manager
        .write(
            WINDOW,
            SessionWriteInput {
                session_id: id,
                data: "\r".into(),
                binary: false,
                history: Some(candidate(&format!("echo {injected}"))),
            },
        )
        .unwrap();
    output.wait_for(&format!("{injected}\r\n")).await;
    let core = s.core.clone();
    let deadline = tokio::time::Instant::now() + WAIT;
    let commands = loop {
        let list = core
            .history_list(HistoryListInput {
                host: Some(host),
                text: None,
                limit: None,
            })
            .expect("history");
        if !list.is_empty() {
            tokio::time::sleep(Duration::from_millis(200)).await;
            break core
                .history_list(HistoryListInput {
                    host: Some(host),
                    text: None,
                    limit: None,
                })
                .unwrap()
                .into_iter()
                .map(|entry| entry.command)
                .collect::<Vec<_>>();
        }
        assert!(tokio::time::Instant::now() < deadline, "history empty");
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(commands, vec![format!("echo {marker}")]);
    s.manager.close(WINDOW, id).unwrap();
    sshd.shutdown().await;
}

#[cfg(desktop)]
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn local_terminal_runs_the_default_shell() {
    use shellpp_core::service::dto::LocalOpenInput;
    let s = stack();
    let shells = s.manager.local_shells().await.expect("shells");
    assert!(!shells.shells.is_empty());
    assert!(shells.default_id.is_some());
    let info = s
        .manager
        .open_local(
            WINDOW,
            LocalOpenInput {
                shell_id: None,
                cols: 100,
                rows: 30,
            },
        )
        .await
        .expect("local_open");
    assert_eq!(info.state, SessionState::Connected);
    let id = info.session_id;
    let output = s.attach(id);
    let marker = format!("shellpp-{}", random());
    // Windows ConPTY asks for the cursor position (DSR) at start and waits
    // for the answer; xterm.js answers it in the app, this test does.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    while cfg!(windows)
        && !output.text().contains("\u{1b}[6n")
        && tokio::time::Instant::now() < deadline
    {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    if output.text().contains("\u{1b}[6n") {
        s.write(id, "\u{1b}[1;1R");
    }
    // Typing before the prompt appears is fine: the shell reads it.
    s.write(id, &format!("echo {marker}\r"));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while output.text().matches(&marker).count() < 2 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "marker never printed: {:?}",
            output.text()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let exit = if cfg!(windows) {
        "exit 3\r"
    } else {
        "exit 3\n"
    };
    s.write(id, exit);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let info = s.events.latest(id).unwrap();
        if info.state == SessionState::Ended {
            assert_eq!(info.exit_status, Some(3));
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "shell did not end");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    s.manager.close(WINDOW, id).unwrap();
}
