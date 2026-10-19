//! Server providers of the parameterized suite (`tests/provider.rs`):
//! `SHELLPP_TEST_SSHD` unset or `russh` runs `shellpp-test-sshd` in process;
//! `openssh` runs the Docker OpenSSH server of `tests/openssh` (start.sh /
//! stop.sh, Linux CI). Without a working Docker daemon the OpenSSH variant
//! is skipped with a printed reason; on Windows it is skipped as well
//! (`bash` may be WSL's, which cannot read Windows paths; the README says to
//! run the suite inside WSL there).
//!
//! `SHELLPP_OPENSSH_SERVER_JSON=<path>` reuses a server that
//! `tests/openssh/start.sh` already started (any OS): the suite reads that
//! `server.json` and leaves the container running.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::json::{self, Json};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider {
    Russh,
    OpenSsh,
}

/// The provider chosen by `SHELLPP_TEST_SSHD`.
pub fn provider() -> Provider {
    match std::env::var("SHELLPP_TEST_SSHD").as_deref() {
        Err(_) | Ok("") | Ok("russh") => Provider::Russh,
        Ok("openssh") => Provider::OpenSsh,
        Ok(other) => panic!("SHELLPP_TEST_SSHD must be russh or openssh, not {other:?}"),
    }
}

fn openssh_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("openssh")
}

/// A running OpenSSH container. One started here is removed on drop.
pub struct OpenSshServer {
    /// The work directory of a container started by this suite.
    workdir: Option<tempfile::TempDir>,
    pub addr: SocketAddr,
    /// Host key types and their `SHA256:` fingerprints.
    pub host_keys: Vec<(String, String)>,
    pub password_user: (String, String),
    /// User name and the private key text (no passphrase).
    pub key_user: (String, String),
    pub ki_user: (String, String),
}

#[allow(clippy::print_stderr)]
fn skip(reason: &str) {
    eprintln!("skipped: OpenSSH provider: {reason}");
}

fn docker_available() -> bool {
    Command::new("docker")
        .arg("info")
        .output()
        .map(|output| output.status.success())
        .unwrap_or(false)
}

impl OpenSshServer {
    /// Starts the container (or reuses one, see the module documentation);
    /// `None` (with a printed reason) when Docker is not available. Any
    /// other failure panics: CI must not pass silently.
    pub fn start() -> Option<OpenSshServer> {
        if let Some(path) = std::env::var_os("SHELLPP_OPENSSH_SERVER_JSON") {
            return Some(Self::from_description(Path::new(&path), None));
        }
        if cfg!(windows) {
            skip("not run on Windows (run the suite inside WSL or on Linux)");
            return None;
        }
        if !docker_available() {
            skip("docker info failed (Docker is not running)");
            return None;
        }
        let workdir = tempfile::tempdir().expect("temp dir");
        let status = Command::new("bash")
            .arg(openssh_dir().join("start.sh"))
            .arg(workdir.path())
            .status();
        match status {
            Ok(status) if status.success() => {}
            Ok(status) if status.code() == Some(3) => {
                skip("start.sh reports that Docker is not available");
                return None;
            }
            Ok(status) => panic!("tests/openssh/start.sh failed: {status}"),
            Err(error) => {
                skip(&format!("bash is not available: {error}"));
                return None;
            }
        }
        let description = workdir.path().join("server.json");
        Some(Self::from_description(&description, Some(workdir)))
    }

    fn from_description(path: &Path, workdir: Option<tempfile::TempDir>) -> OpenSshServer {
        let text = std::fs::read_to_string(path).expect("server.json");
        let description = json::parse(&text).expect("server.json is JSON");
        let string = |keys: &[&str]| -> String {
            description
                .path(keys)
                .and_then(Json::str)
                .unwrap_or_else(|| panic!("server.json lacks {keys:?}"))
                .to_owned()
        };
        let host: std::net::IpAddr = string(&["host"]).parse().expect("host address");
        let port = description
            .get("port")
            .and_then(Json::number)
            .expect("port") as u16;
        let host_keys = description
            .get("host_keys")
            .map(|keys| {
                keys.entries()
                    .iter()
                    .filter_map(|(key_type, fp)| Some((key_type.clone(), fp.str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        let key_path = string(&["users", "publickey", "private_key_path"]);
        let private_key = std::fs::read_to_string(&key_path).expect("client key");
        OpenSshServer {
            addr: SocketAddr::new(host, port),
            host_keys,
            password_user: (
                string(&["users", "password", "name"]),
                string(&["users", "password", "password"]),
            ),
            key_user: (string(&["users", "publickey", "name"]), private_key),
            ki_user: (
                string(&["users", "keyboard_interactive", "name"]),
                string(&["users", "keyboard_interactive", "password"]),
            ),
            workdir,
        }
    }
}

impl Drop for OpenSshServer {
    fn drop(&mut self) {
        if let Some(workdir) = &self.workdir {
            let _ = Command::new("bash")
                .arg(openssh_dir().join("stop.sh"))
                .arg(workdir.path())
                .status();
        }
    }
}
