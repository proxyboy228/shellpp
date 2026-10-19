//! The `history_entry` record (docs/data-model.md, section 13.1; D16).
//!
//! Written only by [`crate::service::Core::history_record`] after the
//! capture rules and the secret detector ([`crate::history`]) allowed the
//! command. A dropped command never reaches this module.

use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::service::dto::RecordRef;

/// The data of a typed command.
pub fn new_typed(host: Option<RecordRef>, command: &str, executed_at: &str) -> Map<String, Value> {
    let mut data = Map::new();
    if let Some(host) = host {
        data.insert("host".into(), host_link(host));
    }
    data.insert("command".into(), Value::String(command.to_owned()));
    data.insert("executed_at".into(), Value::String(executed_at.to_owned()));
    data.insert("origin".into(), json!("typed"));
    data
}

/// A `host_link` value.
pub fn host_link(host: RecordRef) -> Value {
    json!({ "vault_id": host.vault_id, "record_id": host.record_id })
}

/// Reads a `host_link` member (`None` when absent or malformed).
pub fn read_host_link(value: Option<&Value>) -> Option<RecordRef> {
    let object = value?.as_object()?;
    let id = |key: &str| {
        object
            .get(key)
            .and_then(Value::as_str)
            .and_then(|raw| Uuid::parse_str(raw).ok())
    };
    Some(RecordRef {
        vault_id: id("vault_id")?,
        record_id: id("record_id")?,
    })
}

/// The host link of an entry.
pub fn host(data: &Map<String, Value>) -> Option<RecordRef> {
    read_host_link(data.get("host"))
}

/// The command (empty when missing).
pub fn command(data: &Map<String, Value>) -> &str {
    data.get("command").and_then(Value::as_str).unwrap_or("")
}

/// `executed_at` (empty when missing; sorts first).
pub fn executed_at(data: &Map<String, Value>) -> &str {
    data.get("executed_at")
        .and_then(Value::as_str)
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_entry_round_trip() {
        let host = RecordRef {
            vault_id: Uuid::from_u128(1),
            record_id: Uuid::from_u128(2),
        };
        let data = new_typed(Some(host), "ls -la", "2026-10-18T22:31:05.123Z");
        assert_eq!(data["origin"], json!("typed"));
        assert_eq!(self::host(&data), Some(host));
        assert_eq!(command(&data), "ls -la");
        assert_eq!(executed_at(&data), "2026-10-18T22:31:05.123Z");
        let local = new_typed(None, "dir", "2026-10-18T22:31:05.123Z");
        assert!(!local.contains_key("host"));
        assert_eq!(self::host(&local), None);
        assert_eq!(read_host_link(Some(&json!({"vault_id": "x"}))), None);
    }
}
