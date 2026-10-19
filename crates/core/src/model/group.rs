//! The `group` record (docs/data-model.md, section 7.1; stage 1 plan 5.5).
//!
//! Same building steps as [`crate::model::host::build`]. The tree rules
//! (`cycle`, `too_deep`) need the whole vault and are checked by the caller
//! with [`crate::model::tree::Tree`]; this module refuses only a group that
//! names itself as its parent.

use std::collections::BTreeMap;

use secrecy::SecretString;
use serde_json::{Map, Value, json};
use uuid::Uuid;

use crate::error::{FieldError, ValidationReason};
use crate::model::fields::{
    Errors, TAG_COLORS, opt_enum, opt_int, opt_object, opt_string, opt_text, opt_uuid,
    opt_uuid_list, present,
};
use crate::model::host::{MANAGED, MAX_JUMP_HOSTS, MAX_NOTES_BYTES};
use crate::model::merge::{keep_from, merge_level, object};
use crate::model::text;

const TOP_KNOWN: &[&str] = &[
    "name",
    "parent_id",
    "color",
    "notes",
    "ssh",
    "order_key",
    "import_key",
    "os",
];

const SSH_KNOWN: &[&str] = &["port", "username", "identity_id", "key_id", "jump_chain"];

/// Context of a group save.
#[derive(Clone, Copy, Debug)]
pub struct GroupRules {
    /// The record id when updating.
    pub record_id: Option<Uuid>,
}

/// Validates `input` and builds the data to store. Groups have no secret
/// members, so every `secrets` entry is `not_allowed`.
pub fn build(
    input: &Map<String, Value>,
    stored: Option<&Map<String, Value>>,
    secrets: &BTreeMap<String, Option<SecretString>>,
    rules: &GroupRules,
) -> Result<Map<String, Value>, Vec<FieldError>> {
    let mut errors = Errors::new();
    let normalized = normalize(input, rules, &mut errors);
    for path in secrets.keys() {
        errors.push(path.clone(), ValidationReason::NotAllowed);
    }
    let Some(mut normalized) = normalized.filter(|_| errors.is_empty()) else {
        return Err(errors.into_vec());
    };

    let ssh_normalized = match normalized.remove("ssh") {
        Some(Value::Object(ssh)) => ssh,
        _ => Map::new(),
    };
    let ssh = merge_level(
        SSH_KNOWN,
        ssh_normalized,
        object(Some(input), "ssh"),
        object(stored, "ssh"),
    );
    let mut data = merge_level(TOP_KNOWN, normalized, Some(input), stored);
    keep_from(&mut data, stored, MANAGED);
    data.insert("ssh".into(), Value::Object(ssh));
    Ok(data)
}

/// `true` when `data` alone (a draft) would be accepted as a new group.
pub fn is_valid_draft(input: &Map<String, Value>, rules: &GroupRules) -> bool {
    let mut errors = Errors::new();
    normalize(input, rules, &mut errors).is_some() && errors.is_empty()
}

fn normalize(
    input: &Map<String, Value>,
    rules: &GroupRules,
    errors: &mut Errors,
) -> Option<Map<String, Value>> {
    let before = errors.is_empty();
    let mut out = Map::new();
    match present(input, "name") {
        None => errors.push("name", ValidationReason::Required),
        Some(value) => match value.as_str().map(|raw| text::string(raw, 256)) {
            None => errors.push("name", ValidationReason::Invalid),
            Some(Ok(Some(name))) => {
                out.insert("name".into(), Value::String(name));
            }
            Some(Ok(None)) => errors.push("name", ValidationReason::Required),
            Some(Err(reason)) => errors.push("name", reason),
        },
    }
    if let Some(parent) = opt_uuid(input, "parent_id", "parent_id", errors) {
        if Some(parent) == rules.record_id {
            errors.push("parent_id", ValidationReason::Cycle);
        } else {
            out.insert("parent_id".into(), json!(parent));
        }
    }
    if let Some(color) = opt_enum(input, "color", "color", TAG_COLORS, errors) {
        out.insert("color".into(), json!(color));
    }
    if let Some(notes) = opt_text(input, "notes", "notes", MAX_NOTES_BYTES, errors) {
        out.insert("notes".into(), Value::String(notes));
    }
    let empty = Map::new();
    let ssh_input = opt_object(input, "ssh", "ssh", errors).unwrap_or(&empty);
    let mut ssh = Map::new();
    if let Some(port) = opt_int(ssh_input, "port", "ssh.port", 1, 65535, errors) {
        ssh.insert("port".into(), json!(port));
    }
    if let Some(username) = opt_string(ssh_input, "username", "ssh.username", 256, errors) {
        ssh.insert("username".into(), Value::String(username));
    }
    for member in ["identity_id", "key_id"] {
        let field = format!("ssh.{member}");
        if let Some(id) = opt_uuid(ssh_input, member, &field, errors) {
            ssh.insert(member.into(), json!(id));
        }
    }
    if let Some(chain) = opt_uuid_list(
        ssh_input,
        "jump_chain",
        "ssh.jump_chain",
        MAX_JUMP_HOSTS,
        None,
        errors,
    ) {
        ssh.insert("jump_chain".into(), json!(chain));
    }
    out.insert("ssh".into(), Value::Object(ssh));
    (before && errors.is_empty()).then_some(out)
}

/// The group name.
pub fn name(data: &Map<String, Value>) -> &str {
    data.get("name").and_then(Value::as_str).unwrap_or("")
}

/// The raw `parent_id` member.
pub fn parent_id(data: &Map<String, Value>) -> Option<Uuid> {
    data.get("parent_id")
        .and_then(Value::as_str)
        .and_then(|raw| Uuid::parse_str(raw).ok())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ValidationReason::*;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    fn errors_of(
        result: Result<Map<String, Value>, Vec<FieldError>>,
    ) -> Vec<(String, ValidationReason)> {
        result
            .unwrap_err()
            .into_iter()
            .map(|error| (error.field, error.reason))
            .collect()
    }

    #[test]
    fn name_is_required() {
        let rules = GroupRules { record_id: None };
        let none = BTreeMap::new();
        assert_eq!(
            errors_of(build(&map(json!({})), None, &none, &rules)),
            vec![("name".into(), Required)]
        );
        assert_eq!(
            errors_of(build(&map(json!({"name": "  "})), None, &none, &rules)),
            vec![("name".into(), Required)]
        );
        assert_eq!(
            errors_of(build(&map(json!({"name": 5})), None, &none, &rules)),
            vec![("name".into(), Invalid)]
        );
        assert_eq!(
            errors_of(build(
                &map(json!({"name": "x".repeat(257)})),
                None,
                &none,
                &rules
            )),
            vec![("name".into(), TooLong)]
        );
        assert!(!is_valid_draft(&map(json!({"color": "jade"})), &rules));
        assert!(is_valid_draft(&map(json!({"name": "prod"})), &rules));
        let data = build(
            &map(json!({"name": " prod ", "color": "jade"})),
            None,
            &none,
            &rules,
        )
        .unwrap();
        assert_eq!(
            Value::Object(data),
            json!({"name": "prod", "color": "jade", "ssh": {}})
        );
    }

    #[test]
    fn fields_are_validated() {
        let id = Uuid::now_v7();
        let rules = GroupRules {
            record_id: Some(id),
        };
        let mut secrets = BTreeMap::new();
        secrets.insert("ssh.password".to_owned(), None);
        let input = map(json!({
            "name": "g",
            "parent_id": id.to_string(),
            "color": "teal",
            "ssh": {"port": 0, "username": "", "key_id": "x", "jump_chain": "nope"}
        }));
        assert_eq!(
            errors_of(build(&input, None, &secrets, &rules)),
            vec![
                ("parent_id".into(), Cycle),
                ("color".into(), Invalid),
                ("ssh.port".into(), OutOfRange),
                ("ssh.key_id".into(), Invalid),
                ("ssh.jump_chain".into(), Invalid),
                ("ssh.password".into(), NotAllowed),
            ]
        );
    }

    #[test]
    fn unknown_members_are_kept() {
        let rules = GroupRules { record_id: None };
        let stored =
            map(json!({"name": "old", "order_key": "V", "x": 1, "ssh": {"port": 22, "x_ssh": 2}}));
        let input = map(json!({"name": "new", "ssh": {"username": "deploy"}}));
        let data = build(&input, Some(&stored), &BTreeMap::new(), &rules).unwrap();
        assert_eq!(
            Value::Object(data),
            json!({"name": "new", "order_key": "V", "x": 1, "ssh": {"username": "deploy", "x_ssh": 2}})
        );
    }
}
