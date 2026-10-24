//! The API error envelope (docs/api.md "Errors") and request-id propagation into error bodies.

use axum::extract::rejection::{JsonRejection, QueryRejection};
use axum::extract::{FromRequest, FromRequestParts, Request};
use axum::http::request::Parts;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use bcp_core::{AppError, ErrorCode};
use serde::de::DeserializeOwned;
use serde_json::json;

tokio::task_local! {
    pub static REQUEST_ID: String;
}

pub fn current_request_id() -> String {
    REQUEST_ID.try_with(Clone::clone).unwrap_or_default()
}

#[derive(Debug)]
pub struct ApiError(pub AppError);

pub type ApiResult<T> = Result<T, ApiError>;

impl<E: Into<AppError>> From<E> for ApiError {
    fn from(e: E) -> Self {
        ApiError(e.into())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let err = self.0;
        let status = StatusCode::from_u16(err.code.http_status()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        if status.is_server_error() {
            tracing::warn!(code = %err.code, message = %err.message, "request failed");
        }
        metrics::counter!("bcp_api_errors_total", "code" => err.code.as_str()).increment(1);
        let mut body = json!({
            "error": {
                "code": err.code,
                "message": err.message,
                "retryable": err.retryable,
                "request_id": current_request_id(),
            }
        });
        if let Some(details) = err.details {
            body["error"]["details"] = details;
        }
        let mut resp = (status, Json(body)).into_response();
        if let Some(secs) = err.retry_after {
            if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                resp.headers_mut().insert("retry-after", v);
            }
        }
        resp
    }
}

/// `Json<T>` whose rejections use the error envelope.
pub struct ApiJson<T>(pub T);

impl<S, T> FromRequest<S> for ApiJson<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match Json::<T>::from_request(req, state).await {
            Ok(Json(v)) => Ok(ApiJson(v)),
            Err(JsonRejection::BytesRejection(_)) => {
                Err(ApiError(AppError::new(ErrorCode::PayloadTooLarge, "The request body is too large.")))
            }
            Err(JsonRejection::MissingJsonContentType(_)) => {
                Err(ApiError(AppError::validation([("body", "Expected Content-Type: application/json")])))
            }
            Err(e) => Err(ApiError(AppError::validation([("body", e.body_text())]))),
        }
    }
}

/// `Query<T>` whose rejections use the error envelope.
pub struct ApiQuery<T>(pub T);

impl<S, T> FromRequestParts<S> for ApiQuery<T>
where
    T: DeserializeOwned,
    S: Send + Sync,
{
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        match axum::extract::Query::<T>::from_request_parts(parts, state).await {
            Ok(axum::extract::Query(v)) => Ok(ApiQuery(v)),
            Err(e @ QueryRejection::FailedToDeserializeQueryString(_)) => {
                Err(ApiError(AppError::validation([("query", e.body_text())])))
            }
            Err(e) => Err(ApiError(AppError::validation([("query", e.body_text())]))),
        }
    }
}
