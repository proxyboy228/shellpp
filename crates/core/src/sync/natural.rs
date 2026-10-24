//! Natural keys of record types (data-model 18.6): duplicates from two
//! devices (18.3, D93) and the merge at sign-in (18.5). The key compares
//! what the user would call "the same object"; `None` for types without one
//! (history and log entries are written by each device for itself).
//!
//! [`Keys`] gives the keys of data-model 18.6 in the context of one vault:
//! a group, snippet or forwarding rule names its parent (group, package,
//! host) by that parent's natural key, never by its record id, and a host
//! compares its effective port and username (data-model 8.3: its own, its
//! identity's, then inherited from its groups). So the keys of two vaults
//! (a local one and the account's at sign-in, run 7 finding 16) compare
//! although their record ids differ.

use std::collections::HashMap;

use serde_json::{Map, Value};
use uuid::Uuid;

use crate::index::Index;
use crate::model::tree::Tree;
use crate::model::types::RecordKind;

/// Separates a parent's natural key inside a child's.
const NESTED: char = '\u{1d}';

/// Natural keys in the context of one vault's index (data-model 18.6).
pub struct Keys<'a> {
    index: &'a Index,
    tree: Tree,
    /// Groups' keys, computed once.
    groups: std::cell::RefCell<HashMap<Uuid, String>>,
}

impl std::fmt::Debug for Keys<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Keys").finish_non_exhaustive()
    }
}

impl<'a> Keys<'a> {
    pub fn new(index: &'a Index) -> Self {
        Self {
            index,
            tree: index.tree(),
            groups: std::cell::RefCell::new(HashMap::new()),
        }
    }

    /// The natural key of a record of `kind` with `data` in this vault
    /// (`data` need not be in the index: a record about to be inserted).
    pub fn key(&self, kind: RecordKind, data: &Map<String, Value>) -> Option<String> {
        let parts: Vec<String> = match kind {
            RecordKind::Host => {
                let import = text(data, "import_key");
                if !import.is_empty() {
                    vec!["import".into(), import]
                } else {
                    let (port, username) = self.host_effective(data);
                    vec![lower(data, "address"), port.to_string(), username]
                }
            }
            RecordKind::Group => vec![self.parent_group(data), text(data, "name")],
            RecordKind::Snippet => vec![
                self.reference(data, "package_id", RecordKind::SnippetPackage),
                text(data, "label"),
                text(data, "script"),
            ],
            RecordKind::ForwardingRule => vec![
                text(data, "type"),
                self.reference(data, "host_id", RecordKind::Host),
                text(data, "bind_address"),
                number(data, "bind_port", 0),
                text(data, "dest_address"),
                number(data, "dest_port", 0),
            ],
            _ => return key(kind, data),
        };
        joined(parts)
    }

    /// The natural key of the record `data` names in `member`, wrapped so
    /// it stays one part; empty when it names nothing live of `kind`.
    fn reference(&self, data: &Map<String, Value>, member: &str, kind: RecordKind) -> String {
        data.get(member)
            .and_then(Value::as_str)
            .and_then(|id| id.parse::<Uuid>().ok())
            .and_then(|id| self.index.get_kind(&id, kind))
            .and_then(|record| self.key(kind, record.data()))
            .map(|key| format!("{NESTED}{key}{NESTED}"))
            .unwrap_or_default()
    }

    /// The key of a group's parent: the chain of names up to the top level
    /// (a missing parent, or one the cycle rule makes top level, ends it).
    fn parent_group(&self, data: &Map<String, Value>) -> String {
        let Some(parent) = crate::model::group::parent_id(data).filter(|id| self.tree.contains(id))
        else {
            return String::new();
        };
        self.group_path(parent)
    }

    fn group_path(&self, id: Uuid) -> String {
        if let Some(known) = self.groups.borrow().get(&id) {
            return known.clone();
        }
        let mut names = Vec::new();
        for level in self.tree.chain(id, crate::model::inherit::MAX_LEVELS) {
            let name = self
                .index
                .get_kind(&level, RecordKind::Group)
                .map(|record| text(record.data(), "name"))
                .unwrap_or_default();
            names.push(name);
        }
        names.reverse();
        let path = format!("{NESTED}{}{NESTED}", names.join("\u{1f}"));
        self.groups.borrow_mut().insert(id, path.clone());
        path
    }

    /// A host's effective port (22 when nothing sets it) and username.
    fn host_effective(&self, data: &Map<String, Value>) -> (u16, String) {
        let ssh = data.get("ssh").and_then(Value::as_object);
        let own = |name: &str| ssh.and_then(|ssh| ssh.get(name));
        let inherited = self.index.inherited_for_host(&self.tree, data);
        let port = own("port")
            .and_then(Value::as_u64)
            .and_then(|port| u16::try_from(port).ok())
            .filter(|port| *port >= 1)
            .or(inherited.port.map(|port| port.value))
            .unwrap_or(22);
        let identity_user = own("identity_id")
            .and_then(Value::as_str)
            .and_then(|id| id.parse::<Uuid>().ok())
            .and_then(|id| self.index.get_kind(&id, RecordKind::Identity))
            .and_then(|identity| identity.data().get("username").and_then(Value::as_str))
            .map(str::to_owned);
        let username = own("username")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or(identity_user)
            .or(inherited.username.map(|user| user.value))
            .unwrap_or_default();
        (port, username)
    }
}

fn number(data: &Map<String, Value>, name: &str, default: u64) -> String {
    data.get(name)
        .and_then(Value::as_u64)
        .unwrap_or(default)
        .to_string()
}

fn joined(parts: Vec<String>) -> Option<String> {
    if parts.iter().all(String::is_empty) {
        return None;
    }
    Some(parts.join("\u{1f}"))
}

fn text(data: &Map<String, Value>, name: &str) -> String {
    data.get(name)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn lower(data: &Map<String, Value>, name: &str) -> String {
    text(data, name).to_lowercase()
}

fn ssh(data: &Map<String, Value>) -> Map<String, Value> {
    data.get("ssh")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default()
}

/// The natural key of a record of `kind` from its own fields only, as one
/// comparable string: hosts with their own port and username (22 and none
/// when unset), parents by their record ids. [`Keys::key`] is the key of
/// data-model 18.6; this is its part that needs no other record.
pub fn key(kind: RecordKind, data: &Map<String, Value>) -> Option<String> {
    let parts: Vec<String> = match kind {
        RecordKind::Host => {
            let import = text(data, "import_key");
            if !import.is_empty() {
                vec!["import".into(), import]
            } else {
                let ssh = ssh(data);
                vec![
                    lower(data, "address"),
                    ssh.get("port")
                        .and_then(Value::as_u64)
                        .unwrap_or(22)
                        .to_string(),
                    text(&ssh, "username"),
                ]
            }
        }
        RecordKind::Group => vec![text(data, "parent_id"), text(data, "name")],
        RecordKind::Identity => vec![text(data, "label"), text(data, "username")],
        RecordKind::Key => {
            let public = text(data, "public_key");
            if public.is_empty() {
                return None;
            }
            // The key blob (algorithm and base64), without the comment.
            let mut words = public.split_whitespace();
            vec![
                words.next().unwrap_or_default().to_owned(),
                words.next().unwrap_or_default().to_owned(),
            ]
        }
        RecordKind::SnippetPackage => vec![lower(data, "name")],
        RecordKind::Snippet => vec![
            text(data, "package_id"),
            text(data, "label"),
            text(data, "script"),
        ],
        RecordKind::ForwardingRule => vec![
            text(data, "type"),
            text(data, "host_id"),
            text(data, "bind_address"),
            data.get("bind_port")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .to_string(),
            text(data, "dest_address"),
            data.get("dest_port")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .to_string(),
        ],
        RecordKind::KnownHost => vec![
            lower(data, "host"),
            data.get("port")
                .and_then(Value::as_u64)
                .unwrap_or(22)
                .to_string(),
            text(data, "key_type"),
            text(data, "public_key"),
            text(data, "marker"),
        ],
        RecordKind::Workspace => vec![text(data, "name")],
        RecordKind::HighlightRule => {
            if text(data, "kind") == "category" {
                vec!["category".into(), text(data, "category")]
            } else {
                vec![
                    "custom".into(),
                    text(data, "pattern"),
                    data.get("case_insensitive")
                        .and_then(Value::as_bool)
                        .unwrap_or(false)
                        .to_string(),
                ]
            }
        }
        _ => return None,
    };
    if parts.iter().all(String::is_empty) {
        return None;
    }
    Some(parts.join("\u{1f}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    #[test]
    fn hosts_match_by_import_key_else_address_port_user() {
        let a = map(json!({"address": "DB.example.com", "ssh": {"port": 22, "username": "root"}}));
        let b = map(json!({"address": "db.example.com", "ssh": {"username": "root"}}));
        assert_eq!(key(RecordKind::Host, &a), key(RecordKind::Host, &b));
        let c =
            map(json!({"address": "db.example.com", "ssh": {"port": 2222, "username": "root"}}));
        assert_ne!(key(RecordKind::Host, &a), key(RecordKind::Host, &c));
        let d = map(json!({"address": "x", "import_key": "ssh_config:db"}));
        let e = map(json!({"address": "y", "import_key": "ssh_config:db"}));
        assert_eq!(key(RecordKind::Host, &d), key(RecordKind::Host, &e));
    }

    #[test]
    fn history_has_no_natural_key_for_duplicates() {
        let entry = map(json!({"command": "ls"}));
        assert_eq!(key(RecordKind::HistoryEntry, &entry), None);
        assert_eq!(key(RecordKind::ConnectionLogEntry, &entry), None);
    }
}
