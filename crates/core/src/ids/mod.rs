//! Technical identifiers (docs/architecture.md, section 7; docs/decisions.md, decision 2).
//!
//! The display name is "Shell++"; every technical identifier uses `shellpp`.
//! Values marked "frozen" must not change once users have data.
//!
//! Owner: W1.

use uuid::Uuid;

/// Display name shown in the UI.
pub const DISPLAY_NAME: &str = "Shell++";
/// Technical name (slug).
pub const SLUG: &str = "shellpp";
/// Bundle identifier, Tauri `identifier`, app id. Frozen.
pub const BUNDLE_ID: &str = "io.github.proxyboy228.shellpp";
/// Desktop binary name.
pub const BINARY_NAME: &str = "shellpp";
/// Project repository.
pub const REPOSITORY_URL: &str = "https://github.com/proxyboy228/shellpp";
/// Documentation link (Help menu, Settings).
pub const DOCUMENTATION_URL: &str = "https://github.com/proxyboy228/shellpp#readme";
/// "Report an issue" / "Report a Bug" link.
pub const ISSUES_URL: &str = "https://github.com/proxyboy228/shellpp/issues";
/// Changelog link (Settings).
pub const CHANGELOG_URL: &str = "https://github.com/proxyboy228/shellpp/releases";
/// URL scheme without `://`. Frozen.
pub const URL_SCHEME: &str = "shellpp";
/// Prefix of host links, followed by the record id (UI 2.4). Frozen.
pub const HOST_LINK_PREFIX: &str = "shellpp://host/";
/// Directory name used under the OS data, log and cache directories. Frozen.
pub const DATA_DIR_NAME: &str = "shellpp";
/// Database file name inside the data directory. Frozen.
pub const DB_FILE_NAME: &str = "shellpp.db";
/// Directory for pre-migration database copies inside the data directory.
pub const BACKUPS_DIR_NAME: &str = "backups";
/// Diagnostic log file name inside the logs directory.
pub const LOG_FILE_NAME: &str = "shellpp.log";
/// Environment variable that replaces the data directory (tests, portable use).
pub const ENV_DATA_DIR: &str = "SHELLPP_DATA_DIR";
/// OS keychain service. Frozen.
pub const KEYCHAIN_SERVICE: &str = "shellpp";
/// Prefix of the OS keychain account; the full account is `device-key:<db_id>`. Frozen.
pub const KEYCHAIN_ACCOUNT_PREFIX: &str = "device-key:";
/// Plain JSON export format id. Frozen.
pub const EXPORT_FORMAT_V1: &str = "shellpp.export.v1";
/// Encrypted JSON export format id. Frozen.
pub const EXPORT_FORMAT_ENCRYPTED_V1: &str = "shellpp.export.encrypted.v1";
/// Terminal theme schema id. Frozen.
pub const TERMINAL_THEME_SCHEMA_V1: &str = "shellpp.terminal-theme.v1";
/// Prefix of client HKDF, hash and AAD labels. Frozen.
pub const LABEL_PREFIX: &str = "shellpp/v1/";
/// Cargo feature that enables the e2e mocks in `shellpp-desktop`.
pub const E2E_FEATURE: &str = "e2e-mocks";
/// npm package of the desktop frontend.
pub const NPM_PACKAGE: &str = "@shellpp/desktop";
/// Prefix of invitation links (stage 8): `shellpp://invite/<token>?server=<origin>`. Frozen.
pub const INVITE_LINK_PREFIX: &str = "shellpp://invite/";
/// Prefix of server-only HKDF labels. Frozen.
pub const SERVER_LABEL_PREFIX: &str = "shellpp-server/v1/";
/// Prefix of server environment variables.
pub const SERVER_ENV_PREFIX: &str = "SHELLPP_";
/// Base path of the sync HTTP API.
pub const API_BASE_PATH: &str = "/v1";
/// Access token prefix. Frozen.
pub const ACCESS_TOKEN_PREFIX: &str = "shellpp_at_";
/// Refresh token prefix. Frozen.
pub const REFRESH_TOKEN_PREFIX: &str = "shellpp_rt_";
/// Login ticket prefix. Frozen.
pub const LOGIN_TICKET_PREFIX: &str = "shellpp_lt_";
/// Recovery ticket prefix. Frozen.
pub const RECOVERY_TICKET_PREFIX: &str = "shellpp_rc_";
/// User-Agent of update manifest requests (no version, no identifiers).
pub const UPDATE_USER_AGENT: &str = "shellpp";
/// Default update manifest URL (stage 9).
pub const UPDATE_MANIFEST_URL: &str =
    "https://github.com/proxyboy228/shellpp/releases/latest/download/latest.json";
/// Environment variable selecting the SSH test server (tests only: `openssh` or `russh`).
pub const ENV_TEST_SSHD: &str = "SHELLPP_TEST_SSHD";

/// User-Agent of sync API requests: `shellpp/<semver>`.
pub fn sync_user_agent(app_version: &str) -> String {
    format!("{SLUG}/{app_version}")
}

/// JSON Schema `$id`: `urn:shellpp:schema:<name>:v<N>`. Frozen.
pub fn schema_id(name: &str, version: u32) -> String {
    format!("urn:{SLUG}:schema:{name}:v{version}")
}

/// OS keychain account of the device key: `device-key:<db_id>` (lowercase hyphenated UUID).
pub fn keychain_account(db_id: &Uuid) -> String {
    format!("{KEYCHAIN_ACCOUNT_PREFIX}{}", db_id.hyphenated())
}

/// Host link for a record: `shellpp://host/<record_id>`.
pub fn host_link(record_id: &Uuid) -> String {
    format!("{HOST_LINK_PREFIX}{}", record_id.hyphenated())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keychain_account_uses_lowercase_hyphenated_uuid() {
        let id = Uuid::from_u128(0x0199_f6a1_b2c3_7d4e_8f00_1122_3344_5566);
        assert_eq!(
            keychain_account(&id),
            "device-key:0199f6a1-b2c3-7d4e-8f00-112233445566"
        );
        assert_eq!(
            host_link(&id),
            "shellpp://host/0199f6a1-b2c3-7d4e-8f00-112233445566"
        );
    }

    #[test]
    fn derived_identifiers() {
        assert_eq!(sync_user_agent("0.1.0"), "shellpp/0.1.0");
        assert_eq!(schema_id("export", 1), "urn:shellpp:schema:export:v1");
        assert!(HOST_LINK_PREFIX.starts_with(URL_SCHEME));
        assert!(INVITE_LINK_PREFIX.starts_with(URL_SCHEME));
        assert!(DOCUMENTATION_URL.starts_with(REPOSITORY_URL));
        assert!(UPDATE_MANIFEST_URL.starts_with(REPOSITORY_URL));
        assert_eq!(keychain_account(&Uuid::nil()).len(), 11 + 36);
    }
}
