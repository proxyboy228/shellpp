//! Record type codes and names (docs/data-model.md, section 2.3).
//!
//! Type code = u16 `class << 8 | kind`; the payload carries the name and its
//! class must equal the envelope class (docs/storage-format.md, section 5.4,
//! check 7).

use shellpp_sync_api::classes;

use crate::service::dto::RecordType;

/// The record types stage 1 reads and writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RecordKind {
    VaultMeta,
    RecoveryKey,
    DeviceInfo,
    Host,
    Group,
}

impl RecordKind {
    pub const ALL: [RecordKind; 5] = [
        RecordKind::VaultMeta,
        RecordKind::RecoveryKey,
        RecordKind::DeviceInfo,
        RecordKind::Host,
        RecordKind::Group,
    ];

    /// The u16 type code.
    pub fn code(self) -> u16 {
        match self {
            Self::VaultMeta => 0x0101,
            Self::RecoveryKey => 0x0104,
            Self::DeviceInfo => 0x0106,
            Self::Host => 0x0201,
            Self::Group => 0x0202,
        }
    }

    /// The envelope class: `code >> 8`.
    pub fn class(self) -> u8 {
        (self.code() >> 8) as u8
    }

    /// The payload `type` name.
    pub fn name(self) -> &'static str {
        match self {
            Self::VaultMeta => "vault_meta",
            Self::RecoveryKey => "recovery_key",
            Self::DeviceInfo => "device_info",
            Self::Host => "host",
            Self::Group => "group",
        }
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.name() == name)
    }

    /// The payload version this app writes and understands.
    pub fn current_version(self) -> u32 {
        1
    }

    /// The webview type, for hosts and groups.
    pub fn record_type(self) -> Option<RecordType> {
        match self {
            Self::Host => Some(RecordType::Host),
            Self::Group => Some(RecordType::Group),
            _ => None,
        }
    }

    pub fn from_record_type(record_type: RecordType) -> Self {
        match record_type {
            RecordType::Host => Self::Host,
            RecordType::Group => Self::Group,
        }
    }

    /// Dotted paths of the secret members of the type's `data`.
    pub fn secret_paths(self) -> &'static [&'static str] {
        match self {
            Self::Host => crate::model::host::SECRET_PATHS,
            _ => &[],
        }
    }

    /// Singletons: at most one effective live record per vault.
    pub fn is_singleton(self) -> bool {
        matches!(self, Self::VaultMeta | Self::RecoveryKey)
    }
}

/// The class of every type name defined in docs/data-model.md, section 2.3,
/// including types of later stages, so that check 7 also applies to records
/// this app keeps without understanding them. `None` for unknown names and for
/// `conflict` copies, whose class is the class of their original.
pub fn class_of_type_name(name: &str) -> Option<u8> {
    let class = match name {
        "vault_meta" | "user_keypair" | "ssh_id" | "recovery_key" | "member_pin"
        | "device_info" | "roster_entry" => classes::VAULT,
        "host" | "group" | "credential_binding" => classes::HOSTS,
        "key" | "identity" => classes::KEYS,
        "snippet" | "snippet_package" => classes::SNIPPETS,
        "forwarding_rule" => classes::FORWARDING,
        "history_entry" | "history_hint_state" => classes::HISTORY,
        "connection_log_entry" => classes::CONNECTION_LOG,
        "known_host" => classes::KNOWN_HOSTS,
        "terminal_theme" | "highlight_rule" => classes::APPEARANCE,
        "workspace" => classes::WORKSPACES,
        _ => return None,
    };
    Some(class)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codes_classes_and_names_agree() {
        for kind in RecordKind::ALL {
            assert_eq!(RecordKind::from_name(kind.name()), Some(kind));
            assert_eq!(class_of_type_name(kind.name()), Some(kind.class()));
        }
        assert_eq!(RecordKind::Host.code(), 0x0201);
        assert_eq!(RecordKind::Host.class(), 0x02);
        assert_eq!(RecordKind::VaultMeta.class(), 0x01);
        assert_eq!(RecordKind::from_name("key"), None);
        assert_eq!(class_of_type_name("key"), Some(0x03));
        assert_eq!(class_of_type_name("conflict"), None);
        assert_eq!(class_of_type_name("unknown_type"), None);
    }

    #[test]
    fn secret_paths_only_for_hosts() {
        assert_eq!(
            RecordKind::Host.secret_paths(),
            &["ssh.password", "ssh.proxy.password"]
        );
        assert!(RecordKind::Group.secret_paths().is_empty());
    }
}
