//! The command dispatcher of architecture 2: `call(command, input)` runs one
//! command of the core's service API with a JSON input and answers
//! `{"ok": ...}` or `{"error": {"code", "message", "details"}}` with the
//! core's error codes. Plain Rust, tested on the computer.
//!
//! Inputs may carry secrets (the master password, record secrets): the JNI
//! layer hands them over in a buffer it wipes, they are parsed into
//! `SecretString`s, and only the command's name and an error's code are
//! ever logged.

use data_encoding::{BASE64, HEXLOWER};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use shellpp_core::crypto::KdfParams;
use shellpp_core::lock::FirstRunOptions;
use shellpp_core::service::dto::{
    FirstRunInput, KeyGenerateInput, KeyImportInput, PasswordInput, RecordMoveInput, RecordRef,
    RecordSaveInput, RecordsQuery, RecordsRef, RememberDeviceInput, SecretCopyTarget,
    SettingsSetInput,
};
use shellpp_core::service::keychain::KeyLabelInput;
use shellpp_core::service::{ServerCheck, SignInInput, SignInOutcome, SyncReport, TrustChoice};
use shellpp_core::sync::{CertInfo, TlsProblem, fingerprint_text, tls};
use shellpp_core::{Core, CoreError};
use uuid::Uuid;

/// `rustls-platform-verifier` was initialized (Android only; elsewhere the
/// platform verifier needs no setup). Without it the crate panics at the
/// first TLS connection, so every sync command checks it first
/// (architecture 5).
static TLS_READY: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(!cfg!(target_os = "android"));

/// Called once the JNI setup of the platform verifier ran.
pub fn tls_initialized() {
    TLS_READY.store(true, std::sync::atomic::Ordering::SeqCst);
}

/// Why a command failed.
enum Failure {
    Core(CoreError),
    /// The input was not the command's JSON.
    Input,
    Unknown,
}

impl From<CoreError> for Failure {
    fn from(error: CoreError) -> Self {
        Self::Core(error)
    }
}

/// The input of `recently_deleted_restore`.
#[derive(serde::Deserialize)]
struct RecordIds {
    record_ids: Vec<Uuid>,
}

/// The input of `key_label_set`.
#[derive(Deserialize)]
struct KeyLabelJson {
    key: RecordRef,
    label: String,
}

/// The input of `identity_save` (UI spec 7, run 15): `password` set or
/// cleared, or kept when missing.
#[derive(Deserialize)]
struct IdentityJson {
    #[serde(default)]
    record_id: Option<Uuid>,
    label: String,
    #[serde(default)]
    username: String,
    #[serde(default)]
    key_id: Option<Uuid>,
    /// Set when given; `clear_password` removes it; neither keeps it.
    #[serde(default)]
    password: Option<SecretString>,
    #[serde(default)]
    clear_password: bool,
    /// The identity as the dialog opened it (D221, D223 item 2): only the
    /// members the dialog changed are written onto the identity as it is now.
    #[serde(default)]
    base: Option<shellpp_core::service::dto::RecordBase>,
}

/// An identity as the phone shows it (no password, only whether one is set).
fn identity_json(view: &shellpp_core::service::keychain::IdentityView) -> Value {
    json!({
        "vault_id": view.vault_id,
        "record_id": view.record_id,
        "created_at": view.created_at,
        "label": view.label,
        "username": view.username,
        "key_id": view.key_id,
        "password_set": view.password_set,
        "revision": view.revision,
    })
}

/// The origin of a sync command.
#[derive(Deserialize)]
struct OriginInput {
    origin: String,
}

/// A trust choice of the certificate dialog (UI spec 11.1): the system's
/// roots, a pinned certificate (its SHA-256 in hex) or a CA (its DER in
/// base64, from `ca_file_info`).
#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum ChoiceInput {
    System,
    Pin { sha256: String },
    Ca { der: String },
}

#[derive(Deserialize)]
struct TrustInput {
    origin: String,
    choice: ChoiceInput,
}

/// Registration or sign-in (UI spec 11.1).
#[derive(Deserialize)]
struct SignInJson {
    origin: String,
    email: String,
    password: SecretString,
    #[serde(default)]
    registration_code: Option<SecretString>,
    /// The KDF settings the user accepted after a "weaker" or "changed" notice.
    #[serde(default)]
    accepted_kdf: Option<KdfParams>,
    /// The password or the email changed on another device (D200).
    #[serde(default)]
    after_kdf_change: bool,
}

#[derive(Deserialize)]
struct CodeInput {
    code: SecretString,
}

#[derive(Deserialize)]
struct SignOutInput {
    delete_local: bool,
}

#[derive(Deserialize)]
struct FileInput {
    /// The file's bytes in base64.
    file: String,
}

#[derive(Deserialize)]
struct ServerRecoveryInput {
    recovery_key: SecretString,
    new_password: SecretString,
    email: String,
    #[serde(default)]
    totp_code: Option<SecretString>,
}

fn cert_json(info: &CertInfo) -> Value {
    json!({
        "subject": info.subject,
        "issuer": info.issuer,
        "not_before": info.not_before,
        "not_after": info.not_after,
        "fingerprint": fingerprint_text(&info.sha256),
        "sha256": HEXLOWER.encode(&info.sha256),
    })
}

fn problem_json(problem: &TlsProblem) -> Value {
    match problem {
        TlsProblem::UnknownIssuer(info) => {
            json!({ "kind": "unknown_issuer", "cert": cert_json(info) })
        }
        TlsProblem::Expired(info) => json!({ "kind": "expired", "cert": cert_json(info) }),
        TlsProblem::WrongName(info) => json!({ "kind": "wrong_name", "cert": cert_json(info) }),
        TlsProblem::PinChanged {
            pinned,
            new,
            system_trusts,
        } => json!({
            "kind": "pin_changed",
            "pinned": fingerprint_text(pinned),
            "cert": cert_json(new),
            "system_trusts": system_trusts,
        }),
        TlsProblem::Other(reason) => json!({ "kind": "other", "reason": reason }),
    }
}

fn check_json(check: &ServerCheck) -> Value {
    match check {
        ServerCheck::Ready {
            origin,
            invite_only,
            version,
            ..
        } => json!({
            "kind": "ready",
            "origin": origin,
            "invite_only": invite_only,
            "version": version,
        }),
        ServerCheck::Certificate { origin, problem } => json!({
            "kind": "certificate",
            "origin": origin,
            "problem": problem_json(problem),
        }),
    }
}

fn outcome_json(outcome: SignInOutcome) -> Result<Value, Failure> {
    match outcome {
        SignInOutcome::Done(status) => Ok(json!({ "kind": "done", "status": out(Ok(status))? })),
        SignInOutcome::TotpRequired => Ok(json!({ "kind": "totp_required" })),
    }
}

/// The QR code of the `otpauth://` text (UI spec 11.1), as the desktop
/// draws it: `width` modules a side and `dark`, one `1` or `0` a module, row
/// by row; the app adds the quiet zone.
fn qr_json(text: &str) -> Option<Value> {
    let code = qrcode::QrCode::new(text.as_bytes()).ok()?;
    let dark: String = code
        .to_colors()
        .iter()
        .map(|color| {
            if *color == qrcode::Color::Dark {
                '1'
            } else {
                '0'
            }
        })
        .collect();
    Some(json!({ "width": code.width(), "dark": dark }))
}

fn report_json(report: &SyncReport) -> Value {
    json!({
        "pulled": report.pulled,
        "pushed": report.pushed,
        "conflicts": report.conflicts,
        "integrity_warnings": report.integrity_warnings,
        "failed": report.failed,
        "newer_epoch": report.newer_epoch,
        "brought_back": report.brought_back,
        "restored": report.restored,
    })
}

fn choice(input: ChoiceInput) -> Result<TrustChoice, Failure> {
    match input {
        ChoiceInput::System => Ok(TrustChoice::System),
        ChoiceInput::Pin { sha256 } => {
            let bytes = HEXLOWER
                .decode(sha256.to_ascii_lowercase().as_bytes())
                .map_err(|_| Failure::Input)?;
            Ok(TrustChoice::Pin(
                bytes.try_into().map_err(|_| Failure::Input)?,
            ))
        }
        ChoiceInput::Ca { der } => Ok(TrustChoice::Ca(
            BASE64.decode(der.as_bytes()).map_err(|_| Failure::Input)?,
        )),
    }
}

/// Sign-in and registration: the code is kept for the TOTP step by the core.
fn sign_in(core: &Core, input: SignInJson, register: bool) -> Result<Value, Failure> {
    let accepted = input.accepted_kdf;
    let after_change = input.after_kdf_change;
    let input = SignInInput {
        origin: input.origin,
        email: input.email,
        password: input.password,
        registration_code: input.registration_code,
    };
    if register {
        return Ok(json!({ "kind": "done", "status": out(core.sync_register(input))? }));
    }
    let outcome = if after_change {
        core.sync_sign_in_after_kdf_change(input, accepted)
    } else if let Some(accepted) = accepted {
        core.sync_sign_in_accepting_kdf(input, accepted)
    } else {
        core.sync_sign_in(input)
    }?;
    outcome_json(outcome)
}

fn parse<I: DeserializeOwned>(input: &[u8]) -> Result<I, Failure> {
    serde_json::from_slice(input).map_err(|_| Failure::Input)
}

fn out<O: Serialize>(value: Result<O, CoreError>) -> Result<Value, Failure> {
    let value = value?;
    serde_json::to_value(value).map_err(|_| Failure::Core(CoreError::Internal("answer")))
}

/// Runs `command` on `core` with the JSON `input` (empty for commands that
/// take none) and returns the answer as JSON text.
pub fn call(core: &Core, command: &str, input: &[u8]) -> String {
    let answer = match run(core, command, input) {
        Ok(value) => json!({ "ok": value }),
        Err(failure) => {
            let error = match failure {
                Failure::Core(error) => json!({
                    "code": error.code(),
                    "message": error.to_string(),
                    "details": error.details(),
                }),
                Failure::Input => json!({ "code": "bad_input", "message": "bad input" }),
                Failure::Unknown => {
                    json!({ "code": "unknown_command", "message": "unknown command" })
                }
            };
            tracing::debug!(command, code = %error["code"], "command failed");
            json!({ "error": error })
        }
    };
    answer.to_string()
}

fn run(core: &Core, command: &str, input: &[u8]) -> Result<Value, Failure> {
    let syncs = command.starts_with("sync_") || command == "recovery_unlock_server";
    if syncs && !TLS_READY.load(std::sync::atomic::Ordering::SeqCst) {
        return Err(Failure::Core(CoreError::Unavailable));
    }
    match command {
        // Lock (UI spec 4, 5).
        "lock_status" => out(Ok(core.lock_status())),
        "snapshot" => out(Ok(core.snapshot())),
        "first_run_setup" => {
            let input: FirstRunInput = parse(input)?;
            // The phone never offers "Remember on this device" at the first
            // run: fingerprint unlock is turned on in Settings (UI spec 4).
            out(core.first_run_setup_with(
                input.password,
                FirstRunOptions {
                    remember_device: false,
                },
            ))
        }
        "unlock" => {
            let input: PasswordInput = parse(input)?;
            out(core.unlock(input.password))
        }
        "try_device_unlock" => out(core.try_device_unlock()),
        // The master password again while unlocked (Settings, D218).
        "password_check" => {
            let input: PasswordInput = parse(input)?;
            out(core.password_check(input.password))
        }
        "lock" => out(core.lock()),
        "start_new_vault" => out(core.start_new_vault()),
        "remember_device_set" => {
            let input: RememberDeviceInput = parse(input)?;
            out(core.remember_device_set(input.enabled))
        }
        "activity" => {
            core.activity();
            out(Ok(Value::Null))
        }
        "tick" => {
            core.tick();
            out(Ok(Value::Null))
        }
        "auto_lock_due" => out(Ok(core.auto_lock_due())),
        // Settings.
        "settings_get" => out(core.settings_get()),
        "settings_set" => {
            let input: SettingsSetInput = parse(input)?;
            out(core.settings_set(input))
        }
        // Hosts and groups (UI spec 6).
        "vaults" => out(core.vaults()),
        "records_query" => {
            let input: RecordsQuery = parse(input)?;
            out(core.records_query(input))
        }
        "record_get" => {
            let input: RecordRef = parse(input)?;
            out(core.record_get(input))
        }
        "record_save" => {
            let input: RecordSaveInput = parse(input)?;
            out(core.record_save(input))
        }
        "record_delete" => {
            let input: RecordsRef = parse(input)?;
            out(core.record_delete(input))
        }
        "record_restore" => {
            let input: RecordsRef = parse(input)?;
            out(core.record_restore(input))
        }
        "record_move" => {
            let input: RecordMoveInput = parse(input)?;
            out(core.record_move(input))
        }
        "record_duplicate" => {
            let input: RecordRef = parse(input)?;
            out(core.record_duplicate(input))
        }
        "recently_deleted_list" => out(core.recently_deleted_list()),
        "recently_deleted_restore" => {
            let input: RecordIds = parse(input)?;
            out(core.recently_deleted_restore(&input.record_ids))
        }
        // The Keychain (UI spec 7, M4).
        "keys_list" => out(core.keys_list()),
        "key_generate" => {
            let input: KeyGenerateInput = parse(input)?;
            out(core.key_generate(input))
        }
        "key_import" => {
            let input: KeyImportInput = parse(input)?;
            out(core.key_import(input))
        }
        "key_public_key" => {
            let input: RecordRef = parse(input)?;
            out(core.key_public_key(input))
        }
        "key_users" => {
            let input: RecordRef = parse(input)?;
            out(core.key_users(input))
        }
        "key_label_set" => {
            let input: KeyLabelJson = parse(input)?;
            out(core.key_label_set(KeyLabelInput {
                key: input.key,
                label: input.label,
            }))
        }
        // Recent connections (the Connections tab, run 15).
        "recent_connections" => out(core.recent_connections()),
        // Identities (the editors' identity picker and the Keychain, run 15).
        "identities_list" => {
            let list = core.identities_list()?;
            Ok(Value::Array(list.iter().map(identity_json).collect()))
        }
        "identity_save" => {
            let input: IdentityJson = parse(input)?;
            let vault_id = core
                .vaults()?
                .first()
                .map(|vault| vault.vault_id)
                .ok_or(CoreError::Locked)?;
            let password = match (input.clear_password, input.password) {
                (true, _) => shellpp_core::model::identity::PasswordChange::Clear,
                (false, Some(value)) => shellpp_core::model::identity::PasswordChange::Set(value),
                (false, None) => shellpp_core::model::identity::PasswordChange::Keep,
            };
            let view = core.identity_save(shellpp_core::service::keychain::IdentitySaveInput {
                vault_id,
                record_id: input.record_id,
                label: input.label,
                username: input.username,
                key_id: input.key_id,
                password,
                base: input.base,
            })?;
            Ok(identity_json(&view))
        }
        // Known hosts (UI spec 8, M4).
        "known_hosts_list" => out(core.known_hosts_list()),
        "known_host_remove" => {
            let input: RecordsRef = parse(input)?;
            out(core.known_host_remove(input))
        }
        "known_host_restore" => {
            let input: RecordsRef = parse(input)?;
            out(core.known_host_restore(input))
        }
        // Sync (UI spec 11, M3).
        "sync_status" => out(core.sync_status()),
        "sync_server_check" => {
            let input: OriginInput = parse(input)?;
            Ok(check_json(&core.sync_server_check(&input.origin)?))
        }
        "sync_server_trust_verified" => {
            let input: TrustInput = parse(input)?;
            let choice = choice(input.choice)?;
            Ok(check_json(
                &core.sync_server_trust_verified(&input.origin, choice)?,
            ))
        }
        "sync_server_trust_info" => {
            let input: OriginInput = parse(input)?;
            out(core.sync_server_trust_info(&input.origin))
        }
        "ca_file_info" => {
            let input: FileInput = parse(input)?;
            let bytes = BASE64
                .decode(input.file.as_bytes())
                .map_err(|_| Failure::Input)?;
            let der = tls::ca_der_from_file(&bytes).ok_or(Failure::Core(CoreError::field(
                "file",
                shellpp_core::ValidationReason::Invalid,
            )))?;
            let info = tls::cert_info(&der).ok_or(Failure::Core(CoreError::field(
                "file",
                shellpp_core::ValidationReason::Invalid,
            )))?;
            Ok(json!({ "der": BASE64.encode(&der), "cert": cert_json(&info) }))
        }
        "sync_sign_in" => sign_in(core, parse(input)?, false),
        "sync_register" => sign_in(core, parse(input)?, true),
        "sync_sign_in_totp" => {
            let input: CodeInput = parse(input)?;
            outcome_json(core.sync_sign_in_totp(input.code.expose_secret())?)
        }
        "sync_sign_in_cancel" => {
            core.sync_sign_in_cancel();
            out(Ok(Value::Null))
        }
        "sync_weaker_kdf" => out(Ok(core.sync_weaker_kdf())),
        "recovery_key_granted" => out(core.recovery_key_granted()),
        "recovery_key_show" => {
            let input: PasswordInput = parse(input)?;
            out(core.recovery_key_show(input.password))
        }
        "secret_copy" => {
            let input: SecretCopyTarget = parse(input)?;
            out(core.secret_copy(input))
        }
        "sync_totp_setup" => {
            let input: PasswordInput = parse(input)?;
            let setup = core.sync_totp_setup(input.password)?;
            Ok(json!({
                "secret": setup.secret.expose_secret(),
                "otpauth_uri": setup.otpauth_uri.as_ref().map(|uri| uri.expose_secret().to_owned()),
                "qr": setup.otpauth_uri.as_ref().and_then(|uri| qr_json(uri.expose_secret())),
            }))
        }
        "sync_totp_enable" => {
            let input: CodeInput = parse(input)?;
            out(core.sync_totp_enable(input.code.expose_secret()))
        }
        "sync_now" => Ok(report_json(&core.sync_now()?)),
        "sync_auto" => Ok(report_json(&core.sync_auto()?)),
        "sync_due" => out(Ok(core.sync_due())),
        "sync_devices" => out(core.sync_devices()),
        "sync_sign_out" => {
            let input: SignOutInput = parse(input)?;
            out(core.sync_sign_out(input.delete_local))
        }
        "recovery_unlock_server" => {
            let input: ServerRecoveryInput = parse(input)?;
            let totp = input
                .totp_code
                .as_ref()
                .map(|code| code.expose_secret().to_owned());
            out(core.recovery_unlock_server(
                input.recovery_key,
                input.new_password,
                &input.email,
                totp,
            ))
        }
        _ => Err(Failure::Unknown),
    }
}

#[cfg(test)]
mod tests {
    use super::qr_json;

    #[test]
    fn qr_is_a_square_of_modules() {
        let qr = qr_json(
            "otpauth://totp/Shell%2B%2B:a%40b.example?secret=JBSWY3DPEHPK3PXP&issuer=Shell%2B%2B",
        )
        .expect("a QR code");
        let width = qr["width"].as_u64().expect("width") as usize;
        let dark = qr["dark"].as_str().expect("modules");
        assert!(width >= 21, "a QR code is at least 21 modules wide");
        assert_eq!(dark.len(), width * width);
        assert!(dark.chars().all(|c| c == '0' || c == '1'));
        // The finder pattern: the top left corner is dark.
        assert!(dark.starts_with("1111111"));
    }
}
