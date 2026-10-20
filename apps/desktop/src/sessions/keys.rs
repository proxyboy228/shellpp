//! Keys of SSH sessions besides the host's own key record (lead decision
//! 4): the user's key files `~/.ssh/id_*` and the system agent
//! (`SSH_AUTH_SOCK`, the Windows OpenSSH agent pipe, Pageant). Both sources
//! are injectable ([`KeySources`]): production uses [`KeySources::system`],
//! tests a temporary directory with keys generated at run time and their own
//! agent, so a test never reads the real `~/.ssh` or talks to the real agent.
//!
//! A key file's passphrase is asked only when the server accepted the key
//! (`shellpp-ssh`); "Remember for this session" keeps it in memory in
//! [`PassphraseCache`] until the app locks or quits. Nothing here logs paths
//! or key material.

use std::collections::HashMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;

use secrecy::SecretString;
use shellpp_ssh::{AgentLocation, FileKey, SystemAgent};
use zeroize::Zeroizing;

/// Most bytes read from one key file (private keys are a few KiB).
pub const MAX_KEY_FILE_BYTES: u64 = 64 * 1024;
/// Most key files offered per connection (each refused key counts as one
/// authentication attempt on the server).
pub const MAX_KEY_FILES: usize = 8;
/// How long agent discovery may take before an attempt goes on without it.
const AGENT_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(3);

/// Where SSH sessions find key files and the agent.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeySources {
    /// The directory with the `id_*` key files; `None` = no key files.
    pub key_dir: Option<PathBuf>,
    pub agent: AgentSource,
}

impl KeySources {
    /// Production: `~/.ssh` and the system agent.
    pub fn system() -> Self {
        Self {
            key_dir: dirs::home_dir().map(|home| home.join(".ssh")),
            agent: AgentSource::System,
        }
    }

    /// Neither key files nor an agent.
    pub fn none() -> Self {
        Self {
            key_dir: None,
            agent: AgentSource::None,
        }
    }
}

/// Which agent sessions use.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AgentSource {
    /// The first system agent that answers.
    System,
    /// An agent at this location (tests start their own).
    At(AgentLocation),
    None,
}

impl AgentSource {
    /// The agent for one connection attempt, if one is configured and answers.
    pub(crate) async fn resolve(&self) -> Option<SystemAgent> {
        match self {
            Self::System => tokio::time::timeout(AGENT_DISCOVERY_TIMEOUT, SystemAgent::discover())
                .await
                .ok()
                .flatten(),
            Self::At(location) => Some(SystemAgent::at(location.clone())),
            Self::None => None,
        }
    }
}

/// Rank of a key file name: the common modern keys first, then the others
/// by name.
fn rank(name: &str) -> u8 {
    match name {
        "id_ed25519" => 0,
        "id_ecdsa" => 1,
        "id_rsa" => 2,
        _ => 3,
    }
}

/// Whether `name` is a private key file name: `id_*`, not a public key or
/// certificate, not a security key (`*_sk`, which needs hardware).
fn is_key_file_name(name: &str) -> bool {
    name.starts_with("id_")
        && name.len() > 3
        && !name.ends_with(".pub")
        && !name.ends_with("_sk")
        && !name.contains(char::is_whitespace)
}

/// Reads a file of at most [`MAX_KEY_FILE_BYTES`] as UTF-8.
fn read_small(path: &Path) -> Option<Zeroizing<String>> {
    let file = std::fs::File::open(path).ok()?;
    let metadata = file.metadata().ok()?;
    if !metadata.is_file() || metadata.len() > MAX_KEY_FILE_BYTES {
        return None;
    }
    let bytes = read_sized(file, metadata.len())?;
    std::str::from_utf8(&bytes)
        .ok()
        .map(|text| Zeroizing::new(text.to_owned()))
}

/// Reads `len` bytes into a buffer allocated once at its final size: a
/// buffer that grows leaves unwiped copies of the key text behind in freed
/// memory (docs/security.md, section 9.5). The extra byte lets the final
/// read see the end of the file without growing; a file that grew since
/// `len` was taken is refused.
fn read_sized(file: impl std::io::Read, len: u64) -> Option<Zeroizing<Vec<u8>>> {
    let capacity = usize::try_from(len).ok()?.checked_add(1)?;
    let mut bytes = Zeroizing::new(Vec::with_capacity(capacity));
    file.take(len + 1).read_to_end(&mut bytes).ok()?;
    (bytes.len() as u64 <= len).then_some(bytes)
}

/// The key files of `dir` (blocking file reads). Unreadable files, files
/// that are not private keys and anything above [`MAX_KEY_FILES`] are
/// skipped.
pub fn load_key_files(dir: &Path) -> Vec<FileKey> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| is_key_file_name(name))
        .collect();
    names.sort_by(|a, b| rank(a).cmp(&rank(b)).then_with(|| a.cmp(b)));
    let mut keys = Vec::new();
    for name in names {
        if keys.len() >= MAX_KEY_FILES {
            break;
        }
        let Some(text) = read_small(&dir.join(&name)) else {
            continue;
        };
        if !text.contains("PRIVATE KEY") && !text.starts_with("PuTTY-User-Key-File") {
            continue;
        }
        let public_key = read_small(&dir.join(format!("{name}.pub")))
            .map(|line| line.trim().to_owned())
            .filter(|line| !line.is_empty());
        keys.push(FileKey {
            label: name,
            private_key: SecretString::from(text.as_str()),
            public_key,
            passphrase: None,
        });
    }
    tracing::debug!(keys = keys.len(), "key files loaded");
    keys
}

/// Key file passphrases remembered with "Remember for this session", by
/// key file name. Wiped when the app locks and when it quits.
#[derive(Default)]
pub struct PassphraseCache {
    entries: Mutex<HashMap<String, SecretString>>,
}

impl std::fmt::Debug for PassphraseCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PassphraseCache")
            .field("entries", &self.lock().len())
            .finish()
    }
}

impl PassphraseCache {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, SecretString>> {
        self.entries
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn get(&self, label: &str) -> Option<SecretString> {
        self.lock().get(label).cloned()
    }

    pub fn remember(&self, label: &str, passphrase: SecretString) {
        self.lock().insert(label.to_owned(), passphrase);
    }

    pub fn clear(&self) {
        self.lock().clear();
    }

    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret as _;

    use super::*;

    #[test]
    fn key_file_names() {
        assert!(is_key_file_name("id_ed25519"));
        assert!(is_key_file_name("id_rsa"));
        assert!(is_key_file_name("id_work"));
        assert!(!is_key_file_name("id_ed25519.pub"));
        assert!(!is_key_file_name("id_ed25519-cert.pub"));
        assert!(!is_key_file_name("id_ed25519_sk"));
        assert!(!is_key_file_name("id_"));
        assert!(!is_key_file_name("known_hosts"));
        assert!(!is_key_file_name("config"));
        assert!(!is_key_file_name("authorized_keys"));
    }

    #[test]
    fn key_files_are_read_in_order_with_their_public_keys() {
        let dir = tempfile::tempdir().expect("temp dir");
        let key = |name: &str| {
            format!(
                "-----BEGIN OPENSSH PRIVATE KEY-----\n{name}\n-----END OPENSSH PRIVATE KEY-----\n"
            )
        };
        std::fs::write(dir.path().join("id_rsa"), key("rsa")).unwrap();
        std::fs::write(dir.path().join("id_work"), key("work")).unwrap();
        std::fs::write(dir.path().join("id_ed25519"), key("ed")).unwrap();
        std::fs::write(dir.path().join("id_ed25519.pub"), "ssh-ed25519 AAAA test\n").unwrap();
        std::fs::write(dir.path().join("id_notes"), "not a key").unwrap();
        std::fs::write(dir.path().join("config"), "Host *").unwrap();
        std::fs::write(
            dir.path().join("id_huge"),
            "x".repeat(MAX_KEY_FILE_BYTES as usize + 1),
        )
        .unwrap();
        let keys = load_key_files(dir.path());
        let labels: Vec<&str> = keys.iter().map(|key| key.label.as_str()).collect();
        assert_eq!(labels, vec!["id_ed25519", "id_rsa", "id_work"]);
        assert_eq!(keys[0].public_key.as_deref(), Some("ssh-ed25519 AAAA test"));
        assert!(keys[1].public_key.is_none());
        assert!(keys[0].private_key.expose_secret().contains("ed"));
        assert!(load_key_files(&dir.path().join("missing")).is_empty());
    }

    /// The buffer is allocated once: no reallocation leaves a copy of the
    /// key text behind; a file longer than announced is refused.
    #[test]
    fn key_files_are_read_into_one_allocation() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("id_ed25519");
        let text = "k".repeat(3300);
        std::fs::write(&path, &text).unwrap();
        let file = std::fs::File::open(&path).unwrap();
        let bytes = read_sized(file, 3300).expect("read");
        assert_eq!(bytes.len(), 3300);
        assert_eq!(bytes.capacity(), 3301, "allocated once, never grown");
        assert_eq!(&bytes[..], text.as_bytes());

        let file = std::fs::File::open(&path).unwrap();
        assert!(read_sized(file, 3000).is_none(), "the file grew");
        let small = dir.path().join("id_small");
        std::fs::write(&small, "abc").unwrap();
        let bytes = read_sized(std::fs::File::open(&small).unwrap(), 3).expect("read");
        assert_eq!(bytes.capacity(), 4);
        assert_eq!(
            read_small(&small).as_deref().map(String::as_str),
            Some("abc")
        );
    }

    #[test]
    fn the_cache_remembers_until_cleared() {
        let cache = PassphraseCache::default();
        assert!(cache.get("id_rsa").is_none());
        cache.remember("id_rsa", SecretString::from("p".to_owned()));
        assert_eq!(cache.get("id_rsa").unwrap().expose_secret(), "p");
        assert_eq!(cache.len(), 1);
        assert_eq!(format!("{cache:?}"), "PassphraseCache { entries: 1 }");
        cache.clear();
        assert!(cache.is_empty());
    }
}
