//! Server configuration: the Rust form and the JSON file of `serve --config`
//! (stage 2 plan, section 4.1). Field names are the JSON names.

use std::net::SocketAddr;

use secrecy::SecretString;
use serde::Deserialize;

/// Longest accepted `banner_comment` (the whole identification line must
/// stay under 255 bytes, RFC 4253 section 4.2).
const MAX_BANNER_COMMENT: usize = 200;

/// Where the host key comes from.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostKeySource {
    /// A new key of this type generated at start (and again on rotation).
    Generate(crate::keygen::KeyType),
    /// An OpenSSH private key (unencrypted) read from this file. Rotation
    /// generates a new key of the same type.
    File(std::path::PathBuf),
}

impl Default for HostKeySource {
    fn default() -> Self {
        Self::Generate(crate::keygen::KeyType::Ed25519)
    }
}

/// One keyboard-interactive question; all questions of a user are asked in
/// one round, in order, and every answer must match.
#[derive(Clone, Debug, Deserialize)]
pub struct KiQuestion {
    pub prompt: String,
    /// `false` = the client must mask the answer.
    #[serde(default)]
    pub echo: bool,
    pub answer: SecretString,
}

impl KiQuestion {
    pub fn new(prompt: &str, echo: bool, answer: SecretString) -> Self {
        Self {
            prompt: prompt.to_owned(),
            echo,
            answer,
        }
    }
}

/// One account of the test server.
#[derive(Clone, Debug, Deserialize)]
pub struct UserConfig {
    pub name: String,
    /// Password authentication when set.
    #[serde(default)]
    pub password: Option<SecretString>,
    /// Public key authentication: OpenSSH public key lines.
    #[serde(default)]
    pub authorized_keys: Vec<String>,
    /// keyboard-interactive authentication when non-empty.
    #[serde(default)]
    pub keyboard_interactive: Vec<KiQuestion>,
    /// keyboard-interactive name and instruction texts (server text, UI 4.8).
    #[serde(default)]
    pub ki_name: String,
    #[serde(default)]
    pub ki_instruction: String,
}

impl UserConfig {
    /// A user without any authentication method yet.
    pub fn new(name: &str) -> Self {
        Self {
            name: name.to_owned(),
            password: None,
            authorized_keys: Vec::new(),
            keyboard_interactive: Vec::new(),
            ki_name: String::new(),
            ki_instruction: String::new(),
        }
    }

    /// Enables password authentication with `password`.
    pub fn with_password(mut self, password: SecretString) -> Self {
        self.password = Some(password);
        self
    }

    /// Adds an OpenSSH public key line to `authorized_keys`.
    pub fn with_authorized_key(mut self, public_openssh: &str) -> Self {
        self.authorized_keys.push(public_openssh.to_owned());
        self
    }

    /// Enables keyboard-interactive authentication with these questions,
    /// name and instruction.
    pub fn with_keyboard_interactive(
        mut self,
        name: &str,
        instruction: &str,
        questions: Vec<KiQuestion>,
    ) -> Self {
        self.ki_name = name.to_owned();
        self.ki_instruction = instruction.to_owned();
        self.keyboard_interactive = questions;
        self
    }
}

/// What an `exec` request prints (OS detection, UI 12.4). The command text
/// is recorded in the events; the answer does not depend on it.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct ExecAnswer {
    /// Written to stdout, for example `"Linux\nID=debian\nVERSION_ID=12\n"`.
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub exit_status: u32,
}

/// The whole server configuration.
#[derive(Clone, Debug, Deserialize)]
pub struct ServerConfig {
    /// Default `127.0.0.1:0` (a free port). Only loopback addresses are accepted.
    #[serde(default = "default_bind")]
    pub bind: SocketAddr,
    #[serde(default)]
    pub host_key: HostKeySource,
    pub users: Vec<UserConfig>,
    /// Comment part of the SSH identification string, after
    /// `SSH-2.0-shellpp_test_sshd`; for example `"Ubuntu-3ubuntu13"` makes the
    /// client detect Ubuntu from the banner (UI 12.4, source `banner`).
    #[serde(default)]
    pub banner_comment: Option<String>,
    /// Answer of every exec request. `None` refuses exec requests.
    #[serde(default)]
    pub exec: Option<ExecAnswer>,
    /// Text of the SSH authentication banner, if any.
    #[serde(default)]
    pub auth_banner: Option<String>,
    /// MOTD printed when the shell starts (default `"Welcome to shellpp-test-sshd\r\n"`).
    #[serde(default)]
    pub motd: Option<String>,
    /// Port forwarding (stage 5): `direct-tcpip` channels to loopback
    /// destinations and `tcpip-forward` on loopback addresses. Off, both
    /// are refused (OpenSSH `AllowTcpForwarding no`).
    #[serde(default)]
    pub forwarding: bool,
    /// Names env requests may set (OpenSSH `AcceptEnv`; a trailing `*`
    /// matches a prefix); other names are refused. `None` accepts every name.
    #[serde(default)]
    pub accept_env: Option<Vec<String>>,
}

fn default_bind() -> SocketAddr {
    SocketAddr::from(([127, 0, 0, 1], 0))
}

impl ServerConfig {
    /// A server on a free loopback port with a generated Ed25519 host key.
    pub fn new(users: Vec<UserConfig>) -> Self {
        Self {
            bind: default_bind(),
            host_key: HostKeySource::default(),
            users,
            banner_comment: None,
            exec: None,
            auth_banner: None,
            motd: None,
            forwarding: false,
            accept_env: None,
        }
    }

    /// Parses the JSON file of `serve --config`. The error names the
    /// position only: serde messages may quote values (passwords, answers).
    pub fn from_json(text: &str) -> Result<Self, crate::Error> {
        serde_json::from_str(text).map_err(|error| {
            let category = match error.classify() {
                serde_json::error::Category::Io => "i/o error",
                serde_json::error::Category::Syntax => "syntax error",
                serde_json::error::Category::Data => "unexpected or missing value",
                serde_json::error::Category::Eof => "unexpected end",
            };
            crate::Error::Config(format!(
                "JSON {category} at line {} column {}",
                error.line(),
                error.column()
            ))
        })
    }

    /// Checks what [`crate::TestServer::start`] relies on. Messages name
    /// fields and indexes only, never user names or secrets.
    pub(crate) fn validate(&self) -> Result<(), crate::Error> {
        if !self.bind.ip().is_loopback() {
            return Err(config_error("bind must be a loopback address"));
        }
        if let Some(comment) = &self.banner_comment {
            let printable = comment.bytes().all(|byte| (0x20..0x7f).contains(&byte));
            if comment.is_empty() || comment.len() > MAX_BANNER_COMMENT || !printable {
                return Err(config_error(
                    "banner_comment must be 1 to 200 printable ASCII characters",
                ));
            }
        }
        for (index, user) in self.users.iter().enumerate() {
            if user.name.is_empty() || user.name.chars().any(char::is_control) {
                return Err(config_error(&format!("users[{index}].name is invalid")));
            }
            if self.users[..index]
                .iter()
                .any(|other| other.name == user.name)
            {
                return Err(config_error(&format!("users[{index}].name is a duplicate")));
            }
        }
        Ok(())
    }
}

fn config_error(message: &str) -> crate::Error {
    crate::Error::Config(message.to_owned())
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret;

    use super::*;
    use crate::keygen::KeyType;

    #[test]
    fn json_with_every_field() {
        let text = r#"{
            "bind": "127.0.0.1:0",
            "host_key": {"generate": "ecdsa-p256"},
            "users": [{
                "name": "alice",
                "password": "pw-from-test",
                "authorized_keys": ["ssh-ed25519 AAAA test"],
                "keyboard_interactive": [{"prompt": "Code: ", "echo": false, "answer": "a1"}],
                "ki_name": "Server",
                "ki_instruction": "Answer"
            }],
            "banner_comment": "Ubuntu-3ubuntu13",
            "exec": {"stdout": "Linux\nID=debian\n"},
            "auth_banner": "Authorized use only",
            "motd": "hi\r\n"
        }"#;
        let config = ServerConfig::from_json(text).expect("valid json");
        assert!(matches!(
            config.host_key,
            HostKeySource::Generate(KeyType::EcdsaP256)
        ));
        let user = &config.users[0];
        assert_eq!(user.name, "alice");
        assert_eq!(
            user.password.as_ref().map(|p| p.expose_secret().to_owned()),
            Some("pw-from-test".to_owned())
        );
        assert_eq!(user.keyboard_interactive[0].prompt, "Code: ");
        assert!(!user.keyboard_interactive[0].echo);
        assert_eq!(user.ki_name, "Server");
        assert_eq!(config.banner_comment.as_deref(), Some("Ubuntu-3ubuntu13"));
        let exec = config.exec.expect("exec");
        assert_eq!(exec.stdout, "Linux\nID=debian\n");
        assert_eq!(exec.exit_status, 0);
        assert_eq!(config.motd.as_deref(), Some("hi\r\n"));
        config_valid(text);
    }

    fn config_valid(text: &str) {
        ServerConfig::from_json(text)
            .expect("valid json")
            .validate()
            .expect("valid config");
    }

    #[test]
    fn json_defaults() {
        let config = ServerConfig::from_json(r#"{"users": [{"name": "bob"}]}"#).expect("json");
        assert_eq!(config.bind, default_bind());
        assert!(matches!(
            config.host_key,
            HostKeySource::Generate(KeyType::Ed25519)
        ));
        assert!(config.users[0].password.is_none());
        assert!(config.exec.is_none());
        let file =
            ServerConfig::from_json(r#"{"users": [], "host_key": {"file": "k"}}"#).expect("json");
        assert!(matches!(file.host_key, HostKeySource::File(_)));
        let rsa = ServerConfig::from_json(r#"{"users": [], "host_key": {"generate": "rsa-3072"}}"#)
            .expect("json");
        assert!(matches!(
            rsa.host_key,
            HostKeySource::Generate(KeyType::Rsa3072)
        ));
    }

    #[test]
    fn json_errors_do_not_echo_secrets() {
        let error = ServerConfig::from_json(r#"{"users": [{"name": "x", "password": 5731}]}"#)
            .expect_err("bad type");
        assert!(matches!(error, crate::Error::Config(_)));
        assert!(!error.to_string().contains("5731"));
        let error = ServerConfig::from_json(
            r#"{"users": [{"name": "x", "keyboard_interactive": [{"prompt": "p", "echo": "s3cr3t", "answer": "a"}]}]}"#,
        )
        .expect_err("bad type");
        assert!(!error.to_string().contains("s3cr3t"));
        assert!(
            ServerConfig::from_json(r#"{"users": [], "host_key": {"generate": "dsa"}}"#).is_err()
        );
    }

    #[test]
    fn validation_rules() {
        let mut config = ServerConfig::new(vec![UserConfig::new("a"), UserConfig::new("b")]);
        config.validate().expect("valid");

        config.bind = "192.0.2.1:22".parse().expect("addr");
        assert!(config.validate().is_err());
        config.bind = "[::1]:0".parse().expect("addr");
        config.validate().expect("ipv6 loopback is fine");

        config.banner_comment = Some("bad\r\nline".to_owned());
        assert!(config.validate().is_err());
        config.banner_comment = Some(String::new());
        assert!(config.validate().is_err());
        config.banner_comment = Some("Debian-5".to_owned());
        config.validate().expect("valid comment");

        config.users.push(UserConfig::new("a"));
        let error = config.validate().expect_err("duplicate");
        assert!(!error.to_string().contains("\"a\""));
        config.users.pop();
        config.users.push(UserConfig::new(""));
        assert!(config.validate().is_err());
    }
}
