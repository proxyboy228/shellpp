//! The `shellpp-test-sshd` binary: `serve` (ready line, stdin commands,
//! exit on quit and on stdin close) and `keygen` (files and JSON line), as
//! the e2e harness uses them. Every spawned process is waited for or killed.

mod common;

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use common::{Terminal, connect, password_login, random_secret};
use russh::keys::{HashAlg, PublicKey, decode_secret_key};
use serde_json::{Value, json};
use shellpp_test_sshd::shell::prompt;

const BINARY: &str = env!("CARGO_BIN_EXE_shellpp-test-sshd");

/// A running `serve` process; killed on drop if still running.
struct Served {
    child: Child,
    stdin: Option<ChildStdin>,
    lines: Receiver<String>,
}

impl Served {
    fn start(directory: &Path, config: &Value) -> (Self, Value) {
        let path = directory.join("config.json");
        std::fs::write(&path, config.to_string()).expect("write config");
        let mut child = Command::new(BINARY)
            .arg("serve")
            .arg("--config")
            .arg(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn serve");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().expect("stdout");
        let (sender, lines) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut served = Self {
            child,
            stdin,
            lines,
        };
        let ready = served.next_line();
        (served, ready)
    }

    fn next_line(&mut self) -> Value {
        let line = tokio::task::block_in_place(|| self.lines.recv_timeout(common::WAIT))
            .expect("a JSON line in time");
        serde_json::from_str(&line).expect("one JSON object per line")
    }

    fn command(&mut self, command: &str) -> Value {
        let stdin = self.stdin.as_mut().expect("stdin open");
        writeln!(stdin, "{command}").expect("write command");
        stdin.flush().expect("flush");
        self.next_line()
    }

    fn close_stdin(&mut self) {
        self.stdin = None;
    }

    /// Waits for the exit; `None` on timeout.
    fn wait_exit(&mut self, timeout: Duration) -> Option<std::process::ExitStatus> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                return Some(status);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn no_more_lines(&self) -> bool {
        matches!(
            self.lines.recv_timeout(Duration::from_secs(2)),
            Err(RecvTimeoutError::Disconnected)
        )
    }
}

impl Drop for Served {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(None)) {
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

fn port_of(ready: &Value) -> u16 {
    ready["port"]
        .as_u64()
        .and_then(|port| u16::try_from(port).ok())
        .expect("port")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_prints_ready_and_answers_stdin_commands() {
    let directory = tempfile::tempdir().expect("temp dir");
    let password = random_secret();
    let config = json!({
        "users": [{"name": "alice", "password": password}],
        "banner_comment": "Debian-5",
        "exec": {"stdout": "Linux\nID=debian\n"},
        "motd": "cli motd\r\n"
    });
    let (mut served, ready) = Served::start(directory.path(), &config);
    assert_eq!(ready["event"], "ready");
    assert_eq!(ready["host_key_type"], "ssh-ed25519");
    let port = port_of(&ready);
    let fingerprint = ready["host_key_fingerprint"]
        .as_str()
        .expect("fingerprint")
        .to_owned();
    let base64 = ready["host_key_base64"]
        .as_str()
        .expect("base64")
        .to_owned();
    let line = format!("ssh-ed25519 {base64}");
    let ready_key = PublicKey::from_openssh(&line).expect("ready line key parses");
    assert_eq!(
        ready_key.fingerprint(HashAlg::Sha256).to_string(),
        fingerprint
    );

    // The client sees the same key, and the shell works.
    let (_probe, observed) = connect(port).await;
    assert_eq!(
        observed
            .server_key()
            .fingerprint(HashAlg::Sha256)
            .to_string(),
        fingerprint
    );
    let handle = password_login(port, "alice", &password).await;
    let mut terminal = Terminal::open(&handle).await;
    assert_eq!(
        terminal.wait_for(&prompt("alice")).await,
        "cli motd\r\nalice@shellpp-test:~$ "
    );

    let events = served.command("events");
    assert_eq!(events["event"], "events");
    let items = events["items"].as_array().expect("items");
    assert!(
        items.contains(&json!({"kind": "auth", "user": "alice", "method": "password", "ok": true}))
    );
    assert!(items.contains(&json!({"kind": "shell"})));
    assert!(!events.to_string().contains(&password));

    assert_eq!(
        served.command("drop"),
        json!({"event": "dropped", "count": 2})
    );
    assert_eq!(terminal.wait_end().await, common::Ending::WithoutStatus);
    assert_eq!(
        served.command("freeze"),
        json!({"event": "frozen", "count": 0})
    );
    assert_eq!(
        served.command("hold on"),
        json!({"event": "holding", "value": true})
    );
    assert_eq!(
        served.command("hold off"),
        json!({"event": "holding", "value": false})
    );

    assert_eq!(
        served.command("accept off"),
        json!({"event": "accepting", "value": false})
    );
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err()
    );
    assert_eq!(
        served.command("accept on"),
        json!({"event": "accepting", "value": true})
    );
    let _again = password_login(port, "alice", &password).await;

    let rotated = served.command("rotate-host-key");
    assert_eq!(rotated["event"], "rotated");
    assert_eq!(rotated["host_key_type"], "ssh-ed25519");
    assert_ne!(rotated["host_key_fingerprint"], json!(fingerprint));
    assert_ne!(rotated["host_key_base64"], json!(base64));
    assert!(rotated.get("port").is_none());
    let (_probe, observed) = connect(port).await;
    assert_eq!(
        json!(
            observed
                .server_key()
                .fingerprint(HashAlg::Sha256)
                .to_string()
        ),
        rotated["host_key_fingerprint"]
    );

    let unknown = served.command("reboot");
    assert_eq!(unknown["event"], "error");
    assert_eq!(served.command("quit"), json!({"event": "bye"}));
    let status = served.wait_exit(common::WAIT).expect("exits after quit");
    assert!(status.success());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn serve_exits_when_stdin_closes() {
    let directory = tempfile::tempdir().expect("temp dir");
    let config = json!({"users": [{"name": "bob", "password": random_secret()}]});
    let (mut served, ready) = Served::start(directory.path(), &config);
    let port = port_of(&ready);
    served.close_stdin();
    let status = served
        .wait_exit(Duration::from_secs(10))
        .expect("exits when stdin closes");
    assert!(status.success());
    assert!(served.no_more_lines());
    assert!(
        tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_err()
    );
}

#[test]
fn serve_rejects_bad_configs_with_a_usage_or_runtime_status() {
    let directory = tempfile::tempdir().expect("temp dir");
    let secret_value = random_secret();
    let path = directory.path().join("bad.json");
    std::fs::write(
        &path,
        format!(r#"{{"users":[{{"name":"x","password":"{secret_value}","echo":1}}],"bind":"192.0.2.1:22"}}"#),
    )
    .expect("write");
    let output = Command::new(BINARY)
        .args(["serve", "--config"])
        .arg(&path)
        .stdin(Stdio::null())
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("loopback"), "stderr: {stderr}");
    assert!(!stderr.contains(&secret_value));

    let output = Command::new(BINARY)
        .args(["serve"])
        .stdin(Stdio::null())
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(2));
    let output = Command::new(BINARY).output().expect("run");
    assert_eq!(output.status.code(), Some(2));
    let output = Command::new(BINARY)
        .args(["keygen", "--type", "dsa", "--out", "x"])
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(2));
}

#[test]
fn keygen_writes_the_key_files_and_prints_json() {
    let directory = tempfile::tempdir().expect("temp dir");
    let passphrase = random_secret();
    let variable = "SHELLPP_TEST_SSHD_KEYGEN_PASSPHRASE";
    for (name, algorithm) in [
        ("ed25519", "ssh-ed25519"),
        ("ecdsa-p256", "ecdsa-sha2-nistp256"),
        ("rsa-3072", "ssh-rsa"),
    ] {
        let out = directory.path().join(format!("id_{name}"));
        let output = Command::new(BINARY)
            .args(["keygen", "--type", name, "--out"])
            .arg(&out)
            .args(["--passphrase-env", variable, "--comment", "cli-key"])
            .env(variable, &passphrase)
            .output()
            .expect("run keygen");
        assert!(output.status.success(), "keygen {name}");
        let printed: Value = serde_json::from_slice(&output.stdout).expect("one JSON line");
        let public_line = printed["public_key"].as_str().expect("public_key");
        let fingerprint = printed["fingerprint"].as_str().expect("fingerprint");
        assert!(public_line.starts_with(&format!("{algorithm} ")));
        assert!(public_line.ends_with(" cli-key"));

        let public_file = std::fs::read_to_string(directory.path().join(format!("id_{name}.pub")))
            .expect("public key file");
        assert_eq!(public_file, format!("{public_line}\n"));
        let private_file = std::fs::read_to_string(&out).expect("private key file");
        assert!(decode_secret_key(&private_file, None).is_err(), "encrypted");
        let decoded = decode_secret_key(&private_file, Some(&passphrase)).expect("decrypts");
        assert_eq!(
            decoded.fingerprint(HashAlg::Sha256).to_string(),
            fingerprint
        );
        assert!(!String::from_utf8_lossy(&output.stdout).contains(&passphrase));
    }

    // Without the variable (or with it empty) the key is not encrypted.
    let out = directory.path().join("plain");
    let output = Command::new(BINARY)
        .args(["keygen", "--type", "ed25519", "--out"])
        .arg(&out)
        .args(["--passphrase-env", variable])
        .env(variable, "")
        .output()
        .expect("run keygen");
    assert!(output.status.success());
    let printed: Value = serde_json::from_slice(&output.stdout).expect("json");
    assert!(
        printed["public_key"]
            .as_str()
            .is_some_and(|line| line.ends_with(" shellpp-test-key"))
    );
    let private_file = std::fs::read_to_string(&out).expect("private key file");
    let decoded = decode_secret_key(&private_file, None).expect("plain key");
    assert_eq!(
        json!(decoded.fingerprint(HashAlg::Sha256).to_string()),
        printed["fingerprint"]
    );
}
