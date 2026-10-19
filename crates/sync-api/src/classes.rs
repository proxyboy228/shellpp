//! Record class codes (docs/data-model.md, section 2.2; docs/storage-format.md, section 5.1).
//!
//! The class is the clear-text u8 in the envelope header and the unit of the
//! server's class filter. Codes `0x00` and `0x0B`..=`0xFF` are reserved.
//!
//! Owner: W1.

/// Vault metadata, recovery key copy, device info and other vault-level records.
pub const VAULT: u8 = 0x01;
/// Hosts and groups.
pub const HOSTS: u8 = 0x02;
/// Keys and identities.
pub const KEYS: u8 = 0x03;
/// Snippets.
pub const SNIPPETS: u8 = 0x04;
/// Port forwarding rules.
pub const FORWARDING: u8 = 0x05;
/// Command history (decrypted lazily).
pub const HISTORY: u8 = 0x06;
/// Connection log (decrypted lazily).
pub const CONNECTION_LOG: u8 = 0x07;
/// Known hosts.
pub const KNOWN_HOSTS: u8 = 0x08;
/// Custom terminal themes and keyword highlighting rules.
pub const APPEARANCE: u8 = 0x09;
/// Workspaces.
pub const WORKSPACES: u8 = 0x0A;

/// Every known class code in ascending order.
pub const ALL: [u8; 10] = [
    VAULT,
    HOSTS,
    KEYS,
    SNIPPETS,
    FORWARDING,
    HISTORY,
    CONNECTION_LOG,
    KNOWN_HOSTS,
    APPEARANCE,
    WORKSPACES,
];

/// True for the codes `0x01..=0x0A`.
pub fn is_known(class: u8) -> bool {
    (VAULT..=WORKSPACES).contains(&class)
}

/// The class name used in documents and the sync API (`"vault"`, `"hosts"`, ...),
/// or `None` for a reserved code.
pub fn name(class: u8) -> Option<&'static str> {
    Some(match class {
        VAULT => "vault",
        HOSTS => "hosts",
        KEYS => "keys",
        SNIPPETS => "snippets",
        FORWARDING => "forwarding",
        HISTORY => "history",
        CONNECTION_LOG => "connection_log",
        KNOWN_HOSTS => "known_hosts",
        APPEARANCE => "appearance",
        WORKSPACES => "workspaces",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_codes_and_names() {
        assert!(!is_known(0x00));
        assert!(!is_known(0x0B));
        assert!(!is_known(0xFF));
        assert_eq!(name(0x00), None);
        assert_eq!(name(0x0B), None);
        let names: Vec<&str> = ALL.iter().map(|&c| name(c).unwrap()).collect();
        assert_eq!(
            names,
            [
                "vault",
                "hosts",
                "keys",
                "snippets",
                "forwarding",
                "history",
                "connection_log",
                "known_hosts",
                "appearance",
                "workspaces"
            ]
        );
        for code in 0..=u8::MAX {
            assert_eq!(is_known(code), ALL.contains(&code));
            assert_eq!(is_known(code), name(code).is_some());
        }
    }
}
