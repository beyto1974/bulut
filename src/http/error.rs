use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::services::error::ServiceError;

pub struct ApiError(pub ServiceError);

impl From<ServiceError> for ApiError {
    fn from(e: ServiceError) -> Self {
        Self(e)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let (status, message) = match &self.0 {
            ServiceError::NotFound => (StatusCode::NOT_FOUND, "not found".to_string()),
            ServiceError::Invalid(m) => (StatusCode::BAD_REQUEST, m.clone()),
            ServiceError::Conflict(m) => (StatusCode::CONFLICT, m.clone()),
            ServiceError::Storage(m) => {
                // Details stay in the log, the client only gets a generic message.
                tracing::error!(error = %m, "storage failure");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    "internal error".to_string(),
                )
            }
        };
        (status, Json(json!({ "error": message }))).into_response()
    }
}

pub type ApiResult<T> = Result<T, ApiError>;
