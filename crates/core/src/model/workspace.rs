//! Workspaces (`workspace`, docs/data-model.md, section 12; UI 4.5): a saved
//! set of tabs and splits with linked hosts. The tabs have the shape of the
//! last session of Restore (`service::last_session`), checked by the same
//! rules; a workspace links saved hosts and local terminals only (a
//! quick-connect target is not a host link).
//!
//! `host_names` (D94) keeps the name each linked host had when the workspace
//! was saved, so that a host deleted since can still be named ("N hosts not
//! found", UI 6). The core writes it on every save; an import keeps the
//! file's.

use serde_json::{Map, Value, json};

use crate::error::{FieldError, ValidationReason};
use crate::model::fields::{Errors, present};
use crate::model::history_entry::{host_link, read_host_link};
use crate::model::merge::{keep_from, merge_level};
use crate::model::text;
use crate::service::dto::RecordRef;
use crate::service::last_session::{SavedSession, SavedTab, check_tab};

/// Longest name, in characters (`string(256)`).
pub const MAX_NAME_CHARS: usize = 256;
/// Most tabs of a workspace (`list<tab>[32]`).
pub const MAX_TABS: usize = 32;

/// Members a save never takes from its input: the source key of an import
/// (docs/data-model.md, section 3.3), kept on edit.
pub const MANAGED: &[&str] = &["import_key"];

const KNOWN: &[&str] = &["name", "tabs", "import_key", "host_names"];

/// Most entries of `host_names`: one per host the tabs link to.
pub const MAX_HOST_NAMES: usize = 512;
/// Longest name of `host_names`, in characters.
pub const MAX_HOST_NAME_CHARS: usize = 256;

/// Deepest JSON nesting of `tabs` read: a tab with splits four deep and its
/// sessions stay well below. Checked, with the size, before the tabs are
/// read: serde copies the subtree of an untagged pane at every level.
const MAX_TABS_DEPTH: usize = 24;

/// Whether `value` nests deeper than `max` levels.
fn deeper_than(value: &Value, max: usize) -> bool {
    let mut stack = vec![(value, 1usize)];
    while let Some((value, depth)) = stack.pop() {
        if depth > max {
            return true;
        }
        match value {
            Value::Array(items) => stack.extend(items.iter().map(|item| (item, depth + 1))),
            Value::Object(members) => {
                stack.extend(members.values().map(|member| (member, depth + 1)));
            }
            _ => {}
        }
    }
    false
}

/// Whether `value` is larger than a record's payload can be.
fn larger_than_a_record(value: &Value) -> bool {
    let mut counter = crate::model::payload::ByteCounter(0);
    serde_json::to_writer(&mut counter, value).is_err()
        || counter.0 > crate::crypto::MAX_PAYLOAD_LEN
}

/// The tabs of a stored or input workspace: `None` when they are missing or
/// not valid.
pub fn tabs(data: &Map<String, Value>) -> Option<Vec<SavedTab>> {
    serde_json::from_value(data.get("tabs")?.clone()).ok()
}

/// The saved hosts the tabs link to (SSH and Telnet sessions), each once,
/// in the order of the tabs.
pub fn host_links(tabs: &[SavedTab]) -> Vec<RecordRef> {
    let mut links: Vec<RecordRef> = Vec::new();
    for tab in tabs {
        for session in tab.root.sessions() {
            if let SavedSession::Host(link) | SavedSession::Telnet(link) = session
                && !links.iter().any(|known| known.record_id == link.record_id)
            {
                links.push(*link);
            }
        }
    }
    links
}

/// The names saved with a workspace (`host_names`), by host.
pub fn host_names(data: &Map<String, Value>) -> Vec<(RecordRef, String)> {
    data.get("host_names")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let link = read_host_link(entry.get("host"))?;
            let name = entry.get("name")?.as_str()?.to_owned();
            Some((link, name))
        })
        .collect()
}

/// One `host_names` entry.
pub fn host_name_entry(link: RecordRef, name: &str) -> Value {
    let name: String = name.chars().take(MAX_HOST_NAME_CHARS).collect();
    json!({"host": host_link(link), "name": name})
}

/// `host_names` checked: a list of at most [`MAX_HOST_NAMES`] objects with a
/// host link and a name.
fn check_host_names(value: &Value) -> Option<Vec<Value>> {
    let list = value.as_array()?;
    if list.len() > MAX_HOST_NAMES {
        return None;
    }
    list.iter()
        .map(|entry| {
            let entry = entry.as_object()?;
            if entry.keys().any(|key| key != "host" && key != "name") {
                return None;
            }
            let link = read_host_link(entry.get("host"))?;
            let name = entry.get("name")?.as_str()?;
            match text::string(name, MAX_HOST_NAME_CHARS) {
                Ok(Some(name)) => Some(host_name_entry(link, &name)),
                _ => None,
            }
        })
        .collect()
}

/// Checks and normalizes a workspace (`name`, `tabs` and `host_names`).
pub fn build(
    input: &Map<String, Value>,
    stored: Option<&Map<String, Value>>,
) -> Result<Map<String, Value>, Vec<FieldError>> {
    let mut errors = Errors::new();
    let mut normalized = Map::new();
    match present(input, "name")
        .map(|value| value.as_str().map(|raw| text::string(raw, MAX_NAME_CHARS)))
    {
        Some(Some(Ok(Some(name)))) => {
            normalized.insert("name".into(), Value::String(name));
        }
        None | Some(Some(Ok(None))) => errors.push("name", ValidationReason::Required),
        Some(Some(Err(reason))) => errors.push("name", reason),
        Some(None) => errors.push("name", ValidationReason::Invalid),
    }
    match present(input, "tabs") {
        None => errors.push("tabs", ValidationReason::Required),
        Some(value) if larger_than_a_record(value) => {
            errors.push("tabs", ValidationReason::TooLong);
        }
        Some(value) if deeper_than(value, MAX_TABS_DEPTH) => {
            errors.push("tabs", ValidationReason::Invalid);
        }
        Some(value) => match serde_json::from_value::<Vec<SavedTab>>(value.clone()) {
            Ok(list) if list.is_empty() => errors.push("tabs", ValidationReason::Required),
            Ok(list) if list.len() > MAX_TABS => errors.push("tabs", ValidationReason::TooMany),
            Ok(list) => {
                let quick = list.iter().any(|tab| {
                    tab.root
                        .sessions()
                        .iter()
                        .any(|session| matches!(session, SavedSession::Quick { .. }))
                });
                if quick || list.iter().any(|tab| check_tab(tab).is_err()) {
                    errors.push("tabs", ValidationReason::Invalid);
                } else {
                    match serde_json::to_value(&list) {
                        Ok(tabs) => {
                            normalized.insert("tabs".into(), tabs);
                        }
                        Err(_) => errors.push("tabs", ValidationReason::Invalid),
                    }
                }
            }
            Err(_) => errors.push("tabs", ValidationReason::Invalid),
        },
    }
    match input.get("host_names") {
        None | Some(Value::Null) => {}
        Some(value) => match check_host_names(value) {
            Some(list) => {
                normalized.insert("host_names".into(), Value::Array(list));
            }
            None => errors.push("host_names", ValidationReason::Invalid),
        },
    }
    if !errors.is_empty() {
        return Err(errors.into_vec());
    }
    let mut data = merge_level(KNOWN, normalized, Some(input), stored);
    keep_from(&mut data, stored, MANAGED);
    Ok(data)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().cloned().unwrap()
    }

    const HOST: &str = "0199f6a1-0000-7000-8000-0000000000b1";
    const VAULT: &str = "0199f6a1-0000-7000-8000-0000000000a1";

    #[test]
    fn a_workspace_has_a_name_and_linked_tabs() {
        let built = build(
            &map(json!({
                "name": "  Prod  ",
                "tabs": [
                    {"root": {"session": {"kind": "ssh", "host": {"vault_id": VAULT, "record_id": HOST}}}},
                    {"title": "db", "root": {"direction": "row", "sizes": [0.5, 0.5], "children": [
                        {"session": {"kind": "ssh", "host": {"vault_id": VAULT, "record_id": HOST}}},
                        {"session": {"kind": "local"}}
                    ]}}
                ]
            })),
            None,
        )
        .unwrap();
        assert_eq!(built["name"], json!("Prod"));
        assert_eq!(tabs(&built).unwrap().len(), 2);
        // The host linked twice is one link.
        assert_eq!(host_links(&tabs(&built).unwrap()).len(), 1);

        let error = |value: Value| build(&map(value), None).unwrap_err()[0].field.clone();
        assert_eq!(error(json!({"tabs": []})), "name");
        assert_eq!(error(json!({"name": "x", "tabs": []})), "tabs");
        // A quick-connect target is not a host link.
        assert_eq!(
            error(json!({"name": "x", "tabs": [
                {"root": {"session": {"kind": "ssh", "address": "192.0.2.1"}}}
            ]})),
            "tabs"
        );
        // Sizes must sum to 1.
        assert_eq!(
            error(
                json!({"name": "x", "tabs": [{"root": {"direction": "row", "sizes": [0.5, 0.2], "children": [
                    {"session": {"kind": "local"}}, {"session": {"kind": "local"}}
                ]}}]})
            ),
            "tabs"
        );
    }

    /// `host_names` (D94): kept as given when valid; a malformed list is
    /// refused; names are trimmed.
    #[test]
    fn host_names_are_checked() {
        let tabs = json!([{"root": {"session": {"kind": "telnet", "host": {"vault_id": VAULT, "record_id": HOST}}}}]);
        let built = build(
            &map(json!({"name": "x", "tabs": tabs.clone(), "host_names": [
                {"host": {"vault_id": VAULT, "record_id": HOST}, "name": "  router  "}
            ]})),
            None,
        )
        .unwrap();
        let names = host_names(&built);
        assert_eq!(names.len(), 1);
        assert_eq!(names[0].1, "router");
        assert_eq!(
            host_links(&super::tabs(&built).unwrap())[0]
                .record_id
                .to_string(),
            HOST
        );
        for bad in [
            json!("router"),
            json!([{"host": "nope", "name": "router"}]),
            json!([{"host": {"vault_id": VAULT, "record_id": HOST}, "name": ""}]),
            json!([{"host": {"vault_id": VAULT, "record_id": HOST}, "name": "a", "extra": 1}]),
        ] {
            let error = build(
                &map(json!({"name": "x", "tabs": tabs.clone(), "host_names": bad})),
                None,
            )
            .unwrap_err();
            assert_eq!(error[0].field, "host_names");
        }
    }
}
