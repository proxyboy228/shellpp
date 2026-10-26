//! The engine end to end on the computer: a quick connect through the
//! shared session layer to the russh test server in process, the first host
//! key's question, the password prompt, a command and its output on the
//! terminal's screen, a resize and the close; and a saved host in a group,
//! connected with the group's username and the password saved with it.
//! Passwords are made at run time.

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use secrecy::SecretString;
use serde_json::{Value, json};
use shellpp_android::screen;
use shellpp_android::{Engine, Services};
use shellpp_core::platform::memory::{MemoryClipboard, MemorySecretStore, RecordingLauncher};
use shellpp_core::service::dto::SessionTarget;
use shellpp_test_sshd::{ServerConfig, TestServer, UserConfig};

const WAIT: Duration = Duration::from_secs(30);

fn random(prefix: &str) -> String {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).expect("random");
    let suffix: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    format!("{prefix}-{suffix}")
}

/// An engine on a new vault in `files`, unlocked.
fn started(files: &Path) -> Engine {
    let engine = Engine::start(
        files,
        Services {
            secret_store: Arc::new(MemorySecretStore::new()),
            clipboard: Arc::new(MemoryClipboard::new()),
            launcher: Arc::new(RecordingLauncher::new()),
            device_name: "Test phone".to_owned(),
        },
    )
    .expect("engine");
    let answer = engine.call(
        "first_run_setup",
        json!({ "password": random("vault-pw") })
            .to_string()
            .as_bytes(),
    );
    assert!(answer.contains("\"ok\""), "{answer}");
    engine
}

/// The `ok` value of a command.
fn ok(engine: &Engine, command: &str, input: &Value) -> Value {
    let answer: Value =
        serde_json::from_str(&engine.call(command, input.to_string().as_bytes())).expect("json");
    assert!(answer.get("error").is_none(), "{command}: {answer}");
    answer["ok"].clone()
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
        let engine = started(files.path());
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
        let engine = started(files.path());
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_saved_host_connects_with_its_groups_username_and_its_saved_password() {
    let password = random("pw");
    let server = TestServer::start(ServerConfig::new(vec![
        UserConfig::new("deploy").with_password(SecretString::from(password.clone())),
    ]))
    .await
    .expect("test server");
    let port = server.port();
    let files = tempfile::tempdir().expect("files");
    let marker = random("marker");

    let result = tokio::task::spawn_blocking(move || {
        let engine = started(files.path());
        let vault_id = ok(&engine, "vaults", &json!(null))[0]["vault_id"].clone();
        let group = ok(
            &engine,
            "record_save",
            &json!({
                "vault_id": vault_id,
                "type": "group",
                "data": { "name": "Servers", "ssh": { "username": "deploy", "port": port } },
            }),
        );
        let host = ok(
            &engine,
            "record_save",
            &json!({
                "vault_id": vault_id,
                "type": "host",
                "data": { "address": "127.0.0.1", "label": "Test", "group_id": group["record_id"] },
                "secrets": { "ssh.password": password },
            }),
        );
        let target = SessionTarget::Host {
            vault_id: serde_json::from_value(vault_id).expect("vault id"),
            record_id: serde_json::from_value(host["record_id"].clone()).expect("record id"),
        };
        let id = engine.open_target(target, 80, 24, true).expect("open");
        let asked = wait_for(&engine, id, |event| {
            event["prompt"]["kind"] == "host_key_unknown"
        });
        let prompt = asked["prompt"]["id"].as_u64().expect("prompt id");
        engine.reply(id, prompt, "accept", None).expect("accept");
        // No password prompt: the saved one signs in.
        let connected = wait_for(&engine, id, |event| {
            event["state"] == "connected" || event["prompt"]["kind"] == "password"
        });
        assert_eq!(connected["state"], "connected", "{connected}");
        engine
            .send(id, format!("echo {marker}\r").into_bytes())
            .expect("send");
        assert!(screen_has(&engine, id, &marker));
        engine.close(id);
        // The host key was saved in the vault's known hosts.
        let known = engine.core().known_hosts_list().expect("known hosts");
        assert_eq!(known.len(), 1);
    })
    .await;
    drop(server);
    result.expect("the engine's steps");
}

/// The password key (D220, D223 item 13): a host whose password comes from
/// its group's identity; the key's state from the session layer and the
/// vault; Send types the password once without Enter; a session recording
/// its output, a host without a password, a password with a line break and
/// another host are refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn the_password_key_types_the_identitys_password_and_refuses_what_it_must() {
    use shellpp_android::PasswordAnswer;
    use shellpp_core::service::dto::RecordRef;

    let password = random("pw");
    let server = TestServer::start(ServerConfig::new(vec![
        UserConfig::new("deploy").with_password(SecretString::from(password.clone())),
    ]))
    .await
    .expect("test server");
    let port = server.port();
    let files = tempfile::tempdir().expect("files");

    let result = tokio::task::spawn_blocking(move || {
        let engine = started(files.path());
        let vault = ok(&engine, "vaults", &json!(null))[0]["vault_id"].clone();
        let identity = ok(
            &engine,
            "identity_save",
            &json!({"label": "deploy", "username": "deploy", "password": password}),
        );
        let group = ok(
            &engine,
            "record_save",
            &json!({"vault_id": vault, "type": "group", "data": {
                "name": "Servers", "ssh": {"port": port, "identity_id": identity["record_id"]}}}),
        );
        let save_host = |label: &str, extra: Value| {
            let mut data =
                json!({"address": "127.0.0.1", "label": label, "group_id": group["record_id"]});
            for (key, value) in extra.as_object().expect("object") {
                data[key] = value.clone();
            }
            let host = ok(
                &engine,
                "record_save",
                &json!({"vault_id": vault, "type": "host", "data": data}),
            );
            RecordRef {
                vault_id: serde_json::from_value(vault.clone()).expect("vault id"),
                record_id: serde_json::from_value(host["record_id"].clone()).expect("record id"),
            }
        };
        let connect = |host: RecordRef, accept: bool| {
            let id = engine
                .open_target(
                    SessionTarget::Host {
                        vault_id: host.vault_id,
                        record_id: host.record_id,
                    },
                    80,
                    24,
                    true,
                )
                .expect("open");
            if accept {
                let asked = wait_for(&engine, id, |event| {
                    event["prompt"]["kind"] == "host_key_unknown"
                });
                let prompt = asked["prompt"]["id"].as_u64().expect("prompt id");
                engine.reply(id, prompt, "accept", None).expect("accept");
            }
            let connected = wait_for(&engine, id, |event| {
                event["state"] == "connected" || event["prompt"]["kind"] == "password"
            });
            assert_eq!(connected["state"], "connected", "{connected}");
            // Until the session has decided about its recording, it counts
            // as recorded: the key fails closed.
            assert_eq!(connected["recording"], true, "{connected}");
            id
        };
        let decided = |id: u64, recorded: bool| {
            wait_for(&engine, id, |event| {
                event["state"] == "connected" && event["recording"] == recorded
            });
        };

        // The group's identity signs in and gives the key its password.
        let web = save_host("web", json!({}));
        let id = connect(web, true);
        decided(id, false);
        assert_eq!(
            engine.password_state(id, web).expect("state"),
            PasswordAnswer::Ready
        );
        assert!(!engine.records_output(id).expect("recording"));
        assert_eq!(
            engine.send_password(id, web).expect("sent"),
            PasswordAnswer::Ready
        );
        // Typed once, without Enter: the test shell echoes it on its line.
        assert!(screen_has(&engine, id, &password));
        let terminal = engine.terminal(id).expect("terminal");
        let line = terminal
            .screen_text()
            .into_iter()
            .find(|line| line.contains(&password))
            .expect("the echoed line");
        assert_eq!(line.matches(&password).count(), 1, "{line}");
        engine.send(id, b"\x15".to_vec()).expect("clear the line");

        // Another host than the session's is refused.
        let other = save_host("other", json!({}));
        assert_eq!(
            engine.password_state(id, other).expect("state"),
            PasswordAnswer::NotConnected
        );

        // A line break in the password would be an Enter: refused.
        ok(
            &engine,
            "identity_save",
            &json!({"record_id": identity["record_id"], "label": "deploy", "username": "deploy",
                    "password": "first\nsecond"}),
        );
        assert_eq!(
            engine.send_password(id, web).expect("refused"),
            PasswordAnswer::Control
        );
        // No password left (none of its own, its identity's cleared): none.
        ok(
            &engine,
            "identity_save",
            &json!({"record_id": identity["record_id"], "label": "deploy", "username": "deploy",
                    "clear_password": true}),
        );
        assert_eq!(
            engine.password_state(id, web).expect("state"),
            PasswordAnswer::None
        );
        assert_eq!(
            engine.send_password(id, web).expect("refused"),
            PasswordAnswer::None
        );
        engine.close(id);

        // A host that records its output: the session says so, the key refuses.
        ok(
            &engine,
            "identity_save",
            &json!({"record_id": identity["record_id"], "label": "deploy", "username": "deploy",
                    "password": password}),
        );
        let recorded = save_host("recorded", json!({"record_output": "on"}));
        let id = connect(recorded, false);
        assert!(engine.records_output(id).expect("recording"));
        assert_eq!(
            engine.password_state(id, recorded).expect("state"),
            PasswordAnswer::Recorded
        );
        assert_eq!(
            engine.send_password(id, recorded).expect("refused"),
            PasswordAnswer::Recorded
        );
        engine.close(id);
    })
    .await;
    drop(server);
    result.expect("the engine's steps");
}
