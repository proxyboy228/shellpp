//! The `connection_log_entry` record (docs/data-model.md, section 13.3),
//! stage 2 subset: the personal entry of an established SSH session with its
//! snapshots, written at start and updated at the end ([`crate::connlog`]).

use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::model::history_entry::{host_link, read_host_link};
use crate::model::text;
use crate::service::dto::{HostOs, RecordRef};
use crate::service::host_api::ConnectionEndReason;

/// The snapshot members of a new entry.
#[derive(Clone)]
pub struct Start<'a> {
    pub started_at: &'a str,
    pub host: Option<RecordRef>,
    pub host_label: Option<&'a str>,
    pub address: &'a str,
    pub port: u16,
    pub username: Option<&'a str>,
    pub os: Option<&'a HostOs>,
    pub device_id: Uuid,
}

impl std::fmt::Debug for Start<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Start")
            .field("host", &self.host)
            .finish_non_exhaustive()
    }
}

/// A snapshot string: `string(n)` rules, cut to `max` characters instead of
/// refused (the values come from validated records).
fn snapshot(value: &str, max: usize) -> Option<String> {
    let cut: String = value
        .chars()
        .filter(|c| !c.is_control())
        .take(max)
        .collect();
    text::string(&cut, max).ok().flatten()
}

/// The data of a new personal entry (`protocol = ssh`, `saved = false`).
pub fn new_ssh(start: &Start<'_>) -> Map<String, Value> {
    let mut data = Map::new();
    data.insert("started_at".into(), json!(start.started_at));
    data.insert("protocol".into(), json!("ssh"));
    if let Some(host) = start.host {
        data.insert("host".into(), host_link(host));
    }
    if let Some(label) = start.host_label.and_then(|label| snapshot(label, 256)) {
        data.insert("host_label".into(), Value::String(label));
    }
    data.insert(
        "address".into(),
        Value::String(snapshot(start.address, 255).unwrap_or_default()),
    );
    data.insert("port".into(), json!(start.port));
    if let Some(username) = start.username.and_then(|name| snapshot(name, 256)) {
        data.insert("username".into(), Value::String(username));
    }
    if let Some(os) = start.os
        && let Ok(value) = serde_json::to_value(os)
    {
        data.insert("os".into(), value);
    }
    data.insert("device_id".into(), json!(start.device_id));
    data.insert("saved".into(), Value::Bool(false));
    data
}

/// The `end_reason` value.
pub fn reason_name(reason: ConnectionEndReason) -> &'static str {
    match reason {
        ConnectionEndReason::Closed => "closed",
        ConnectionEndReason::ExitStatus => "exit_status",
        ConnectionEndReason::Dropped => "dropped",
    }
}

/// Sets the end members.
pub fn set_end(
    data: &mut Map<String, Value>,
    ended_at: &str,
    reason: &str,
    exit_status: Option<u32>,
) {
    data.insert("ended_at".into(), json!(ended_at));
    data.insert("end_reason".into(), json!(reason));
    match exit_status {
        Some(status) => {
            data.insert("exit_status".into(), json!(status));
        }
        None => {
            data.remove("exit_status");
        }
    }
}

/// `ended_at`, when the session ended and its time is known.
pub fn ended_at(data: &Map<String, Value>) -> Option<&str> {
    data.get("ended_at").and_then(Value::as_str)
}

/// `end_reason` (`closed`, `exit_status`, `dropped`, `unknown`).
pub fn end_reason(data: &Map<String, Value>) -> Option<&str> {
    data.get("end_reason").and_then(Value::as_str)
}

pub fn exit_status(data: &Map<String, Value>) -> Option<u32> {
    data.get("exit_status")
        .and_then(Value::as_u64)
        .and_then(|status| u32::try_from(status).ok())
}

/// The bookmark of the Logs screen (UI 11.1).
pub fn saved(data: &Map<String, Value>) -> bool {
    data.get("saved").and_then(Value::as_bool).unwrap_or(false)
}

/// `started_at` (empty when missing).
pub fn started_at(data: &Map<String, Value>) -> &str {
    data.get("started_at").and_then(Value::as_str).unwrap_or("")
}

/// Marks an entry left open by an earlier run of the app: `end_reason =
/// unknown` without `ended_at`, because the end time is not known
/// (docs/data-model.md, section 13.3).
pub fn set_unknown_end(data: &mut Map<String, Value>) {
    data.insert("end_reason".into(), json!(crate::connlog::UNKNOWN_END));
}

/// `true` while the entry has neither `ended_at` nor `end_reason`.
pub fn is_open(data: &Map<String, Value>) -> bool {
    data.get("ended_at").is_none_or(Value::is_null)
        && data.get("end_reason").is_none_or(Value::is_null)
}

/// The connecting device.
pub fn device_id(data: &Map<String, Value>) -> Option<Uuid> {
    data.get("device_id")
        .and_then(Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
}

pub fn host(data: &Map<String, Value>) -> Option<RecordRef> {
    read_host_link(data.get("host"))
}

pub fn host_label(data: &Map<String, Value>) -> Option<&str> {
    data.get("host_label")
        .and_then(Value::as_str)
        .filter(|label| !label.is_empty())
}

pub fn address(data: &Map<String, Value>) -> &str {
    data.get("address").and_then(Value::as_str).unwrap_or("")
}

pub fn port(data: &Map<String, Value>) -> u16 {
    data.get("port")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())
        .filter(|port| *port > 0)
        .unwrap_or(22)
}

pub fn username(data: &Map<String, Value>) -> Option<&str> {
    data.get("username")
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
}

pub fn os(data: &Map<String, Value>) -> Option<HostOs> {
    data.get("os")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

/// `true` for a personal SSH entry of this app's stage-2 subset.
pub fn is_ssh(data: &Map<String, Value>) -> bool {
    data.get("protocol").and_then(Value::as_str) == Some("ssh")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::dto::{OsFamily, OsSource};

    #[test]
    fn entry_start_and_end() {
        let device = Uuid::from_u128(7);
        let host = RecordRef {
            vault_id: Uuid::from_u128(1),
            record_id: Uuid::from_u128(2),
        };
        let os = HostOs {
            family: OsFamily::Linux,
            distro: Some("debian".into()),
            version: None,
            source: OsSource::Banner,
            detected_at: "2026-10-18T22:31:05.123Z".into(),
        };
        let long_label = "L".repeat(300);
        let mut data = new_ssh(&Start {
            started_at: "2026-10-18T22:31:05.123Z",
            host: Some(host),
            host_label: Some(&long_label),
            address: "db1.example.com",
            port: 2222,
            username: Some("deploy"),
            os: Some(&os),
            device_id: device,
        });
        assert_eq!(data["protocol"], json!("ssh"));
        assert_eq!(data["saved"], json!(false));
        assert_eq!(host_label(&data).map(|l| l.chars().count()), Some(256));
        assert_eq!(self::host(&data), Some(host));
        assert_eq!(address(&data), "db1.example.com");
        assert_eq!(port(&data), 2222);
        assert_eq!(username(&data), Some("deploy"));
        assert_eq!(self::os(&data), Some(os));
        assert_eq!(device_id(&data), Some(device));
        assert!(is_open(&data));
        assert!(is_ssh(&data));
        set_end(
            &mut data,
            "2026-10-18T23:00:00.000Z",
            reason_name(ConnectionEndReason::ExitStatus),
            Some(3),
        );
        assert!(!is_open(&data));
        assert_eq!(data["end_reason"], json!("exit_status"));
        assert_eq!(data["exit_status"], json!(3));
        set_end(&mut data, "2026-10-18T23:00:00.000Z", "dropped", None);
        assert!(!data.contains_key("exit_status"));
    }

    #[test]
    fn unknown_end_closes_without_a_time() {
        let mut data = new_ssh(&Start {
            started_at: "2026-10-18T22:31:05.123Z",
            host: None,
            host_label: None,
            address: "198.51.100.7",
            port: 22,
            username: None,
            os: None,
            device_id: Uuid::from_u128(1),
        });
        assert!(is_open(&data));
        set_unknown_end(&mut data);
        assert!(!is_open(&data));
        assert_eq!(data["end_reason"], json!("unknown"));
        assert!(!data.contains_key("ended_at"));
        assert!(!data.contains_key("host"));
    }
}
