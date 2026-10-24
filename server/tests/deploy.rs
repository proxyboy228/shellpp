//! The deployment examples (plan run 7, section 3.6; interpretation 7):
//! the server image built from `server/Dockerfile` runs behind Caddy with
//! `tls internal` (access over a VPN) and two real clients sync through it,
//! trusting Caddy's root CA for that origin; the Let's Encrypt Caddyfile and
//! the cloudflared configuration get a syntax check; the backup script
//! deletes files older than its retention. Run 9: the encrypted backup
//! recipe of the README end to end (docs/sync-protocol.md, section 12),
//! a failing dump that leaves no backup, and the compose file's trusted
//! proxy, server secret and backup image (D176, D177).
//!
//! Runs with `SHELLPP_TEST_DEPLOY=docker` (Docker running, network for the
//! images); otherwise it returns at once. Every container and network it
//! starts carries the labels `shellpp-test=deploy` and `shellpp-run=<run>`
//! (`support::run_label`) and is removed at the end.

mod support;

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

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
            // With their anonymous volumes (PostgreSQL's, Caddy's).
            let _ = docker(&["rm", "-f", "-v", id]);
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
            "--label",
            run_label(),
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
        "--label",
        run_label(),
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
        "--label",
        run_label(),
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
    server_image();

    let network = format!("shellpp-test-{}", random_hex(4));
    let mut cleanup = Cleanup(Vec::new(), None);
    let (ok, text) = docker(&[
        "network",
        "create",
        "--label",
        "shellpp-test=deploy",
        "--label",
        run_label(),
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
        "--label".into(),
        run_label().into(),
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
        "--label".into(),
        run_label().into(),
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
        "--label",
        run_label(),
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

// ---------------------------------------------------------------------------
// Run 9: images, the backup recipe and the compose file
// ---------------------------------------------------------------------------

/// Builds `tag` from `dockerfile` in `context`; on a shared machine with
/// `CARGO_BUILD_JOBS` set, the server's build uses that many jobs.
fn build(tag: &str, dockerfile: &Path, context: &Path) {
    let jobs = std::env::var("CARGO_BUILD_JOBS").ok();
    let mut args = vec!["build".to_owned()];
    if let Some(jobs) = jobs {
        args.extend(["--build-arg".to_owned(), format!("BUILD_JOBS={jobs}")]);
    }
    args.extend([
        "-f".to_owned(),
        dockerfile.display().to_string(),
        "-t".to_owned(),
        tag.to_owned(),
        context.display().to_string(),
    ]);
    let (ok, text) = docker(&args.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(
        ok,
        "{tag} builds: {}",
        &text[text.len().saturating_sub(2000)..]
    );
}

/// The server image, built once per test process.
fn server_image() -> &'static str {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        let root = repo_root();
        build(
            "shellpp-server:test",
            &root.join("server").join("Dockerfile"),
            &root,
        );
    });
    "shellpp-server:test"
}

/// The backup image of the compose file (D176), built once per process.
/// Runs the backup script's prune in Alpine's busybox (the shell of the
/// backup image) on `dir`, after `setup` (a shell snippet that makes
/// files in /backups), with the extra environment `env`.
fn prune_in(dir: &Path, setup: &str, env: &[&str]) -> String {
    let script = deploy_dir().join("backup.sh");
    let mount_script = format!("{}:/backup.sh:ro", script.display());
    let mount_dir = format!("{}:/backups", dir.display());
    let command = format!(
        "{setup}
/bin/sh /backup.sh"
    );
    let mut args = vec![
        "run",
        "--rm",
        "--label",
        "shellpp-test=deploy",
        "--label",
        run_label(),
        "-v",
        &mount_script,
        "-v",
        &mount_dir,
        "-e",
        "AGE_RECIPIENT=age1unused",
        "-e",
        "BACKUP_PRUNE_ONLY=1",
    ];
    for value in env {
        args.push("-e");
        args.push(value);
    }
    args.extend(["--entrypoint", "/bin/sh", "alpine:3.22", "-c", &command]);
    let (ok, text) = docker(&args);
    assert!(ok, "the prune ran: {text}");
    text
}

/// D199: a run of empty or much smaller dumps after good ones (a database
/// emptied by mistake) deletes nothing, however old the good backups are,
/// until the operator accepts the smaller size.
#[test]
fn the_backup_rotation_keeps_good_backups_when_dumps_shrink() {
    if !enabled() {
        return;
    }
    let dir = tempfile::tempdir().expect("dir");
    // Three good backups long past the retention, then three tiny ones.
    let setup = "set -e; cd /backups
for day in 10 11 12; do
  head -c 10000 /dev/zero > shellpp-202001${day}T000000Z.sql.age
  touch -d \"2020-01-$day 00:00:00\" shellpp-202001${day}T000000Z.sql.age
done
now=$(date -u +%s)
for back in 3 2 1; do
  name=shellpp-small-$back.sql.age
  head -c 100 /dev/zero > $name
  touch -d \"$(date -u -d @$((now - back * 86400)) '+%Y-%m-%d %H:%M:%S')\" $name
done";
    let text = prune_in(dir.path(), setup, &["BACKUP_RETENTION_DAYS=30"]);
    assert!(text.contains("pruning paused"), "{text}");
    assert_eq!(backups_in(dir.path()).len(), 6, "nothing deleted: {text}");

    // The operator checked: the database really shrank.
    let text = prune_in(
        dir.path(),
        "true",
        &["BACKUP_RETENTION_DAYS=30", "BACKUP_ACCEPT_SMALLER=1"],
    );
    assert_eq!(
        backups_in(dir.path()),
        vec![
            "shellpp-small-1.sql.age",
            "shellpp-small-2.sql.age",
            "shellpp-small-3.sql.age"
        ],
        "{text}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join(".accepted-size"))
            .expect("the accepted backup")
            .trim(),
        "shellpp-small-1.sql.age"
    );
}

/// D199: besides the daily backups of the retention, the first backup of
/// each of the last weeks and months stays.
#[test]
fn the_backup_rotation_keeps_weekly_and_monthly_backups() {
    if !enabled() {
        return;
    }
    let dir = tempfile::tempdir().expect("dir");
    // A backup a day for 120 days, the same size; a retention of one day.
    let setup = "set -e; cd /backups; now=$(date -u +%s); day=0
while [ $day -lt 120 ]; do
  name=$(printf 'shellpp-age-%03d.sql.age' $day)
  head -c 1000 /dev/zero > $name
  touch -d \"$(date -u -d @$((now - day * 86400 - 60)) '+%Y-%m-%d %H:%M:%S')\" $name
  day=$((day + 1))
done";
    let text = prune_in(dir.path(), setup, &["BACKUP_RETENTION_DAYS=1"]);
    let ages: Vec<u32> = backups_in(dir.path())
        .iter()
        .map(|name| name["shellpp-age-".len()..][..3].parse().expect("an age"))
        .collect();
    assert!(
        ages.contains(&0) && ages.contains(&1),
        "the retention: {ages:?}"
    );
    assert!(
        ages.iter().any(|age| (14..28).contains(age)),
        "a weekly backup: {ages:?}
{text}"
    );
    assert!(
        ages.iter().any(|age| (31..120).contains(age)),
        "a monthly backup: {ages:?}"
    );
    assert!(!ages.contains(&119), "the oldest went: {ages:?}");
    assert!(ages.len() <= 2 + 4 + 3, "no more than that: {ages:?}");
}

fn backup_image() -> &'static str {
    static BUILT: OnceLock<()> = OnceLock::new();
    BUILT.get_or_init(|| {
        let dir = deploy_dir();
        build("shellpp-backup:test", &dir.join("backup.Dockerfile"), &dir);
    });
    "shellpp-backup:test"
}

/// `docker` with the arguments, asserting success: its standard output.
fn docker_ok(args: &[&str]) -> String {
    let output = Command::new("docker")
        .args(args)
        .output()
        .expect("docker runs");
    assert!(
        output.status.success(),
        "docker {}: {}{}",
        args.first().copied().unwrap_or(""),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// An age key pair made by `age-keygen` of the backup image: the identity
/// file's text and the public key.
fn age_key() -> (String, String) {
    let identity = docker_ok(&[
        "run",
        "--rm",
        "--label",
        "shellpp-test=deploy",
        "--label",
        run_label(),
        "--entrypoint",
        "age-keygen",
        backup_image(),
    ]);
    let public = identity
        .lines()
        .find_map(|line| line.strip_prefix("# public key: "))
        .expect("a public key")
        .trim()
        .to_owned();
    (identity, public)
}

fn backups_in(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("dir")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .filter(|name| name.starts_with("shellpp-"))
        .collect();
    names.sort();
    names
}

/// D176 (run 8's agent C, S7): with `pipefail`, a dump that fails makes
/// the backup fail and leaves no file that looks like a backup (no
/// `.sql.age`, no `.part`), and nothing is pruned, so an old backup stays.
/// Before, age encrypted the empty output of the failed dump and the script
/// reported a backup.
#[test]
fn a_failed_dump_fails_the_backup_and_leaves_no_backup_file() {
    if !enabled() {
        return;
    }
    let (_, public) = age_key();
    let dir = tempfile::tempdir().expect("dir");
    let old = dir.path().join("shellpp-20200101T000000Z.sql.age");
    std::fs::write(&old, b"an old backup").expect("old");
    let file = std::fs::File::options()
        .write(true)
        .open(&old)
        .expect("old");
    file.set_modified(
        std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_577_836_800),
    )
    .expect("mtime");
    drop(file);
    // No database at that address: pg_dump fails at once.
    let (ok, text) = docker(&[
        "run",
        "--rm",
        "--label",
        "shellpp-test=deploy",
        "--label",
        run_label(),
        "-v",
        &format!("{}:/backups", dir.path().display()),
        "-e",
        &format!("AGE_RECIPIENT={public}"),
        "-e",
        "BACKUP_ONCE=1",
        "-e",
        "BACKUP_RETENTION_DAYS=30",
        "-e",
        "PGHOST=127.0.0.1",
        "-e",
        "PGPORT=1",
        "-e",
        "PGUSER=shellpp",
        "-e",
        "PGDATABASE=shellpp",
        "-e",
        "PGCONNECT_TIMEOUT=5",
        backup_image(),
    ]);
    assert!(!ok, "the backup failed: {text}");
    assert!(text.contains("backup failed"), "{text}");
    assert_eq!(
        backups_in(dir.path()),
        vec!["shellpp-20200101T000000Z.sql.age"],
        "no new file, no .part, and the old backup was not pruned"
    );
}

/// The README's backup and restore (docs/sync-protocol.md, section 12: "the
/// encrypted backup recipe and a restore from it"): the backup image dumps
/// the server's PostgreSQL through age; the dump is decrypted and loaded
/// into a clean PostgreSQL; `admin restored`; the server comes back on it
/// at the same address. Two clients keep their earlier data, notice the
/// restore (a new `server_generation`, section 6.9), and the record made
/// after the backup comes back from the device that has it. The server
/// reads its secret from a file, as with Docker secrets (D177).
#[test]
fn the_encrypted_backup_restores_into_a_clean_postgres() {
    if !enabled() {
        return;
    }
    let image = server_image();
    let backup = backup_image();
    let network = format!("shellpp-test-{}", random_hex(4));
    let mut cleanup = Cleanup(Vec::new(), None);
    docker_ok(&[
        "network",
        "create",
        "--label",
        "shellpp-test=deploy",
        "--label",
        run_label(),
        &network,
    ]);
    cleanup.1 = Some(network.clone());
    let password = random_hex(16);
    let postgres = |alias: &str, cleanup: &mut Cleanup| {
        let id = docker_ok(&[
            "run",
            "-d",
            "--label",
            "shellpp-test=deploy",
            "--label",
            run_label(),
            "--network",
            &network,
            "--network-alias",
            alias,
            "-e",
            "POSTGRES_DB=shellpp",
            "-e",
            "POSTGRES_USER=shellpp",
            "-e",
            &format!("POSTGRES_PASSWORD={password}"),
            "postgres:17-alpine",
        ]);
        let id = id.trim().to_owned();
        cleanup.0.push(id.clone());
        let mut ready = false;
        for _ in 0..120 {
            // Over TCP: the entrypoint's first, temporary server listens
            // only on its socket.
            let (ok, _) = docker(&[
                "exec",
                &id,
                "pg_isready",
                "-h",
                "127.0.0.1",
                "-U",
                "shellpp",
                "-d",
                "shellpp",
            ]);
            if ok {
                ready = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        assert!(ready, "{alias} is ready");
    };
    postgres("pg-old", &mut cleanup);
    postgres("pg-new", &mut cleanup);

    let secrets = tempfile::tempdir().expect("dir");
    let mut secret = vec![0u8; 48];
    getrandom::fill(&mut secret).expect("random");
    std::fs::write(
        secrets.path().join("server_secret"),
        format!("{}\n", BASE64.encode(&secret)),
    )
    .expect("secret file");
    // The image's user 10001 reads it (a temporary folder is 0700 on Unix).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let open = |path: &Path, mode| {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).expect("mode")
        };
        open(secrets.path(), 0o755);
        open(&secrets.path().join("server_secret"), 0o644);
    }
    // One address for both servers: the clients' keys are bound to it.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("a free port")
        .port();
    let origin = format!("http://127.0.0.1:{port}");
    let start_server = |database: &str, cleanup: &mut Cleanup| -> String {
        let url = format!("postgres://shellpp:{password}@{database}:5432/shellpp");
        let id = docker_ok(&[
            "run",
            "-d",
            "--label",
            "shellpp-test=deploy",
            "--label",
            run_label(),
            "--network",
            &network,
            "-p",
            &format!("127.0.0.1:{port}:8080"),
            "-v",
            &format!("{}:/run/secrets:ro", secrets.path().display()),
            "-e",
            &format!("SHELLPP_DATABASE_URL={url}"),
            "-e",
            &format!("SHELLPP_PUBLIC_URL={origin}"),
            "-e",
            "SHELLPP_SERVER_SECRET_FILE=/run/secrets/server_secret",
            "-e",
            "SHELLPP_REGISTRATION=open",
            "--entrypoint",
            "sh",
            image,
            "-c",
            "shellpp-server migrate && exec shellpp-server serve",
        ]);
        let id = id.trim().to_owned();
        cleanup.0.push(id.clone());
        let addr = format!("127.0.0.1:{port}");
        let mut ready = false;
        for _ in 0..120 {
            if raw_http_at(&addr, "GET", "/readyz", &[], b"")
                .is_some_and(|answer| answer.status == 200)
            {
                ready = true;
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(500));
        }
        let (_, logs) = docker(&["logs", &id]);
        assert!(ready, "the server answers on {database}: {logs}");
        assert!(!logs.contains("read from the environment"), "{logs}");
        id
    };
    let first = start_server("pg-old", &mut cleanup);

    let user_password = new_password();
    let email = email();
    let input = || shellpp_core::service::SignInInput {
        origin: origin.clone(),
        email: email.clone(),
        password: user_password.clone(),
        registration_code: None,
    };
    let sign_in = |client: &Client| {
        let outcome = client.core.sync_sign_in(input()).expect("signed in");
        assert!(matches!(outcome, SignInOutcome::Done(_)));
    };
    let a = Client::local("A", &user_password);
    a.host("before-backup.example.com");
    a.core.sync_register(input()).expect("registered");
    let b = Client::empty("B");
    sign_in(&b);
    assert_eq!(b.addresses(), vec!["before-backup.example.com"]);

    // The backup, as the compose file's service makes it.
    let (identity, public) = age_key();
    let backups = tempfile::tempdir().expect("dir");
    let out = docker_ok(&[
        "run",
        "--rm",
        "--label",
        "shellpp-test=deploy",
        "--label",
        run_label(),
        "--network",
        &network,
        "-v",
        &format!("{}:/backups", backups.path().display()),
        "-e",
        "PGHOST=pg-old",
        "-e",
        "PGUSER=shellpp",
        "-e",
        "PGDATABASE=shellpp",
        "-e",
        &format!("PGPASSWORD={password}"),
        "-e",
        &format!("AGE_RECIPIENT={public}"),
        "-e",
        "BACKUP_ONCE=1",
        backup,
    ]);
    assert!(out.contains("backup written"), "{out}");
    let files = backups_in(backups.path());
    assert_eq!(files.len(), 1, "one backup, no .part: {files:?}");
    assert!(files[0].ends_with(".sql.age"));
    let bytes = std::fs::read(backups.path().join(&files[0])).expect("backup");
    assert!(bytes.starts_with(b"age-encryption.org/v1"), "encrypted");
    assert!(
        !String::from_utf8_lossy(&bytes).contains("CREATE TABLE"),
        "no plaintext dump"
    );

    // A record made after the backup.
    a.host("after-backup.example.com");
    a.sync();

    // The restore: the server stops; the dump is decrypted with the key
    // kept elsewhere and loaded into a clean PostgreSQL; `admin restored`.
    docker_ok(&["rm", "-f", &first]);
    let keys = tempfile::tempdir().expect("dir");
    std::fs::write(keys.path().join("backup.key"), &identity).expect("identity");
    let load = format!(
        "set -eu -o pipefail; age -d -i /key/backup.key /backups/{} | psql -h pg-new -U shellpp -d shellpp -v ON_ERROR_STOP=1 -q",
        files[0]
    );
    docker_ok(&[
        "run",
        "--rm",
        "--label",
        "shellpp-test=deploy",
        "--label",
        run_label(),
        "--network",
        &network,
        "-v",
        &format!("{}:/backups:ro", backups.path().display()),
        "-v",
        &format!("{}:/key:ro", keys.path().display()),
        "-e",
        &format!("PGPASSWORD={password}"),
        "--entrypoint",
        "sh",
        backup,
        "-c",
        &load,
    ]);
    let restored = docker_ok(&[
        "run",
        "--rm",
        "--label",
        "shellpp-test=deploy",
        "--label",
        run_label(),
        "--network",
        &network,
        "-e",
        &format!("SHELLPP_DATABASE_URL=postgres://shellpp:{password}@pg-new:5432/shellpp"),
        image,
        "admin",
        "restored",
    ]);
    assert!(
        restored.contains("server generation replaced"),
        "{restored}"
    );
    start_server("pg-new", &mut cleanup);

    // Both devices notice the restore and keep their data; the record the
    // backup lacked comes back from A.
    let report = a.sync();
    assert!(report.restored, "A ran the restore procedure");
    assert_eq!(report.conflicts, 0);
    assert_eq!(
        a.addresses(),
        vec!["after-backup.example.com", "before-backup.example.com"]
    );
    let report = b.sync();
    assert!(report.restored, "B ran the restore procedure");
    assert_eq!(
        b.addresses(),
        vec!["after-backup.example.com", "before-backup.example.com"]
    );
    // A new device sees everything on the restored server.
    let c = Client::empty("C");
    sign_in(&c);
    assert_eq!(
        c.addresses(),
        vec!["after-backup.example.com", "before-backup.example.com"]
    );
    drop(cleanup);
}

/// The compose file as Compose reads it (D176, D177): the server
/// trusts exactly the gateway of the project's network, the same value as
/// `.env.example`; the server secret comes from the Docker secret and only
/// `server` gets it; the backup service runs the image built from
/// `backup.Dockerfile`, not a stock Alpine that installs its tools.
#[test]
fn the_compose_file_trusts_its_gateway_and_hands_over_the_secret() {
    if !enabled() {
        return;
    }
    let dir = tempfile::tempdir().expect("dir");
    let env = dir.path().join("test.env");
    std::fs::write(
        &env,
        "POSTGRES_PASSWORD=x\nSHELLPP_PUBLIC_URL=https://sync.example.com\nBACKUP_AGE_RECIPIENT=age1x\n",
    )
    .expect("env");
    let json = docker_ok(&[
        "compose",
        "-f",
        &deploy_dir()
            .join("docker-compose.yml")
            .display()
            .to_string(),
        "--env-file",
        &env.display().to_string(),
        "-p",
        "shellpp-config-check",
        "config",
        "--format",
        "json",
    ]);
    let config: serde_json::Value = serde_json::from_str(&json).expect("compose config");
    let gateway = config["networks"]["default"]["ipam"]["config"][0]["gateway"]
        .as_str()
        .expect("a fixed gateway");
    let services = &config["services"];
    let trusted = services["server"]["environment"]["SHELLPP_TRUSTED_PROXIES"]
        .as_str()
        .expect("trusted proxies");
    assert_eq!(trusted, format!("{gateway}/32"));
    let example = std::fs::read_to_string(deploy_dir().join(".env.example")).expect("example");
    assert!(
        example
            .lines()
            .any(|line| line == format!("SHELLPP_TRUSTED_PROXIES={trusted}")),
        ".env.example names the same address"
    );
    assert_eq!(
        services["server"]["environment"]["SHELLPP_SERVER_SECRET_FILE"],
        "/run/secrets/server_secret"
    );
    assert!(services["server"]["environment"]["SHELLPP_SERVER_SECRET"].is_null());
    assert_eq!(services["server"]["secrets"][0]["source"], "server_secret");
    assert!(
        config["secrets"]["server_secret"]["file"]
            .as_str()
            .is_some_and(|file| file.replace('\\', "/").ends_with("secrets/server_secret"))
    );
    assert!(services["migrate"]["environment"]["SHELLPP_SERVER_SECRET_FILE"].is_null());
    assert_eq!(
        services["backup"]["build"]["dockerfile"],
        "backup.Dockerfile"
    );
    assert!(
        services["backup"]["volumes"]
            .as_array()
            .is_some_and(|volumes| volumes
                .iter()
                .all(|volume| volume["target"] != "/backup.sh"))
    );
    assert!(
        !example.contains("SHELLPP_SERVER_SECRET="),
        "no secret in .env"
    );
}
