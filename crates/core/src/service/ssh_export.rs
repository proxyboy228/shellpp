//! Export of the SSH hosts as an OpenSSH client config and of the keys as
//! OpenSSH key files (SPEC principle 4; docs/storage-format.md, sections
//! 13.4 and 13.5; night plan, stage 3 item 11): into a folder the user
//! chooses, `shellpp-ssh-config` and, when asked, `shellpp_<key alias>`,
//! `.pub` and `-cert.pub` for every key.
//!
//! - Values are the effective ones after group inheritance and the host's
//!   identity; `IdentityFile`/`CertificateFile` point at the exported files
//!   (`~/.ssh/shellpp_<alias>`) and are comments when the keys are not
//!   exported; a jump host that is not exported makes `ProxyJump` a comment;
//!   proxies and passwords are never written.
//! - A private key that is not encrypted is encrypted with the passphrase
//!   given for the export, when one is given (UI 7.3 offers it); an
//!   encrypted one is written as it is. A saved passphrase is never written.
//! - Private key files get mode 0600 and the others 0644 on Unix; on
//!   Windows the files inherit the folder's access list (a profile folder
//!   allows only the user, SYSTEM and Administrators).
//! - Existing files are never replaced unless `overwrite` is set; without
//!   it the export lists them and writes nothing.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use ssh_key::{LineEnding, PrivateKey};
use unicode_normalization::UnicodeNormalization;
use uuid::Uuid;

use super::Core;
use super::records::parts;
use crate::error::{CoreError, FieldError, ValidationReason};
use crate::model::key;
use crate::model::time::format_timestamp;
use crate::model::types::RecordKind;

/// The config file name (docs/storage-format.md, section 13.1).
pub const CONFIG_FILE: &str = "shellpp-ssh-config";

/// Input of `ssh_files_export`.
#[derive(Clone, Deserialize)]
pub struct SshFilesExportInput {
    pub dir: PathBuf,
    /// Also write the keys as OpenSSH files.
    #[serde(default)]
    pub keys: bool,
    /// Encrypts the keys that are not encrypted (optional).
    #[serde(default)]
    pub passphrase: Option<SecretString>,
    /// Replace files that exist.
    #[serde(default)]
    pub overwrite: bool,
}

impl std::fmt::Debug for SshFilesExportInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SshFilesExportInput")
            .field("keys", &self.keys)
            .field("overwrite", &self.overwrite)
            .finish_non_exhaustive()
    }
}

/// Result of `ssh_files_export`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct SshFilesExportSummary {
    pub hosts: usize,
    /// Hosts that are not SSH hosts (Telnet only), left out.
    pub skipped_hosts: usize,
    pub keys: usize,
    /// The files written, in order.
    pub files: Vec<PathBuf>,
}

/// An alias of the config (docs/storage-format.md, section 13.4, rules 1–3).
pub fn alias_base(text: &str, record_id: &Uuid) -> String {
    let lower: String = text.nfc().collect::<String>().to_lowercase();
    let mut out = String::new();
    for c in lower.chars() {
        let c = if c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '.' | '_' | '-') {
            c
        } else {
            '-'
        };
        if c == '-' && out.ends_with('-') {
            continue;
        }
        out.push(c);
    }
    let trimmed = out.trim_matches(|c| c == '-' || c == '.').to_owned();
    if trimmed.is_empty() {
        let hex = record_id.simple().to_string();
        format!("host-{}", &hex[hex.len() - 8..])
    } else {
        trimmed
    }
}

/// Unique aliases in the given order (rule 4: `-2`, `-3`, ...).
fn unique(bases: Vec<(Uuid, String)>) -> HashMap<Uuid, String> {
    let mut used: HashMap<String, usize> = HashMap::new();
    let mut out = HashMap::new();
    for (id, base) in bases {
        let mut alias = base.clone();
        let mut n = 1;
        while used.contains_key(&alias) {
            n += 1;
            alias = format!("{base}-{n}");
        }
        used.insert(alias.clone(), 1);
        out.insert(id, alias);
    }
    out
}

/// A `SetEnv` value: double-quoted when it has spaces; `None` when it
/// holds a quote or a line break (then the line is a comment).
fn env_value(value: &str) -> Option<String> {
    if value.contains(['"', '\n', '\r']) {
        None
    } else if value.contains(char::is_whitespace) {
        Some(format!("\"{value}\""))
    } else {
        Some(value.to_owned())
    }
}

fn ssh_str<'a>(data: &'a Map<String, Value>, member: &str) -> Option<&'a str> {
    data.get("ssh")?
        .get(member)?
        .as_str()
        .filter(|text| !text.is_empty())
}

fn ssh_uuid(data: &Map<String, Value>, member: &str) -> Option<Uuid> {
    ssh_str(data, member).and_then(|raw| Uuid::parse_str(raw).ok())
}

fn write_file(path: &Path, text: &[u8], private: bool) -> std::io::Result<()> {
    std::fs::write(path, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        let mode = if private { 0o600 } else { 0o644 };
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))?;
    }
    #[cfg(not(unix))]
    let _ = private;
    Ok(())
}

impl Core {
    /// Writes `shellpp-ssh-config` (and the key files) into `input.dir`
    /// (docs/storage-format.md, 13.4–13.5). Errors: `dir` `invalid` (not a
    /// folder), `files` `duplicate` (existing files and no `overwrite`),
    /// `passphrase` `too_short` (under 8 characters).
    pub fn ssh_files_export(
        &self,
        input: SshFilesExportInput,
    ) -> Result<SshFilesExportSummary, CoreError> {
        if !input.dir.is_dir() {
            return Err(CoreError::Validation(vec![FieldError::new(
                "dir",
                ValidationReason::Invalid,
            )]));
        }
        let passphrase = input
            .passphrase
            .as_ref()
            .filter(|value| !value.expose_secret().is_empty());
        if let Some(passphrase) = passphrase
            && !crate::crypto::password_length_ok(passphrase)
        {
            return Err(CoreError::Validation(vec![FieldError::new(
                "passphrase",
                ValidationReason::TooShort,
            )]));
        }
        let now = self.now();
        let mut summary = SshFilesExportSummary::default();
        // (file name, content, private) of every file, built under the lock.
        let mut files: Vec<(String, zeroize::Zeroizing<Vec<u8>>, bool)> = Vec::new();
        {
            let mut guard = self.inner();
            let parts = parts(&mut guard)?;
            let index = &parts.session.index;
            let tree = index.tree();
            let group_name = |id: &Uuid| -> String {
                index
                    .get_kind(id, RecordKind::Group)
                    .and_then(|record| record.data().get("name").and_then(Value::as_str))
                    .unwrap_or_default()
                    .to_owned()
            };
            // Keys and their aliases.
            let mut keys: Vec<(Uuid, String)> = index
                .of_kind(RecordKind::Key)
                .map(|record| (record.record_id, key::label(record.data()).to_owned()))
                .collect();
            keys.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
            let key_alias = unique(
                keys.iter()
                    .map(|(id, label)| (*id, alias_base(label, id)))
                    .collect(),
            );
            // Hosts in the order of rule 4: group path, label, record id.
            let mut hosts: Vec<(String, String, Uuid, &Map<String, Value>)> = Vec::new();
            for record in index.of_kind(RecordKind::Host) {
                let data = record.data();
                if data.get("primary_protocol").and_then(Value::as_str) == Some("telnet") {
                    summary.skipped_hosts += 1;
                    continue;
                }
                let group = crate::index::Index::host_group(&tree, data);
                let mut path: Vec<String> = Vec::new();
                if let Some(group) = group {
                    path = tree.path(&group).iter().map(&group_name).collect();
                    path.push(group_name(&group));
                }
                let label = data
                    .get("label")
                    .and_then(Value::as_str)
                    .filter(|label| !label.is_empty())
                    .or_else(|| data.get("address").and_then(Value::as_str))
                    .unwrap_or_default()
                    .to_owned();
                hosts.push((path.join(" / "), label, record.record_id, data));
            }
            hosts.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
            let host_alias = unique(
                hosts
                    .iter()
                    .map(|(_, label, id, _)| (*id, alias_base(label, id)))
                    .collect(),
            );

            let mut config = format!(
                "# Exported by Shell++ {} at {}\n",
                self.config.app_version,
                format_timestamp(now)
            );
            for (path, _, id, data) in &hosts {
                let inherited = index.inherited_for_host(&tree, data);
                let identity = ssh_uuid(data, "identity_id")
                    .or_else(|| inherited.identity_id.as_ref().map(|value| value.value))
                    .and_then(|id| index.get_kind(&id, RecordKind::Identity))
                    .map(|record| record.data());
                let username = ssh_str(data, "username")
                    .map(str::to_owned)
                    .or_else(|| {
                        identity
                            .and_then(|identity| identity.get("username"))
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                    })
                    .or_else(|| inherited.username.as_ref().map(|value| value.value.clone()));
                let key_id = ssh_uuid(data, "key_id")
                    .or_else(|| {
                        identity
                            .and_then(|identity| identity.get("key_id"))
                            .and_then(Value::as_str)
                            .and_then(|raw| Uuid::parse_str(raw).ok())
                    })
                    .or_else(|| inherited.key_id.as_ref().map(|value| value.value));
                let port = data
                    .get("ssh")
                    .and_then(|ssh| ssh.get("port"))
                    .and_then(Value::as_u64)
                    .or_else(|| inherited.port.as_ref().map(|value| u64::from(value.value)))
                    .unwrap_or(22);
                config.push('\n');
                if !path.is_empty() {
                    config.push_str(&format!("# Group: {path}\n"));
                }
                config.push_str(&format!("Host {}\n", host_alias[id]));
                if let Some(address) = data.get("address").and_then(Value::as_str) {
                    config.push_str(&format!("  HostName {address}\n"));
                }
                if port != 22 {
                    config.push_str(&format!("  Port {port}\n"));
                }
                if let Some(username) = username {
                    config.push_str(&format!("  User {username}\n"));
                }
                if let Some(key_id) = key_id
                    && let Some(alias) = key_alias.get(&key_id)
                {
                    let prefix = if input.keys { "" } else { "# " };
                    config.push_str(&format!("  {prefix}IdentityFile ~/.ssh/shellpp_{alias}\n"));
                    let has_certificate = index
                        .get(&key_id)
                        .and_then(|record| record.data().get("certificate"))
                        .is_some_and(|value| value.is_string());
                    if has_certificate {
                        config.push_str(&format!(
                            "  {prefix}CertificateFile ~/.ssh/shellpp_{alias}-cert.pub\n"
                        ));
                    }
                }
                let chain: Vec<Uuid> = data
                    .get("ssh")
                    .and_then(|ssh| ssh.get("jump_chain"))
                    .and_then(Value::as_array)
                    .map(|chain| {
                        chain
                            .iter()
                            .filter_map(Value::as_str)
                            .filter_map(|raw| Uuid::parse_str(raw).ok())
                            .collect()
                    })
                    .or_else(|| {
                        inherited
                            .jump_chain
                            .as_ref()
                            .map(|value| value.value.clone())
                    })
                    .unwrap_or_default();
                if !chain.is_empty() {
                    let aliases: Vec<Option<&String>> =
                        chain.iter().map(|hop| host_alias.get(hop)).collect();
                    let all = aliases.iter().all(Option::is_some);
                    let names: Vec<&str> = aliases
                        .iter()
                        .map(|alias| alias.map_or("(not exported)", String::as_str))
                        .collect();
                    let prefix = if all { "" } else { "# " };
                    config.push_str(&format!("  {prefix}ProxyJump {}\n", names.join(",")));
                }
                if data
                    .get("ssh")
                    .and_then(|ssh| ssh.get("agent_forwarding"))
                    .and_then(Value::as_bool)
                    == Some(true)
                {
                    config.push_str("  ForwardAgent yes\n");
                }
                let keepalive = data
                    .get("ssh")
                    .and_then(|ssh| ssh.get("keepalive_s"))
                    .and_then(Value::as_u64)
                    .or_else(|| {
                        crate::index::Index::host_group(&tree, data).and_then(|group| {
                            tree.chain(group, usize::MAX).into_iter().find_map(|id| {
                                index
                                    .get_kind(&id, RecordKind::Group)
                                    .and_then(|record| record.data().get("ssh"))
                                    .and_then(|ssh| ssh.get("keepalive_s"))
                                    .and_then(Value::as_u64)
                            })
                        })
                    });
                if let Some(keepalive) = keepalive {
                    config.push_str(&format!("  ServerAliveInterval {keepalive}\n"));
                }
                for pair in data
                    .get("ssh")
                    .and_then(|ssh| ssh.get("env"))
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                {
                    let name = pair.get("name").and_then(Value::as_str).unwrap_or_default();
                    let value = pair
                        .get("value")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    match env_value(value) {
                        Some(value) => config.push_str(&format!("  SetEnv {name}={value}\n")),
                        None => config.push_str(&format!(
                            "  # SetEnv {name}: the value holds a quote or a line break\n"
                        )),
                    }
                }
                if data
                    .get("ssh")
                    .and_then(|ssh| ssh.get("proxy"))
                    .is_some_and(Value::is_object)
                {
                    config
                        .push_str("  # Proxy: OpenSSH has no option for an HTTP or SOCKS proxy\n");
                }
                summary.hosts += 1;
            }
            files.push((
                CONFIG_FILE.to_owned(),
                zeroize::Zeroizing::new(config.into_bytes()),
                false,
            ));

            if input.keys {
                for (id, label) in &keys {
                    let Some(record) = index.get_kind(id, RecordKind::Key) else {
                        continue;
                    };
                    let data = record.data();
                    let alias = &key_alias[id];
                    let Some(text) = key::private_key(data) else {
                        continue;
                    };
                    let Ok(private) = PrivateKey::from_openssh(text.as_bytes()) else {
                        continue;
                    };
                    let private_text = match passphrase.filter(|_| !private.is_encrypted()) {
                        Some(passphrase) => private
                            .encrypt(
                                &mut getrandom::SysRng,
                                passphrase.expose_secret().as_bytes(),
                            )
                            .and_then(|encrypted| encrypted.to_openssh(LineEnding::LF))
                            .map_err(|_| CoreError::Internal("key encryption"))?,
                        None => zeroize::Zeroizing::new(text.to_owned()),
                    };
                    files.push((
                        format!("shellpp_{alias}"),
                        zeroize::Zeroizing::new(private_text.as_bytes().to_vec()),
                        true,
                    ));
                    let comment: String = label.chars().filter(|c| !c.is_control()).collect();
                    let public = key::public_key(data)
                        .map(|line| {
                            let mut parts = line.split_whitespace();
                            let algorithm = parts.next().unwrap_or_default();
                            let blob = parts.next().unwrap_or_default();
                            format!("{algorithm} {blob} {comment}\n")
                        })
                        .unwrap_or_default();
                    files.push((
                        format!("shellpp_{alias}.pub"),
                        zeroize::Zeroizing::new(public.into_bytes()),
                        false,
                    ));
                    if let Some(certificate) = data.get("certificate").and_then(Value::as_str) {
                        files.push((
                            format!("shellpp_{alias}-cert.pub"),
                            zeroize::Zeroizing::new(format!("{certificate}\n").into_bytes()),
                            false,
                        ));
                    }
                    summary.keys += 1;
                }
            }
        }
        let existing: Vec<String> = files
            .iter()
            .filter(|(name, _, _)| input.dir.join(name).exists())
            .map(|(name, _, _)| name.clone())
            .collect();
        if !existing.is_empty() && !input.overwrite {
            return Err(CoreError::Validation(vec![FieldError::new(
                "files",
                ValidationReason::Duplicate,
            )]));
        }
        for (name, content, private) in &files {
            let path = input.dir.join(name);
            write_file(&path, content, *private)
                .map_err(|_| CoreError::Internal("export write"))?;
            summary.files.push(path);
        }
        tracing::info!(
            hosts = summary.hosts,
            keys = summary.keys,
            "ssh files exported"
        );
        Ok(summary)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aliases_follow_the_rules() {
        let id = Uuid::parse_str("0199f6a1-b2c4-7a10-9b2e-5f6a7b8c9d0e").unwrap();
        assert_eq!(alias_base("Web 1 (prod)", &id), "web-1-prod");
        assert_eq!(alias_base("..db..", &id), "db");
        assert_eq!(alias_base("Ünïcode", &id), "n-code");
        assert_eq!(alias_base("***", &id), "host-7b8c9d0e");
        let aliases = unique(vec![
            (Uuid::from_u128(1), "web".into()),
            (Uuid::from_u128(2), "web".into()),
            (Uuid::from_u128(3), "web".into()),
        ]);
        assert_eq!(aliases[&Uuid::from_u128(2)], "web-2");
        assert_eq!(aliases[&Uuid::from_u128(3)], "web-3");
        assert_eq!(env_value("a b").as_deref(), Some("\"a b\""));
        assert_eq!(env_value("x\"y"), None);
    }
}
