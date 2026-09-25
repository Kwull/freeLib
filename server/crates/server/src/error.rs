//! API errors: HTTP status + `{"error": code, "message": text}`.

use axum::Json;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use freelib_catalog::CatalogError;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
}

pub type ApiResult<T> = Result<T, ApiError>;

/// Error code of a catalog that was replaced by a re-import mid-request (retried once by
/// [`crate::state::AppState::catalog_call`], 503 if it still happens).
pub const STALE: &str = "stale";

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> ApiError {
        ApiError {
            status,
            code,
            message: message.into(),
        }
    }
    pub fn bad_request(m: impl Into<String>) -> ApiError {
        Self::new(StatusCode::BAD_REQUEST, "bad_request", m)
    }
    pub fn unauthorized(m: impl Into<String>) -> ApiError {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", m)
    }
    pub fn forbidden(m: impl Into<String>) -> ApiError {
        Self::new(StatusCode::FORBIDDEN, "forbidden", m)
    }
    pub fn not_found(m: impl Into<String>) -> ApiError {
        Self::new(StatusCode::NOT_FOUND, "not_found", m)
    }
    pub fn conflict(m: impl Into<String>) -> ApiError {
        Self::new(StatusCode::CONFLICT, "conflict", m)
    }
    pub fn unsupported(m: impl Into<String>) -> ApiError {
        Self::new(StatusCode::NOT_IMPLEMENTED, "unsupported_format", m)
    }
    pub fn rate_limited(m: impl Into<String>) -> ApiError {
        Self::new(StatusCode::TOO_MANY_REQUESTS, "rate_limited", m)
    }
    pub fn internal(m: impl Into<String>) -> ApiError {
        let m = m.into();
        tracing::error!("internal error: {m}");
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "internal", m)
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code, self.message)
    }
}

impl std::error::Error for ApiError {}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({"error": self.code, "message": self.message})),
        )
            .into_response()
    }
}

impl From<CatalogError> for ApiError {
    fn from(e: CatalogError) -> Self {
        match e {
            CatalogError::BadCursor => ApiError::bad_request("invalid cursor"),
            CatalogError::NotFound(_) => ApiError::not_found("library not imported"),
            CatalogError::SchemaVersion { .. } => ApiError::new(
                StatusCode::CONFLICT,
                "conflict",
                "catalog must be re-imported (schema version)",
            ),
            CatalogError::Stale => ApiError::new(
                StatusCode::SERVICE_UNAVAILABLE,
                STALE,
                "the library was just re-imported, please retry",
            ),
            e => ApiError::internal(e.to_string()),
        }
    }
}

impl From<rusqlite::Error> for ApiError {
    fn from(e: rusqlite::Error) -> Self {
        ApiError::internal(format!("database: {e}"))
    }
}

impl From<std::io::Error> for ApiError {
    fn from(e: std::io::Error) -> Self {
        ApiError::internal(format!("io: {e}"))
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(e: anyhow::Error) -> Self {
        match e.downcast::<ApiError>() {
            Ok(a) => a,
            Err(e) => ApiError::internal(format!("{e:#}")),
        }
    }
}

impl From<tokio::task::JoinError> for ApiError {
    fn from(e: tokio::task::JoinError) -> Self {
        ApiError::internal(format!("task failed: {e}"))
    }
}
