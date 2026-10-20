//! Command history through the whole session layer: a real terminal model
//! as the session's terminal (keys through `TerminalHandle::send_typed`, as
//! the view sends them), the russh test server's shell (server-side echo,
//! `read-secret` with echo off, `alt-screen`) and the real core in a
//! temporary profile. History is recorded for saved hosts only.

use std::time::Instant;

use shellpp_core::service::dto::HistoryListInput;
use shellpp_test_sshd::UserConfig;

use super::*;
use crate::terminal::TerminalHandle;

/// Types `text` one key per write, as the terminal view does.
fn type_keys(terminal: &TerminalHandle, text: &str) {
    for c in text.chars() {
        terminal.send_typed(c.to_string().into_bytes());
    }
}

/// Waits until a row of the screen satisfies `matches`.
async fn screen(terminal: &TerminalHandle, what: &str, matches: impl Fn(&str) -> bool) {
    let deadline = Instant::now() + WAIT;
    loop {
        if terminal.screen_text().iter().any(|row| matches(row)) {
            return;
        }
        assert!(Instant::now() < deadline, "timeout waiting for {what}");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// The stored commands of `host` (newest first), once there are `count`.
async fn stored(core: &Core, host: Option<RecordRef>, count: usize) -> Vec<String> {
    let deadline = Instant::now() + WAIT;
    loop {
        let commands: Vec<String> = core
            .history_list(HistoryListInput {
                host,
                text: None,
                limit: None,
            })
            .unwrap()
            .into_iter()
            .map(|entry| entry.command)
            .collect();
        if commands.len() >= count {
            return commands;
        }
        assert!(
            Instant::now() < deadline,
            "timeout waiting for {count} entries, have {commands:?}"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

/// Opens `target` with a real terminal model, accepts the host key and
/// answers the username and password when asked; waits for the shell prompt.
async fn connect(
    harness: &Harness,
    target: SessionTarget,
    user: &str,
    password: &str,
) -> (TestSession, TerminalHandle, async_channel::Receiver<()>) {
    let (terminal, woken) = TerminalHandle::headless(80, 24);
    let opened = harness
        .manager
        .open(
            OpenRequest::Ssh(target.clone()),
            (80, 24),
            Arc::new(terminal.clone()) as Arc<dyn SessionTerminal>,
        )
        .expect("session opens");
    let mut session = TestSession {
        view: opened.handle.view(),
        handle: opened.handle,
        events: opened.events,
        // Unused: the output goes to the terminal model.
        terminal: Arc::new(TestTerminal::default()),
        states: Vec::new(),
        prompts: Vec::new(),
        answered: 0,
    };
    session.drain();
    session.accept_key().await;
    if matches!(target, SessionTarget::Quick { .. }) {
        session.answer("username", user, false).await;
        session.answer("password", password, false).await;
    }
    session.state(SessionState::Connected).await;
    let prompt = shellpp_test_sshd::shell::prompt(user);
    let prompt = prompt.trim_end().to_owned();
    screen(&terminal, "the prompt", |row| row.starts_with(&prompt)).await;
    (session, terminal, woken)
}

/// Runs `command` at the prompt and waits for the row `done`.
async fn run(terminal: &TerminalHandle, command: &str, done: &str) {
    type_keys(terminal, command);
    type_keys(terminal, "\r");
    screen(terminal, done, |row| row == done).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_saved_host_records_its_commands_but_never_hidden_input() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![
        UserConfig::new("alice").with_password(secret(&password)),
    ])
    .await;
    let target = harness.password_host(&server, "alice", &password);
    let SessionTarget::Host {
        vault_id,
        record_id,
    } = target.clone()
    else {
        unreachable!("a saved host")
    };
    let host = RecordRef {
        vault_id,
        record_id,
    };
    let (session, terminal, _woken) = connect(&harness, target, "alice", &password).await;

    run(&terminal, "echo hello", "hello").await;
    // `read-secret`: the server's password prompt with echo off.
    type_keys(&terminal, "read-secret\r");
    screen(&terminal, "the secret prompt", |row| {
        row.starts_with(shellpp_test_sshd::shell::SECRET_PROMPT.trim_end())
    })
    .await;
    let hidden = random_secret("Hidden");
    type_keys(&terminal, &hidden);
    type_keys(&terminal, "\r");
    screen(&terminal, "ok", |row| row == "ok").await;
    // A full-screen program, left with `q`.
    type_keys(&terminal, "alt-screen\r");
    screen(&terminal, "the alternate screen", |row| {
        row.contains(shellpp_test_sshd::shell::ALT_SCREEN_TEXT)
    })
    .await;
    type_keys(&terminal, "q");
    screen(&terminal, "the prompt again", |row| {
        row.starts_with("alice@shellpp-test:~$") && !row.contains("alt-screen")
    })
    .await;
    // A leading space: not kept.
    run(&terminal, " echo private", "private").await;
    run(&terminal, "echo done", "done").await;

    // `read-secret` itself is not kept either: it contains "secret"
    // (docs/security.md, section 10.2, S5).
    let commands = stored(&harness.core, Some(host), 3).await;
    assert_eq!(commands, ["echo done", "alt-screen", "echo hello"]);
    assert!(commands.iter().all(|command| !command.contains(&hidden)));
    session.handle.close();
    server.shutdown().await;
}

/// Waits until the session read its "Record history" flag more than
/// `after` times, and a moment more for the value to be stored.
async fn flag_read(harness: &Harness, after: usize) {
    let deadline = Instant::now() + WAIT;
    while harness.calls.history_flag_reads.load(Ordering::SeqCst) <= after {
        assert!(Instant::now() < deadline, "timeout waiting for the flag");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
}

/// "Record command history" off: captured lines do not leave the terminal
/// (docs/security.md, section 10.1, rule 4); turned on again, the next
/// command is recorded.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn recording_off_keeps_lines_in_the_terminal() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![
        UserConfig::new("alice").with_password(secret(&password)),
    ])
    .await;
    let target = harness.password_host(&server, "alice", &password);
    let SessionTarget::Host {
        vault_id,
        record_id,
    } = target.clone()
    else {
        unreachable!("a saved host")
    };
    let host = RecordRef {
        vault_id,
        record_id,
    };
    harness.set("terminal.record_history", json!(false));
    let (session, terminal, _woken) = connect(&harness, target, "alice", &password).await;
    flag_read(&harness, 0).await;
    run(&terminal, "echo off", "off").await;
    // Recording is asynchronous: give it the time a recorded line needs.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(harness.calls.history_calls.load(Ordering::SeqCst), 0);

    harness.set("terminal.record_history", json!(true));
    let reads = harness.calls.history_flag_reads.load(Ordering::SeqCst);
    crate::sessions::ui::refresh_history_flags();
    flag_read(&harness, reads).await;
    run(&terminal, "echo on", "on").await;
    assert_eq!(stored(&harness.core, Some(host), 1).await, ["echo on"]);
    assert_eq!(harness.calls.history_calls.load(Ordering::SeqCst), 1);
    session.handle.close();
    server.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_quick_connect_records_nothing() {
    let harness = Harness::new();
    let password = random_secret("pw");
    let server = start_server(vec![
        UserConfig::new("alice").with_password(secret(&password)),
    ])
    .await;
    let target = SessionTarget::Quick {
        address: "127.0.0.1".to_owned(),
        port: Some(server.port()),
        username: None,
    };
    let (session, terminal, _woken) = connect(&harness, target, "alice", &password).await;
    run(&terminal, "echo quick", "quick").await;
    // Recording is asynchronous: give it the time a saved host needs.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(stored(&harness.core, None, 0).await.is_empty());
    session.handle.close();
    server.shutdown().await;
}

/// The password user of a running OpenSSH test server (`server.json` of
/// `tests/openssh/start.sh`): its port, name and password. Only a loopback
/// address is accepted.
fn openssh_server(path: &str) -> (u16, String, String) {
    let text = std::fs::read_to_string(path).expect("server.json readable");
    let json: Value = serde_json::from_str(&text).expect("server.json is JSON");
    let host: std::net::IpAddr = json["host"]
        .as_str()
        .and_then(|host| host.parse().ok())
        .expect("server.json: host");
    assert!(host.is_loopback(), "the test server must be local");
    let port = u16::try_from(json["port"].as_u64().expect("server.json: port")).unwrap();
    let user = json
        .pointer("/users/password/name")
        .and_then(Value::as_str)
        .expect("server.json: password user");
    let password = json
        .pointer("/users/password/password")
        .and_then(Value::as_str)
        .expect("server.json: password");
    (port, user.to_owned(), password.to_owned())
}

/// Number of rows that look like a bash prompt of `user` (`user@...$`).
fn prompts(terminal: &TerminalHandle, user: &str) -> usize {
    terminal
        .screen_text()
        .iter()
        .filter(|row| row.starts_with(&format!("{user}@")) && row.contains("$"))
        .count()
}

/// Runs `keys` (Enter included) and waits for the next prompt.
async fn at_bash(terminal: &TerminalHandle, user: &str, keys: &str) {
    let before = prompts(terminal, user);
    type_keys(terminal, keys);
    let deadline = Instant::now() + WAIT;
    while prompts(terminal, user) <= before {
        assert!(Instant::now() < deadline, "timeout waiting for the prompt");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    // The prompt is drawn in pieces (title, colors): let it finish.
    tokio::time::sleep(Duration::from_millis(150)).await;
}

/// Waits for a row that starts with `text`, then answers it with `answer`
/// and Enter (a password prompt without echo).
async fn answer(terminal: &TerminalHandle, text: &str, answer: &str) {
    screen(terminal, text, |row| {
        row.trim_end().ends_with(text.trim_end())
    })
    .await;
    tokio::time::sleep(Duration::from_millis(100)).await;
    type_keys(terminal, answer);
    type_keys(terminal, "\r");
}

/// The owner's sequences against a real bash over OpenSSH in Docker:
/// `su` and `passwd` with their real prompts, `ssh-keygen` passphrases, a
/// bracketed paste through bash's own bracketed paste mode, `read -s`, a
/// leading space. Run it with a server that `tests/openssh/start.sh`
/// started:
///
/// ```text
/// SHELLPP_OPENSSH_SERVER_JSON=<workdir>/server.json \
///   cargo test -p shellpp-desktop --lib -- --ignored real_bash
/// ```
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "needs OpenSSH in Docker (SHELLPP_OPENSSH_SERVER_JSON)"]
async fn real_bash_over_openssh() {
    // Without a server there is nothing to run against (`--include-ignored`
    // on a machine without Docker passes).
    let Ok(path) = std::env::var("SHELLPP_OPENSSH_SERVER_JSON") else {
        return;
    };
    let (port, user, password) = openssh_server(&path);
    let harness = Harness::new();
    let target = harness.host(port, json!({"ssh": {"username": user}}), Some(&password));
    let SessionTarget::Host {
        vault_id,
        record_id,
    } = target.clone()
    else {
        unreachable!("a saved host")
    };
    let host = RecordRef {
        vault_id,
        record_id,
    };
    let (terminal, _woken) = TerminalHandle::headless(120, 60);
    let opened = harness
        .manager
        .open(
            OpenRequest::Ssh(target),
            (120, 60),
            Arc::new(terminal.clone()) as Arc<dyn SessionTerminal>,
        )
        .expect("session opens");
    let mut session = TestSession {
        view: opened.handle.view(),
        handle: opened.handle,
        events: opened.events,
        terminal: Arc::new(TestTerminal::default()),
        states: Vec::new(),
        prompts: Vec::new(),
        answered: 0,
    };
    session.drain();
    session.accept_key().await;
    session.state(SessionState::Connected).await;
    let deadline = Instant::now() + WAIT;
    while prompts(&terminal, &user) == 0 {
        assert!(Instant::now() < deadline, "no bash prompt");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    at_bash(&terminal, &user, "echo hello\r").await;
    // su: PAM's "Password: " with echo off; a wrong password.
    type_keys(&terminal, "su\r");
    answer(&terminal, "Password:", "Wrong-su-pw-1").await;
    let before = prompts(&terminal, &user);
    let deadline = Instant::now() + WAIT;
    while prompts(&terminal, &user) <= before {
        assert!(Instant::now() < deadline, "su did not return");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    // A paste: bash asked for bracketed paste at its prompt (bash 5.1+), so
    // it gets the markers, as the terminal view sends them.
    terminal.send_typed(crate::terminal::paste::paste_bytes("echo pasted", true));
    tokio::time::sleep(Duration::from_millis(200)).await;
    at_bash(&terminal, &user, "\r").await;
    // passwd: "Current password: " with echo off; a wrong one.
    type_keys(&terminal, "passwd\r");
    answer(&terminal, "Current password:", "Wrong-passwd-2").await;
    let before = prompts(&terminal, &user);
    let deadline = Instant::now() + WAIT;
    while prompts(&terminal, &user) <= before {
        assert!(Instant::now() < deadline, "passwd did not return");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    // ssh-keygen: two passphrase prompts without echo.
    // A new file each run (the container may be reused).
    let keygen = format!("ssh-keygen -q -t ed25519 -f /tmp/{}", random_secret("hk"));
    type_keys(&terminal, &format!("{keygen}\r"));
    answer(&terminal, "(empty for no passphrase):", "Key-phrase-3").await;
    answer(&terminal, "Enter same passphrase again:", "Key-phrase-3").await;
    let before = prompts(&terminal, &user);
    let deadline = Instant::now() + WAIT;
    while prompts(&terminal, &user) <= before {
        assert!(Instant::now() < deadline, "ssh-keygen did not return");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(150)).await;
    // read -s: no echo, no line feed; the next prompt follows on that row.
    type_keys(&terminal, "read -s v\r");
    tokio::time::sleep(Duration::from_millis(300)).await;
    at_bash(&terminal, &user, "Silent-4\r").await;
    at_bash(&terminal, &user, " echo private\r").await;
    at_bash(&terminal, &user, "echo done\r").await;

    let commands = stored(&harness.core, Some(host), 7).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let commands_now = stored(&harness.core, Some(host), 7).await;
    assert_eq!(commands, commands_now, "nothing else arrived");
    assert_eq!(
        commands,
        [
            "echo done",
            "read -s v",
            keygen.as_str(),
            "passwd",
            "echo pasted",
            "su",
            "echo hello",
        ]
    );
    for command in &commands {
        assert!(!command.contains("Wrong-"));
        assert!(!command.contains("Key-phrase"));
        assert!(!command.contains("Silent"));
        assert!(!command.contains('\u{1b}') && !command.contains("200~"));
    }
    session.handle.close();
}
