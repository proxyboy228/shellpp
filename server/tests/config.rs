//! The server binary's configuration (docs/architecture.md, section 10.2;
//! D177): the server secret from a file, as Docker secrets give it, the
//! warning in the log for one in the environment, both refused, and a secret
//! that opens none of the stored TOTP secrets refused at start. The
//! first test runs it against a database it refuses (not migrated), so it stops
//! right after reading its configuration. Secrets are generated at run
//! time and never appear in the output.

mod support;

use std::io::BufRead as _;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use serde_json::json;
use support::*;

fn secret() -> String {
    let mut bytes = [0u8; 48];
    getrandom::fill(&mut bytes).expect("random");
    data_encoding::BASE64.encode(&bytes)
}

/// `shellpp-server serve` with `vars` and nothing else of `SHELLPP_*`.
fn serve(dir: &Path, vars: &[(&str, &str)]) -> Output {
    let url = format!("sqlite:{}", dir.join("server.db").display());
    command(&url, vars)
        .output()
        .expect("the server binary runs")
}

fn command(url: &str, vars: &[(&str, &str)]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_shellpp-server"));
    for (name, _) in std::env::vars() {
        if name.starts_with("SHELLPP_") {
            command.env_remove(name);
        }
    }
    command
        .arg("serve")
        .env("SHELLPP_DATABASE_URL", url)
        .env("SHELLPP_PUBLIC_URL", "https://sync.example.com")
        .env("SHELLPP_BIND", "127.0.0.1:0")
        .env("SHELLPP_LOG", "info");
    for (name, value) in vars {
        command.env(name, value);
    }
    command
}

fn log(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn the_server_secret_is_read_from_a_file_and_the_environment_warns() {
    let dir = tempfile::tempdir().expect("dir");
    let secret = secret();
    let file = dir.path().join("server_secret");
    std::fs::write(&file, format!("{secret}\n")).expect("secret file");
    let file = file.display().to_string();

    // From the file: past the configuration (the schema stops it), no
    // warning about the secret.
    let output = serve(dir.path(), &[("SHELLPP_SERVER_SECRET_FILE", &file)]);
    let text = log(&output);
    assert!(!output.status.success());
    assert!(text.contains("migrate"), "stopped at the schema: {text}");
    assert!(!text.contains("read from the environment"), "{text}");

    // From the environment: the same, with the warning in the log.
    let output = serve(dir.path(), &[("SHELLPP_SERVER_SECRET", &secret)]);
    let text = log(&output);
    assert!(text.contains("migrate"), "stopped at the schema: {text}");
    assert!(
        text.contains("SHELLPP_SERVER_SECRET is read from the environment")
            && text.contains("SHELLPP_SERVER_SECRET_FILE")
            && text.contains("WARN"),
        "the warning: {text}"
    );

    // Both: refused before anything else.
    let output = serve(
        dir.path(),
        &[
            ("SHELLPP_SERVER_SECRET", &secret),
            ("SHELLPP_SERVER_SECRET_FILE", &file),
        ],
    );
    let text = log(&output);
    assert!(!output.status.success());
    assert!(text.contains("both set"), "{text}");
    assert!(!text.contains("migrate"), "{text}");

    // The secret itself never shows.
    for output in [
        serve(dir.path(), &[("SHELLPP_SERVER_SECRET", &secret)]),
        serve(dir.path(), &[("SHELLPP_SERVER_SECRET_FILE", &file)]),
    ] {
        assert!(!log(&output).contains(&secret[..12]));
    }
}

fn b64(bytes: &[u8]) -> String {
    data_encoding::BASE64URL_NOPAD.encode(bytes)
}

/// Review finding (D177): a server secret that opens none of the TOTP
/// secrets stored in the database stops `serve` with a message that says
/// what to do; the secret the database was made with starts it. Without the
/// check, a regenerated secret answered every TOTP sign-in with an error.
#[test]
fn serve_refuses_a_secret_that_opens_none_of_the_totp_secrets() {
    let mut server = Server::start();
    let auth_key = shellpp_server::secrets::random_bytes::<32>();
    let register = json!({
        "email": email(),
        "kdf": {"alg": "argon2id", "v": 1, "m_kib": 65536, "t": 3, "p": 4},
        "kdf_salt": b64(&[1; 16]),
        "auth_key": b64(&auth_key),
        "recovery_auth_key": b64(&shellpp_server::secrets::random_bytes::<32>()),
        "personal_vault": {
            "vault_id": uuid::Uuid::now_v7(),
            "epoch": 1,
            "kek_wrap": b64(&[4; 73]),
            "recovery_wrap": b64(&[5; 73]),
            "epoch_chain": [],
        },
        "x25519_public_key": b64(&[6; 32]),
        "ssh_id_public_key": "ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIK0 shellpp:x",
        "device_id": uuid::Uuid::now_v7(),
    })
    .to_string();
    let json_type = ("content-type", "application/json");
    let answer = raw_http(
        &server,
        "POST",
        "/v1/auth/register",
        &[json_type],
        register.as_bytes(),
    );
    assert_eq!(answer.status, 201, "{}", answer.body);
    let bearer = format!(
        "Bearer {}",
        answer.body["tokens"]["access_token"].as_str().unwrap()
    );
    let setup = json!({"current_auth_key": b64(&auth_key)}).to_string();
    let answer = raw_http(
        &server,
        "POST",
        "/v1/account/totp/setup",
        &[json_type, ("authorization", &bearer)],
        setup.as_bytes(),
    );
    assert_eq!(answer.status, 200, "{}", answer.body);
    server.stop();
    let url = server.database_url.clone();
    let dir = tempfile::tempdir().expect("dir");
    let file = dir.path().join("server_secret");

    // Another secret: refused, saying why and what to do.
    std::fs::write(&file, secret()).expect("secret file");
    let path = file.display().to_string();
    let output = command(&url, &[("SHELLPP_SERVER_SECRET_FILE", &path)])
        .output()
        .expect("runs");
    let text = log(&output);
    assert!(!output.status.success());
    assert!(text.contains("opens none of the TOTP secrets"), "{text}");
    assert!(text.contains("admin user reset-totp"), "{text}");

    // The secret the database was made with: it starts listening.
    std::fs::write(
        &file,
        data_encoding::BASE64.encode(&server.config.server_secret),
    )
    .expect("secret file");
    let mut child = command(&url, &[("SHELLPP_SERVER_SECRET_FILE", &path)])
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .expect("starts");
    let stderr = child.stderr.take().expect("stderr");
    let (lines, seen) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for line in std::io::BufReader::new(stderr)
            .lines()
            .map_while(Result::ok)
        {
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let mut listening = false;
    while let Ok(line) = seen.recv_timeout(Duration::from_secs(60)) {
        if line.contains("listening") {
            listening = true;
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    assert!(listening, "the right secret starts the server");
}
