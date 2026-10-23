//! Natural keys of record types (data-model 18.6): duplicates from two
//! devices (18.3, D93) and the merge at sign-in (18.5). The key compares
//! what the user would call "the same object"; `None` for types without one
//! (history and log entries are written by each device for itself).

use serde_json::{Map, Value};

use crate::model::types::RecordKind;

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

/// The natural key of a record of `kind`, as one comparable string.
/// Hosts use their own port and username (22 and none when unset): the
/// inherited values need the other records, which a key must not.
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
        RecordKind::Group => vec![
            text(data, "parent_id"),
            text(data, "name"),
        ],
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
        let c = map(json!({"address": "db.example.com", "ssh": {"port": 2222, "username": "root"}}));
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
