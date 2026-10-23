//! The HTTP API v1 (docs/sync-protocol.md, section 3): routes, extractors
//! (JSON bodies with section 8 errors, the bearer token, the client IP) and
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
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post, put};
use axum::{Json, Router};
use serde::Deserialize;
use shellpp_sync_api::api::{
    API_VERSION, AccountDeleteRequest, CurrentAuth, EmailChangeRequest, Health, KeyEpochResponse,
    Limits, LoginRequest, LoginTotpRequest, PasswordChangeRequest, PersonalKeyEpochRequest,
    PreloginRequest, PushRequest, RecoverBeginRequest, RecoverCompleteRequest,
    RecoveryKeyReissueRequest, RefreshRequest, RegisterRequest, ServerInfo, SshIdRequest,
    TokensResponse, TotpCode, TotpDisableRequest, code, limits,
};
use uuid::Uuid;

use crate::accounts::{self, Principal};
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

/// The signed-in device of a request with `Authorization: Bearer`.
pub struct Auth(pub Principal);

impl FromRequestParts<AppState> for Auth {
    type Rejection = ApiError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = parts
            .headers
            .get(axum::http::header::AUTHORIZATION)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.strip_prefix("Bearer "))
            .map(str::trim)
            .ok_or_else(ApiError::unauthenticated)?
            .to_owned();
        let principal = state
            .db
            .run(TxKind::Write, move |tx| {
                accounts::authenticate(tx, &token, model::now())
            })
            .await?;
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
            .map(|info| info.0.ip())
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
        Ok(ClientIp(client_ip(peer, forwarded, trusted)))
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
    let forwarded = forwarded
        .flat_map(|value| value.split(','))
        .filter_map(forwarded_ip)
        .collect::<Vec<_>>();
    forwarded
        .iter()
        .rev()
        .copied()
        .find(|ip| !trusted(*ip))
        .unwrap_or(peer)
}

/// One element of `X-Forwarded-For`: `ip`, `ip:port`, `[ipv6]` or
/// `[ipv6]:port` (some proxies add the client's port); `None` for anything
/// else, which is skipped.
fn forwarded_ip(element: &str) -> Option<IpAddr> {
    let element = element.trim();
    if let Ok(ip) = element.parse::<IpAddr>() {
        return Some(ip);
    }
    if let Ok(addr) = element.parse::<SocketAddr>() {
        return Some(addr.ip());
    }
    let inner = element.strip_prefix('[')?.strip_suffix(']')?;
    inner.parse::<Ipv6Addr>().ok().map(IpAddr::V6)
}

fn limit_auth(state: &Shared, ip: IpAddr) -> ApiResult<()> {
    state.limiters.auth(ip).map_err(ApiError::rate_limited)
}

/// The router of the whole API.
pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/v1/server", get(server_info))
        .route("/v1/auth/prelogin", post(prelogin))
        .route("/v1/auth/register", post(register))
        .route("/v1/auth/login", post(login))
        .route("/v1/auth/login/totp", post(login_totp))
        .route("/v1/auth/token/refresh", post(refresh))
        .route("/v1/auth/token/revoke", post(revoke))
        .route("/v1/auth/recover/begin", post(recover_begin))
        .route("/v1/auth/recover/complete", post(recover_complete))
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
    ClientIp(ip): ClientIp,
    Body(request): Body<PreloginRequest>,
) -> ApiResult<Response> {
    limit_auth(&state, ip)?;
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
    ClientIp(ip): ClientIp,
    Body(request): Body<RegisterRequest>,
) -> ApiResult<Response> {
    limit_auth(&state, ip)?;
    let shared = state.clone();
    let response = state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::register(tx, &shared, request, model::now())
        })
        .await?;
    Ok((StatusCode::CREATED, Json(response)).into_response())
}

async fn login(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Body(request): Body<LoginRequest>,
) -> ApiResult<Response> {
    limit_auth(&state, ip)?;
    let shared = state.clone();
    let response = state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::login(tx, &shared, request, model::now())
        })
        .await?;
    Ok(Json(response).into_response())
}

async fn login_totp(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Body(request): Body<LoginTotpRequest>,
) -> ApiResult<Response> {
    limit_auth(&state, ip)?;
    let shared = state.clone();
    // A wrong code must still count its attempt: the failure is committed.
    let response = state
        .db
        .run(TxKind::Write, move |tx| -> ApiResult<ApiResult<_>> {
            sticky(accounts::login_totp(tx, &shared, request, model::now()))
        })
        .await??;
    Ok(Json(response).into_response())
}

async fn refresh(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Body(request): Body<RefreshRequest>,
) -> ApiResult<Response> {
    limit_auth(&state, ip)?;
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
    ClientIp(ip): ClientIp,
    Body(request): Body<RefreshRequest>,
) -> ApiResult<StatusCode> {
    limit_auth(&state, ip)?;
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
    ClientIp(ip): ClientIp,
    Body(request): Body<RecoverBeginRequest>,
) -> ApiResult<Response> {
    limit_auth(&state, ip)?;
    let shared = state.clone();
    let response = state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::recover_begin(tx, &shared, request, model::now())
        })
        .await?;
    Ok(Json(response).into_response())
}

async fn recover_complete(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    Body(request): Body<RecoverCompleteRequest>,
) -> ApiResult<Response> {
    limit_auth(&state, ip)?;
    let shared = state.clone();
    let tokens = state
        .db
        .run(TxKind::Write, move |tx| -> ApiResult<ApiResult<_>> {
            // A wrong TOTP code counts its attempt even though it fails.
            sticky(accounts::recover_complete(
                tx,
                &shared,
                request,
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
    let shared = state.clone();
    let response = state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::email_change(tx, &shared, &principal, request, model::now())
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
    let shared = state.clone();
    let response = state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::totp_setup(tx, &shared, &principal, request)
        })
        .await?;
    Ok(Json(response).into_response())
}

async fn totp_enable(
    State(state): State<AppState>,
    Auth(principal): Auth,
    Body(request): Body<TotpCode>,
) -> ApiResult<StatusCode> {
    let shared = state.clone();
    state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::totp_enable(tx, &shared, &principal, request, model::now())
        })
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn totp_disable(
    State(state): State<AppState>,
    Auth(principal): Auth,
    Body(request): Body<TotpDisableRequest>,
) -> ApiResult<StatusCode> {
    let shared = state.clone();
    state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::totp_disable(tx, &shared, &principal, request, model::now())
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
    Auth(principal): Auth,
    Body(request): Body<AccountDeleteRequest>,
) -> ApiResult<StatusCode> {
    let shared = state.clone();
    state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::account_delete(tx, &shared, &principal, request, model::now())
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
    Path(device_id): Path<Uuid>,
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
    Path(vault_id): Path<Uuid>,
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
    Path(vault_id): Path<Uuid>,
    Body(request): Body<PersonalKeyEpochRequest>,
) -> ApiResult<Response> {
    let shared = state.clone();
    let current_epoch = state
        .db
        .run(TxKind::Write, move |tx| {
            accounts::personal_key_epoch(tx, &shared, &principal, vault_id, request, model::now())
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
    Path(vault_id): Path<Uuid>,
    query: Result<Query<PullQuery>, axum::extract::rejection::QueryRejection>,
) -> ApiResult<Response> {
    let Query(query) = query.map_err(|_| ApiError::bad_request("malformed query"))?;
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
    Path(vault_id): Path<Uuid>,
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
        // Garbage is skipped; only trusted addresses: the peer.
        assert_eq!(
            client_ip(peer, ["192.0.2.1, junk"].into_iter(), trusted),
            ip("192.0.2.1")
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
