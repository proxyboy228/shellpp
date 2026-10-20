//! Known hosts import against a real server (owner's stage 2b, steps 1.5
//! and 3; UI 10.1, 10.2): a known_hosts file written at run time with a
//! HASHED entry (`|1|salt|hash`) for the server's real host key is imported,
//! and a connection through the app's session layer
//! (`shellpp_desktop::sessions::SessionManager`, the manager of the session
//! tabs) matches it without the first-connection dialog. An imported entry
//! with another key for the same address blocks the connection with the
//! changed-key dialog; an imported `@revoked` entry with the real key blocks
//! it without any dialog.
//!
//! The server is chosen like the session tests (`servers`: OpenSSH in Docker
//! when a Docker daemon runs, else the russh test server; the choice is
//! printed). Every profile is temporary (in-memory secret store), the
//! known_hosts files live in a temporary directory, and nothing reads the
//! real `~/.ssh`, the real agent or a real host. The server's host key is
//! learned the way a user would: a first connection in a separate profile
//! accepts it, and that profile's known_hosts export gives the key line.

mod servers;

use std::collections::BTreeMap;
use std::io::Write as _;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures::channel::mpsc::UnboundedReceiver;
use secrecy::SecretString;
use serde_json::{Value, json};
use servers::TestSshd;
use shellpp_core::Core;
use shellpp_core::known_hosts::{fingerprint_sha256, hash_host};
use shellpp_core::service::dto::{
    KnownHostsImportSummary, RecordSaveInput, RecordType, RecordView, VaultKind,
};
use shellpp_desktop::sessions::transport::BoxFuture;
use shellpp_desktop::sessions::{
    AgentSource, HostKeyChoice, KeySources, ManagerOptions, OpenRequest, PromptKind, PromptReply,
    SessionError, SessionEvent, SessionHandle, SessionManager, SessionState, SessionTarget,
    SessionTerminal, SessionView, TerminalClosed,
};
use shellpp_desktop::testing::TestProfile;
use shellpp_test_sshd::{KeyType, generate_key};
use uuid::Uuid;

/// Connecting on a busy CI machine.
const WAIT: Duration = Duration::from_secs(45);

/// The terminal stand-in: records the output.
#[derive(Default)]
struct RecordingTerminal {
    output: Mutex<Vec<u8>>,
}

impl SessionTerminal for RecordingTerminal {
    fn feed<'a>(&'a self, bytes: &'a [u8]) -> BoxFuture<'a, Result<(), TerminalClosed>> {
        Box::pin(async move {
            self.output.lock().unwrap().extend_from_slice(bytes);
            Ok(())
        })
    }

    fn attach(&self, _: SessionHandle) {}
}

/// One session as the test sees it.
struct Session {
    handle: SessionHandle,
    events: UnboundedReceiver<SessionEvent>,
    terminal: Arc<RecordingTerminal>,
    view: SessionView,
    /// Every prompt kind shown, in order.
    prompts: Vec<&'static str>,
}

impl Session {
    fn drain(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            let view = event.view().clone();
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

    fn wait(&mut self, what: &str, condition: impl Fn(&SessionView) -> bool) -> SessionView {
        let deadline = Instant::now() + WAIT;
        loop {
            self.drain();
            if condition(&self.view) {
                return self.view.clone();
            }
            assert!(
                Instant::now() < deadline,
                "timeout waiting for {what}; view: {:?}; prompts: {:?}",
                self.view,
                self.prompts
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// Waits until the session is connected or has stopped (failed,
    /// closed) or asks something.
    fn settle(&mut self) -> SessionView {
        self.wait("connected, a prompt or the end", |view| {
            matches!(
                view.state,
                SessionState::Connected | SessionState::Failed | SessionState::Closed
            ) || view.prompt.is_some()
        })
    }

    /// Runs the marker command and waits for its output.
    fn run_marker(&mut self, server: &TestSshd) {
        let commands = server.commands();
        let from = self.terminal.output.lock().unwrap().len();
        let deadline = Instant::now() + WAIT;
        let line = format!("{}\r", commands.marker);
        while self.handle.write(line.as_bytes()).is_err() {
            assert!(Instant::now() < deadline, "input not taken");
            std::thread::sleep(Duration::from_millis(20));
        }
        loop {
            let text = String::from_utf8_lossy(&self.terminal.output.lock().unwrap()).into_owned();
            if text
                .get(from..)
                .is_some_and(|rest| rest.contains(commands.marker_output))
            {
                return;
            }
            self.drain();
            assert!(Instant::now() < deadline, "timeout waiting for the marker");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn close(mut self) {
        self.handle.close();
        self.wait("closed", |view| {
            matches!(view.state, SessionState::Closed | SessionState::Failed)
        });
    }
}

/// A temporary profile with the session manager over its core and a saved
/// host for the server's password user. Fields drop in order: the manager,
/// the core, the profile.
struct Profile {
    manager: SessionManager,
    core: Arc<Core>,
    _events: shellpp_desktop::platform::events::EventReceiver,
    host: Uuid,
    vault_id: Uuid,
    _profile: TestProfile,
}

impl Profile {
    fn new(server: &TestSshd) -> Self {
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
        let host = save_host(&core, vault_id, server.addr, &server.password_user);
        Self {
            manager,
            core,
            _events: opened.events,
            host,
            vault_id,
            _profile: profile,
        }
    }

    fn open(&self) -> Session {
        let terminal = Arc::new(RecordingTerminal::default());
        let opened = self
            .manager
            .open(
                OpenRequest::Ssh(SessionTarget::Host {
                    vault_id: self.vault_id,
                    record_id: self.host,
                }),
                (100, 30),
                terminal.clone(),
            )
            .expect("the session opens");
        let view = opened.handle.view();
        Session {
            handle: opened.handle,
            events: opened.events,
            terminal,
            view,
            prompts: Vec::new(),
        }
    }

    /// Imports a known_hosts file written at run time into `dir`, the way
    /// the Known Hosts screen does (the file's bytes to the core).
    fn import(&self, dir: &std::path::Path, name: &str, text: &str) -> KnownHostsImportSummary {
        let path = dir.join(name);
        let mut file = std::fs::File::create(&path).expect("known_hosts file");
        file.write_all(text.as_bytes()).expect("written");
        drop(file);
        let bytes = std::fs::read(&path).expect("read back");
        self.core.known_hosts_import(&bytes).expect("imported")
    }
}

impl Drop for Profile {
    fn drop(&mut self) {
        self.manager.shutdown();
    }
}

fn save_host(core: &Core, vault_id: Uuid, addr: SocketAddr, user: &servers::Account) -> Uuid {
    let data = json!({
        "label": "known hosts test",
        "address": addr.ip().to_string(),
        "ssh": {"port": addr.port(), "username": user.name},
    });
    let Value::Object(data) = data else {
        unreachable!()
    };
    let mut secrets: BTreeMap<String, Option<SecretString>> = BTreeMap::new();
    secrets.insert("ssh.password".to_owned(), Some(user.secret.clone()));
    match core
        .record_save(RecordSaveInput {
            vault_id,
            record_id: None,
            record_type: RecordType::Host,
            data,
            secrets,
        })
        .expect("host saved")
    {
        RecordView::Host(view) => view.record_id,
        RecordView::Group(_) => unreachable!(),
    }
}

/// A different public key of the same algorithm: (key type, base64).
fn other_key(key_type: &str) -> (String, String) {
    let kind = match key_type {
        "ssh-ed25519" => KeyType::Ed25519,
        "ecdsa-sha2-nistp256" => KeyType::EcdsaP256,
        "ssh-rsa" => KeyType::Rsa3072,
        other => panic!("no generator for host key type {other}"),
    };
    let key = generate_key(kind, "other", None).expect("key generated");
    let mut parts = key.public_openssh.split_whitespace();
    let (Some(algorithm), Some(blob)) = (parts.next(), parts.next()) else {
        panic!("a public key line");
    };
    assert_eq!(algorithm, key_type);
    (algorithm.to_owned(), blob.to_owned())
}

/// Hashed known_hosts imported, then a connection matches it (no dialog);
/// another key for the address blocks with the changed-key dialog; a
/// revoked entry with the real key blocks without a dialog.
#[test]
fn imported_hashed_entries_are_checked_on_connect() {
    let server = TestSshd::start();
    {
        let mut stderr = std::io::stderr().lock();
        let _ = writeln!(stderr, "known_hosts test: server = {}", server.kind.label());
    }
    let files = tempfile::tempdir().expect("temporary directory");
    let addr = server.addr;
    let ip = addr.ip().to_string();

    // 1. The server's real host key, learned by a first connection ("Accept
    //    and save") in a separate profile and read from its export.
    let (key_type, public_key) = {
        let learn = Profile::new(&server);
        let mut session = learn.open();
        let view = session.settle();
        let Some(prompt) = view.prompt.clone() else {
            panic!("the first connection asks; view: {view:?}");
        };
        let PromptKind::HostKeyUnknown { key } = &prompt.kind else {
            panic!("a first-connection dialog, not {}", prompt.kind.name());
        };
        assert!(server.host_key_fingerprints().contains(&key.fingerprint));
        session
            .handle
            .reply(
                prompt.id,
                PromptReply::HostKey(HostKeyChoice::AcceptAndSave),
            )
            .expect("reply taken");
        session.wait("connected", |view| view.state == SessionState::Connected);
        session.close();
        let export = learn.core.known_hosts_export().expect("export");
        let line = export
            .lines()
            .find(|line| line.starts_with(&format!("[{ip}]:{} ", addr.port())))
            .unwrap_or_else(|| panic!("an exported line for the server"));
        let mut parts = line.split_whitespace().skip(1);
        let (key_type, public_key) = (parts.next().unwrap(), parts.next().unwrap());
        assert_eq!(
            fingerprint_sha256(public_key).as_deref(),
            Some(key.fingerprint.as_str())
        );
        (key_type.to_owned(), public_key.to_owned())
    };
    let salt: [u8; 20] = shellpp_core::crypto::random_array().expect("random");
    let hashed = hash_host(&salt, &ip, addr.port());

    // 2. A hashed entry with the real key: the connection matches it, with
    //    no first-connection dialog.
    {
        let profile = Profile::new(&server);
        let summary = profile.import(
            files.path(),
            "known_hosts_match",
            &format!("# written by the test\n{hashed} {key_type} {public_key}\n"),
        );
        assert_eq!(
            summary,
            KnownHostsImportSummary {
                imported: 1,
                duplicates: 0,
                skipped: 0,
                invalid: 0,
            }
        );
        let listed = profile.core.known_hosts_list().expect("list");
        assert_eq!(listed.len(), 1);
        assert!(listed[0].hashed, "stored as a hashed entry");
        assert_eq!(listed[0].host, hashed);
        assert_eq!(listed[0].display_address, None);
        assert_eq!(listed[0].host_ids, vec![profile.host]);
        let mut session = profile.open();
        let view = session.settle();
        assert_eq!(
            view.state,
            SessionState::Connected,
            "connected without a dialog; prompts: {:?}",
            session.prompts
        );
        assert!(
            !session
                .prompts
                .iter()
                .any(|name| name.starts_with("host_key")),
            "no host key dialog: {:?}",
            session.prompts
        );
        session.run_marker(&server);
        session.close();
        // Nothing new was saved: the hashed entry matched.
        assert_eq!(profile.core.known_hosts_list().expect("list").len(), 1);
    }

    // 3. A hashed entry with another key for the same address: blocked with
    //    the changed-key dialog showing the imported key's fingerprint.
    {
        let profile = Profile::new(&server);
        let (other_type, other_public) = other_key(&key_type);
        let other_salt: [u8; 20] = shellpp_core::crypto::random_array().expect("random");
        let other_hashed = hash_host(&other_salt, &ip, addr.port());
        profile.import(
            files.path(),
            "known_hosts_changed",
            &format!("{other_hashed} {other_type} {other_public}\n"),
        );
        let mut session = profile.open();
        let view = session.settle();
        let prompt = view
            .prompt
            .clone()
            .unwrap_or_else(|| panic!("the changed-key dialog; view: {view:?}"));
        let PromptKind::HostKeyChanged {
            key,
            old_fingerprint,
            ..
        } = &prompt.kind
        else {
            panic!("the changed-key dialog, not {}", prompt.kind.name());
        };
        assert_eq!(
            Some(old_fingerprint.as_str()),
            fingerprint_sha256(&other_public).as_deref()
        );
        assert_eq!(
            Some(key.fingerprint.clone()),
            fingerprint_sha256(&public_key)
        );
        assert_ne!(view.state, SessionState::Connected);
        session
            .handle
            .reply(prompt.id, PromptReply::HostKey(HostKeyChoice::Cancel))
            .expect("reply taken");
        let view = session.wait("failed", |view| view.state == SessionState::Failed);
        assert!(view.message().is_some());
        // The imported entry is kept as it was.
        let listed = profile.core.known_hosts_list().expect("list");
        assert_eq!(listed.len(), 1);
        assert_eq!(
            Some(listed[0].fingerprint.as_str()),
            fingerprint_sha256(&other_public).as_deref()
        );
    }

    // 4. A revoked entry with the real key: blocked, no dialog at all.
    {
        let profile = Profile::new(&server);
        let summary = profile.import(
            files.path(),
            "known_hosts_revoked",
            &format!(
                "@revoked {hashed} {key_type} {public_key}\n\
                 @cert-authority *.example.com {key_type} {public_key}\n"
            ),
        );
        assert_eq!((summary.imported, summary.skipped), (1, 1));
        let mut session = profile.open();
        let view = session.settle();
        assert_eq!(view.state, SessionState::Failed, "{view:?}");
        assert_eq!(view.error, Some(SessionError::HostKeyRevoked));
        assert!(
            session.prompts.is_empty(),
            "no dialog: {:?}",
            session.prompts
        );
        assert_eq!(
            view.host_key.as_ref().map(|key| key.fingerprint.clone()),
            fingerprint_sha256(&public_key)
        );
        assert!(view.reconnect.is_none(), "a revoked key is never retried");
    }
}
