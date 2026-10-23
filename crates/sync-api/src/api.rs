//! Request and response bodies of the sync API v1 (docs/sync-protocol.md,
//! sections 3-8). Shared by the client (`shellpp-core`) and the server
//! (`shellpp-server`); no cryptography and no I/O.
//!
//! Binary values are base64url without padding on the wire ([`B64`]);
//! timestamps are RFC 3339 UTC text with milliseconds, kept as strings here.
//! Unknown JSON fields are ignored on both sides (section 13): no type denies
//! them. Fields that only the team vault endpoints use (stage 8) are left out
//! until those endpoints exist.

use std::fmt;

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use uuid::Uuid;

/// The API version this crate describes.
pub const API_VERSION: &str = "v1";

/// Binary data, base64url without padding on the wire.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct B64(pub Vec<u8>);

impl B64 {
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self(bytes.into())
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// The bytes when they are exactly `N` long.
    pub fn exact<const N: usize>(&self) -> Option<[u8; N]> {
        self.0.as_slice().try_into().ok()
    }
}

impl fmt::Debug for B64 {
    /// Lengths only: wraps, keys and envelopes never reach a log.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "B64({} bytes)", self.0.len())
    }
}

impl Serialize for B64 {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&data_encoding::BASE64URL_NOPAD.encode(&self.0))
    }
}

impl<'de> Deserialize<'de> for B64 {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        data_encoding::BASE64URL_NOPAD
            .decode(text.as_bytes())
            .map(B64)
            .map_err(|_| serde::de::Error::custom("invalid base64url"))
    }
}

/// A secret string (auth keys, tokens, codes): `Debug` never shows it.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Secret(pub String);

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(..)")
    }
}

/// KDF parameters (docs/security.md, section 5.4), the JSON of `local_auth.kdf`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Kdf {
    pub alg: String,
    pub v: u32,
    pub m_kib: u32,
    pub t: u32,
    pub p: u32,
}

impl Kdf {
    /// Within the floor and the mobile ceiling (section 4.2; the server
    /// checks these on register, password change, recovery and email change).
    pub fn server_acceptable(&self) -> bool {
        self.alg == "argon2id"
            && self.v == 1
            && self.m_kib >= 32_768
            && self.t >= 3
            && u64::from(self.m_kib) * u64::from(self.t) >= 131_072
            && (1..=16).contains(&self.p)
            && self.m_kib <= 262_144
            && self.t <= 16
    }

    /// The desktop preset (64 MiB, t = 3, p = 4).
    pub fn desktop() -> Self {
        Self {
            alg: "argon2id".to_owned(),
            v: 1,
            m_kib: 65_536,
            t: 3,
            p: 4,
        }
    }
}

/// `GET /v1/server` (section 3.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub version: String,
    pub api_versions: Vec<String>,
    pub registration: Registration,
    pub smtp: bool,
    pub limits: Limits,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_generation: Option<B64>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Registration {
    Open,
    Invite,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    pub max_envelope_bytes: u64,
    pub max_push_items: u64,
    pub max_push_body_bytes: u64,
    pub default_pull_limit: u64,
    pub max_pull_limit: u64,
    pub tombstone_retention_days: u64,
}

/// `{"status": "ok"}` of `/healthz` and `/readyz`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Health {
    pub status: String,
}

/// `POST /v1/auth/prelogin`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreloginRequest {
    pub email: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreloginResponse {
    pub kdf: Kdf,
    pub kdf_salt: B64,
}

/// One epoch of a personal vault's chain at registration (section 4.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChainLink {
    pub epoch: u32,
    pub prev_key_wrap: B64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalVaultRegistration {
    pub vault_id: Uuid,
    pub epoch: u32,
    pub kek_wrap: B64,
    pub recovery_wrap: B64,
    #[serde(default)]
    pub epoch_chain: Vec<ChainLink>,
}

/// `POST /v1/auth/register`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub registration_code: Option<Secret>,
    pub kdf: Kdf,
    pub kdf_salt: B64,
    pub auth_key: B64,
    pub recovery_auth_key: B64,
    pub personal_vault: PersonalVaultRegistration,
    pub x25519_public_key: B64,
    pub ssh_id_public_key: String,
    pub device_id: Uuid,
}

/// The tokens of every response that issues them (section 4.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Tokens {
    pub access_token: Secret,
    pub access_expires_at: String,
    pub refresh_token: Secret,
    pub refresh_expires_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegisterResponse {
    pub user_id: Uuid,
    pub tokens: Tokens,
}

/// `POST /v1/auth/login`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub auth_key: B64,
    pub device_id: Uuid,
}

/// The answer of a login: tokens and the account, or the TOTP step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub login_ticket: Option<Secret>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Tokens>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<AccountBundle>,
}

/// `POST /v1/auth/login/totp`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoginTotpRequest {
    pub login_ticket: Secret,
    pub code: Secret,
}

/// The personal vault in the account bundle.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalVaultState {
    pub vault_id: Uuid,
    pub epoch: u32,
    pub kek_wrap: B64,
    pub recovery_wrap: B64,
}

/// The storage limits of an account and its usage (D93).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Quota {
    pub max_records: Option<u64>,
    pub max_bytes: Option<u64>,
    pub records: u64,
    pub bytes: u64,
}

/// `GET /v1/account` and the `account` of a login (section 4.4).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountBundle {
    pub user_id: Uuid,
    pub email: String,
    pub email_verified: bool,
    pub totp_enabled: bool,
    pub kdf: Kdf,
    pub kdf_salt: B64,
    pub personal_vault: PersonalVaultState,
    pub x25519_public_key: B64,
    pub ssh_id_public_key: String,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quota: Option<Quota>,
}

/// `POST /v1/auth/token/refresh` and `/revoke`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshRequest {
    pub refresh_token: Secret,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TokensResponse {
    pub tokens: Tokens,
}

/// The new epoch carried by a password change, a recovery or a reissue
/// (section 5.1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rotation {
    pub epoch: u32,
    pub prev_key_wrap: B64,
    pub recovery_wrap: B64,
}

/// `POST /v1/account/password`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PasswordChangeRequest {
    pub current_auth_key: B64,
    pub kdf: Kdf,
    pub kdf_salt: B64,
    pub auth_key: B64,
    pub kek_wrap: B64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rotation: Option<Rotation>,
}

/// `POST /v1/auth/recover/begin`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverBeginRequest {
    pub email: String,
    pub recovery_auth_key: B64,
    pub device_id: Uuid,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverBeginResponse {
    pub recovery_ticket: Secret,
    pub email: String,
    pub recovery_wrap: B64,
    pub personal_vault_id: Uuid,
    pub epoch: u32,
    pub totp_required: bool,
}

/// `POST /v1/auth/recover/complete`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverCompleteRequest {
    pub recovery_ticket: Secret,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totp_code: Option<Secret>,
    pub email: String,
    pub kdf: Kdf,
    pub kdf_salt: B64,
    pub auth_key: B64,
    pub kek_wrap: B64,
    pub rotation: Rotation,
    pub device_id: Uuid,
}

/// A record pushed together with another request (the reissue's
/// `recovery_key` record, section 5.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecordWrite {
    pub base_revision: u64,
    pub envelope: B64,
}

/// `POST /v1/account/recovery-key`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryKeyReissueRequest {
    pub current_auth_key: B64,
    pub recovery_auth_key: B64,
    pub kek_wrap: B64,
    pub rotation: Rotation,
    pub record: RecordWrite,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryKeyReissueResponse {
    pub tokens: Tokens,
    pub seq: u64,
}

/// `POST /v1/account/email` (without SMTP the change takes effect at once).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailChangeRequest {
    pub current_auth_key: B64,
    pub new_email: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kdf: Option<Kdf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kdf_salt: Option<B64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_key: Option<B64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kek_wrap: Option<B64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EmailChangeResponse {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email_verified: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tokens: Option<Tokens>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<String>,
}

/// `POST /v1/account/totp/setup` and the requests that only prove the password.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CurrentAuth {
    pub current_auth_key: B64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TotpSetupResponse {
    pub secret: Secret,
    pub otpauth_uri: Secret,
}

/// `POST /v1/account/totp/enable`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TotpCode {
    pub code: Secret,
}

/// `POST /v1/account/totp/disable`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TotpDisableRequest {
    pub current_auth_key: B64,
    pub code: Secret,
}

/// `PUT /v1/account/ssh-id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshIdRequest {
    pub ssh_id_public_key: String,
}

/// `POST /v1/account/delete`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountDeleteRequest {
    pub current_auth_key: B64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub totp_code: Option<Secret>,
}

/// `GET /v1/devices`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Device {
    pub device_id: Uuid,
    pub created_at: String,
    pub last_seen_at: String,
    pub revoked: bool,
    pub current: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceList {
    pub devices: Vec<Device>,
}

/// `GET /v1/vaults` (section 7.2).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultSummary {
    pub vault_id: Uuid,
    pub kind: String,
    pub role: String,
    pub current_epoch: u32,
    pub last_seq: u64,
    pub rotation_pending: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultList {
    pub vaults: Vec<VaultSummary>,
}

/// One epoch of `GET /v1/vaults/{id}/keys`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EpochInfo {
    pub epoch: u32,
    pub prev_key_wrap: Option<B64>,
    pub created_by: Uuid,
    pub created_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct VaultKeys {
    pub current_epoch: u32,
    pub epoch_chain: Vec<EpochInfo>,
    /// HPKE wraps of a team vault (stage 8); empty for the personal vault.
    #[serde(default)]
    pub wraps: Vec<serde_json::Value>,
}

/// The `kek_wrap` and `recovery_wrap` of a personal vault epoch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountWraps {
    pub kek_wrap: B64,
    pub recovery_wrap: B64,
}

/// `POST /v1/vaults/{personal}/key-epochs` (section 7.7; the re-post of
/// section 5.1 uses it too).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalKeyEpochRequest {
    pub current_auth_key: B64,
    pub epoch: u32,
    pub prev_key_wrap: B64,
    pub account_wraps: AccountWraps,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEpochResponse {
    pub current_epoch: u32,
}

/// One record of a pull or of a conflict's `current`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SeqEnvelope {
    pub seq: u64,
    pub envelope: B64,
}

/// `GET /v1/vaults/{id}/changes` (section 6.3).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PullResponse {
    pub records: Vec<SeqEnvelope>,
    pub next_cursor: u64,
    pub has_more: bool,
    pub current_epoch: u32,
    pub min_valid_seq: u64,
}

/// `POST /v1/vaults/{id}/changes` (section 6.5).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushRequest {
    pub items: Vec<RecordWrite>,
}

/// Item statuses of a push (sections 6.5, 8).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemStatus {
    Ok,
    Conflict,
    StaleKeyEpoch,
    InvalidEnvelope,
    InsufficientRole,
    ClassNotAllowed,
    PayloadTooLarge,
    QuotaExceeded,
    /// A status this client does not know (a newer server): keep pending.
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushResult {
    pub record_id: Uuid,
    pub status: ItemStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seq: Option<u64>,
    /// For `conflict`: the stored version, or `null` when the record is absent.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "nullable_current"
    )]
    pub current: Option<Option<SeqEnvelope>>,
}

/// `current` is three-state on the wire: absent, `null` or a version.
mod nullable_current {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::SeqEnvelope;

    pub fn serialize<S: Serializer>(
        value: &Option<Option<SeqEnvelope>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        match value {
            Some(inner) => inner.serialize(serializer),
            None => serializer.serialize_none(),
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Option<Option<SeqEnvelope>>, D::Error> {
        Option::<SeqEnvelope>::deserialize(deserializer).map(Some)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushResponse {
    pub results: Vec<PushResult>,
}

/// The error body of every failed request (section 8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorBody {
    pub error: ErrorInfo,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ErrorInfo {
    pub code: String,
    pub message: String,
    #[serde(default)]
    pub details: serde_json::Map<String, serde_json::Value>,
}

/// Error codes of section 8.
pub mod code {
    pub const BAD_REQUEST: &str = "bad_request";
    pub const INVALID_ENVELOPE: &str = "invalid_envelope";
    pub const WEAK_KDF_PARAMS: &str = "weak_kdf_params";
    pub const UNAUTHENTICATED: &str = "unauthenticated";
    pub const INVALID_CREDENTIALS: &str = "invalid_credentials";
    pub const INVALID_TOTP: &str = "invalid_totp";
    pub const TICKET_EXPIRED: &str = "ticket_expired";
    pub const REFRESH_TOKEN_REUSED: &str = "refresh_token_reused";
    pub const ACCOUNT_DELETED: &str = "account_deleted";
    pub const FORBIDDEN: &str = "forbidden";
    pub const NOT_MEMBER: &str = "not_member";
    pub const INSUFFICIENT_ROLE: &str = "insufficient_role";
    pub const CLASS_NOT_ALLOWED: &str = "class_not_allowed";
    pub const REGISTRATION_CLOSED: &str = "registration_closed";
    pub const ACCOUNT_DISABLED: &str = "account_disabled";
    pub const NOT_FOUND: &str = "not_found";
    pub const REVISION_CONFLICT: &str = "revision_conflict";
    pub const STALE_KEY_EPOCH: &str = "stale_key_epoch";
    pub const EPOCH_CONFLICT: &str = "epoch_conflict";
    pub const EMAIL_TAKEN: &str = "email_taken";
    pub const SOLE_OWNER: &str = "sole_owner";
    pub const CURSOR_EXPIRED: &str = "cursor_expired";
    pub const PAYLOAD_TOO_LARGE: &str = "payload_too_large";
    pub const VALIDATION_FAILED: &str = "validation_failed";
    pub const RATE_LIMITED: &str = "rate_limited";
    pub const INTERNAL: &str = "internal";
    pub const UNAVAILABLE: &str = "unavailable";
}

/// Token prefixes (docs/architecture.md, section 7).
pub mod token_prefix {
    pub const ACCESS: &str = "shellpp_at_";
    pub const REFRESH: &str = "shellpp_rt_";
    pub const LOGIN_TICKET: &str = "shellpp_lt_";
    pub const RECOVERY_TICKET: &str = "shellpp_rc_";
}

/// Size limits (section 10.2).
pub mod limits {
    pub const MAX_ENVELOPE_BYTES: u64 = 262_232;
    pub const MAX_PUSH_ITEMS: u64 = 500;
    pub const MAX_PUSH_BODY_BYTES: u64 = 16 * 1024 * 1024;
    pub const DEFAULT_PULL_LIMIT: u64 = 500;
    pub const MAX_PULL_LIMIT: u64 = 1000;
    /// A pull stops after this many envelope bytes (always one record at least).
    pub const MAX_PULL_BYTES: u64 = 8 * 1024 * 1024;
    pub const TOMBSTONE_RETENTION_DAYS: u64 = 90;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn b64_round_trip_and_debug_hides_bytes() {
        let value = B64(vec![0xfb, 0xff, 0x00, 0x01]);
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(json, "\"-_8AAQ\"");
        assert_eq!(serde_json::from_str::<B64>(&json).unwrap(), value);
        assert_eq!(format!("{value:?}"), "B64(4 bytes)");
        assert!(serde_json::from_str::<B64>("\"-_8AAQ==\"").is_err());
        assert_eq!(value.exact::<4>(), Some([0xfb, 0xff, 0x00, 0x01]));
        assert_eq!(value.exact::<3>(), None);
    }

    #[test]
    fn secrets_never_show_in_debug() {
        let token = Secret("shellpp_rt_abc".to_owned());
        assert_eq!(format!("{token:?}"), "Secret(..)");
    }

    #[test]
    fn push_result_current_is_three_state() {
        let absent = PushResult {
            record_id: Uuid::nil(),
            status: ItemStatus::Ok,
            seq: Some(3),
            current: None,
        };
        let json = serde_json::to_value(&absent).unwrap();
        assert!(json.get("current").is_none());
        let null = PushResult {
            status: ItemStatus::Conflict,
            seq: None,
            current: Some(None),
            ..absent.clone()
        };
        let json = serde_json::to_string(&null).unwrap();
        assert!(json.contains("\"current\":null"), "{json}");
        let back: PushResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.current, Some(None));
        let unknown: PushResult = serde_json::from_str(
            r#"{"record_id":"00000000-0000-0000-0000-000000000000","status":"brand_new"}"#,
        )
        .unwrap();
        assert_eq!(unknown.status, ItemStatus::Unknown);
        assert_eq!(unknown.current, None);
    }

    #[test]
    fn kdf_bounds_of_the_server() {
        assert!(Kdf::desktop().server_acceptable());
        let mobile = Kdf {
            m_kib: 32_768,
            t: 4,
            p: 2,
            ..Kdf::desktop()
        };
        assert!(mobile.server_acceptable());
        let cheap = Kdf {
            m_kib: 32_768,
            t: 3,
            ..Kdf::desktop()
        };
        assert!(!cheap.server_acceptable());
        let huge = Kdf {
            m_kib: 1_048_576,
            ..Kdf::desktop()
        };
        assert!(!huge.server_acceptable(), "above the mobile ceiling");
    }

    #[test]
    fn unknown_fields_are_ignored() {
        let info: PreloginResponse =
            serde_json::from_str(r#"{"kdf":{"alg":"argon2id","v":1,"m_kib":65536,"t":3,"p":4,"x":1},"kdf_salt":"AAAA","extra":true}"#)
                .unwrap();
        assert_eq!(info.kdf, Kdf::desktop());
    }
}
