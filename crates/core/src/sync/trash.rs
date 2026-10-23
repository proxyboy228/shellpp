//! Recently deleted (data-model 16.6; UI 14; D93): which types it keeps,
//! and which types a resync only counts instead of listing them.

use crate::model::types::RecordKind;

/// Types whose deletion keeps the last live version for 90 days.
/// Not kept: history entries and hint states (deleting one removes a
/// secret), bookkeeping types, the last session and conflict copies (those
/// are not record kinds of the index at all).
pub fn kept(kind: RecordKind) -> bool {
    matches!(
        kind,
        RecordKind::Group
            | RecordKind::Host
            | RecordKind::Key
            | RecordKind::Identity
            | RecordKind::Snippet
            | RecordKind::SnippetPackage
            | RecordKind::ForwardingRule
            | RecordKind::KnownHost
            | RecordKind::Workspace
            | RecordKind::HighlightRule
            | RecordKind::ConnectionLogEntry
    )
}

/// Whether a type name is one the user can delete in the interface (a
/// resync lists those it pushed back; the others are only counted,
/// docs/sync-protocol.md, section 6.9, step 5).
pub fn deletable_type(type_name: &str) -> bool {
    !matches!(
        type_name,
        "vault_meta"
            | "user_keypair"
            | "ssh_id"
            | "recovery_key"
            | "member_pin"
            | "roster_entry"
            | "device_info"
            | "history_hint_state"
            | "last_session"
    )
}

/// How long an entry stays (D93): 90 days.
pub const KEEP_DAYS: u64 = 90;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kept_types_follow_data_model_16_6() {
        assert!(kept(RecordKind::Host));
        assert!(kept(RecordKind::ConnectionLogEntry));
        assert!(!kept(RecordKind::HistoryEntry));
        assert!(!kept(RecordKind::HistoryHintState));
        assert!(!kept(RecordKind::VaultMeta));
        assert!(!kept(RecordKind::RecoveryKey));
        assert!(!kept(RecordKind::DeviceInfo));
        assert!(!kept(RecordKind::LastSession));
        assert!(deletable_type("host"));
        assert!(!deletable_type("recovery_key"));
    }
}
