//! The deployment examples (plan run 7, section 3.6; interpretation 7):
//! the server image built from `server/Dockerfile` runs behind Caddy with
//! `tls internal` (access over a VPN) and two real clients sync through it,
//! trusting Caddy's root CA for that origin; the Let's Encrypt Caddyfile and
//! the cloudflared configuration get a syntax check; the backup script
//! deletes files older than its retention.
//!
//! Runs with `SHELLPP_TEST_DEPLOY=docker` (Docker running, network for the
//! images); otherwise it returns at once. Every container and network it
//! starts carries the label `shellpp-test=deploy` and is removed at the end.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;

use data_encoding::BASE64;
use shellpp_core::service::{SignInOutcome, TrustChoice};
use support::*;

fn enabled() -> bool {
    std::env::var("SHELLPP_TEST_DEPLOY").as_deref() == Ok("docker")
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("repo")
        .to_path_buf()
}

fn deploy_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("deploy")
}

fn docker(args: &[&str]) -> (bool, String) {
    let output = Command::new("docker")
        .args(args)
        .output()
        .expect("docker runs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    (output.status.success(), text)
}

/// Removes what the test started, whatever happened.
struct Cleanup(Vec<String>, Option<String>);

impl Drop for Cleanup {
    fn drop(&mut self) {
        for id in &self.0 {
            let _ = docker(&["rm", "-f", id]);
        }
        if let Some(network) = &self.1 {
            let _ = docker(&["network", "rm", network]);
        }
    }
}

fn mount(dir: &Path, target: &str) -> String {
    format!("{}:{target}:ro", dir.display())
}

#[test]
fn example_configurations_validate() {
    if !enabled() {
        return;
    }
    let dir = deploy_dir();
    for file in ["Caddyfile.example", "Caddyfile.internal.example"] {
        let (ok, text) = docker(&[
            "run",
            "--rm",
            "--label",
            "shellpp-test=deploy",
            "-v",
            &mount(&dir, "/cfg"),
            "caddy:2.10-alpine",
            "caddy",
            "validate",
            "--adapter",
            "caddyfile",
            "--config",
            &format!("/cfg/{file}"),
        ]);
        assert!(ok, "{file} is valid: {text}");
    }
    let (ok, text) = docker(&[
        "run",
        "--rm",
        "--label",
        "shellpp-test=deploy",
        "-v",
        &mount(&dir, "/cfg"),
        "cloudflare/cloudflared:latest",
        "tunnel",
        "--config",
        "/cfg/cloudflared.example.yml",
        "ingress",
        "validate",
    ]);
    assert!(ok, "cloudflared ingress is valid: {text}");
}

#[test]
fn the_backup_script_deletes_files_past_retention() {
    if !enabled() {
        return;
    }
    let dir = tempfile::tempdir().expect("dir");
    let script = deploy_dir().join("backup.sh");
    // An old and a new backup file; prune only.
    let (ok, text) = docker(&[
        "run",
        "--rm",
        "--label",
        "shellpp-test=deploy",
        "-v",
        &format!("{}:/backup.sh:ro", script.display()),
        "-v",
        &format!("{}:/backups", dir.path().display()),
        "-e",
        "AGE_RECIPIENT=age1unused",
        "-e",
        "BACKUP_PRUNE_ONLY=1",
        "-e",
        "BACKUP_RETENTION_DAYS=30",
        "--entrypoint",
        "/bin/sh",
        "alpine:3.22",
        "-c",
        "touch -d '2020-01-01 00:00:00' /backups/shellpp-old.sql.age && touch /backups/shellpp-new.sql.age && /bin/sh /backup.sh",
    ]);
    assert!(ok, "the prune ran: {text}");
    assert!(
        !dir.path().join("shellpp-old.sql.age").exists(),
        "the old file went"
    );
    assert!(
        dir.path().join("shellpp-new.sql.age").exists(),
        "the new file stays"
    );
}

#[test]
fn two_clients_sync_through_caddy_with_its_internal_ca() {
    if !enabled() {
        return;
    }
    let root = repo_root();
    let (ok, text) = docker(&[
        "build",
        "-f",
        &root.join("server").join("Dockerfile").display().to_string(),
        "-t",
        "shellpp-server:test",
        &root.display().to_string(),
    ]);
    assert!(
        ok,
        "the image builds: {}",
        &text[text.len().saturating_sub(2000)..]
    );

    let network = format!("shellpp-test-{}", random_hex(4));
    let mut cleanup = Cleanup(Vec::new(), None);
    let (ok, text) = docker(&[
        "network",
        "create",
        "--label",
        "shellpp-test=deploy",
        &network,
    ]);
    assert!(ok, "network: {text}");
    cleanup.1 = Some(network.clone());

    let mut secret = vec![0u8; 48];
    getrandom::fill(&mut secret).expect("random");
    let env = [
        "-e".to_owned(),
        "SHELLPP_DATABASE_URL=sqlite:/data/server.db".to_owned(),
        "-e".to_owned(),
        "SHELLPP_PUBLIC_URL=https://localhost".to_owned(),
        "-e".to_owned(),
        format!("SHELLPP_SERVER_SECRET={}", BASE64.encode(&secret)),
        "-e".to_owned(),
        "SHELLPP_REGISTRATION=open".to_owned(),
    ];
    let volume = format!("shellpp-test-data-{}", random_hex(4));
    let mut migrate: Vec<String> = vec![
        "run".into(),
        "--rm".into(),
        "--label".into(),
        "shellpp-test=deploy".into(),
        "-v".into(),
        format!("{volume}:/data"),
    ];
    migrate.extend(env.iter().cloned());
    migrate.extend(["shellpp-server:test".into(), "migrate".into()]);
    let (ok, text) = docker(&migrate.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(ok, "migrate: {text}");

    let mut serve: Vec<String> = vec![
        "run".into(),
        "-d".into(),
        "--label".into(),
        "shellpp-test=deploy".into(),
        "--network".into(),
        network.clone(),
        "--network-alias".into(),
        "shellpp".into(),
        "-v".into(),
        format!("{volume}:/data"),
    ];
    serve.extend(env.iter().cloned());
    serve.push("shellpp-server:test".into());
    let (ok, server_id) = docker(&serve.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(ok, "serve: {server_id}");
    cleanup.0.push(server_id.trim().to_owned());

    let config = tempfile::tempdir().expect("dir");
    std::fs::write(
        config.path().join("Caddyfile"),
        "localhost:8443 {\n\ttls internal\n\treverse_proxy shellpp:8080\n}\n",
    )
    .expect("caddyfile");
    let (ok, caddy_id) = docker(&[
        "run",
        "-d",
        "--label",
        "shellpp-test=deploy",
        "--network",
        &network,
        "-p",
        "127.0.0.1::8443",
        "-v",
        &mount(config.path(), "/etc/caddy"),
        "caddy:2.10-alpine",
    ]);
    assert!(ok, "caddy: {caddy_id}");
    let caddy_id = caddy_id.trim().to_owned();
    cleanup.0.push(caddy_id.clone());
    let (_, port) = docker(&["port", &caddy_id, "8443/tcp"]);
    let port: u16 = port
        .lines()
        .next()
        .and_then(|line| line.rsplit(':').next())
        .and_then(|port| port.trim().parse().ok())
        .expect("a mapped port");

    // Caddy's root CA, once it made it.
    let mut pem = String::new();
    for _ in 0..60 {
        let (ok, text) = docker(&[
            "exec",
            &caddy_id,
            "cat",
            "/data/caddy/pki/authorities/local/root.crt",
        ]);
        if ok && text.contains("BEGIN CERTIFICATE") {
            pem = text;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let body: String = pem
        .lines()
        .filter(|line| !line.starts_with("-----"))
        .collect();
    let ca = BASE64.decode(body.trim().as_bytes()).expect("root CA");

    let origin = format!("https://localhost:{port}");
    let password = new_password();
    let email = email();
    let a = Client::local("A", &password);
    a.host("behind-caddy.example.com");
    // Without trust the system refuses Caddy's internal CA.
    let refused = a.core.sync_server_check(&origin).expect("check");
    assert!(
        matches!(
            refused,
            shellpp_core::service::ServerCheck::Certificate { .. }
        ),
        "the internal CA is not a system root"
    );
    a.core
        .sync_server_trust(&origin, TrustChoice::Ca(ca.clone()))
        .expect("trust");
    let mut ready = false;
    for _ in 0..60 {
        if matches!(
            a.core.sync_server_check(&origin),
            Ok(shellpp_core::service::ServerCheck::Ready { .. })
        ) {
            ready = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    assert!(ready, "the server answers through Caddy");
    a.core
        .sync_register(shellpp_core::service::SignInInput {
            origin: origin.clone(),
            email: email.clone(),
            password: password.clone(),
            registration_code: None,
        })
        .expect("registered through Caddy");
    let b = Client::empty("B");
    b.core
        .sync_server_trust(&origin, TrustChoice::Ca(ca))
        .expect("trust");
    let outcome = b
        .core
        .sync_sign_in(shellpp_core::service::SignInInput {
            origin,
            email,
            password,
            registration_code: None,
        })
        .expect("signed in through Caddy");
    assert!(matches!(outcome, SignInOutcome::Done(_)));
    assert_eq!(b.addresses(), vec!["behind-caddy.example.com"]);
    drop(cleanup);
    let _ = docker(&["volume", "rm", &volume]);
}
