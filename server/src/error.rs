//! The error of a request: an HTTP status, a code of section 8 and details
//! (docs/sync-protocol.md, section 8). Messages are for logs and never hold
//! secrets; the client shows its own text by code.

use axum::Json;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::{Map, Value};
use shellpp_sync_api::api::{ErrorBody, ErrorInfo, code};

use crate::db::DbError;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: &'static str,
    pub details: Map<String, Value>,
    /// Seconds for `Retry-After` (`429`).
    pub retry_after: Option<u64>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: &'static str) -> Self {
        Self {
            status,
            code,
            message,
            details: Map::new(),
            retry_after: None,
        }
    }

    pub fn detail(mut self, key: &str, value: impl Into<Value>) -> Self {
        self.details.insert(key.to_owned(), value.into());
        self
    }

    pub fn bad_request(message: &'static str) -> Self {
        Self::new(StatusCode::BAD_REQUEST, code::BAD_REQUEST, message)
    }
    pub fn unauthenticated() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            code::UNAUTHENTICATED,
            "missing, invalid or expired token",
        )
    }
    pub fn invalid_credentials() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            code::INVALID_CREDENTIALS,
            "invalid credentials",
        )
    }
    pub fn validation(field: &str) -> Self {
        Self::new(
            StatusCode::UNPROCESSABLE_ENTITY,
            code::VALIDATION_FAILED,
            "validation failed",
        )
        .detail("field", field)
    }
    pub fn not_member() -> Self {
        Self::new(
            StatusCode::FORBIDDEN,
            code::NOT_MEMBER,
            "not a member of the vault",
        )
    }
    pub fn not_found() -> Self {
        Self::new(StatusCode::NOT_FOUND, code::NOT_FOUND, "not found")
    }
    pub fn internal() -> Self {
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            code::INTERNAL,
            "internal error",
        )
    }
    pub fn rate_limited(seconds: u64) -> Self {
        let mut error = Self::new(
            StatusCode::TOO_MANY_REQUESTS,
            code::RATE_LIMITED,
            "too many requests",
        );
        error.retry_after = Some(seconds.max(1));
        error
    }
    pub fn weak_kdf() -> Self {
        Self::new(
            StatusCode::BAD_REQUEST,
            code::WEAK_KDF_PARAMS,
            "key derivation parameters out of bounds",
        )
    }
}

impl From<DbError> for ApiError {
    fn from(error: DbError) -> Self {
        match error {
            DbError::Unavailable => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                code::UNAVAILABLE,
                "database unavailable",
            ),
            // A duplicate the checks before the write did not see (two
            // requests at once): a protocol pair of section 8, not 409 with
            // `validation_failed` (review finding).
            DbError::Unique => Self::new(
                StatusCode::UNPROCESSABLE_ENTITY,
                code::VALIDATION_FAILED,
                "conflicting value",
            ),
            DbError::Failed(reason) => {
                tracing::error!(%reason, "database error");
                Self::internal()
            }
        }
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ErrorBody {
            error: ErrorInfo {
                code: self.code.to_owned(),
                message: self.message.to_owned(),
                details: self.details,
            },
        };
        let mut response = (self.status, Json(body)).into_response();
        if let Some(seconds) = self.retry_after
            && let Ok(value) = HeaderValue::from_str(&seconds.to_string())
        {
            response.headers_mut().insert("retry-after", value);
        }
        response
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
