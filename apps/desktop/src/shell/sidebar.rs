//! The sidebar of the Vaults tab (UI 1.2): six sections, 220 px, no vault
//! header (the vault is chosen on the Vaults tab).

use gpui::SharedString;
use gpui_kit::assets::IconName as Lucide;

use crate::i18n;

/// A sidebar section, top to bottom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Section {
    #[default]
    Hosts,
    Keychain,
    PortForwarding,
    Snippets,
    KnownHosts,
    Logs,
}

impl Section {
    /// All sections in sidebar order.
    pub const ALL: [Section; 6] = [
        Section::Hosts,
        Section::Keychain,
        Section::PortForwarding,
        Section::Snippets,
        Section::KnownHosts,
        Section::Logs,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Section::Hosts => i18n::nav::HOSTS,
            Section::Keychain => i18n::nav::KEYCHAIN,
            Section::PortForwarding => i18n::nav::PORT_FORWARDING,
            Section::Snippets => i18n::nav::SNIPPETS,
            Section::KnownHosts => i18n::nav::KNOWN_HOSTS,
            Section::Logs => i18n::nav::LOGS,
        }
    }

    /// Icons by meaning: server, key, arrows both ways, braces, fingerprint, scroll.
    pub fn icon(self) -> Lucide {
        match self {
            Section::Hosts => Lucide::Server,
            Section::Keychain => Lucide::KeyRound,
            Section::PortForwarding => Lucide::ArrowLeftRight,
            Section::Snippets => Lucide::Braces,
            Section::KnownHosts => Lucide::FingerprintPattern,
            Section::Logs => Lucide::ScrollText,
        }
    }

    /// Stable element id of the sidebar item (tests).
    pub fn element_id(self) -> SharedString {
        let id = match self {
            Section::Hosts => "nav-hosts",
            Section::Keychain => "nav-keychain",
            Section::PortForwarding => "nav-port-forwarding",
            Section::Snippets => "nav-snippets",
            Section::KnownHosts => "nav-known-hosts",
            Section::Logs => "nav-logs",
        };
        id.into()
    }

    /// Whether the section has a screen yet (the others show the placeholder).
    pub fn is_built(self) -> bool {
        matches!(self, Section::Hosts)
    }
}
