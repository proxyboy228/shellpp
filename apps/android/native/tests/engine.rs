//! The prototype's engine end to end on the computer: a quick connect
//! through the shared session layer to the russh test server in process,
//! the first host key's question, the password prompt, a command and its
//! output on the terminal's screen, a resize and the close. The password is
//! made at run time.

use std::time::{Duration, Instant};

use secrecy::SecretString;
use serde_json::Value;
use shellpp_android::Engine;
use shellpp_android::screen;
use shellpp_test_sshd::{ServerConfig, TestServer, UserConfig};

const WAIT: Duration = Duration::from_secs(30);

fn random(prefix: &str) -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("random");
    let suffix: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{prefix}-{suffix}")
}

/// The next event of session `id` that `pick` accepts.
fn wait_for(engine: &Engine, id: u64, pick: impl Fn(&Value) -> bool) -> Value {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        let Some(event) = engine.next_event(Duration::from_millis(200)) else {
            continue;
        };
        let event: Value = serde_json::from_str(&event).expect("json");
        if event["id"] == id && pick(&event) {
            return event;
        }
    }
    panic!("no such event in time");
}

fn screen_has(engine: &Engine, id: u64, text: &str) -> bool {
    let deadline = Instant::now() + WAIT;
    while Instant::now() < deadline {
        let terminal = engine.terminal(id).expect("terminal");
        if terminal
            .screen_text()
            .iter()
            .any(|line| line.contains(text))
        {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_quick_connect_asks_the_host_key_and_the_password_then_runs_a_command() {
    let password = random("pw");
    let server = TestServer::start(ServerConfig::new(vec![
        UserConfig::new("alice").with_password(SecretString::from(password.clone())),
    ]))
    .await
    .expect("test server");
    let port = server.port();
    let files = tempfile::tempdir().expect("files");
    let marker = random("marker");

    let result = tokio::task::spawn_blocking(move || {
        let engine = Engine::start(files.path()).expect("engine");
        let id = engine
            .open("127.0.0.1", Some(port), Some("alice"), 80, 24, true)
            .expect("open");

        let asked = wait_for(&engine, id, |event| {
            event["prompt"]["kind"] == "host_key_unknown"
        });
        let fingerprint = asked["prompt"]["fingerprint"].as_str().unwrap_or_default();
        assert!(fingerprint.starts_with("SHA256:"), "{asked}");
        let prompt = asked["prompt"]["id"].as_u64().expect("prompt id");
        engine.reply(id, prompt, "accept", None).expect("accept");

        let asked = wait_for(&engine, id, |event| event["prompt"]["kind"] == "password");
        assert_eq!(asked["prompt"]["username"], "alice");
        let prompt = asked["prompt"]["id"].as_u64().expect("prompt id");
        engine
            .reply(id, prompt, "text", Some(SecretString::from(password)))
            .expect("password");
        wait_for(&engine, id, |event| event["state"] == "connected");

        engine
            .send(id, format!("echo {marker}\r").into_bytes())
            .expect("send");
        assert!(screen_has(&engine, id, &marker));
        let terminal = engine.terminal(id).expect("terminal");
        let lines = terminal.screen_text();
        assert!(
            lines.iter().filter(|line| line.contains(&marker)).count() >= 2,
            "the command and its output: {lines:?}"
        );
        let snapshot = screen::snapshot(&terminal);
        assert_eq!(u16::from_le_bytes([snapshot[0], snapshot[1]]), 80);

        // A new size reaches the server (the test shell's `size` command).
        engine.resize(id, 100, 30).expect("resize");
        engine.send(id, b"size\r".to_vec()).expect("send");
        assert!(
            screen_has(&engine, id, "100x30"),
            "{:?}",
            terminal.screen_text()
        );

        // A wake by hand reaches the next waiter (a recreated screen's
        // pump hands on the wake the old one took).
        engine.wake(id);
        assert!(engine.await_wake(id));

        // `exit 0` closes the session.
        engine.send(id, b"exit 0\r".to_vec()).expect("send");
        wait_for(&engine, id, |event| event["state"] == "closed");
        engine.close(id);
        assert!(engine.open_sessions().is_empty());
        engine.wake(id);
        assert!(!engine.await_wake(id), "a closed session has no wakes");
    })
    .await;
    drop(server);
    result.expect("the engine's steps");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_wrong_address_fails_with_the_session_layers_message() {
    let files = tempfile::tempdir().expect("files");
    tokio::task::spawn_blocking(move || {
        let engine = Engine::start(files.path()).expect("engine");
        let id = engine
            .open("127.0.0.1", Some(1), Some("alice"), 80, 24, false)
            .expect("open");
        let failed = wait_for(&engine, id, |event| event["state"] == "failed");
        assert!(
            failed["error"]
                .as_str()
                .is_some_and(|text| !text.is_empty()),
            "{failed}"
        );
        // An empty address is refused by the session layer before connecting.
        assert!(engine.open(" ", None, None, 80, 24, false).is_err());
    })
    .await
    .expect("the engine's steps");
}
