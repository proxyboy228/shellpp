//! Workspaces (`workspace`, docs/data-model.md, section 12; UI 4.5): a saved
//! set of tabs and splits with linked hosts. The tabs have the shape of the
//! last session of Restore (`service::last_session`), checked by the same
//! rules; a workspace links saved hosts and local terminals only (a
//! quick-connect target is not a host link).

use serde_json::{Map, Value};

use crate::error::{FieldError, ValidationReason};
use crate::model::fields::{Errors, present};
use crate::model::merge::{keep_from, merge_level};
use crate::model::text;
use crate::service::last_session::{SavedSession, SavedTab, check_tab};

/// Longest name, in characters (`string(256)`).
pub const MAX_NAME_CHARS: usize = 256;
/// Most tabs of a workspace (`list<tab>[32]`).
pub const MAX_TABS: usize = 32;

const KNOWN: &[&str] = &["name", "tabs"];

/// The tabs of a stored or input workspace: `None` when they are missing or
/// not valid.
pub fn tabs(data: &Map<String, Value>) -> Option<Vec<SavedTab>> {
    serde_json::from_value(data.get("tabs")?.clone()).ok()
}

/// Checks and normalizes a workspace (`name` and `tabs`).
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
    if !errors.is_empty() {
        return Err(errors.into_vec());
    }
    let mut data = merge_level(KNOWN, normalized, Some(input), stored);
    keep_from(&mut data, stored, &[]);
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
}
