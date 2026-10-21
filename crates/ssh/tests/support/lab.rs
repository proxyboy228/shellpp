//! The OpenSSH lab of stage 5 (`tests/openssh/lab.sh`): jump1 on loopback,
//! jump2, target and app1-app3 only inside an internal Docker network. Run
//! with `SHELLPP_TEST_SSHD=openssh`; without Docker (or with another
//! provider) the lab tests print why they are skipped.
//! `SHELLPP_OPENSSH_LAB_JSON=<path>` reuses a lab that `lab.sh up` started
//! and leaves it running.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::json::{self, Json};
use super::openssh::{Provider, provider};

/// A running lab. One started here is removed on drop.
pub struct Lab {
    workdir: Option<tempfile::TempDir>,
    /// jump1 as this machine reaches it.
    pub jump1: SocketAddr,
    /// Host keys (`type` -> `SHA256:` fingerprint) by server name.
    pub host_keys: Vec<(String, Vec<(String, String)>)>,
    pub password_user: (String, String),
    /// User name and the private key text.
    pub key_user: (String, String),
    /// What `target`'s HTTP service on port 8080 answers.
    pub http_body: String,
    /// tinyproxy (HTTP CONNECT) and microsocks (SOCKS5) on loopback, with
    /// their user and password.
    pub http_proxy: SocketAddr,
    pub socks_proxy: SocketAddr,
    pub proxy_user: (String, String),
}

#[allow(clippy::print_stderr)]
fn skip(reason: &str) {
    eprintln!("skipped: OpenSSH lab: {reason}");
}

fn lab_script() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("tests")
        .join("openssh")
        .join("lab.sh")
}

impl Lab {
    /// Starts the lab (or reuses one); `None` with a printed reason when the
    /// provider is not OpenSSH or Docker is not available.
    pub fn start() -> Option<Lab> {
        if let Some(path) = std::env::var_os("SHELLPP_OPENSSH_LAB_JSON") {
            return Some(Self::from_description(Path::new(&path), None));
        }
        if provider() != Provider::OpenSsh {
            skip("SHELLPP_TEST_SSHD is not openssh");
            return None;
        }
        if cfg!(windows) {
            skip("not run on Windows (run the suite inside WSL or on Linux)");
            return None;
        }
        let workdir = tempfile::tempdir().expect("temp dir");
        let status = Command::new("bash")
            .arg(lab_script())
            .arg("up")
            .arg(workdir.path())
            .stdout(std::process::Stdio::null())
            .status();
        match status {
            Ok(status) if status.success() => {}
            Ok(status) if status.code() == Some(3) => {
                skip("lab.sh reports that Docker is not available");
                return None;
            }
            Ok(status) => panic!("tests/openssh/lab.sh up failed: {status}"),
            Err(error) => {
                skip(&format!("bash is not available: {error}"));
                return None;
            }
        }
        let description = workdir.path().join("lab.json");
        Some(Self::from_description(&description, Some(workdir)))
    }

    fn from_description(path: &Path, workdir: Option<tempfile::TempDir>) -> Lab {
        let text = std::fs::read_to_string(path).expect("lab.json");
        let description = json::parse(&text).expect("lab.json is JSON");
        let string = |keys: &[&str]| -> String {
            description
                .path(keys)
                .and_then(Json::str)
                .unwrap_or_else(|| panic!("lab.json lacks {keys:?}"))
                .to_owned()
        };
        let host: std::net::IpAddr = string(&["host"]).parse().expect("host address");
        let port = description
            .get("port")
            .and_then(Json::number)
            .expect("port") as u16;
        let host_keys = description
            .get("host_keys")
            .map(|servers| {
                servers
                    .entries()
                    .iter()
                    .map(|(server, keys)| {
                        let keys = keys
                            .entries()
                            .iter()
                            .filter_map(|(key_type, fp)| {
                                Some((key_type.clone(), fp.str()?.to_owned()))
                            })
                            .collect();
                        (server.clone(), keys)
                    })
                    .collect()
            })
            .unwrap_or_default();
        let key_path = string(&["users", "publickey", "private_key_path"]);
        let proxy_port = |key: &str| {
            description
                .path(&["proxy", key])
                .and_then(Json::number)
                .unwrap_or_else(|| panic!("lab.json lacks proxy.{key}")) as u16
        };
        Lab {
            http_proxy: SocketAddr::new(host, proxy_port("http_port")),
            socks_proxy: SocketAddr::new(host, proxy_port("socks_port")),
            proxy_user: (string(&["proxy", "user"]), string(&["proxy", "password"])),
            jump1: SocketAddr::new(host, port),
            host_keys,
            password_user: (
                string(&["users", "password", "name"]),
                string(&["users", "password", "password"]),
            ),
            key_user: (
                string(&["users", "publickey", "name"]),
                std::fs::read_to_string(key_path).expect("client key"),
            ),
            http_body: string(&["http_body"]),
            workdir,
        }
    }

    /// The `SHA256:` fingerprints of `server`'s host keys.
    pub fn fingerprints(&self, server: &str) -> Vec<String> {
        self.host_keys
            .iter()
            .find(|(name, _)| name == server)
            .map(|(_, keys)| keys.iter().map(|(_, fp)| fp.clone()).collect())
            .unwrap_or_default()
    }

    /// The container of `server` (for `docker exec`).
    pub fn container(&self, server: &str) -> String {
        let dir = self.dir();
        let names = std::fs::read_to_string(dir.join("containers")).expect("containers");
        names
            .lines()
            .find(|name| name.starts_with(&format!("shellpp-lab-{server}-")))
            .unwrap_or_else(|| panic!("no container for {server}"))
            .to_owned()
    }

    fn dir(&self) -> PathBuf {
        match &self.workdir {
            Some(workdir) => workdir.path().to_path_buf(),
            None => {
                PathBuf::from(std::env::var_os("SHELLPP_OPENSSH_LAB_JSON").expect("lab.json path"))
                    .parent()
                    .map(Path::to_path_buf)
                    .unwrap_or_default()
            }
        }
    }

    /// Ends every SSH connection of `server` (the server keeps accepting).
    pub fn drop_connections(&self, server: &str) {
        let status = Command::new("docker")
            .args([
                "exec",
                &self.container(server),
                "shellpp-openssh-control",
                "drop-connections",
            ])
            .stdout(std::process::Stdio::null())
            .status()
            .expect("docker exec");
        assert!(status.success(), "drop-connections on {server}: {status}");
    }
}

impl Drop for Lab {
    fn drop(&mut self) {
        if let Some(workdir) = &self.workdir {
            let _ = Command::new("bash")
                .arg(lab_script())
                .arg("down")
                .arg(workdir.path())
                .status();
        }
    }
}
