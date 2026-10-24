//! The server binary's configuration (docs/architecture.md, section 10.2;
//! D177): the server secret from a file, as Docker secrets give it, the
//! warning in the log for one in the environment, and both refused. The
//! binary runs against a database it refuses (not migrated), so it stops
//! right after reading its configuration. Secrets are generated at run
//! time and never appear in the output.

use std::path::Path;
use std::process::{Command, Output};

fn secret() -> String {
    let mut bytes = [0u8; 48];
    getrandom::fill(&mut bytes).expect("random");
    data_encoding::BASE64.encode(&bytes)
}

/// `shellpp-server serve` with `vars` and nothing else of `SHELLPP_*`.
fn serve(dir: &Path, vars: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_shellpp-server"));
    for (name, _) in std::env::vars() {
        if name.starts_with("SHELLPP_") {
            command.env_remove(name);
        }
    }
    command
        .arg("serve")
        .env(
            "SHELLPP_DATABASE_URL",
            format!("sqlite:{}", dir.join("server.db").display()),
        )
        .env("SHELLPP_PUBLIC_URL", "https://sync.example.com")
        .env("SHELLPP_BIND", "127.0.0.1:0")
        .env("SHELLPP_LOG", "info");
    for (name, value) in vars {
        command.env(name, value);
    }
    command.output().expect("the server binary runs")
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
