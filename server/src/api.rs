//! The HTTP API v1 (docs/sync-protocol.md, section 3): routes, extractors
//! (JSON bodies, path parameters and query strings with section 8 errors,
//! the bearer token, the client IP) and
//! thin handlers over the operations in `accounts` and `records`. The team
//! vault endpoints come with stage 8.

use std::net::{IpAddr, Ipv6Addr, SocketAddr};
use std::sync::Arc;

use axum::extract::rejection::JsonRejection;
use axum::extract::{
    ConnectInfo, DefaultBodyLimit, FromRequest, FromRequestParts, Path, Query, Request, State,
};
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use shellpp_sync_api::api::{
    API_VERSION, AccountDeleteRequest, B64, CurrentAuth, EmailChangeRequest, Health,
    KeyEpochResponse, Limits, LoginRequest, LoginTotpRequest, PasswordChangeRequest,
    PersonalKeyEpochRequest, PreloginRequest, PushRequest, RecoverBeginRequest,
    RecoverCompleteRequest, RecoveryKeyReissueRequest, RefreshRequest, RegisterRequest, ServerInfo,
    SshIdRequest, TokensResponse, TotpCode, TotpDisableRequest, code, limits, token_prefix,
};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::accounts::{self, CheckedKey, KeyKind, Principal};
use crate::db::TxKind;
use crate::error::{ApiError, ApiResult};
use crate::model;
use crate::records;
use crate::state::Shared;

type AppState = Arc<Shared>;

/// A JSON body; a malformed one is `400 bad_request`, an oversized one
/// `413 payload_too_large` (section 8).
pub struct Body<T>(pub T);

impl<T, S> FromRequest<S> for Body<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(request: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(request, state).await {
            Ok(Json(value)) => Ok(Body(value)),
            Err(JsonRejection::BytesRejection(rejection))
                if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE =>
            {
                Err(ApiError::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    code::PAYLOAD_TOO_LARGE,
                    "request body too large",
                ))
            }
            Err(_) => Err(ApiError::bad_request("malformed JSON body")),
        }
    }
}

/// Path parameters (the ids in `/v1/devices/{device_id}` and
/// `/v1/vaults/{vault_id}/...`); a malformed one is `400 bad_request`
/// with the error body of section 8, not axum's own plain text.
pub struct PathParam<T>(pub T);

impl<T, S> FromRequestParts<S> for PathParam<T>
where
    T: serde::de::DeserializeOwned + Send,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Path::<T>::from_request_parts(parts, state).await {
            Ok(Path(value)) => Ok(PathParam(value)),
            Err(_) => Err(ApiError::bad_request("malformed path parameter")),
        }
    }
}

/// A query string; a malformed one is `400 bad_request` (section 8).
pub struct QueryParams<T>(pub T);

impl<T, S> FromRequestParts<S> for QueryParams<T>
where
    T: serde::de::DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match Query::<T>::from_request_parts(parts, state).await {
            Ok(Query(value)) => Ok(QueryParams(value)),
            Err(_) => Err(ApiError::bad_request("malformed query")),
        }
    }
}

/// The signed-in device of a request with `Authorization: Bearer`.
pub struct Auth(pub Principal);

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        // Bogus tokens are counted per client IP and stopped before the
        // database once over the limit (each lookup is a write transaction,
        // the global lock on SQLite; review finding).
        let ClientIp(ip) = ClientIp::from_request_parts(parts, state).await?;
        state
            .limiters
            .token_allowed(ip)
            .map_err(ApiError::rate_limited)?;
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::trim)
            .filter(|token| token.starts_with(token_prefix::ACCESS))
            // Wiped on drop (the header map's own copy is hyper's).
            .map(|token| Zeroizing::new(token.to_owned()));
        let Some(token) = token else {
            state.limiters.token_failed(ip);
            return Err(ApiError::unauthenticated());
        };
        // Only a token the server never issued counts: an expired one, or
        // one of a revoked device, is the ordinary 401 every client meets
        // when its access token runs out (review finding: counting those
        // locked out many devices behind one address).
        let principal = match state
            .db
            .run(TxKind::Write, move |tx| {
                accounts::authenticate_token(tx, &token, model::now())
            })
            .await?
        {
            Ok(principal) => principal,
            Err(accounts::TokenRefusal::Unknown) => {
                state.limiters.token_failed(ip);
                return Err(ApiError::unauthenticated());
            }
            Err(accounts::TokenRefusal::Refused) => return Err(ApiError::unauthenticated()),
        };
        state
            .limiters
            .api(principal.device_id)
            .map_err(ApiError::rate_limited)?;
        Ok(Auth(principal))
    }
}

/// The client's IP: the socket peer, or the last address in
/// `X-Forwarded-For` that is not a trusted proxy when the peer is one
/// (docs/security.md, section 14.4). Used in memory only.
pub struct ClientIp(pub IpAddr);

impl FromRequestParts<AppState> for ClientIp {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let peer = parts
            .extensions
            .get::<ConnectInfo<SocketAddr>>()
            .map(|info| info.0.ip().to_canonical())
            .unwrap_or(IpAddr::from([127, 0, 0, 1]));
        let trusted = |ip: IpAddr| {
            state
                .config
                .trusted_proxies
                .iter()
                .any(|net| net.contains(ip))
        };
        let forwarded = parts
            .headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|value| value.to_str().ok());
        if !trusted(peer) && parts.headers.contains_key("x-forwarded-for") {
            note_untrusted_forwarding();
        }
        Ok(ClientIp(client_ip(peer, forwarded, trusted)))
    }
}

/// Logs once per process that `X-Forwarded-For` came from a peer outside
/// `SHELLPP_TRUSTED_PROXIES` and was ignored: behind a proxy that is not
/// trusted, every client shares the proxy's address in the rate limits
/// (D176). Neither address is logged.
fn note_untrusted_forwarding() {
    static NOTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if !NOTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        tracing::warn!(
            "X-Forwarded-For from a peer outside SHELLPP_TRUSTED_PROXIES was ignored; \
             behind a proxy, trust the address it connects from (server README, \
             \"Client addresses\")"
        );
    }
}

/// The client of a request from `peer`: `peer` itself unless it is
/// trusted; then the last address of the `X-Forwarded-For` values that is
/// not a trusted proxy (`peer` when there is none).
fn client_ip<'a>(
    peer: IpAddr,
    forwarded: impl Iterator<Item = &'a str>,
    trusted: impl Fn(IpAddr) -> bool,
) -> IpAddr {
    if !trusted(peer) {
        return peer;
    }
    let elements: Vec<&str> = forwarded.flat_map(|value| value.split(',')).collect();
    // From the right: past the trusted proxies to the first other element.
    // One that does not parse (`unknown`, a zone id) stops the walk at the
    // last trusted hop: going on to its left would let the client's own,
    // forged elements decide the key (security.md 14.4; review finding).
    let mut last_trusted = peer;
    for element in elements.into_iter().rev() {
        match forwarded_ip(element) {
            Some(ip) if trusted(ip) => last_trusted = ip,
            Some(ip) => return ip,
            None => return last_trusted,
        }
    }
    last_trusted
}

/// One element of `X-Forwarded-For`: `ip`, `ip:port`, `[ipv6]` or
/// `[ipv6]:port` (some proxies add the client's port); `None` for anything
/// else, which ends the walk of [`client_ip`].
fn forwarded_ip(element: &str) -> Option<IpAddr> {
    let element = element.trim();
    if let Ok(ip) = element.parse::<IpAddr>() {
        return Some(ip.to_canonical());
    }
    if let Ok(addr) = element.parse::<SocketAddr>() {
        return Some(addr.ip().to_canonical());
    }
    let inner = element.strip_prefix('[')?.strip_suffix(']')?;
    inner
        .parse::<Ipv6Addr>()
        .ok()
        .map(|ip| IpAddr::V6(ip).to_canonical())
}

/// The per-IP limit of the unauthenticated auth endpoints (section 10.1),
/// as a middleware: it runs before the body is read, so a request over the
/// limit costs no parse (review finding).
async fn limit_auth(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    request: Request,
    next: Next,
) -> ApiResult<Response> {
    state.limiters.auth(ip).map_err(ApiError::rate_limited)?;
    Ok(next.run(request).await)
}

/// Runs `check` (an Argon2id check or hash of `accounts`) on a blocking
/// thread, outside any database transaction, in one of the slots of
/// `Shared::argon2_slots` (run 7 finding 1; D174). The blocking task holds
/// the slot: a request dropped while its check runs (the client closed the
/// connection) does not free it early.
async fn argon2<T: Send + 'static>(
    state: &AppState,
    check: impl FnOnce(&Shared) -> ApiResult<T> + Send + 'static,
) -> ApiResult<T> {
    // A bounded queue, for a bounded time (cross review of D174): a flood
    // beyond it is answered `429` at once instead of holding its body in
    // memory while it waits.
    struct Waiting<'a>(&'a std::sync::atomic::AtomicUsize);
    impl Drop for Waiting<'_> {
        fn drop(&mut self) {
            self.0.fetch_sub(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    let busy = || ApiError::rate_limited(ARGON2_RETRY_AFTER);
    let before = state
        .argon2_waiting
        .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let waiting = Waiting(&state.argon2_waiting);
    if before >= ARGON2_MAX_WAITING {
        return Err(busy());
    }
    let slot = tokio::time::timeout(ARGON2_MAX_WAIT, state.argon2_slots.clone().acquire_owned())
        .await
        .map_err(|_| busy())?
        .map_err(|_| ApiError::internal())?;
    drop(waiting);
    let shared = state.clone();
    tokio::task::spawn_blocking(move || {
        let result = check(&shared);
        drop(slot);
        result
    })
    .await
    .map_err(|_| ApiError::internal())?
}

/// How many requests wait for an Argon2id slot at most; more are `429`.
pub const ARGON2_MAX_WAITING: usize = 64;
/// How long a request waits for a slot at most; then `429`.
pub const ARGON2_MAX_WAIT: std::time::Duration = std::time::Duration::from_secs(20);
/// The `Retry-After` of a request turned away by the queue.
const ARGON2_RETRY_AFTER: u64 = 5;

/// A signed-in request's `current_auth_key`, checked before the write
/// transaction that acts on it, which takes the user with
/// [`CheckedKey::user`] (D174).
async fn current_auth_key(
    state: &AppState,
    principal: Principal,
    key: &B64,
) -> ApiResult<CheckedKey> {
    let key = Zeroizing::new(key.as_bytes().to_vec());
    argon2(state, move |shared| {
        accounts::check_current_auth_key(shared, &principal, &key)
    })
    .await
}

/// The key of a `login` or `recover/begin` for `email`, checked before the
/// write transaction (D174).
async fn sign_in_key(
    state: &AppState,
    email: &str,
    kind: KeyKind,
    key: &B64,
) -> ApiResult<CheckedKey> {
    let email = email.to_owned();
    let key = Zeroizing::new(key.as_bytes().to_vec());
    argon2(state, move |shared| {
        accounts::check_sign_in(shared, &email, kind, &key)
    })
    .await
}

/// The router of the whole API.
pub fn router(state: AppState) -> Router {
    // The unauthenticated auth endpoints: the per-IP limit before the body,
    // and a body limit of their own (section 10.2).
    let auth = Router::new()
        .route("/v1/auth/prelogin", post(prelogin))
        .route("/v1/auth/register", post(register))
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/login/totp", post(login_totp))
        .route("/v1/auth/token/refresh", post(refresh))
        .route("/v1/auth/token/revoke", post(revoke))
        .route("/v1/auth/recover/begin", post(recover_begin))
        .route("/v1/auth/recover/complete", post(recover_complete))
        .route_layer(middleware::from_fn_with_state(state.clone(), limit_auth))
        .layer(DefaultBodyLimit::max(limits::MAX_AUTH_BODY_BYTES as usize));
    Router::new()
        .merge(auth)
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/v1/server", get(server_info))
        .route("/v1/account", get(account))
        .route("/v1/account/password", post(password))
        .route("/v1/account/recovery-key", post(recovery_key))
        .route("/v1/account/email", post(email))
        .route("/v1/account/email/verify", post(email_verify))
        .route("/v1/account/totp/setup", post(totp_setup))
        .route("/v1/account/totp/enable", post(totp_enable))
        .route("/v1/account/totp/disable", post(totp_disable))
        .route("/v1/account/ssh-id", put(ssh_id))
        .route("/v1/account/delete", post(account_delete))
        .route("/v1/devices", get(devices))
        .route("/v1/devices/{device_id}", delete(device_revoke))
        .route("/v1/vaults", get(vaults))
        .route("/v1/vaults/{vault_id}/keys", get(vault_keys))
        .route("/v1/vaults/{vault_id}/key-epochs", post(key_epochs))
        .route("/v1/vaults/{vault_id}/changes", get(pull).post(push))
        // The push body limit (section 10.2) with room for the JSON around
        // the base64 envelopes; a larger body is `413`.
        .layer(DefaultBodyLimit::max(limits::MAX_PUSH_BODY_BYTES as usize))
        .fallback(not_found)
        .with_state(state)
}

async fn not_found() -> ApiError {
    ApiError::not_found()
}

async fn healthz() -> Json<Health> {
    Json(Health {
        status: "ok".to_owned(),
    })
}

async fn readyz(State(state): State<AppState>) -> Response {
    let ok = state
        .db
        .run(TxKind::Read, |tx| -> ApiResult<()> {
            tx.query_one("SELECT 1", &[], &[crate::db::Ty::Int])?;
            Ok(())
        })
        .await
        .is_ok();
    if ok {
        Json(Health {
            status: "ok".to_owned(),
        })
        .into_response()
    } else {
        (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(Health {
                status: "unavailable".to_owned(),
            }),
        )
            .into_response()
    }
}

async fn server_info(State(state): State<AppState>) -> ApiResult<Json<ServerInfo>> {
    let generation = state
        .db
        .run(TxKind::Read, records::server_generation)
        .await?;
    Ok(Json(ServerInfo {
        version: env!("CARGO_PKG_VERSION").to_owned(),
        api_versions: vec![API_VERSION.to_owned()],
        registration: state.config.registration,
        smtp: false,
        limits: Limits {
            max_envelope_bytes: limits::MAX_ENVELOPE_BYTES,
            max_push_items: limits::MAX_PUSH_ITEMS,
            max_push_body_bytes: limits::MAX_PUSH_BODY_BYTES,
            default_pull_limit: limits::DEFAULT_PULL_LIMIT,
            max_pull_limit: limits::MAX_PULL_LIMIT,
            tombstone_retention_days: state.config.tombstone_retention_days,
        },
        server_generation: generation.map(shellpp_sync_api::api::B64),
    }))
}

async fn prelogin(
    State(state): State<AppState>,
    Body(request): Body<PreloginRequest>,
) -> ApiResult<Response> {
    let shared = state.clone();
    let response = state
        .db
        .run(TxKind::Read, move |tx| {
            accounts::prelogin(tx, &shared, request)
        })
        .await?;
    Ok(Json(response).into_response())
}

async fn register(
    State(state): State<AppState>,
    Body(request): Body<RegisterRequest>,
) -> ApiResult<Response> {
    let (request, verifiers) = argon2(&state, move |shared| {
        accounts::register_verifiers(shared, &request, model::now()).map(|v| (request, v))
    })
    .await?;
    let shared = state.clone();
    let response = state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::register(tx, &shared, request, verifiers, model::now())
        })
        .await?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn login(
    State(state): State<AppState>,
    Body(request): Body<LoginRequest>,
) -> ApiResult<Response> {
    let checked = sign_in_key(&state, &request.email, KeyKind::Auth, &request.auth_key).await?;
    let shared = state.clone();
    let response = state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::login(tx, &shared, &checked, request, model::now())
        })
        .await?;
    Ok(Json(response).into_response())
}

async fn login_totp(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Body(request): Body<LoginTotpRequest>,
) -> ApiResult<Response> {
    let shared = state.clone();
    // A wrong code must still count its attempt: the failure is committed.
    let response = state
        .db
        .run(TxKind::Write, move |tx| -> ApiResult<ApiResult<_>> {
            sticky(accounts::login_totp(tx, &shared, request, ip, model::now()))
        })
        .await??;
    Ok(Json(response).into_response())
}

async fn refresh(
    State(state): State<AppState>,
    Body(request): Body<RefreshRequest>,
) -> ApiResult<Response> {
    // A reuse revokes the family and still answers `401`: commit first.
    let tokens = state
        .db
        .run(TxKind::Write, move |tx| -> ApiResult<ApiResult<_>> {
            sticky(accounts::refresh(tx, &request.refresh_token, model::now()))
        })
        .await??;
    Ok(Json(TokensResponse { tokens }).into_response())
}

async fn revoke(
    State(state): State<AppState>,
    Body(request): Body<RefreshRequest>,
) -> ApiResult<StatusCode> {
    state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::revoke(tx, &request.refresh_token, model::now())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn recover_begin(
    State(state): State<AppState>,
    Body(request): Body<RecoverBeginRequest>,
) -> ApiResult<Response> {
    let checked = sign_in_key(
        &state,
        &request.email,
        KeyKind::Recovery,
        &request.recovery_auth_key,
    )
    .await?;
    let response = state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::recover_begin(tx, &checked, request, model::now())
        })
        .await?;
    Ok(Json(response).into_response())
}

async fn recover_complete(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Body(request): Body<RecoverCompleteRequest>,
) -> ApiResult<Response> {
    let shared = state.clone();
    let tokens = state
        .db
        .run(TxKind::Write, move |tx| -> ApiResult<ApiResult<_>> {
            // A wrong TOTP code counts its attempt even though it fails.
            sticky(accounts::recover_complete(
                tx,
                &shared,
                request,
                ip,
                model::now(),
            ))
        })
        .await??;
    Ok(Json(TokensResponse { tokens }).into_response())
}

/// Commits the writes of the failures that must stick (a ticket's attempt
/// counter, the family a reused refresh token revoked) and rolls back on
/// any other error.
fn sticky<T>(result: ApiResult<T>) -> ApiResult<ApiResult<T>> {
    match result {
        Err(error)
            if matches!(
                error.code,
                code::INVALID_TOTP | code::TICKET_EXPIRED | code::REFRESH_TOKEN_REUSED
            ) =>
        {
            Ok(Err(error))
        }
        other => other.map(Ok),
    }
}

async fn account(State(state): State<AppState>, Auth(principal): Auth) -> ApiResult<Response> {
    let shared = state.clone();
    let response = state
        .db
        .run(TxKind::Read, move |tx| {
            accounts::account(tx, &shared, &principal)
        })
        .await?;
    Ok(Json(response).into_response())
}

async fn password(
    State(state): State<AppState>,
    Auth(principal): Auth,
    Body(request): Body<PasswordChangeRequest>,
) -> ApiResult<Response> {
    let shared = state.clone();
    let tokens = state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::password_change(tx, &shared, &principal, request, model::now())
        })
        .await?;
    Ok(Json(TokensResponse { tokens }).into_response())
}

async fn recovery_key(
    State(state): State<AppState>,
    Auth(principal): Auth,
    Body(request): Body<RecoveryKeyReissueRequest>,
) -> ApiResult<Response> {
    let shared = state.clone();
    let response = state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::recovery_key_reissue(tx, &shared, &principal, request, model::now())
        })
        .await?;
    Ok(Json(response).into_response())
}

async fn email(
    State(state): State<AppState>,
    Auth(principal): Auth,
    Body(request): Body<EmailChangeRequest>,
) -> ApiResult<Response> {
    let checked = current_auth_key(&state, principal, &request.current_auth_key).await?;
    let response = state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::email_change(tx, &principal, &checked, request, model::now())
        })
        .await?;
    Ok(Json(response).into_response())
}

/// Without SMTP no code is ever sent, so there is nothing to verify.
async fn email_verify(Auth(_principal): Auth) -> ApiError {
    ApiError::validation("code")
}

async fn totp_setup(
    State(state): State<AppState>,
    Auth(principal): Auth,
    Body(request): Body<CurrentAuth>,
) -> ApiResult<Response> {
    let checked = current_auth_key(&state, principal, &request.current_auth_key).await?;
    let shared = state.clone();
    let response = state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::totp_setup(tx, &shared, &checked)
        })
        .await?;
    Ok(Json(response).into_response())
}

async fn totp_enable(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Auth(principal): Auth,
    Body(request): Body<TotpCode>,
) -> ApiResult<StatusCode> {
    let shared = state.clone();
    state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::totp_enable(tx, &shared, &principal, request, ip, model::now())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn totp_disable(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Auth(principal): Auth,
    Body(request): Body<TotpDisableRequest>,
) -> ApiResult<StatusCode> {
    let checked = current_auth_key(&state, principal, &request.current_auth_key).await?;
    let shared = state.clone();
    state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::totp_disable(tx, &shared, &checked, request, ip, model::now())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn ssh_id(
    State(state): State<AppState>,
    Auth(principal): Auth,
    Body(request): Body<SshIdRequest>,
) -> ApiResult<StatusCode> {
    state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::ssh_id(tx, &principal, request)
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn account_delete(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Auth(principal): Auth,
    Body(request): Body<AccountDeleteRequest>,
) -> ApiResult<StatusCode> {
    let checked = current_auth_key(&state, principal, &request.current_auth_key).await?;
    let shared = state.clone();
    state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::account_delete(tx, &shared, &checked, request, ip, model::now())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn devices(State(state): State<AppState>, Auth(principal): Auth) -> ApiResult<Response> {
    let response = state
        .db
        .run(TxKind::Read, move |tx| accounts::devices(tx, &principal))
        .await?;
    Ok(Json(response).into_response())
}

async fn device_revoke(
    State(state): State<AppState>,
    Auth(principal): Auth,
    PathParam(device_id): PathParam<Uuid>,
) -> ApiResult<StatusCode> {
    state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::device_revoke(tx, &principal, device_id, model::now())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn vaults(State(state): State<AppState>, Auth(principal): Auth) -> ApiResult<Response> {
    let response = state
        .db
        .run(TxKind::Read, move |tx| records::vaults(tx, &principal))
        .await?;
    Ok(Json(response).into_response())
}

async fn vault_keys(
    State(state): State<AppState>,
    Auth(principal): Auth,
    PathParam(vault_id): PathParam<Uuid>,
) -> ApiResult<Response> {
    let response = state
        .db
        .run(TxKind::Read, move |tx| {
            records::keys(tx, &principal, vault_id)
        })
        .await?;
    Ok(Json(response).into_response())
}

async fn key_epochs(
    State(state): State<AppState>,
    Auth(principal): Auth,
    PathParam(vault_id): PathParam<Uuid>,
    Body(request): Body<PersonalKeyEpochRequest>,
) -> ApiResult<Response> {
    // An unknown or foreign vault answers before the key is checked.
    let key = Zeroizing::new(request.current_auth_key.as_bytes().to_vec());
    let checked = argon2(&state, move |shared| {
        accounts::check_current_auth_key_after(shared, &principal, &key, |user| {
            if user.personal_vault_id == vault_id {
                Ok(())
            } else {
                Err(ApiError::not_member())
            }
        })
    })
    .await?;
    let current_epoch = state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::personal_key_epoch(tx, &checked, vault_id, request, model::now())
        })
        .await?;
    Ok(Json(KeyEpochResponse { current_epoch }).into_response())
}

#[derive(Debug, Deserialize)]
struct PullQuery {
    since: Option<u64>,
    limit: Option<u64>,
    classes: Option<String>,
}

async fn pull(
    State(state): State<AppState>,
    Auth(principal): Auth,
    PathParam(vault_id): PathParam<Uuid>,
    QueryParams(query): QueryParams<PullQuery>,
) -> ApiResult<Response> {
    let filter = records::parse_classes(query.classes.as_deref())?;
    let since = query.since.unwrap_or(0);
    let response = state
        .db
        .run(TxKind::Read, move |tx| {
            records::pull(tx, &principal, vault_id, since, query.limit, filter)
        })
        .await?;
    Ok(Json(response).into_response())
}

async fn push(
    State(state): State<AppState>,
    Auth(principal): Auth,
    PathParam(vault_id): PathParam<Uuid>,
    Body(request): Body<PushRequest>,
) -> ApiResult<Response> {
    state
        .limiters
        .push(principal.device_id)
        .map_err(ApiError::rate_limited)?;
    let defaults = (state.config.quota_max_records, state.config.quota_max_bytes);
    let response = state
        .db
        .run(TxKind::Write, move |tx| {
            records::push(
                tx,
                &principal,
                vault_id,
                &request.items,
                defaults,
                model::now(),
            )
        })
        .await?;
    Ok(Json(response).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Cidr;

    fn ip(text: &str) -> IpAddr {
        text.parse().unwrap()
    }

    fn state() -> AppState {
        let secret = data_encoding::BASE64.encode(&crate::secrets::random_bytes::<32>());
        let config = crate::config::Config::from_env(|name: &str| match name {
            "SHELLPP_DATABASE_URL" => Some("sqlite::memory:".to_owned()),
            "SHELLPP_PUBLIC_URL" => Some("https://sync.example.com".to_owned()),
            "SHELLPP_SERVER_SECRET" => Some(secret.clone()),
            _ => None,
        })
        .unwrap();
        Shared::new(crate::db::Db::open("sqlite::memory:").unwrap(), config)
    }

    /// Cross review (D174): with every slot busy, at most
    /// `ARGON2_MAX_WAITING` requests wait, for at most `ARGON2_MAX_WAIT`;
    /// one more is turned away at once with `429` and a `Retry-After`.
    #[tokio::test(start_paused = true)]
    async fn the_argon2_queue_is_bounded_in_length_and_time() {
        let state = state();
        let slots = state.argon2_slots.available_permits() as u32;
        let _all = state
            .argon2_slots
            .clone()
            .acquire_many_owned(slots)
            .await
            .unwrap();
        let waiters: Vec<_> = (0..ARGON2_MAX_WAITING)
            .map(|_| {
                let state = state.clone();
                tokio::spawn(async move { argon2(&state, |_| Ok(())).await })
            })
            .collect();
        // Let them all queue.
        while state
            .argon2_waiting
            .load(std::sync::atomic::Ordering::Relaxed)
            < ARGON2_MAX_WAITING
        {
            tokio::task::yield_now().await;
        }
        let refused = argon2(&state, |_| Ok(())).await.unwrap_err();
        assert_eq!(refused.status, StatusCode::TOO_MANY_REQUESTS);
        assert!(refused.retry_after.is_some());
        // The queued ones give up after the longest wait.
        tokio::time::advance(ARGON2_MAX_WAIT + std::time::Duration::from_secs(1)).await;
        for waiter in waiters {
            let error = waiter.await.unwrap().unwrap_err();
            assert_eq!(error.status, StatusCode::TOO_MANY_REQUESTS);
        }
        assert_eq!(
            state
                .argon2_waiting
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
    }

    /// Review finding (D174): a check keeps its Argon2id slot until it ends,
    /// also when its request is dropped meanwhile (the client closed the
    /// connection); else closing connections would run any number at once.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_dropped_request_keeps_its_argon2_slot_until_the_check_ends() {
        let state = state();
        let slots = state.argon2_slots.available_permits();
        let (started, has_started) = std::sync::mpsc::channel::<()>();
        let (release, released) = std::sync::mpsc::channel::<()>();
        let request = tokio::spawn({
            let state = state.clone();
            async move {
                argon2(&state, move |_| {
                    started.send(()).unwrap();
                    let _ = released.recv();
                    Ok(())
                })
                .await
            }
        });
        tokio::task::spawn_blocking(move || has_started.recv().unwrap())
            .await
            .unwrap();
        request.abort();
        let _ = request.await;
        assert_eq!(
            state.argon2_slots.available_permits(),
            slots - 1,
            "the check still runs"
        );
        release.send(()).unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while state.argon2_slots.available_permits() != slots {
            assert!(std::time::Instant::now() < deadline, "the slot came back");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }

    /// An IPv4 client reaching a server bound to `[::]` arrives as
    /// `::ffff:a.b.c.d`: it is the same client, inside IPv4 CIDRs of
    /// trusted proxies and in one rate-limit bucket (review finding).
    #[test]
    fn ipv4_mapped_addresses_count_as_ipv4() {
        assert_eq!(forwarded_ip("::ffff:192.0.2.1"), Some(ip("192.0.2.1")));
        assert_eq!(
            forwarded_ip("[::ffff:192.0.2.1]:4711"),
            Some(ip("192.0.2.1"))
        );
        let proxy = Cidr::parse("10.0.0.0/8").unwrap();
        let trusted = |ip: IpAddr| proxy.contains(ip);
        let mapped_proxy = ip("::ffff:10.0.0.5").to_canonical();
        assert!(trusted(mapped_proxy), "the mapped proxy is trusted");
        assert_eq!(
            client_ip(mapped_proxy, ["::ffff:192.0.2.9"].into_iter(), trusted),
            ip("192.0.2.9")
        );
    }

    #[test]
    fn forwarded_elements_with_and_without_ports() {
        assert_eq!(forwarded_ip("192.0.2.1"), Some(ip("192.0.2.1")));
        assert_eq!(forwarded_ip(" 192.0.2.1:4711 "), Some(ip("192.0.2.1")));
        assert_eq!(forwarded_ip("2001:db8::1"), Some(ip("2001:db8::1")));
        assert_eq!(forwarded_ip("[2001:db8::1]"), Some(ip("2001:db8::1")));
        assert_eq!(forwarded_ip("[2001:db8::1]:4711"), Some(ip("2001:db8::1")));
        // Garbage stays skipped.
        for garbage in [
            "",
            "unknown",
            "_hidden",
            "192.0.2.1:",
            "192.0.2.1:99999",
            "192.0.2.1:x",
            "[192.0.2.1]",
            "[192.0.2.1]:4711",
            "[2001:db8::1",
            "[2001:db8::1]:",
            "[2001:db8::1]x",
            "2001:db8::1]:4711",
            "192.0.2.300",
        ] {
            assert_eq!(forwarded_ip(garbage), None, "{garbage:?}");
        }
    }

    #[test]
    fn the_client_is_the_last_untrusted_forwarded_address() {
        let proxies = [Cidr::parse("10.0.0.0/8").unwrap()];
        let trusted = |ip: IpAddr| proxies.iter().any(|net| net.contains(ip));
        let peer = ip("10.0.0.2");
        // A spoofed first element, then the client the proxy saw, with its port.
        assert_eq!(
            client_ip(peer, ["198.51.100.7, 192.0.2.1:4711"].into_iter(), trusted),
            ip("192.0.2.1")
        );
        assert_eq!(
            client_ip(
                peer,
                ["198.51.100.7", "[2001:db8::1]:4711, 10.0.0.3:80"].into_iter(),
                trusted
            ),
            ip("2001:db8::1")
        );
        // An element that does not parse ends the walk at the last trusted
        // hop: what the client wrote on its left never becomes the key
        // (review finding: skipping it let a forged element through when
        // a proxy wrote `unknown` or a zone id).
        assert_eq!(
            client_ip(peer, ["192.0.2.1, junk"].into_iter(), trusted),
            peer
        );
        assert_eq!(
            client_ip(
                peer,
                ["198.51.100.7, unknown, 10.0.0.3"].into_iter(),
                trusted
            ),
            ip("10.0.0.3")
        );
        assert_eq!(
            client_ip(
                peer,
                ["198.51.100.7, [fe80::1%eth0]:4711"].into_iter(),
                trusted
            ),
            peer
        );
        assert_eq!(
            client_ip(peer, ["10.0.0.3:80, junk"].into_iter(), trusted),
            peer
        );
        assert_eq!(client_ip(peer, std::iter::empty(), trusted), peer);
        // An untrusted peer: the header is not read.
        let outsider = ip("203.0.113.9");
        assert_eq!(
            client_ip(outsider, ["192.0.2.1:4711"].into_iter(), trusted),
            outsider
        );
    }
}
