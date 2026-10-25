//! The HTTP side of the sync client (docs/sync-protocol.md, sections 2-8):
//! the server origin rules, a blocking reqwest client over rustls with the
//! trust of section 2.1, typed calls of the API and its errors.
//!
//! Secrets travel only in bodies and the `Authorization` header; requests
//! carry nothing else of ours than `User-Agent: shellpp/<version>`.

use std::io::Read as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use reqwest::blocking::Client;
use reqwest::{Method, StatusCode};
use serde::Serialize;
use serde::de::DeserializeOwned;
use shellpp_sync_api::api::{self as wire, ErrorBody};
use uuid::Uuid;

use super::tls::{self, Observation, SystemRoots, TlsProblem, TrustMode};

/// Connect timeout and whole-request timeout (section 2).
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);
/// A token refresh (D93: retried at once within the 60 s window).
pub const REFRESH_TIMEOUT: Duration = Duration::from_secs(20);

/// The largest answer the client reads (D188; docs/sync-protocol.md, section
/// 10.2). The largest legitimate one but a push is a pull page: at most
/// 8 MiB of envelopes plus the one that crossed it (262 232 bytes), under
/// 12 MiB as base64url in JSON; every other answer is far smaller.
pub const MAX_ANSWER_BYTES: usize = 16 * 1024 * 1024;

/// The largest answer to a push of `items` items: each may come back as a
/// conflict carrying the server's envelope (349 643 base64url characters at
/// most) with its record id, seq and status.
pub fn push_answer_limit(items: usize) -> usize {
    // The largest envelope as base64url without padding, and its record id,
    // seq and status.
    const ENVELOPE: usize = wire::limits::MAX_ENVELOPE_BYTES as usize;
    const ITEM: usize = (ENVELOPE * 4).div_ceil(3) + 512;
    MAX_ANSWER_BYTES.saturating_add(items.saturating_mul(ITEM))
}

/// Why a server address was refused before any request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OriginError {
    #[error("not an https:// address")]
    NotUrl,
    /// Plain HTTP is accepted only for localhost, 127.0.0.1 and [::1].
    #[error("http:// is accepted only for this computer")]
    HttpNotLocal,
}

/// The normalized origin of a server address (section 2): scheme and host in
/// lowercase, the port only when not the default, no path. A bare host is
/// taken as `https://`. Plain HTTP only for the loopback names.
pub fn normalize_origin(input: &str) -> Result<String, OriginError> {
    let input = input.trim();
    let with_scheme = if input.contains("://") {
        input.to_owned()
    } else {
        format!("https://{input}")
    };
    let (scheme, rest) = with_scheme.split_once("://").ok_or(OriginError::NotUrl)?;
    let scheme = scheme.to_ascii_lowercase();
    let default_port = match scheme.as_str() {
        "https" => 443,
        "http" => 80,
        _ => return Err(OriginError::NotUrl),
    };
    let authority = rest.split(['/', '?', '#']).next().unwrap_or_default();
    if authority.is_empty() || authority.contains('@') || authority.chars().any(char::is_whitespace)
    {
        return Err(OriginError::NotUrl);
    }
    let (host, port) = if let Some(stripped) = authority.strip_prefix('[') {
        let (host, after) = stripped.split_once(']').ok_or(OriginError::NotUrl)?;
        let port = match after.strip_prefix(':') {
            Some(port) => Some(port.parse::<u16>().map_err(|_| OriginError::NotUrl)?),
            None if after.is_empty() => None,
            None => return Err(OriginError::NotUrl),
        };
        (format!("[{}]", host.to_ascii_lowercase()), port)
    } else {
        match authority.rsplit_once(':') {
            Some((host, port)) => (
                host.to_ascii_lowercase(),
                Some(port.parse::<u16>().map_err(|_| OriginError::NotUrl)?),
            ),
            None => (authority.to_ascii_lowercase(), None),
        }
    };
    if host.is_empty() || host == "[]" {
        return Err(OriginError::NotUrl);
    }
    if scheme == "http" && !matches!(host.as_str(), "localhost" | "127.0.0.1" | "[::1]") {
        return Err(OriginError::HttpNotLocal);
    }
    Ok(match port {
        Some(port) if port != default_port => format!("{scheme}://{host}:{port}"),
        _ => format!("{scheme}://{host}"),
    })
}

/// A failed request.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum HttpError {
    #[error("invalid server address: {0}")]
    Origin(#[from] OriginError),
    /// No answer: no network, a refused or reset connection.
    #[error("the server did not answer")]
    Network,
    #[error("the request timed out")]
    Timeout,
    /// The certificate was refused (section 2.1).
    #[error("the server certificate was refused")]
    Tls(Box<TlsProblem>),
    /// The server answered with an error body of section 8.
    #[error("the server answered {status} {code}")]
    Api {
        status: u16,
        code: String,
        details: serde_json::Map<String, serde_json::Value>,
        retry_after: Option<u64>,
    },
    /// The answer is not what the API defines.
    #[error("the server sent a malformed answer")]
    Malformed,
    /// The client refused the answer: unsafe KDF parameters, a wrap that
    /// does not open, a rolled-back epoch, an unsupported account state.
    #[error("the server answer was refused: {0}")]
    Refused(&'static str),
}

impl HttpError {
    /// The error code of an API error.
    pub fn api_code(&self) -> Option<&str> {
        match self {
            Self::Api { code, .. } => Some(code),
            _ => None,
        }
    }

    pub fn status(&self) -> Option<u16> {
        match self {
            Self::Api { status, .. } => Some(*status),
            _ => None,
        }
    }

    /// Worth retrying later: no answer, `429` or a `5xx`.
    pub fn retryable(&self) -> bool {
        match self {
            Self::Network | Self::Timeout => true,
            Self::Api { status, .. } => *status == 429 || *status >= 500,
            _ => false,
        }
    }

    /// A short code for `sync_state.last_error` and the status line.
    pub fn code(&self) -> String {
        match self {
            Self::Origin(_) => "invalid_origin".to_owned(),
            Self::Network => "network".to_owned(),
            Self::Timeout => "timeout".to_owned(),
            Self::Tls(problem) if matches!(**problem, TlsProblem::PinChanged { .. }) => {
                "certificate_changed".to_owned()
            }
            Self::Tls(_) => "certificate_refused".to_owned(),
            Self::Api { code, .. } => code.clone(),
            Self::Malformed => "malformed_answer".to_owned(),
            Self::Refused(reason) => (*reason).to_owned(),
        }
    }
}

/// An error and its sources, as one line.
fn error_chain(error: &dyn std::error::Error) -> String {
    let mut text = error.to_string();
    let mut source = error.source();
    while let Some(next) = source {
        text.push_str(": ");
        text.push_str(&next.to_string());
        source = next.source();
    }
    text
}

/// The body of `response`, at most `limit` bytes; more is `answer_too_large`.
fn read_limited(response: reqwest::blocking::Response, limit: usize) -> Result<Vec<u8>, HttpError> {
    let too_large = || HttpError::Refused("answer_too_large");
    if response
        .content_length()
        .is_some_and(|length| length > limit as u64)
    {
        return Err(too_large());
    }
    let mut bytes = Vec::new();
    response
        .take((limit as u64).saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| {
            let timed_out = error.kind() == std::io::ErrorKind::TimedOut
                || error
                    .get_ref()
                    .and_then(|inner| inner.downcast_ref::<reqwest::Error>())
                    .is_some_and(reqwest::Error::is_timeout)
                || std::error::Error::source(&error)
                    .and_then(|source| source.downcast_ref::<reqwest::Error>())
                    .is_some_and(reqwest::Error::is_timeout);
            if timed_out {
                HttpError::Timeout
            } else {
                HttpError::Network
            }
        })?;
    if bytes.len() > limit {
        return Err(too_large());
    }
    Ok(bytes)
}

/// The client of one server origin.
pub struct HttpApi {
    origin: String,
    client: Client,
    observed: Observation,
    user_agent: String,
}

impl std::fmt::Debug for HttpApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpApi")
            .field("origin", &self.origin)
            .finish()
    }
}

impl HttpApi {
    /// A client for `origin` (normalized) with the given trust.
    pub fn new(origin: &str, trust: TrustMode, app_version: &str) -> Result<HttpApi, HttpError> {
        Self::with_roots(origin, trust, &SystemRoots::Platform, app_version)
    }

    /// A client for `origin` whose "system's trust" is `roots` (tests use a
    /// fixed set instead of the OS store).
    pub fn with_roots(
        origin: &str,
        trust: TrustMode,
        roots: &SystemRoots,
        app_version: &str,
    ) -> Result<HttpApi, HttpError> {
        let origin = normalize_origin(origin)?;
        let observed: Observation = Arc::new(Mutex::new(None));
        let config = tls::client_config(trust, roots, observed.clone())
            .map_err(|error| HttpError::Tls(Box::new(TlsProblem::Other(error.to_string()))))?;
        let client = Client::builder()
            .use_preconfigured_tls(config)
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .no_proxy()
            .build()
            .map_err(|_| HttpError::Network)?;
        Ok(HttpApi {
            origin,
            client,
            observed,
            user_agent: format!("shellpp/{app_version}"),
        })
    }

    pub fn origin(&self) -> &str {
        &self.origin
    }

    fn take_problem(&self) -> Option<TlsProblem> {
        self.observed
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take()
    }

    fn send<B: Serialize, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
        token: Option<&str>,
        timeout: Duration,
    ) -> Result<Option<T>, HttpError> {
        self.send_limited(method, path, body, token, timeout, MAX_ANSWER_BYTES)
    }

    /// [`Self::send`] reading at most `limit` bytes of the answer: a longer
    /// one (announced or sent) is refused as `answer_too_large` before it is
    /// parsed, so a hostile server cannot make the client hold more.
    fn send_limited<B: Serialize, T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&B>,
        token: Option<&str>,
        timeout: Duration,
        limit: usize,
    ) -> Result<Option<T>, HttpError> {
        let _ = self.take_problem();
        let mut request = self
            .client
            .request(method, format!("{}{path}", self.origin))
            .header(reqwest::header::USER_AGENT, &self.user_agent)
            .timeout(timeout);
        if let Some(token) = token {
            request = request.bearer_auth(token);
        }
        if let Some(body) = body {
            request = request.json(body);
        }
        let response = match request.send() {
            Ok(response) => response,
            Err(error) => {
                if let Some(problem) = self.take_problem() {
                    return Err(HttpError::Tls(Box::new(problem)));
                }
                // Why, for the log: the screens only say "network" or
                // "timeout" (after run 14 a phone said "network" with no
                // trace). Without the URL: the log names no server
                // (security.md 9.1); tokens go in a header anyway.
                let timeout = error.is_timeout();
                tracing::warn!(path, error = %error_chain(&error.without_url()), "sync request failed");
                return Err(if timeout {
                    HttpError::Timeout
                } else {
                    HttpError::Network
                });
            }
        };
        // Tests: the server handled the request, its answer is lost.
        #[cfg(any(test, feature = "test-hooks"))]
        if lose_answer::take(path) {
            return Err(HttpError::Network);
        }
        let status = response.status();
        let retry_after = response
            .headers()
            .get(reqwest::header::RETRY_AFTER)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.trim().parse::<u64>().ok());
        let bytes = read_limited(response, limit)?;
        if status.is_success() {
            if status == StatusCode::NO_CONTENT || bytes.is_empty() {
                return Ok(None);
            }
            return serde_json::from_slice(&bytes)
                .map(Some)
                .map_err(|_| HttpError::Malformed);
        }
        let (code, details) = match serde_json::from_slice::<ErrorBody>(&bytes) {
            Ok(body) => (body.error.code, body.error.details),
            Err(_) => (format!("http_{}", status.as_u16()), serde_json::Map::new()),
        };
        Err(HttpError::Api {
            status: status.as_u16(),
            code,
            details,
            retry_after,
        })
    }

    fn get<T: DeserializeOwned>(&self, path: &str, token: Option<&str>) -> Result<T, HttpError> {
        self.send::<(), T>(Method::GET, path, None, token, REQUEST_TIMEOUT)?
            .ok_or(HttpError::Malformed)
    }

    fn post<B: Serialize, T: DeserializeOwned>(
        &self,
        path: &str,
        body: &B,
        token: Option<&str>,
    ) -> Result<T, HttpError> {
        self.send::<B, T>(Method::POST, path, Some(body), token, REQUEST_TIMEOUT)?
            .ok_or(HttpError::Malformed)
    }

    fn post_empty<B: Serialize>(
        &self,
        path: &str,
        body: &B,
        token: Option<&str>,
    ) -> Result<(), HttpError> {
        self.send::<B, serde_json::Value>(Method::POST, path, Some(body), token, REQUEST_TIMEOUT)
            .map(|_| ())
    }

    pub fn server_info(&self) -> Result<wire::ServerInfo, HttpError> {
        self.get("/v1/server", None)
    }

    pub fn prelogin(&self, email: &str) -> Result<wire::PreloginResponse, HttpError> {
        self.post(
            "/v1/auth/prelogin",
            &wire::PreloginRequest {
                email: email.to_owned(),
            },
            None,
        )
    }

    pub fn register(
        &self,
        request: &wire::RegisterRequest,
    ) -> Result<wire::RegisterResponse, HttpError> {
        self.post("/v1/auth/register", request, None)
    }

    pub fn login(&self, request: &wire::LoginRequest) -> Result<wire::LoginResponse, HttpError> {
        self.post("/v1/auth/login", request, None)
    }

    pub fn login_totp(
        &self,
        request: &wire::LoginTotpRequest,
    ) -> Result<wire::LoginResponse, HttpError> {
        self.post("/v1/auth/login/totp", request, None)
    }

    pub fn refresh(&self, refresh_token: &str) -> Result<wire::Tokens, HttpError> {
        let body = wire::RefreshRequest {
            refresh_token: wire::Secret(refresh_token.to_owned()),
        };
        self.send::<_, wire::TokensResponse>(
            Method::POST,
            "/v1/auth/token/refresh",
            Some(&body),
            None,
            REFRESH_TIMEOUT,
        )?
        .map(|response| response.tokens)
        .ok_or(HttpError::Malformed)
    }

    pub fn revoke(&self, refresh_token: &str) -> Result<(), HttpError> {
        self.post_empty(
            "/v1/auth/token/revoke",
            &wire::RefreshRequest {
                refresh_token: wire::Secret(refresh_token.to_owned()),
            },
            None,
        )
    }

    pub fn recover_begin(
        &self,
        request: &wire::RecoverBeginRequest,
    ) -> Result<wire::RecoverBeginResponse, HttpError> {
        self.post("/v1/auth/recover/begin", request, None)
    }

    pub fn recover_complete(
        &self,
        request: &wire::RecoverCompleteRequest,
    ) -> Result<wire::Tokens, HttpError> {
        self.post::<_, wire::TokensResponse>("/v1/auth/recover/complete", request, None)
            .map(|response| response.tokens)
    }

    pub fn account(&self, access: &str) -> Result<wire::AccountBundle, HttpError> {
        self.get("/v1/account", Some(access))
    }

    pub fn password_change(
        &self,
        access: &str,
        request: &wire::PasswordChangeRequest,
    ) -> Result<wire::Tokens, HttpError> {
        self.post::<_, wire::TokensResponse>("/v1/account/password", request, Some(access))
            .map(|response| response.tokens)
    }

    pub fn recovery_key_reissue(
        &self,
        access: &str,
        request: &wire::RecoveryKeyReissueRequest,
    ) -> Result<wire::RecoveryKeyReissueResponse, HttpError> {
        self.post("/v1/account/recovery-key", request, Some(access))
    }

    pub fn credentials_change(
        &self,
        access: &str,
        request: &wire::CredentialsChangeRequest,
    ) -> Result<wire::RecoveryKeyReissueResponse, HttpError> {
        self.post("/v1/account/credentials", request, Some(access))
    }

    pub fn totp_setup(
        &self,
        access: &str,
        request: &wire::CurrentAuth,
    ) -> Result<wire::TotpSetupResponse, HttpError> {
        self.post("/v1/account/totp/setup", request, Some(access))
    }

    pub fn totp_enable(&self, access: &str, request: &wire::TotpCode) -> Result<(), HttpError> {
        self.post_empty("/v1/account/totp/enable", request, Some(access))
    }

    pub fn totp_disable(
        &self,
        access: &str,
        request: &wire::TotpDisableRequest,
    ) -> Result<(), HttpError> {
        self.post_empty("/v1/account/totp/disable", request, Some(access))
    }

    pub fn ssh_id(&self, access: &str, request: &wire::SshIdRequest) -> Result<(), HttpError> {
        self.send::<_, serde_json::Value>(
            Method::PUT,
            "/v1/account/ssh-id",
            Some(request),
            Some(access),
            REQUEST_TIMEOUT,
        )
        .map(|_| ())
    }

    pub fn account_delete(
        &self,
        access: &str,
        request: &wire::AccountDeleteRequest,
    ) -> Result<(), HttpError> {
        self.post_empty("/v1/account/delete", request, Some(access))
    }

    pub fn devices(&self, access: &str) -> Result<wire::DeviceList, HttpError> {
        self.get("/v1/devices", Some(access))
    }

    pub fn device_revoke(&self, access: &str, device_id: Uuid) -> Result<(), HttpError> {
        self.send::<(), serde_json::Value>(
            Method::DELETE,
            &format!("/v1/devices/{device_id}"),
            None,
            Some(access),
            REQUEST_TIMEOUT,
        )
        .map(|_| ())
    }

    pub fn vaults(&self, access: &str) -> Result<wire::VaultList, HttpError> {
        self.get("/v1/vaults", Some(access))
    }

    pub fn vault_keys(&self, access: &str, vault_id: Uuid) -> Result<wire::VaultKeys, HttpError> {
        self.get(&format!("/v1/vaults/{vault_id}/keys"), Some(access))
    }

    pub fn personal_key_epoch(
        &self,
        access: &str,
        vault_id: Uuid,
        request: &wire::PersonalKeyEpochRequest,
    ) -> Result<wire::KeyEpochResponse, HttpError> {
        self.post(
            &format!("/v1/vaults/{vault_id}/key-epochs"),
            request,
            Some(access),
        )
    }

    pub fn pull(
        &self,
        access: &str,
        vault_id: Uuid,
        since: u64,
        limit: Option<u64>,
        classes: Option<&[u8]>,
    ) -> Result<wire::PullResponse, HttpError> {
        let mut path = format!("/v1/vaults/{vault_id}/changes?since={since}");
        if let Some(limit) = limit {
            path.push_str(&format!("&limit={limit}"));
        }
        if let Some(classes) = classes {
            let list: Vec<String> = classes.iter().map(u8::to_string).collect();
            path.push_str(&format!("&classes={}", list.join(",")));
        }
        self.get(&path, Some(access))
    }

    pub fn push(
        &self,
        access: &str,
        vault_id: Uuid,
        items: Vec<wire::RecordWrite>,
    ) -> Result<wire::PushResponse, HttpError> {
        let limit = push_answer_limit(items.len());
        self.send_limited::<_, wire::PushResponse>(
            Method::POST,
            &format!("/v1/vaults/{vault_id}/changes"),
            Some(&wire::PushRequest { items }),
            Some(access),
            REQUEST_TIMEOUT,
            limit,
        )?
        .ok_or(HttpError::Malformed)
    }
}

/// Tests: the next answer to `path` on this thread is lost after the
/// server handled the request, as a dropped connection would lose it
/// (`test-hooks`; never in a release build).
#[cfg(any(test, feature = "test-hooks"))]
pub mod lose_answer {
    use std::cell::RefCell;

    thread_local! {
        static PATH: RefCell<Option<String>> = const { RefCell::new(None) };
    }

    /// The next answer to a path that starts with `prefix` (`/v1/...`) on
    /// this thread is lost.
    pub fn once(prefix: &str) {
        PATH.with(|next| *next.borrow_mut() = Some(prefix.to_owned()));
    }

    /// Whether the answer is still to be lost (no request matched yet).
    pub fn armed() -> bool {
        PATH.with(|next| next.borrow().is_some())
    }

    pub(super) fn take(path: &str) -> bool {
        PATH.with(|next| {
            let mut next = next.borrow_mut();
            if next
                .as_deref()
                .is_some_and(|prefix| path.starts_with(prefix))
            {
                *next = None;
                true
            } else {
                false
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn origins_follow_section_2() {
        assert_eq!(
            normalize_origin("Sync.Example.com").as_deref(),
            Ok("https://sync.example.com")
        );
        assert_eq!(
            normalize_origin("https://sync.example.com:443/v1/").as_deref(),
            Ok("https://sync.example.com")
        );
        assert_eq!(
            normalize_origin("https://Sync.Example.com:8443").as_deref(),
            Ok("https://sync.example.com:8443")
        );
        assert_eq!(
            normalize_origin("http://localhost:8080").as_deref(),
            Ok("http://localhost:8080")
        );
        assert_eq!(
            normalize_origin("http://127.0.0.1:1").as_deref(),
            Ok("http://127.0.0.1:1")
        );
        assert_eq!(
            normalize_origin("http://[::1]:9").as_deref(),
            Ok("http://[::1]:9")
        );
        // Plain HTTP for any other host is refused before a request.
        assert_eq!(
            normalize_origin("http://sync.example.com"),
            Err(OriginError::HttpNotLocal)
        );
        assert_eq!(
            normalize_origin("http://192.168.1.10:8080"),
            Err(OriginError::HttpNotLocal)
        );
        assert_eq!(normalize_origin("ftp://x"), Err(OriginError::NotUrl));
        assert_eq!(
            normalize_origin("https://user@host"),
            Err(OriginError::NotUrl)
        );
        assert_eq!(normalize_origin("https://"), Err(OriginError::NotUrl));
    }

    #[test]
    fn errors_have_codes_and_retry_rules() {
        assert!(HttpError::Network.retryable());
        let api = |status| HttpError::Api {
            status,
            code: "x".into(),
            details: Default::default(),
            retry_after: None,
        };
        assert!(api(503).retryable());
        assert!(api(429).retryable());
        assert!(!api(401).retryable());
        assert_eq!(api(401).code(), "x");
        assert_eq!(HttpError::Timeout.code(), "timeout");
    }
}
