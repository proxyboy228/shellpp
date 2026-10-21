//! The `snippet`, `snippet_package` and `forwarding_rule` records
//! (docs/data-model.md, sections 10 and 11; UI 8, 9).
//!
//! Each `build_*` validates the members the app edits, keeps unknown members
//! of the input and of the stored data (data-model 1.2) and keeps the
//! managed `import_key`. Errors use the member names.

use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::error::{FieldError, ValidationReason};
use crate::model::fields::{
    Errors, opt_bool, opt_enum, opt_int, opt_string, opt_uuid, opt_uuid_list, present,
};
use crate::model::merge::{keep_from, merge_level};
use crate::model::{order_key, text};

/// Members only the core writes; input values are ignored.
pub const MANAGED: &[&str] = &["import_key"];

/// Largest `script`, in UTF-8 bytes (`text(65536)`).
pub const MAX_SCRIPT_BYTES: usize = 65_536;
/// Longest `label` / `name`, in Unicode scalar values.
pub const MAX_LABEL_CHARS: usize = 256;
/// Most tags and targets of a snippet.
pub const MAX_TAGS: usize = 32;
pub const MAX_TARGETS: usize = 256;

const SNIPPET_KNOWN: &[&str] = &[
    "script",
    "label",
    "package_id",
    "tags",
    "target_host_ids",
    "order_key",
    "import_key",
];
const PACKAGE_KNOWN: &[&str] = &["name", "order_key", "import_key"];
const RULE_KNOWN: &[&str] = &[
    "type",
    "host_id",
    "label",
    "bind_address",
    "bind_port",
    "dest_address",
    "dest_port",
    "autostart_on_connect",
    "autostart_on_launch",
    "import_key",
];

fn order_key_member(
    input: &Map<String, Value>,
    normalized: &mut Map<String, Value>,
    errors: &mut Errors,
) {
    if let Some(value) = present(input, "order_key") {
        match value.as_str() {
            Some(key) if order_key::is_valid(key) => {
                normalized.insert("order_key".into(), Value::String(key.to_owned()));
            }
            _ => errors.push("order_key", ValidationReason::Invalid),
        }
    }
}

fn finish(
    known: &[&str],
    normalized: Map<String, Value>,
    input: &Map<String, Value>,
    stored: Option<&Map<String, Value>>,
    errors: Errors,
) -> Result<Map<String, Value>, Vec<FieldError>> {
    if !errors.is_empty() {
        return Err(errors.into_vec());
    }
    let mut data = merge_level(known, normalized, Some(input), stored);
    keep_from(&mut data, stored, MANAGED);
    Ok(data)
}

/// `tags`: unique tags (case-insensitive), at most [`MAX_TAGS`].
fn tags(input: &Map<String, Value>, errors: &mut Errors) -> Option<Value> {
    let value = present(input, "tags")?;
    let Some(items) = value.as_array() else {
        errors.push("tags", ValidationReason::Invalid);
        return None;
    };
    if items.len() > MAX_TAGS {
        errors.push("tags", ValidationReason::TooMany);
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    for item in items {
        match item.as_str().map(text::tag) {
            Some(Ok(tag)) => {
                if out.iter().any(|seen| text::fold(seen) == text::fold(&tag)) {
                    errors.push("tags", ValidationReason::Duplicate);
                    return None;
                }
                out.push(tag);
            }
            Some(Err(reason)) => {
                errors.push("tags", reason);
                return None;
            }
            None => {
                errors.push("tags", ValidationReason::Invalid);
                return None;
            }
        }
    }
    Some(json!(out))
}

/// Validates a `snippet` (data-model 10.1): `script` required and not
/// blank, `label`, `package_id`, `tags`, `target_host_ids` (unique hosts).
/// References are checked by the caller (they need the index).
pub fn build_snippet(
    input: &Map<String, Value>,
    stored: Option<&Map<String, Value>>,
) -> Result<Map<String, Value>, Vec<FieldError>> {
    let mut errors = Errors::new();
    let mut normalized = Map::new();
    match present(input, "script")
        .map(|value| value.as_str().map(|raw| text::text(raw, MAX_SCRIPT_BYTES)))
    {
        None => errors.push("script", ValidationReason::Required),
        Some(Some(Ok(Some(script)))) if !script.trim().is_empty() => {
            normalized.insert("script".into(), Value::String(script));
        }
        Some(Some(Ok(_))) => errors.push("script", ValidationReason::Required),
        Some(Some(Err(reason))) => errors.push("script", reason),
        Some(None) => errors.push("script", ValidationReason::Invalid),
    }
    if let Some(label) = opt_string(input, "label", "label", MAX_LABEL_CHARS, &mut errors) {
        normalized.insert("label".into(), Value::String(label));
    }
    if let Some(package) = opt_uuid(input, "package_id", "package_id", &mut errors) {
        normalized.insert("package_id".into(), json!(package));
    }
    if let Some(tags) = tags(input, &mut errors) {
        normalized.insert("tags".into(), tags);
    }
    if let Some(targets) = opt_uuid_list(
        input,
        "target_host_ids",
        "target_host_ids",
        MAX_TARGETS,
        None,
        &mut errors,
    ) {
        normalized.insert("target_host_ids".into(), json!(targets));
    }
    order_key_member(input, &mut normalized, &mut errors);
    finish(SNIPPET_KNOWN, normalized, input, stored, errors)
}

/// Validates a `snippet_package` (data-model 10.2): `name` required.
pub fn build_package(
    input: &Map<String, Value>,
    stored: Option<&Map<String, Value>>,
) -> Result<Map<String, Value>, Vec<FieldError>> {
    let mut errors = Errors::new();
    let mut normalized = Map::new();
    match present(input, "name")
        .map(|value| value.as_str().map(|raw| text::string(raw, MAX_LABEL_CHARS)))
    {
        Some(Some(Ok(Some(name)))) => {
            normalized.insert("name".into(), Value::String(name));
        }
        None | Some(Some(Ok(None))) => errors.push("name", ValidationReason::Required),
        Some(Some(Err(reason))) => errors.push("name", reason),
        Some(None) => errors.push("name", ValidationReason::Invalid),
    }
    order_key_member(input, &mut normalized, &mut errors);
    finish(PACKAGE_KNOWN, normalized, input, stored, errors)
}

/// A bind address: an IP literal or `localhost` (data-model 11); empty is
/// `127.0.0.1`.
fn bind_address(raw: &str) -> Result<String, ValidationReason> {
    let raw = raw.trim();
    if raw.is_empty() {
        return Ok("127.0.0.1".to_owned());
    }
    if raw.eq_ignore_ascii_case("localhost") {
        return Ok("localhost".to_owned());
    }
    let bare = raw.trim_start_matches('[').trim_end_matches(']');
    bare.parse::<std::net::IpAddr>()
        .map(|ip| ip.to_string())
        .map_err(|_| ValidationReason::Invalid)
}

/// Whether a bind address opens the port on more than loopback (the UI
/// warns: `0.0.0.0`, `::` and every non-loopback address, docs/security.md
/// 11.3).
pub fn is_wide_bind(address: &str) -> bool {
    match address.parse::<std::net::IpAddr>() {
        Ok(ip) => !ip.is_loopback(),
        Err(_) => !address.eq_ignore_ascii_case("localhost"),
    }
}

/// Validates a `forwarding_rule` (data-model 11): `type` and `host_id`
/// required, `bind_port` required, `dest_address` and `dest_port` required
/// for `local` and `remote` and not allowed for `dynamic`; `bind_address`
/// defaults to `127.0.0.1`.
pub fn build_rule(
    input: &Map<String, Value>,
    stored: Option<&Map<String, Value>>,
) -> Result<Map<String, Value>, Vec<FieldError>> {
    let mut errors = Errors::new();
    let mut normalized = Map::new();
    let kind = opt_enum(
        input,
        "type",
        "type",
        &["local", "remote", "dynamic"],
        &mut errors,
    );
    match kind {
        Some(kind) => {
            normalized.insert("type".into(), json!(kind));
        }
        None if present(input, "type").is_none() => errors.push("type", ValidationReason::Required),
        None => {}
    }
    match opt_uuid(input, "host_id", "host_id", &mut errors) {
        Some(host) => {
            normalized.insert("host_id".into(), json!(host));
        }
        None if present(input, "host_id").is_none() => {
            errors.push("host_id", ValidationReason::Required)
        }
        None => {}
    }
    if let Some(label) = opt_string(input, "label", "label", MAX_LABEL_CHARS, &mut errors) {
        normalized.insert("label".into(), Value::String(label));
    }
    let bind = match present(input, "bind_address") {
        None => Some("127.0.0.1".to_owned()),
        Some(value) => match value.as_str().map(bind_address) {
            Some(Ok(address)) => Some(address),
            Some(Err(reason)) => {
                errors.push("bind_address", reason);
                None
            }
            None => {
                errors.push("bind_address", ValidationReason::Invalid);
                None
            }
        },
    };
    if let Some(bind) = bind {
        normalized.insert("bind_address".into(), Value::String(bind));
    }
    match opt_int(input, "bind_port", "bind_port", 1, 65_535, &mut errors) {
        Some(port) => {
            normalized.insert("bind_port".into(), json!(port));
        }
        None if present(input, "bind_port").is_none() => {
            errors.push("bind_port", ValidationReason::Required)
        }
        None => {}
    }
    let dest_address = opt_string(input, "dest_address", "dest_address", 255, &mut errors);
    let dest_port = opt_int(input, "dest_port", "dest_port", 1, 65_535, &mut errors);
    match kind {
        Some("dynamic") => {
            if present(input, "dest_address").is_some() {
                errors.push("dest_address", ValidationReason::NotAllowed);
            }
            if present(input, "dest_port").is_some() {
                errors.push("dest_port", ValidationReason::NotAllowed);
            }
        }
        Some(_) => {
            match dest_address {
                Some(address) => match crate::model::fields::address(&address) {
                    Ok(Some(address)) => {
                        normalized.insert("dest_address".into(), Value::String(address));
                    }
                    Ok(None) => errors.push("dest_address", ValidationReason::Required),
                    Err(reason) => errors.push("dest_address", reason),
                },
                None if present(input, "dest_address").is_none() => {
                    errors.push("dest_address", ValidationReason::Required)
                }
                None => {}
            }
            match dest_port {
                Some(port) => {
                    normalized.insert("dest_port".into(), json!(port));
                }
                None if present(input, "dest_port").is_none() => {
                    errors.push("dest_port", ValidationReason::Required)
                }
                None => {}
            }
        }
        None => {}
    }
    for flag in ["autostart_on_connect", "autostart_on_launch"] {
        if let Some(value) = opt_bool(input, flag, flag, &mut errors) {
            normalized.insert(flag.into(), json!(value));
        }
    }
    finish(RULE_KNOWN, normalized, input, stored, errors)
}

/// A uuid member of stored data.
pub fn uuid_member(data: &Map<String, Value>, key: &str) -> Option<Uuid> {
    data.get(key)
        .and_then(Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
}

/// The uuid list member of stored data.
pub fn uuid_list(data: &Map<String, Value>, key: &str) -> Vec<Uuid> {
    data.get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .filter_map(|raw| Uuid::parse_str(raw).ok())
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    fn fields(errors: Vec<FieldError>) -> Vec<(String, ValidationReason)> {
        errors
            .into_iter()
            .map(|error| (error.field, error.reason))
            .collect()
    }

    #[test]
    fn a_snippet_needs_a_script() {
        let built = build_snippet(
            &map(json!({"script": "uptime", "label": "Up", "x_future": 1})),
            None,
        )
        .unwrap();
        assert_eq!(built["script"], "uptime");
        assert_eq!(built["x_future"], 1, "unknown members are kept");
        assert_eq!(
            fields(build_snippet(&map(json!({"script": "  \n "})), None).unwrap_err()),
            [("script".to_owned(), ValidationReason::Required)]
        );
        assert_eq!(
            fields(build_snippet(&map(json!({"label": "x"})), None).unwrap_err()),
            [("script".to_owned(), ValidationReason::Required)]
        );
        let host = Uuid::now_v7().to_string();
        let error = build_snippet(
            &map(json!({"script": "x", "target_host_ids": [host, host]})),
            None,
        )
        .unwrap_err();
        assert_eq!(error[0].field, "target_host_ids");
        let error =
            build_snippet(&map(json!({"script": "x", "tags": ["a", "A"]})), None).unwrap_err();
        assert_eq!(
            fields(error),
            [("tags".to_owned(), ValidationReason::Duplicate)]
        );
    }

    #[test]
    fn a_package_needs_a_name() {
        assert!(build_package(&map(json!({"name": "Deploy"})), None).is_ok());
        assert_eq!(
            fields(build_package(&map(json!({"name": ""})), None).unwrap_err()),
            [("name".to_owned(), ValidationReason::Required)]
        );
    }

    #[test]
    fn forwarding_rules_follow_their_type() {
        let host = Uuid::now_v7().to_string();
        let local = build_rule(
            &map(json!({"type": "local", "host_id": host, "bind_port": 15432,
                "dest_address": "db.internal", "dest_port": 5432})),
            None,
        )
        .unwrap();
        // The default bind address is loopback.
        assert_eq!(local["bind_address"], "127.0.0.1");
        let dynamic = build_rule(
            &map(
                json!({"type": "dynamic", "host_id": host, "bind_port": 1080,
                "bind_address": "0.0.0.0"}),
            ),
            None,
        )
        .unwrap();
        assert_eq!(dynamic["bind_address"], "0.0.0.0");
        assert!(is_wide_bind("0.0.0.0") && is_wide_bind("::") && is_wide_bind("192.168.1.5"));
        assert!(!is_wide_bind("127.0.0.1") && !is_wide_bind("localhost") && !is_wide_bind("::1"));
        let error = build_rule(
            &map(json!({"type": "dynamic", "host_id": host, "bind_port": 1080, "dest_port": 22})),
            None,
        )
        .unwrap_err();
        assert_eq!(
            fields(error),
            [("dest_port".to_owned(), ValidationReason::NotAllowed)]
        );
        let error = build_rule(
            &map(json!({"type": "remote", "host_id": host, "bind_port": 8080})),
            None,
        )
        .unwrap_err();
        assert_eq!(
            fields(error),
            [
                ("dest_address".to_owned(), ValidationReason::Required),
                ("dest_port".to_owned(), ValidationReason::Required)
            ]
        );
        let error = build_rule(
            &map(json!({"type": "local", "host_id": host, "bind_port": 70000,
                "bind_address": "db.example", "dest_address": "x", "dest_port": 1})),
            None,
        )
        .unwrap_err();
        let errors = fields(error);
        assert!(errors.contains(&("bind_address".to_owned(), ValidationReason::Invalid)));
        assert!(errors.iter().any(|(field, _)| field == "bind_port"));
        let error = build_rule(&map(json!({})), None).unwrap_err();
        let errors = fields(error);
        for field in ["type", "host_id", "bind_port"] {
            assert!(
                errors.contains(&(field.to_owned(), ValidationReason::Required)),
                "{field}"
            );
        }
    }
}
