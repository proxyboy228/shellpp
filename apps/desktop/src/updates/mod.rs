//! The update check (UI 1.4 Help > Check for updates, 12.7; D7, D97):
//! notification only. One HTTPS GET of the update manifest (the release's
//! `latest.json`, its `version` compared with this build's by semver),
//! without identifiers: the only header of ours is
//! `User-Agent: shellpp/<version>`. Nothing is downloaded or installed; an
//! update found is a toast and an item of the Help menu that open the
//! release page in the system browser.
//!
//! Manual "Check now" works always; the check at launch only when
//! `updates.auto_check` is on (off by default, principle 2), at most once a
//! day (`updates.last_check`).
//!
//! A manifest counts only with its minisign signature (`<manifest>.minisig`)
//! made with the release key built into the app (D170, [`signature`]); a
//! build without that key asks nothing and reports that it cannot check.

use std::io::Read as _;
use std::time::Duration;

use gpui::{App, Global};
use serde_json::Value;

pub mod signature;
pub mod strings;
#[cfg(test)]
mod tests;

/// The manifest of the project's latest release (docs/architecture.md, 7).
pub const DEFAULT_MANIFEST_URL: &str =
    "https://github.com/proxyboy228/shellpp/releases/latest/download/latest.json";

/// The settings of the check (data-model 16.1).
pub const AUTO_CHECK: &str = "updates.auto_check";
pub const MANIFEST_URL: &str = "updates.manifest_url";
pub const LAST_CHECK: &str = "updates.last_check";

/// The check at launch waits this long after the last one.
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// A request gives up after this long (redirects included).
const TIMEOUT: Duration = Duration::from_secs(15);
/// The largest manifest read.
const MAX_MANIFEST_BYTES: u64 = 256 * 1024;

/// This build's version.
pub fn current_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

/// What a check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// A newer release: its version and the page to open.
    Newer { version: String, page: String },
    /// This build is the latest (or newer than the manifest's).
    UpToDate,
}

/// Why a check did not answer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CheckError {
    /// The source is not an `https://` URL (plain HTTP only to this
    /// machine, for tests).
    #[error("the update source is not an https:// address")]
    NotHttps,
    /// No answer: no network, a refused or timed-out connection, an HTTP
    /// error status.
    #[error("the update source did not answer")]
    Unreachable,
    /// An answer that is not a manifest with a semver `version`.
    #[error("the update source did not send an update manifest")]
    BadManifest,
    /// This build has no release key (`release/minisign.pub`), so no
    /// manifest could count: nothing was asked.
    #[error("this build has no key to check updates with")]
    NoKey,
    /// The manifest has no signature of this build's release key: missing,
    /// made with another key, or the manifest changed.
    #[error("the update manifest is not signed with the release key")]
    BadSignature,
}

/// The update found by the last check of this run (the Help menu's item).
#[derive(Clone, Debug, Default)]
pub struct Found(pub Option<(String, String)>);

impl Global for Found {}

/// The update found in this run: its version and page.
pub fn found(cx: &App) -> Option<(String, String)> {
    cx.try_global::<Found>().and_then(|found| found.0.clone())
}

/// Keeps what a check found for the Help menu (an answer replaces it; a
/// failure leaves it), and when the source was last asked
/// (`updates.last_check`: the check at launch is daily, failed or not).
pub fn remember(
    result: &Result<Outcome, CheckError>,
    core: Option<&shellpp_core::Core>,
    cx: &mut App,
) {
    if let Ok(outcome) = result {
        let found = match outcome {
            Outcome::Newer { version, page } => Some((version.clone(), page.clone())),
            Outcome::UpToDate => None,
        };
        cx.set_global(Found(found));
    }
    let now = now();
    if let Some(core) = core
        && let Err(error) = core.settings_set(shellpp_core::service::dto::SettingsSetInput {
            values: [(LAST_CHECK.to_owned(), serde_json::json!(now))]
                .into_iter()
                .collect(),
            reset: Vec::new(),
        })
    {
        tracing::warn!(
            code = error.code(),
            "the time of the update check was not kept"
        );
    }
}

/// Seconds since the epoch, now.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| since.as_secs())
}

/// The source of the settings: the URL set, else the default.
pub fn manifest_url(values: &serde_json::Map<String, Value>) -> String {
    values
        .get(MANIFEST_URL)
        .and_then(Value::as_str)
        .filter(|url| !url.trim().is_empty())
        .unwrap_or(DEFAULT_MANIFEST_URL)
        .to_owned()
}

/// Whether the check at launch is due: on, and the last one at least
/// [`CHECK_INTERVAL`] before `now` (seconds since the epoch).
pub fn due_at_launch(values: &serde_json::Map<String, Value>, now: u64) -> bool {
    let on = values
        .get(AUTO_CHECK)
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let last = values.get(LAST_CHECK).and_then(Value::as_u64).unwrap_or(0);
    // A last check in the future (the clock was ahead, or went back) does
    // not stop the check.
    on && (last > now || now - last >= CHECK_INTERVAL.as_secs())
}

/// The check at launch (blocking): asks the source of `values` only when
/// [`due_at_launch`]; `None` without a request.
pub fn at_launch(
    values: &serde_json::Map<String, Value>,
    now: u64,
) -> Option<Result<Outcome, CheckError>> {
    at_launch_with(values, now, signature::PublicKey::release().as_ref())
}

/// [`at_launch`] with the release key `key`.
fn at_launch_with(
    values: &serde_json::Map<String, Value>,
    now: u64,
    key: Option<&signature::PublicKey>,
) -> Option<Result<Outcome, CheckError>> {
    due_at_launch(values, now).then(|| check_with(&manifest_url(values), current_version(), key))
}

/// Whether `url` may be asked: `Ok(true)` for `https://`, `Ok(false)` for
/// plain HTTP to this machine's loopback (the tests). The host is parsed;
/// a user part (`user@`) is refused, so no `Authorization` is sent and
/// `http://127.0.0.1:@elsewhere/` is not "this machine".
fn allowed(url: &str) -> Result<bool, CheckError> {
    let uri: ureq::http::Uri = url.trim().parse().map_err(|_| CheckError::NotHttps)?;
    let authority = uri.authority().ok_or(CheckError::NotHttps)?;
    if authority.as_str().contains('@') {
        return Err(CheckError::NotHttps);
    }
    match uri.scheme_str().map(str::to_ascii_lowercase).as_deref() {
        Some("https") => Ok(true),
        Some("http") if matches!(authority.host(), "127.0.0.1" | "localhost" | "[::1]") => {
            Ok(false)
        }
        _ => Err(CheckError::NotHttps),
    }
}

/// Asks `url` for the manifest and its signature, checks the signature
/// with the release key and compares the manifest's version with `current`
/// (blocking: run it off the UI thread).
pub fn check(url: &str, current: &str) -> Result<Outcome, CheckError> {
    check_with(url, current, signature::PublicKey::release().as_ref())
}

/// [`check`] with the release key `key`; without one nothing is asked.
fn check_with(
    url: &str,
    current: &str,
    key: Option<&signature::PublicKey>,
) -> Result<Outcome, CheckError> {
    let https = allowed(url)?;
    let key = key.ok_or(CheckError::NoKey)?;
    let mut config = ureq::Agent::config_builder();
    if !https {
        // This machine: never through a proxy of the environment.
        config = config.proxy(None);
    }
    let agent: ureq::Agent = config
        .timeout_global(Some(TIMEOUT))
        .https_only(https)
        .max_redirects(5)
        .http_status_as_error(true)
        .user_agent(format!("shellpp/{current}"))
        .build()
        .into();
    // The manifest and its signature are two requests, and "latest" can
    // move between them (a release published in that second): a mismatch
    // asks for both once more.
    let mut retried = false;
    loop {
        let body = fetch(&agent, url.trim(), MAX_MANIFEST_BYTES).map_err(|error| match error {
            Fetch::TooLarge => CheckError::BadManifest,
            Fetch::Status(_) | Fetch::Failed => CheckError::Unreachable,
        })?;
        // A missing signature (a client error status) or one too large is
        // not the release key's; a server that is down or busy (5xx, 429)
        // or no answer at all is "did not answer".
        let sig = fetch(
            &agent,
            &signature::signature_url(url),
            signature::MAX_SIGNATURE_BYTES,
        )
        .map_err(|error| match error {
            Fetch::Status(429 | 500..) | Fetch::Failed => CheckError::Unreachable,
            Fetch::Status(_) | Fetch::TooLarge => CheckError::BadSignature,
        })?;
        let sig = String::from_utf8(sig).map_err(|_| CheckError::BadSignature)?;
        match key.verify(&body, &sig) {
            Ok(comment) if names_manifest(comment, &body) => return compare(&body, current),
            Err(signature::SignatureError::Mismatch) if !retried => retried = true,
            _ => return Err(CheckError::BadSignature),
        }
    }
}

/// Whether a verified trusted comment is the one the release workflow
/// writes for `body`: `shellpp <version> latest.json`, with the manifest's
/// version (a leading `v` aside). Another signed file of a release is not a
/// manifest. A manifest without a version is left to [`compare`].
fn names_manifest(comment: &str, body: &[u8]) -> bool {
    let Some(named) = comment
        .strip_prefix("shellpp ")
        .and_then(|rest| rest.strip_suffix(" latest.json"))
    else {
        return false;
    };
    let unprefixed = |text: &str| text.trim().trim_start_matches('v').to_owned();
    serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|manifest| manifest.get("version")?.as_str().map(unprefixed))
        .is_none_or(|version| version == unprefixed(named))
}

/// Why [`fetch`] got no body.
enum Fetch {
    /// An HTTP error status.
    Status(u16),
    /// More than the limit.
    TooLarge,
    /// No answer: no network, a refused or timed-out connection.
    Failed,
}

/// One GET of `url`: its body, at most `max` bytes.
fn fetch(agent: &ureq::Agent, url: &str, max: u64) -> Result<Vec<u8>, Fetch> {
    let mut response = agent.get(url).call().map_err(|error| match error {
        ureq::Error::StatusCode(status) => Fetch::Status(status),
        _ => Fetch::Failed,
    })?;
    let mut body = Vec::new();
    response
        .body_mut()
        .as_reader()
        .take(max + 1)
        .read_to_end(&mut body)
        .map_err(|_| Fetch::Failed)?;
    if body.len() as u64 > max {
        return Err(Fetch::TooLarge);
    }
    Ok(body)
}

/// The manifest `body` against `current`: `version` (a leading `v` taken
/// off) newer by semver is an update; its page is the manifest's `url`
/// when it is `https://`, else the release tag's page of the repository.
pub fn compare(body: &[u8], current: &str) -> Result<Outcome, CheckError> {
    let manifest: Value = serde_json::from_slice(body).map_err(|_| CheckError::BadManifest)?;
    let text = manifest
        .get("version")
        .and_then(Value::as_str)
        .ok_or(CheckError::BadManifest)?
        .trim();
    let text = text.strip_prefix('v').unwrap_or(text);
    let latest = semver::Version::parse(text).map_err(|_| CheckError::BadManifest)?;
    let ours = semver::Version::parse(current).map_err(|_| CheckError::BadManifest)?;
    // Semver precedence: build metadata (`+ci.5`) does not count.
    if latest.cmp_precedence(&ours) != std::cmp::Ordering::Greater {
        return Ok(Outcome::UpToDate);
    }
    let page = manifest
        .get("url")
        .and_then(Value::as_str)
        .filter(|url| url.starts_with("https://") && url.len() <= 2048)
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{}/releases/tag/v{latest}", env!("CARGO_PKG_REPOSITORY")));
    Ok(Outcome::Newer {
        version: latest.to_string(),
        page,
    })
}
