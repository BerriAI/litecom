use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde_json::json;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error("{0}")]
    Forbidden(String),
    #[error("Sign in to continue.")]
    Unauthorized,
    #[error("This item is unavailable.")]
    NotFound,
    #[error("{0}")]
    Conflict(String),
    #[error("Please wait before trying again.")]
    RateLimited,
    #[error("{0}")]
    Unavailable(String),
    #[error(transparent)]
    Internal(#[from] anyhow::Error),
}

impl From<std::io::Error> for Error {
    fn from(error: std::io::Error) -> Self {
        Self::Internal(error.into())
    }
}

impl From<rusqlite::Error> for Error {
    fn from(value: rusqlite::Error) -> Self {
        Self::Internal(value.into())
    }
}
impl From<serde_json::Error> for Error {
    fn from(value: serde_json::Error) -> Self {
        Self::Internal(value.into())
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let (status, code) = match &self {
            Self::Invalid(_) => (StatusCode::BAD_REQUEST, "invalid_request"),
            Self::Forbidden(_) => (StatusCode::FORBIDDEN, "forbidden"),
            Self::Unauthorized => (StatusCode::UNAUTHORIZED, "sign_in_required"),
            Self::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            Self::Conflict(_) => (StatusCode::CONFLICT, "conflict"),
            Self::RateLimited => (StatusCode::TOO_MANY_REQUESTS, "rate_limited"),
            Self::Unavailable(_) => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            Self::Internal(_) => (StatusCode::INTERNAL_SERVER_ERROR, "internal_error"),
        };
        let message = if let Self::Internal(ref e) = self {
            tracing::error!(error = ?e, "request failed");
            "Something went wrong. Please retry.".to_owned()
        } else {
            self.to_string()
        };
        let mut response = (
            status,
            Json(json!({"error": {"code": code, "message": message}})),
        )
            .into_response();
        if status == StatusCode::TOO_MANY_REQUESTS {
            response
                .headers_mut()
                .insert("retry-after", axum::http::HeaderValue::from_static("60"));
        }
        response
    }
}

pub fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::Forbidden(message.into()))
    }
}
pub fn validate(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::Invalid(message.into()))
    }
}
pub fn conflict(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::Conflict(message.into()))
    }
}
