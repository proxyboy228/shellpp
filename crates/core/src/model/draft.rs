//! The draft rule (UI 1.3; D20; docs/data-model.md, section 2.1): a new object
//! is held in memory by the UI until all its required fields are valid; only
//! then is the first record version written. The core never stores drafts: a
//! `record_save` without `record_id` writes nothing unless the whole data
//! object validates, which is exactly what [`creatable`] reports.

use serde_json::{Map, Value};

use crate::model::group::{self, GroupRules};
use crate::model::host::{self, HostRules};
use crate::model::types::RecordKind;

/// `true` when `data` would be accepted as a new record of `kind` in the
/// personal vault (hosts and groups only).
pub fn creatable(kind: RecordKind, data: &Map<String, Value>) -> bool {
    match kind {
        RecordKind::Host => host::is_valid_draft(
            data,
            &HostRules {
                record_id: None,
                personal_vault: true,
            },
        ),
        RecordKind::Group => group::is_valid_draft(data, &GroupRules { record_id: None }),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(value: Value) -> Map<String, Value> {
        value.as_object().unwrap().clone()
    }

    #[test]
    fn required_fields_decide() {
        assert!(!creatable(RecordKind::Host, &map(json!({"label": "web"}))));
        assert!(creatable(
            RecordKind::Host,
            &map(json!({"address": "web.example.com"}))
        ));
        // Any invalid field keeps the object a draft.
        assert!(!creatable(
            RecordKind::Host,
            &map(json!({"address": "web.example.com", "ssh": {"port": "x"}}))
        ));
        assert!(!creatable(RecordKind::Group, &map(json!({}))));
        assert!(creatable(RecordKind::Group, &map(json!({"name": "prod"}))));
        assert!(!creatable(
            RecordKind::VaultMeta,
            &map(json!({"name": "x"}))
        ));
    }
}
