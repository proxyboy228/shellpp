//! Test SSH servers of the session integration tests (`tests/sessions.rs`).
//!
//! Which server runs (`SHELLPP_TEST_SSHD`):
//! - unset, empty or `auto`: OpenSSH in Docker (`tests/openssh`) when a
//!   Docker daemon for Linux containers answers `docker info` and bash is
//!   available (Git Bash on Windows, never WSL's `bash.exe`); otherwise the
//!   russh test server (`shellpp-test-sshd`) in process;
//! - `openssh`: OpenSSH in Docker, required (a test panics when it cannot
//!   start, so CI never passes on the wrong server);
//! - `russh`: the russh test server.
//!
//! `SHELLPP_OPENSSH_SERVER_JSON=<path>` reuses a server that
//! `tests/openssh/start.sh` already started (its directory must also hold
//! the `container` file for `control.sh`); that container is left running.
//!
//! Every server announces itself with one line on stderr (written past the
//! test harness's output capture), so a test log always says which server a
//! run used and why.
//!
//! Accounts (the same names on both servers, each with one authentication
//! method): `pwuser` (password), `keyuser` (public key: a fresh Ed25519 key,
//! no passphrase, written as `id_ed25519` into [`TestSshd::key_dir`], a
//! temporary directory laid out like `~/.ssh`) and `kiuser`
//! (keyboard-interactive: one question, `Password: `, not echoed). Passwords,
//! keys and host keys are generated for each server; nothing is read from the
//! real `~/.ssh` or the real agent. Servers listen on 127.0.0.1 only.
//!
//! The russh test server offers one method set per server, so `kiuser` lives
//! on a second russh server ([`TestSshd::ki_addr`]); on OpenSSH both
//! addresses are the same.

#![allow(dead_code)] // each test uses a subset

use std::io::Write as _;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use secrecy::{ExposeSecret, SecretString};
use serde_json::Value;
use shellpp_test_sshd::{KeyType, KiQuestion, ServerConfig, TestServer, UserConfig, generate_key};

pub const PASSWORD_USER: &str = "pwuser";
pub const KEY_USER: &str = "keyuser";
pub const KI_USER: &str = "kiuser";
/// The keyboard-interactive question of `kiuser` on both servers (PAM's).
pub const KI_PROMPT: &str = "Password: ";

/// Which server runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerKind {
    /// OpenSSH in Docker (`tests/openssh`).
    OpenSsh,
    /// `shellpp-test-sshd` in process.
    Russh,
}

impl ServerKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenSsh => "OpenSSH in Docker (tests/openssh)",
            Self::Russh => "russh test server (tests/sshd, in process)",
        }
    }
}

/// A user name and its password or keyboard-interactive answer.
#[derive(Clone)]
pub struct Account {
    pub name: String,
    pub secret: SecretString,
}

impl std::fmt::Debug for Account {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Account").field("name", &self.name).finish()
    }
}

/// Shell commands of the server and what they print, for checks on the
/// session output (bytes) or on the terminal screen (rows).
#[derive(Clone, Copy, Debug)]
pub struct Commands {
    /// A command line (without the line ending) that prints [`MARKER`]; the
    /// marker never appears in the echoed command line itself.
    pub marker: &'static str,
    /// What the output contains after the command ran (the marker on its
    /// own line).
    pub marker_output: &'static str,
}

/// The text that [`Commands::marker`] prints, alone on its line.
pub const MARKER: &str = "shellpp-42";

const RUSSH_COMMANDS: Commands = Commands {
    marker: "echo shellpp-42",
    marker_output: "\r\nshellpp-42\r\n",
};

const OPENSSH_COMMANDS: Commands = Commands {
    marker: "echo shellpp-$((6*7))",
    marker_output: "shellpp-42\r\n",
};

/// A running test server (see the module documentation). Dropping it stops
/// the server (a reused OpenSSH container keeps running).
pub struct TestSshd {
    pub kind: ServerKind,
    /// The address of `pwuser` and `keyuser`.
    pub addr: SocketAddr,
    /// The address of `kiuser`.
    pub ki_addr: SocketAddr,
    pub password_user: Account,
    pub key_user: String,
    /// A temporary directory like `~/.ssh` with `id_ed25519` and
    /// `id_ed25519.pub` of `keyuser` (no passphrase): the key directory the
    /// tests inject into the session layer.
    pub key_dir: tempfile::TempDir,
    pub ki_user: Account,
    inner: Inner,
}

impl std::fmt::Debug for TestSshd {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TestSshd")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

enum Inner {
    Russh {
        /// The servers' own runtime: they keep serving while a test blocks
        /// or drives GPUI's executor.
        runtime: tokio::runtime::Runtime,
        main: Option<TestServer>,
        ki: Option<TestServer>,
    },
    OpenSsh {
        /// The work directory of a container started here (removed with
        /// the container); `None` for a reused one.
        started: Option<tempfile::TempDir>,
        /// The directory of `server.json` and `container`.
        workdir: PathBuf,
        bash: PathBuf,
        /// Key type -> `SHA256:` fingerprint, updated on rotation.
        host_keys: std::sync::Mutex<Vec<(String, String)>>,
    },
}

/// Writes one line to the real stderr, past the test harness's capture.
fn announce(line: &str) {
    let mut stderr = std::io::stderr().lock();
    let _ = writeln!(stderr, "{line}");
    let _ = stderr.flush();
}

fn repo_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn openssh_dir() -> PathBuf {
    repo_dir().join("tests").join("openssh")
}

/// A random secret (hex) for passwords and answers.
pub fn random_secret() -> SecretString {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("random bytes");
    let text: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    SecretString::from(text)
}

/// The requested server kind, or the reason for the automatic choice.
enum Choice {
    OpenSsh { bash: PathBuf, required: bool },
    Russh(String),
}

fn choose() -> Choice {
    let requested = std::env::var("SHELLPP_TEST_SSHD").unwrap_or_default();
    match requested.as_str() {
        "russh" => Choice::Russh("SHELLPP_TEST_SSHD=russh".to_owned()),
        "openssh" => match find_bash() {
            Some(bash) => Choice::OpenSsh {
                bash,
                required: true,
            },
            None => panic!("SHELLPP_TEST_SSHD=openssh, but bash was not found (set SHELLPP_BASH)"),
        },
        "" | "auto" => {
            if std::env::var_os("SHELLPP_OPENSSH_SERVER_JSON").is_some() {
                return match find_bash() {
                    Some(bash) => Choice::OpenSsh {
                        bash,
                        required: true,
                    },
                    None => panic!("SHELLPP_OPENSSH_SERVER_JSON is set, but bash was not found"),
                };
            }
            match docker_linux_daemon() {
                Err(reason) => Choice::Russh(reason),
                Ok(()) => match find_bash() {
                    Some(bash) => Choice::OpenSsh {
                        bash,
                        required: false,
                    },
                    None => Choice::Russh("Docker is running, but bash was not found".to_owned()),
                },
            }
        }
        other => panic!("SHELLPP_TEST_SSHD must be auto, russh or openssh, not {other:?}"),
    }
}

/// `Ok` when `docker info` answers for Linux containers.
fn docker_linux_daemon() -> Result<(), String> {
    let output = Command::new("docker")
        .args(["info", "--format", "{{.OSType}}"])
        .stdin(Stdio::null())
        .output();
    match output {
        Err(_) => Err("Docker is not installed".to_owned()),
        Ok(output) if !output.status.success() => {
            Err("Docker is not running (docker info failed)".to_owned())
        }
        Ok(output) => {
            let os = String::from_utf8_lossy(&output.stdout).trim().to_owned();
            if os == "linux" {
                Ok(())
            } else {
                Err(format!("the Docker daemon runs {os} containers, not linux"))
            }
        }
    }
}

/// bash for the scripts of `tests/openssh`: `SHELLPP_BASH`, else on Windows
/// Git's bash (found next to `git`; WSL's `bash.exe` cannot read Windows
/// paths), else `bash` on the PATH.
fn find_bash() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("SHELLPP_BASH") {
        return Some(PathBuf::from(path));
    }
    if cfg!(windows) {
        let output = Command::new("git")
            .arg("--exec-path")
            .stdin(Stdio::null())
            .output()
            .ok()?;
        // <git>/mingw64/libexec/git-core -> <git>/bin/bash.exe
        let exec_path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        let bash = exec_path.ancestors().nth(3)?.join("bin").join("bash.exe");
        return bash.is_file().then_some(bash);
    }
    Command::new("bash")
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .status()
        .ok()
        .filter(|status| status.success())
        .map(|_| PathBuf::from("bash"))
}

impl TestSshd {
    /// Starts the server chosen as the module documentation says.
    pub fn start() -> TestSshd {
        match choose() {
            Choice::Russh(reason) => {
                let server = Self::start_russh();
                announce(&format!(
                    "session tests: server = {} ({reason})",
                    server.kind.label()
                ));
                server
            }
            Choice::OpenSsh { bash, required } => match Self::start_openssh(&bash) {
                Ok(server) => {
                    announce(&format!("session tests: server = {}", server.kind.label()));
                    server
                }
                Err(error) if !required => {
                    let server = Self::start_russh();
                    announce(&format!(
                        "session tests: server = {} (OpenSSH did not start: {error})",
                        server.kind.label()
                    ));
                    server
                }
                Err(error) => panic!("the OpenSSH test server did not start: {error}"),
            },
        }
    }

    /// The russh test servers: `pwuser` and `keyuser` on one, `kiuser` on
    /// another (keyboard-interactive only).
    pub fn start_russh() -> TestSshd {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("test-sshd")
            .enable_all()
            .build()
            .expect("server runtime");
        let password = random_secret();
        let ki_answer = random_secret();
        let key = generate_key(KeyType::Ed25519, "shellpp-session-test", None).expect("key");
        let key_dir = key_dir_with(key.private_openssh.expose_secret(), &key.public_openssh);
        let main_config = ServerConfig::new(vec![
            UserConfig::new(PASSWORD_USER).with_password(password.clone()),
            UserConfig::new(KEY_USER).with_authorized_key(&key.public_openssh),
        ]);
        let ki_config =
            ServerConfig::new(vec![UserConfig::new(KI_USER).with_keyboard_interactive(
                "",
                "",
                vec![KiQuestion::new(KI_PROMPT, false, ki_answer.clone())],
            )]);
        let (main, ki) = runtime.block_on(async {
            let main = TestServer::start(main_config)
                .await
                .expect("russh test server starts");
            let ki = TestServer::start(ki_config)
                .await
                .expect("russh test server (keyboard-interactive) starts");
            (main, ki)
        });
        TestSshd {
            kind: ServerKind::Russh,
            addr: main.addr(),
            ki_addr: ki.addr(),
            password_user: Account {
                name: PASSWORD_USER.to_owned(),
                secret: password,
            },
            key_user: KEY_USER.to_owned(),
            key_dir,
            ki_user: Account {
                name: KI_USER.to_owned(),
                secret: ki_answer,
            },
            inner: Inner::Russh {
                runtime,
                main: Some(main),
                ki: Some(ki),
            },
        }
    }

    /// OpenSSH in Docker through `tests/openssh/start.sh`, or the server of
    /// `SHELLPP_OPENSSH_SERVER_JSON`.
    pub fn start_openssh(bash: &Path) -> Result<TestSshd, String> {
        let (started, workdir) = match std::env::var_os("SHELLPP_OPENSSH_SERVER_JSON") {
            Some(path) => {
                let path = PathBuf::from(path);
                let workdir = path
                    .parent()
                    .ok_or("SHELLPP_OPENSSH_SERVER_JSON has no directory")?
                    .to_path_buf();
                (None, workdir)
            }
            None => {
                let dir = tempfile::Builder::new()
                    .prefix("shellpp-openssh-")
                    .tempdir()
                    .map_err(|error| format!("temporary directory: {error}"))?;
                let output = Command::new(bash)
                    .arg(openssh_dir().join("start.sh"))
                    .arg(dir.path())
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .output()
                    .map_err(|error| format!("bash did not run: {error}"))?;
                if !output.status.success() {
                    // start.sh never prints secrets; its last lines say why.
                    let stderr = String::from_utf8_lossy(&output.stderr);
                    let tail: Vec<&str> = stderr.lines().rev().take(5).collect();
                    let tail: Vec<&str> = tail.into_iter().rev().collect();
                    return Err(format!(
                        "start.sh exited with {} ({})",
                        output.status,
                        tail.join(" | ")
                    ));
                }
                let workdir = dir.path().to_path_buf();
                (Some(dir), workdir)
            }
        };
        let text = std::fs::read_to_string(workdir.join("server.json"))
            .map_err(|error| format!("server.json: {error}"))?;
        let json: Value =
            serde_json::from_str(&text).map_err(|_| "server.json is not valid JSON".to_owned())?;
        let string = |path: &[&str]| -> Result<String, String> {
            path.iter()
                .try_fold(&json, |value, key| value.get(key))
                .and_then(Value::as_str)
                .map(str::to_owned)
                .ok_or_else(|| format!("server.json has no {}", path.join(".")))
        };
        let host: std::net::IpAddr = string(&["host"])?
            .parse()
            .map_err(|_| "server.json: bad host".to_owned())?;
        let port = json
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok())
            .ok_or("server.json: bad port")?;
        let addr = SocketAddr::new(host, port);
        let host_keys = parse_host_keys(&json);
        let key_path = string(&["users", "publickey", "private_key_path"])?;
        let private_key =
            std::fs::read_to_string(&key_path).map_err(|error| format!("client key: {error}"))?;
        let public_key = std::fs::read_to_string(format!("{key_path}.pub"))
            .map_err(|error| format!("client public key: {error}"))?;
        Ok(TestSshd {
            kind: ServerKind::OpenSsh,
            addr,
            ki_addr: addr,
            password_user: Account {
                name: string(&["users", "password", "name"])?,
                secret: SecretString::from(string(&["users", "password", "password"])?),
            },
            key_user: string(&["users", "publickey", "name"])?,
            key_dir: key_dir_with(&private_key, public_key.trim()),
            ki_user: Account {
                name: string(&["users", "keyboard_interactive", "name"])?,
                secret: SecretString::from(string(&["users", "keyboard_interactive", "password"])?),
            },
            inner: Inner::OpenSsh {
                started,
                workdir,
                bash: bash.to_path_buf(),
                host_keys: std::sync::Mutex::new(host_keys),
            },
        })
    }

    /// Shell commands for this server.
    pub fn commands(&self) -> Commands {
        match self.kind {
            ServerKind::OpenSsh => OPENSSH_COMMANDS,
            ServerKind::Russh => RUSSH_COMMANDS,
        }
    }

    /// The `SHA256:` fingerprints the server at [`Self::addr`] may present
    /// (OpenSSH offers one key per type; the client picks the type).
    pub fn host_key_fingerprints(&self) -> Vec<String> {
        match &self.inner {
            Inner::Russh { main, .. } => {
                main.iter().map(TestServer::host_key_fingerprint).collect()
            }
            Inner::OpenSsh { host_keys, .. } => host_keys
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .iter()
                .map(|(_, fingerprint)| fingerprint.clone())
                .collect(),
        }
    }

    /// The same for [`Self::ki_addr`].
    pub fn ki_host_key_fingerprints(&self) -> Vec<String> {
        match &self.inner {
            Inner::Russh { ki, .. } => ki.iter().map(TestServer::host_key_fingerprint).collect(),
            Inner::OpenSsh { .. } => self.host_key_fingerprints(),
        }
    }

    /// New host keys for the server at [`Self::addr`]; open sessions keep
    /// running, new connections see the new key.
    pub fn rotate_host_key(&self) {
        match &self.inner {
            Inner::Russh { runtime, main, .. } => {
                let server = main.as_ref().expect("server running");
                runtime
                    .block_on(server.rotate_host_key())
                    .expect("host key rotated");
            }
            Inner::OpenSsh {
                workdir,
                bash,
                host_keys,
                ..
            } => {
                control(bash, workdir, "rotate-host-key");
                let text = std::fs::read_to_string(workdir.join("server.json"))
                    .expect("server.json after rotation");
                let json: Value = serde_json::from_str(&text).expect("server.json");
                *host_keys
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner) = parse_host_keys(&json);
            }
        }
    }

    /// Closes every connection to the server at [`Self::addr`] without an
    /// exit status (a transport drop for the client); the server keeps
    /// accepting new connections. Returns how many were dropped (OpenSSH:
    /// connection processes killed).
    pub fn drop_connections(&self) -> usize {
        match &self.inner {
            Inner::Russh { main, .. } => main.as_ref().expect("server running").drop_connections(),
            Inner::OpenSsh { workdir, bash, .. } => control(bash, workdir, "drop-connections")
                .trim()
                .parse()
                .expect("a count from control.sh"),
        }
    }

    /// Live connections to the server at [`Self::addr`] (russh only).
    pub fn connection_count(&self) -> Option<usize> {
        match &self.inner {
            Inner::Russh { main, .. } => main.as_ref().map(TestServer::connection_count),
            Inner::OpenSsh { .. } => None,
        }
    }
}

impl Drop for TestSshd {
    fn drop(&mut self) {
        match &mut self.inner {
            Inner::Russh { runtime, main, ki } => {
                let main = main.take();
                let ki = ki.take();
                runtime.block_on(async {
                    if let Some(server) = main {
                        server.shutdown().await;
                    }
                    if let Some(server) = ki {
                        server.shutdown().await;
                    }
                });
            }
            Inner::OpenSsh { started, bash, .. } => {
                if let Some(dir) = started.as_ref() {
                    let _ = Command::new(&*bash)
                        .arg(openssh_dir().join("stop.sh"))
                        .arg(dir.path())
                        .stdin(Stdio::null())
                        .stdout(Stdio::null())
                        .stderr(Stdio::null())
                        .status();
                }
            }
        }
    }
}

/// Runs `tests/openssh/control.sh <workdir> <command>`; returns its stdout.
fn control(bash: &Path, workdir: &Path, command: &str) -> String {
    let output = Command::new(bash)
        .arg(openssh_dir().join("control.sh"))
        .arg(workdir)
        .arg(command)
        .stdin(Stdio::null())
        .output()
        .expect("bash runs control.sh");
    assert!(
        output.status.success(),
        "control.sh {command} failed ({}): {}",
        output.status,
        String::from_utf8_lossy(&output.stderr).trim()
    );
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn parse_host_keys(json: &Value) -> Vec<(String, String)> {
    json.get("host_keys")
        .and_then(Value::as_object)
        .map(|keys| {
            keys.iter()
                .filter_map(|(key_type, fingerprint)| {
                    Some((key_type.clone(), fingerprint.as_str()?.to_owned()))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// A temporary `~/.ssh`-like directory with `id_ed25519` and its `.pub`.
fn key_dir_with(private_openssh: &str, public_openssh: &str) -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("shellpp-test-keys-")
        .tempdir()
        .expect("key directory");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(dir.path().join("id_ed25519"))
        .expect("private key file");
    file.write_all(private_openssh.as_bytes())
        .expect("private key written");
    std::fs::write(
        dir.path().join("id_ed25519.pub"),
        format!("{public_openssh}\n"),
    )
    .expect("public key written");
    dir
}

/// Polls `condition` every 20 ms until it holds or `timeout` passes.
pub fn eventually(timeout: Duration, mut condition: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if condition() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
