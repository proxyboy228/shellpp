//! The SSH test server of the developer benchmarks and terminal screenshots
//! (feature `dev-tools`): never a real host.
//!
//! Two kinds, both outside the app's process (so the app's memory and CPU
//! figures do not include the server):
//! - **OpenSSH in Docker** (`--test-server <server.json>`): the file that
//!   `tests/openssh/start.sh` writes. Server-side faults go through
//!   `tests/openssh/control.sh` (bash: `SHELLPP_BASH`, Git's bash on
//!   Windows, else `bash`). The scripts own the container.
//! - **russh test server** (default): `shellpp-test-sshd serve` started as a
//!   child process (`SHELLPP_TEST_SSHD_EXE`, else the binary next to this
//!   one: `cargo build -p shellpp-test-sshd`), with one password user and a
//!   generated password. Faults go through its stdin (`rotate-host-key`,
//!   `drop`); it exits when the app does (stdin closes).
//!
//! Only the password user is used here: `pwuser` on both servers. Nothing
//! secret is printed or logged; the russh configuration file (it holds the
//! generated password) is deleted as soon as the server is up.

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use secrecy::SecretString;
use serde_json::{Value, json};

/// The user of the benchmarks and screenshots on both servers.
pub const USER: &str = "pwuser";

/// How long a control command of the russh server may take.
const REPLY_TIMEOUT: Duration = Duration::from_secs(20);

/// Which server runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServerKind {
    OpenSsh,
    Russh,
}

impl ServerKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenSsh => "OpenSSH in Docker (tests/openssh)",
            Self::Russh => "russh test server (tests/sshd)",
        }
    }
}

/// A running test server.
pub struct DevServer {
    pub kind: ServerKind,
    pub addr: SocketAddr,
    pub user: String,
    pub password: SecretString,
    /// `SHA256:` fingerprints the server may present (one per key type).
    pub host_key_fingerprints: Vec<String>,
    control: Control,
}

impl std::fmt::Debug for DevServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DevServer")
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

enum Control {
    Russh {
        child: Child,
        stdin: Option<ChildStdin>,
        replies: mpsc::Receiver<Value>,
    },
    OpenSsh {
        workdir: PathBuf,
    },
}

impl DevServer {
    /// `--test-server <path>` when given, otherwise the russh child process.
    pub fn start(server_json: Option<&Path>) -> Result<Self, String> {
        match server_json {
            Some(path) => Self::from_server_json(path),
            None => Self::spawn_russh(),
        }
    }

    /// The OpenSSH server described by `server.json` of
    /// `tests/openssh/start.sh` (its directory also holds `container`).
    /// Only a loopback address is accepted: the tools send the test user's
    /// password, so they never talk to another machine.
    pub fn from_server_json(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        let json: Value = serde_json::from_str(&text)
            .map_err(|_| format!("{} is not valid JSON", path.display()))?;
        let host: std::net::IpAddr = json
            .get("host")
            .and_then(Value::as_str)
            .and_then(|host| host.parse().ok())
            .ok_or("server.json: bad host")?;
        if !host.is_loopback() {
            return Err("server.json: the test server must have a loopback address".to_owned());
        }
        let port = json
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok())
            .ok_or("server.json: bad port")?;
        let user = json
            .pointer("/users/password/name")
            .and_then(Value::as_str)
            .ok_or("server.json: no password user")?
            .to_owned();
        let password = json
            .pointer("/users/password/password")
            .and_then(Value::as_str)
            .ok_or("server.json: no password")?;
        let workdir = path
            .parent()
            .ok_or("server.json has no directory")?
            .to_path_buf();
        Ok(Self {
            kind: ServerKind::OpenSsh,
            addr: SocketAddr::new(host, port),
            user,
            password: SecretString::from(password.to_owned()),
            host_key_fingerprints: host_keys(&json),
            control: Control::OpenSsh { workdir },
        })
    }

    /// Starts `shellpp-test-sshd serve` with one password user.
    pub fn spawn_russh() -> Result<Self, String> {
        let exe = test_sshd_exe()?;
        let password = crate::testing::generated_password();
        let config = json!({
            "bind": "127.0.0.1:0",
            "host_key": {"generate": "ed25519"},
            "users": [{"name": USER, "password": secrecy::ExposeSecret::expose_secret(&password)}],
            // OS detection of the session layer (an exec request), as a
            // Debian server answers it.
            "exec": {"stdout": "Linux\nID=debian\nVERSION_ID=13\n", "exit_status": 0},
            "banner_comment": "Debian-5",
        });
        let dir = crate::profile::TempProfile::create("shellpp-test-sshd")
            .map_err(|error| format!("temporary directory: {error}"))?;
        let config_path = dir.path().join("server-config.json");
        write_private(&config_path, config.to_string().as_bytes())
            .map_err(|error| format!("server configuration: {error}"))?;
        let mut child = Command::new(&exe)
            .arg("serve")
            .arg("--config")
            .arg(&config_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|error| format!("{} did not start: {error}", exe.display()))?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (sender, replies) = mpsc::channel();
        std::thread::Builder::new()
            .name("test-sshd-stdout".to_owned())
            .spawn(move || {
                for line in BufReader::new(stdout).lines() {
                    let Ok(line) = line else { break };
                    let Ok(value) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    if sender.send(value).is_err() {
                        break;
                    }
                }
            })
            .map_err(|error| format!("reader thread: {error}"))?;
        let stdin = child.stdin.take();
        let mut control = Control::Russh {
            child,
            stdin,
            replies,
        };
        let ready = match wait_reply(&mut control, "ready") {
            Ok(ready) => ready,
            Err(error) => {
                stop(&mut control);
                return Err(error);
            }
        };
        // The server read its configuration; the password is no longer on
        // disk once the directory is gone.
        drop(dir);
        let port = ready
            .get("port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok())
            .ok_or("the test server reported no port")?;
        let fingerprint = ready
            .get("host_key_fingerprint")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        Ok(Self {
            kind: ServerKind::Russh,
            addr: SocketAddr::from(([127, 0, 0, 1], port)),
            user: USER.to_owned(),
            password,
            host_key_fingerprints: vec![fingerprint],
            control,
        })
    }

    /// New host keys; sessions already open keep running.
    pub fn rotate_host_key(&mut self) -> Result<(), String> {
        match &mut self.control {
            Control::Russh { .. } => {
                send(&mut self.control, "rotate-host-key")?;
                let reply = wait_reply(&mut self.control, "rotated")?;
                let fingerprint = reply
                    .get("host_key_fingerprint")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                self.host_key_fingerprints = vec![fingerprint.to_owned()];
                Ok(())
            }
            Control::OpenSsh { workdir } => {
                let workdir = workdir.clone();
                run_control(&workdir, "rotate-host-key")?;
                let text = std::fs::read_to_string(workdir.join("server.json"))
                    .map_err(|error| format!("server.json: {error}"))?;
                let json: Value =
                    serde_json::from_str(&text).map_err(|_| "server.json is not valid JSON")?;
                self.host_key_fingerprints = host_keys(&json);
                Ok(())
            }
        }
    }

    /// Closes every connection without an exit status (a transport drop);
    /// the server keeps accepting. Returns the server's count.
    pub fn drop_connections(&mut self) -> Result<usize, String> {
        match &mut self.control {
            Control::Russh { .. } => {
                send(&mut self.control, "drop")?;
                let reply = wait_reply(&mut self.control, "dropped")?;
                Ok(reply.get("count").and_then(Value::as_u64).unwrap_or(0) as usize)
            }
            Control::OpenSsh { workdir } => {
                let workdir = workdir.clone();
                let output = run_control(&workdir, "drop-connections")?;
                Ok(output.trim().parse().unwrap_or(0))
            }
        }
    }

    /// Saves a host for this server (address, port, user and its saved
    /// password) in the personal vault of an unlocked core; returns the
    /// vault and record ids.
    pub fn save_host(
        &self,
        core: &shellpp_core::Core,
        label: &str,
    ) -> Result<(uuid::Uuid, uuid::Uuid), String> {
        use shellpp_core::service::dto::{RecordSaveInput, RecordType, RecordView, VaultKind};

        let vault_id = core
            .vaults()
            .map_err(|error| error.code().to_owned())?
            .into_iter()
            .find(|vault| vault.kind == VaultKind::Personal)
            .map(|vault| vault.vault_id)
            .ok_or("no personal vault")?;
        let Value::Object(data) = json!({
            "label": label,
            "address": self.addr.ip().to_string(),
            "ssh": {"port": self.addr.port(), "username": self.user},
        }) else {
            return Err("host data".to_owned());
        };
        let mut secrets = std::collections::BTreeMap::new();
        secrets.insert("ssh.password".to_owned(), Some(self.password.clone()));
        let view = core
            .record_save(RecordSaveInput {
                vault_id,
                record_id: None,
                record_type: RecordType::Host,
                data,
                secrets,
            })
            .map_err(|error| error.code().to_owned())?;
        let record_id = match view {
            RecordView::Host(view) => view.record_id,
            RecordView::Group(view) => view.record_id,
        };
        Ok((vault_id, record_id))
    }

    /// Whether the server has a real bash with `ls --color`, `htop` and `mc`
    /// (the russh test server has its own small shell instead).
    pub fn has_real_shell(&self) -> bool {
        self.kind == ServerKind::OpenSsh
    }
}

impl Drop for DevServer {
    fn drop(&mut self) {
        stop(&mut self.control);
    }
}

/// Ends the russh child: `quit`, then closing stdin, then a kill.
fn stop(control: &mut Control) {
    if let Control::Russh { child, stdin, .. } = control {
        if let Some(mut input) = stdin.take() {
            let _ = input.write_all(b"quit\n");
            let _ = input.flush();
        }
        for _ in 0..50 {
            if matches!(child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = child.kill();
        let _ = child.wait();
    }
}

fn send(control: &mut Control, command: &str) -> Result<(), String> {
    match control {
        Control::Russh {
            stdin: Some(stdin), ..
        } => stdin
            .write_all(format!("{command}\n").as_bytes())
            .and_then(|()| stdin.flush())
            .map_err(|error| format!("test server stdin: {error}")),
        _ => Err("the test server is not running".to_owned()),
    }
}

/// The next reply line whose `event` is `event` (an `error` reply fails).
fn wait_reply(control: &mut Control, event: &str) -> Result<Value, String> {
    let Control::Russh { replies, .. } = control else {
        return Err("not a russh test server".to_owned());
    };
    let deadline = std::time::Instant::now() + REPLY_TIMEOUT;
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        let reply = replies
            .recv_timeout(left)
            .map_err(|_| format!("no {event:?} from the test server"))?;
        match reply.get("event").and_then(Value::as_str) {
            Some(name) if name == event => return Ok(reply),
            Some("error") => {
                let message = reply
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown error");
                return Err(format!("test server: {message}"));
            }
            _ => continue,
        }
    }
}

/// `SHELLPP_TEST_SSHD_EXE`, else `shellpp-test-sshd` next to this binary.
fn test_sshd_exe() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("SHELLPP_TEST_SSHD_EXE") {
        return Ok(PathBuf::from(path));
    }
    let name = format!("shellpp-test-sshd{}", std::env::consts::EXE_SUFFIX);
    let exe = std::env::current_exe().map_err(|error| format!("current exe: {error}"))?;
    // Unit tests run from target/<profile>/deps.
    for dir in exe.ancestors().skip(1).take(2) {
        let candidate = dir.join(&name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "{name} not found next to this binary (cargo build -p shellpp-test-sshd, or set SHELLPP_TEST_SSHD_EXE)"
    ))
}

fn host_keys(json: &Value) -> Vec<String> {
    json.get("host_keys")
        .and_then(Value::as_object)
        .map(|keys| {
            keys.values()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// bash for `tests/openssh/control.sh`: `SHELLPP_BASH`, else Git's bash on
/// Windows (WSL's `bash.exe` cannot read Windows paths), else `bash`.
fn bash() -> Result<PathBuf, String> {
    if let Some(path) = std::env::var_os("SHELLPP_BASH") {
        return Ok(PathBuf::from(path));
    }
    if cfg!(windows) {
        let output = Command::new("git")
            .arg("--exec-path")
            .stdin(Stdio::null())
            .output()
            .map_err(|_| "git not found (needed to find Git's bash; or set SHELLPP_BASH)")?;
        let exec_path = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim());
        return exec_path
            .ancestors()
            .nth(3)
            .map(|git| git.join("bin").join("bash.exe"))
            .filter(|bash| bash.is_file())
            .ok_or_else(|| "Git's bash not found (set SHELLPP_BASH)".to_owned());
    }
    Ok(PathBuf::from("bash"))
}

fn run_control(workdir: &Path, command: &str) -> Result<String, String> {
    let script = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("openssh")
        .join("control.sh");
    let output = Command::new(bash()?)
        .arg(script)
        .arg(workdir)
        .arg(command)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("control.sh did not run: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "control.sh {command} failed ({}): {}",
            output.status,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

/// Writes a file readable by the owner only (Unix mode 0600; on Windows the
/// private temporary directory's ACL applies).
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(bytes)?;
    file.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The server.json of `tests/openssh/start.sh`, with a generated password.
    #[test]
    fn reads_the_openssh_server_description() {
        let dir = crate::profile::TempProfile::create("shellpp-test-server-json").unwrap();
        let password = crate::profile::random_suffix().unwrap();
        let path = dir.path().join("server.json");
        let text = json!({
            "host": "127.0.0.1",
            "port": 49153,
            "host_keys": {"ssh-ed25519": "SHA256:aaaa", "ssh-rsa": "SHA256:bbbb"},
            "users": {
                "password": {"name": "pwuser", "password": password},
                "publickey": {"name": "keyuser", "private_key_path": "/x/client_ed25519"},
                "keyboard_interactive": {"name": "kiuser", "password": "other"}
            },
            "container": "shellpp-openssh-0123456789ab"
        });
        std::fs::write(&path, text.to_string()).unwrap();
        let server = DevServer::from_server_json(&path).unwrap();
        assert_eq!(server.kind, ServerKind::OpenSsh);
        assert_eq!(server.addr, "127.0.0.1:49153".parse().unwrap());
        assert_eq!(server.user, "pwuser");
        assert_eq!(
            secrecy::ExposeSecret::expose_secret(&server.password),
            password
        );
        let mut fingerprints = server.host_key_fingerprints.clone();
        fingerprints.sort();
        assert_eq!(fingerprints, ["SHA256:aaaa", "SHA256:bbbb"]);
        assert!(server.has_real_shell());
        // Nothing to stop for a server the scripts own.
        drop(server);
        assert!(DevServer::from_server_json(&dir.path().join("missing.json")).is_err());

        // Another machine is refused: the tools would send it the password.
        for host in ["192.0.2.10", "::ffff:192.0.2.10", "0.0.0.0"] {
            let mut other = text.clone();
            other["host"] = json!(host);
            let path = dir.path().join("other.json");
            std::fs::write(&path, other.to_string()).unwrap();
            let error = DevServer::from_server_json(&path).unwrap_err();
            assert!(error.contains("loopback"), "{host}: {error}");
        }
        let mut loopback6 = text.clone();
        loopback6["host"] = json!("::1");
        std::fs::write(&path, loopback6.to_string()).unwrap();
        assert!(DevServer::from_server_json(&path).is_ok());
    }

    /// The russh child process when its binary is built (it is when the
    /// whole workspace is): starts, rotates its host key, drops connections
    /// and exits with the app.
    #[test]
    fn russh_child_process_starts_and_answers_control_commands() {
        if test_sshd_exe().is_err() {
            // Not built in this target directory (e.g. `cargo test -p
            // shellpp-desktop` alone); the scripts build it.
            return;
        }
        let mut server = DevServer::spawn_russh().unwrap();
        assert_eq!(server.kind, ServerKind::Russh);
        assert!(server.addr.ip().is_loopback());
        let before = server.host_key_fingerprints.clone();
        assert!(before[0].starts_with("SHA256:"));
        server.rotate_host_key().unwrap();
        assert_ne!(server.host_key_fingerprints, before);
        assert_eq!(server.drop_connections().unwrap(), 0);
        assert!(!server.has_real_shell());
        let stream = std::net::TcpStream::connect(server.addr).unwrap();
        drop(stream);
        drop(server);
    }
}
