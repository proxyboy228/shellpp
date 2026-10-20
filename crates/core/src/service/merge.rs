//! Merges of the Keychain (night plan, stage 3 item 7; UI 3.1, 7.2;
//! docs/data-model.md, sections 9.4 and 18.3):
//!
//! - The identity hint: hosts of the vault with the same effective username
//!   and the same saved password, or the same effective username and the
//!   same effective key, that use no identity yet. Accepting creates one
//!   identity (label = the username; the password or the key copied) and
//!   moves every one of these hosts onto it: `ssh.identity_id` is set,
//!   `ssh.username`, `ssh.password` and `ssh.key_id` are cleared. One
//!   transaction.
//! - Identical keys: live keys with the same SHA256 fingerprint. The one
//!   with the smallest record id stays and takes `certificate`,
//!   `passphrase` and `fido2` from the others where it has none (a
//!   passphrase only when it opens the kept key); every reference (the key
//!   slots of hosts and groups, `identity.key_id`) moves to it; the others
//!   are removed. One transaction.
//!
//! Nothing here logs usernames, passwords or keys.

use std::collections::HashMap;

use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use super::Core;
use super::dto::{KeyView, RecordRef};
use super::records::{Plan, check_vault, parts};
use crate::crypto::ct_eq;
use crate::error::CoreError;
use crate::keys;
use crate::model::identity::{self, PasswordChange};
use crate::model::types::RecordKind;
use crate::model::wipe::WipeOnDrop;
use crate::model::{key, text};

/// The key slots of hosts and groups (docs/data-model.md, section 7.3).
const KEY_SLOTS: [&str; 3] = ["key_id", "certificate_key_id", "fido2_key_id"];

/// What the hosts of an identity hint share.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityHintKind {
    Password,
    Key,
}

/// The hint of UI 3.1: N hosts use the same credentials.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct IdentityHint {
    pub username: String,
    pub kind: IdentityHintKind,
    /// The hosts, the asked one included (two or more), by record id.
    pub hosts: Vec<Uuid>,
}

/// The credentials of a host that a hint compares.
struct Credentials {
    id: Uuid,
    username: String,
    password: Option<String>,
    key: Option<Uuid>,
}

fn ssh_member<'a>(data: &'a Map<String, Value>, member: &str) -> Option<&'a Value> {
    data.get("ssh")
        .and_then(Value::as_object)
        .and_then(|ssh| ssh.get(member))
        .filter(|value| !value.is_null())
}

fn uuid_member(data: &Map<String, Value>, member: &str) -> Option<Uuid> {
    ssh_member(data, member)
        .and_then(Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
}

impl Core {
    /// The credentials of every host without an identity: effective username
    /// and key (inherited from groups), own saved password.
    fn host_credentials(session: &super::session::Session) -> Vec<Credentials> {
        let tree = session.index.tree();
        session
            .index
            .of_kind(RecordKind::Host)
            .filter_map(|record| {
                let data = record.data();
                let inherited = session.index.inherited_for_host(&tree, data);
                if uuid_member(data, "identity_id").is_some() || inherited.identity_id.is_some() {
                    return None;
                }
                let username = ssh_member(data, "username")
                    .and_then(Value::as_str)
                    .map(str::to_owned)
                    .or_else(|| inherited.username.map(|value| value.value))?;
                Some(Credentials {
                    id: record.record_id,
                    username,
                    password: ssh_member(data, "password")
                        .and_then(Value::as_str)
                        .filter(|password| !password.is_empty())
                        .map(str::to_owned),
                    key: uuid_member(data, "key_id")
                        .or_else(|| inherited.key_id.map(|value| value.value)),
                })
            })
            .collect()
    }

    fn hint_for(credentials: &[Credentials], host: Uuid) -> Option<IdentityHint> {
        let own = credentials.iter().find(|entry| entry.id == host)?;
        let group = |same: &dyn Fn(&Credentials) -> bool| -> Vec<Uuid> {
            let mut ids: Vec<Uuid> = credentials
                .iter()
                .filter(|entry| entry.username == own.username && same(entry))
                .map(|entry| entry.id)
                .collect();
            ids.sort();
            ids
        };
        if let Some(password) = &own.password {
            let hosts = group(&|entry: &Credentials| {
                entry
                    .password
                    .as_deref()
                    .is_some_and(|other| ct_eq(other.as_bytes(), password.as_bytes()))
            });
            if hosts.len() >= 2 {
                return Some(IdentityHint {
                    username: own.username.clone(),
                    kind: IdentityHintKind::Password,
                    hosts,
                });
            }
        }
        if let Some(key) = own.key {
            let hosts = group(&|entry: &Credentials| entry.key == Some(key));
            if hosts.len() >= 2 {
                return Some(IdentityHint {
                    username: own.username.clone(),
                    kind: IdentityHintKind::Key,
                    hosts,
                });
            }
        }
        None
    }

    /// The identity hint of `host` (UI 3.1), or `None` when fewer than two
    /// hosts share its credentials.
    pub fn identity_hint(&self, host: RecordRef) -> Result<Option<IdentityHint>, CoreError> {
        let mut guard = self.inner();
        let parts = parts(&mut guard)?;
        check_vault(parts.session, Some(host.vault_id))?;
        let credentials = Self::host_credentials(parts.session);
        Ok(Self::hint_for(&credentials, host.record_id))
    }

    /// Accepts the identity hint of `host`: one identity for every host of
    /// the hint (computed again now), the hosts moved onto it. Returns the
    /// new identity's record id. `not_found` when there is no hint any more.
    pub fn identity_from_hint(&self, host: RecordRef) -> Result<Uuid, CoreError> {
        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        check_vault(parts.session, Some(host.vault_id))?;
        let credentials = Self::host_credentials(parts.session);
        let hint = Self::hint_for(&credentials, host.record_id).ok_or(CoreError::NotFound)?;
        let own = credentials
            .iter()
            .find(|entry| entry.id == host.record_id)
            .ok_or(CoreError::NotFound)?;
        let label = text::string(&hint.username, identity::MAX_LABEL_CHARS)
            .ok()
            .flatten()
            .unwrap_or_else(|| {
                hint.username
                    .chars()
                    .take(identity::MAX_LABEL_CHARS)
                    .collect()
            });
        let mut input = Map::new();
        input.insert("label".into(), json!(label));
        input.insert("username".into(), json!(hint.username));
        let password = match hint.kind {
            IdentityHintKind::Password => match &own.password {
                Some(password) => PasswordChange::Set(SecretString::from(password.clone())),
                None => PasswordChange::Keep,
            },
            IdentityHintKind::Key => {
                if let Some(key) = own.key {
                    input.insert("key_id".into(), json!(key.to_string()));
                }
                PasswordChange::Keep
            }
        };
        let built = identity::build(&input, None, &password).map_err(CoreError::Validation)?;
        let identity_id = parts.ids.next(now);
        let mut plan = Plan::default();
        plan.set(RecordKind::Identity, identity_id, built);
        for id in &hint.hosts {
            let Some(record) = parts.session.index.get_kind(id, RecordKind::Host) else {
                continue;
            };
            let mut data = WipeOnDrop::new(record.data().clone());
            let ssh = data
                .entry("ssh")
                .or_insert_with(|| Value::Object(Map::new()));
            if let Value::Object(ssh) = ssh {
                if let Some(mut old) = ssh.remove("password") {
                    crate::model::wipe::wipe_value(&mut old);
                }
                ssh.remove("username");
                ssh.remove("key_id");
                ssh.insert("identity_id".into(), json!(identity_id.to_string()));
            }
            plan.set(RecordKind::Host, *id, data.into_inner());
        }
        let writes = plan.prepare(&mut parts, now)?;
        self.commit_records(&mut parts, writes, Vec::new(), Vec::new())?;
        tracing::info!(hosts = hint.hosts.len(), "identity created from the hint");
        Ok(identity_id)
    }

    /// Groups of live keys with the same fingerprint (two or more each), by
    /// record id (UI 7.2).
    pub fn key_duplicates(&self) -> Result<Vec<Vec<Uuid>>, CoreError> {
        let mut guard = self.inner();
        let parts = parts(&mut guard)?;
        let mut by_print: HashMap<String, Vec<Uuid>> = HashMap::new();
        for record in parts.session.index.of_kind(RecordKind::Key) {
            if let Some(print) = keys::record_fingerprint(record.data()) {
                by_print.entry(print).or_default().push(record.record_id);
            }
        }
        let mut groups: Vec<Vec<Uuid>> = by_print
            .into_values()
            .filter(|ids| ids.len() >= 2)
            .map(|mut ids| {
                ids.sort();
                ids
            })
            .collect();
        groups.sort();
        Ok(groups)
    }

    /// Merges the keys with the fingerprint of `key` into the one with the
    /// smallest record id (docs/data-model.md, section 18.3) and returns it.
    pub fn keys_merge(&self, key: RecordRef) -> Result<KeyView, CoreError> {
        // The passphrases of the others are checked against the kept key
        // without the mutex (bcrypt-pbkdf).
        let (keep, others, keep_text) = {
            let mut guard = self.inner();
            let parts = parts(&mut guard)?;
            check_vault(parts.session, Some(key.vault_id))?;
            let record = parts
                .session
                .index
                .get_kind(&key.record_id, RecordKind::Key)
                .ok_or(CoreError::NotFound)?;
            let print = keys::record_fingerprint(record.data()).ok_or(CoreError::NotFound)?;
            let mut ids: Vec<Uuid> = parts
                .session
                .index
                .of_kind(RecordKind::Key)
                .filter(|record| keys::record_fingerprint(record.data()).as_deref() == Some(&print))
                .map(|record| record.record_id)
                .collect();
            ids.sort();
            let keep = ids.remove(0);
            let keep_text = parts
                .session
                .index
                .get(&keep)
                .and_then(|record| key::private_key(record.data()))
                .map(|text| zeroize::Zeroizing::new(text.to_owned()));
            let others: Vec<(Uuid, Option<SecretString>)> = ids
                .iter()
                .map(|id| {
                    let passphrase = parts
                        .session
                        .index
                        .get(id)
                        .and_then(|record| key::passphrase(record.data()))
                        .map(|value| SecretString::from(value.to_owned()));
                    (*id, passphrase)
                })
                .collect();
            (keep, others, keep_text)
        };
        let opening: Option<SecretString> = keep_text.as_ref().and_then(|text| {
            others
                .iter()
                .filter_map(|(_, passphrase)| passphrase.as_ref())
                .find(|passphrase| keys::passphrase_opens(text, passphrase).is_ok())
                .cloned()
        });

        let now = self.now();
        let mut guard = self.inner();
        let mut parts = parts(&mut guard)?;
        let Some(kept) = parts.session.index.get_kind(&keep, RecordKind::Key) else {
            return Err(CoreError::NotFound);
        };
        let mut kept = WipeOnDrop::new(kept.data().clone());
        let removed: Vec<Uuid> = others
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| parts.session.index.get_kind(id, RecordKind::Key).is_some())
            .collect();
        for id in &removed {
            let Some(other) = parts.session.index.get(id) else {
                continue;
            };
            for member in ["certificate", "fido2"] {
                if kept.get(member).is_none_or(Value::is_null)
                    && let Some(value) = other.data().get(member).filter(|value| !value.is_null())
                {
                    kept.insert(member.into(), value.clone());
                }
            }
        }
        if key::passphrase(&kept).is_none()
            && let Some(passphrase) = &opening
        {
            key::set_passphrase(&mut kept, Some(passphrase.expose_secret()));
        }
        let mut plan = Plan::default();
        plan.set(RecordKind::Key, keep, kept.into_inner());
        let moved: HashMap<Uuid, Uuid> = removed.iter().map(|id| (*id, keep)).collect();
        for kind in [RecordKind::Host, RecordKind::Group, RecordKind::Identity] {
            let changed: Vec<(Uuid, Map<String, Value>)> = parts
                .session
                .index
                .of_kind(kind)
                .filter_map(|record| {
                    let mut data = record.data().clone();
                    let mut touched = false;
                    let slots: Vec<&mut Value> = if kind == RecordKind::Identity {
                        data.get_mut("key_id").into_iter().collect()
                    } else {
                        match data.get_mut("ssh").and_then(Value::as_object_mut) {
                            Some(ssh) => ssh
                                .iter_mut()
                                .filter(|(name, _)| KEY_SLOTS.contains(&name.as_str()))
                                .map(|(_, value)| value)
                                .collect(),
                            None => Vec::new(),
                        }
                    };
                    for slot in slots {
                        if let Some(new) = slot
                            .as_str()
                            .and_then(|raw| Uuid::parse_str(raw).ok())
                            .and_then(|id| moved.get(&id))
                        {
                            *slot = json!(new.to_string());
                            touched = true;
                        }
                    }
                    if touched {
                        Some((record.record_id, data))
                    } else {
                        crate::model::wipe::wipe_map(&mut data);
                        None
                    }
                })
                .collect();
            for (id, data) in changed {
                plan.set(kind, id, data);
            }
        }
        let vault_id = parts.session.ring.vault_id();
        let bound = parts.store.vault(&vault_id)?.is_some_and(|row| row.bound);
        let mut deletions = Vec::new();
        let mut tombstones = Vec::new();
        for id in &removed {
            let Some(previous) = parts.store.record(&vault_id, id)? else {
                continue;
            };
            if !bound && previous.synced_revision == 0 {
                deletions.push((*id, previous.class));
            } else {
                tombstones.push(parts.session.prepare_tombstone(
                    RecordKind::Key,
                    &previous,
                    now,
                )?);
            }
        }
        let writes = plan.prepare(&mut parts, now)?;
        self.commit_records(&mut parts, writes, deletions, tombstones)?;
        tracing::info!(merged = removed.len(), "identical keys merged");
        Self::key_view_of(parts.session, &keep)
    }
}

#[cfg(test)]
mod tests {
    //! Identical keys arrive only by sync (an import matches keys by
    //! fingerprint), so the duplicate is written through the session here.

    use secrecy::SecretString;
    use serde_json::json;

    use super::*;
    use crate::platform::OsKind;
    use crate::platform::memory::MemoryPlatform;
    use crate::service::CoreConfig;
    use crate::service::dto::{KeyGenerateInput, RecordSaveInput, RecordType};

    fn core() -> (tempfile::TempDir, MemoryPlatform, Core) {
        let dir = tempfile::tempdir().unwrap();
        let mem = MemoryPlatform::new();
        let core = Core::open(
            CoreConfig {
                data_dir: dir.path().join("data"),
                app_version: "0.1.0".into(),
            },
            mem.platform(OsKind::Linux, "TEST"),
        )
        .unwrap();
        let password: String = crate::crypto::random_array::<12>()
            .unwrap()
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        core.first_run_setup(SecretString::from(format!("Pw-{password}")))
            .unwrap();
        (dir, mem, core)
    }

    #[test]
    fn identical_keys_merge_into_the_oldest_and_references_move() {
        let (_dir, _mem, core) = core();
        let vault_id = core.vaults().unwrap()[0].vault_id;
        let generated = core
            .key_generate(KeyGenerateInput {
                vault_id,
                label: "original".into(),
                kind: crate::keys::generate::KeyKind::Ed25519,
                passphrase: None,
                save_passphrase: false,
            })
            .unwrap()
            .key;
        // The same key under a newer id, with a member the original lacks.
        let duplicate = {
            let now = core.now();
            let mut guard = core.inner();
            let mut parts = parts(&mut guard).unwrap();
            let mut data = parts
                .session
                .index
                .get(&generated.record_id)
                .unwrap()
                .data()
                .clone();
            data.insert("label".into(), json!("copy"));
            data.insert("fido2".into(), json!({"resident": false}));
            let id = parts.ids.next(now);
            let mut plan = Plan::default();
            plan.set(RecordKind::Key, id, data);
            let writes = plan.prepare(&mut parts, now).unwrap();
            core.commit_records(&mut parts, writes, Vec::new(), Vec::new())
                .unwrap();
            id
        };
        assert!(duplicate > generated.record_id);
        let host = core
            .record_save(RecordSaveInput {
                vault_id,
                record_id: None,
                record_type: RecordType::Host,
                data: json!({"address": "192.0.2.9", "ssh": {"key_id": duplicate.to_string()}})
                    .as_object()
                    .unwrap()
                    .clone(),
                secrets: Default::default(),
            })
            .unwrap();
        assert_eq!(
            core.key_duplicates().unwrap(),
            vec![vec![generated.record_id, duplicate]]
        );

        let kept = core
            .keys_merge(RecordRef {
                vault_id,
                record_id: duplicate,
            })
            .unwrap();
        assert_eq!(kept.record_id, generated.record_id, "the oldest stays");
        assert_eq!(kept.label, "original");
        assert!(core.key_duplicates().unwrap().is_empty());
        assert_eq!(core.keys_list().unwrap().len(), 1);
        let mut guard = core.inner();
        let parts = parts(&mut guard).unwrap();
        let data = parts.session.index.get(&host.record_id()).unwrap().data();
        assert_eq!(
            data["ssh"]["key_id"],
            json!(generated.record_id.to_string()),
            "the host's reference moved"
        );
        let key = parts
            .session
            .index
            .get(&generated.record_id)
            .unwrap()
            .data();
        assert_eq!(
            key["fido2"],
            json!({"resident": false}),
            "taken from the copy"
        );
    }
}
