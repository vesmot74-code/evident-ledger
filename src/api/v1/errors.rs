use axum::{
    body::Body,
    http::{Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;
use uuid::Uuid;

tokio::task_local! {
    static REQUEST_ID: Uuid;
}

#[derive(Clone, Copy, Debug)]
pub struct RequestId(pub Uuid);

#[derive(Debug, Clone, Serialize)]
pub struct ErrorEnvelope {
    pub error: ErrorBody,
}

#[derive(Debug, Clone, Serialize)]
pub struct ErrorBody {
    pub code: String,
    pub message: String,
    pub request_id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiError {
    Unauthorized,
    Forbidden,
    NotFound,
    Conflict,
    InvalidRequest,
    Internal,
    NotImplemented,
}

impl ApiError {
    pub fn status_code(self) -> StatusCode {
        match self {
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden => StatusCode::FORBIDDEN,
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::Conflict => StatusCode::CONFLICT,
            ApiError::InvalidRequest => StatusCode::BAD_REQUEST,
            ApiError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::NotImplemented => StatusCode::NOT_IMPLEMENTED,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            ApiError::Unauthorized => "unauthorized",
            ApiError::Forbidden => "forbidden",
            ApiError::NotFound => "not_found",
            ApiError::Conflict => "conflict",
            ApiError::InvalidRequest => "invalid_request",
            ApiError::Internal => "internal_error",
            ApiError::NotImplemented => "not_implemented",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            ApiError::Unauthorized => "Missing or invalid API key",
            ApiError::Forbidden => "Access denied",
            ApiError::NotFound => "Resource not found",
            ApiError::Conflict => "Request conflict",
            ApiError::InvalidRequest => "Invalid request",
            ApiError::Internal => "Internal server error",
            ApiError::NotImplemented => "Not implemented",
        }
    }

    pub fn envelope(self, request_id: Uuid) -> ErrorEnvelope {
        ErrorEnvelope {
            error: ErrorBody {
                code: self.code().to_string(),
                message: self.message().to_string(),
                request_id: request_id.to_string(),
            },
        }
    }

    fn current_request_id() -> Uuid {
        REQUEST_ID
            .try_with(|id| *id)
            .unwrap_or_else(|_| Uuid::new_v4())
    }

    /// Request ID for the active v1 request (success responses, tracing).
    pub fn request_id() -> Uuid {
        Self::current_request_id()
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let request_id = Self::current_request_id();
        (self.status_code(), Json(self.envelope(request_id))).into_response()
    }
}

pub async fn request_id_layer(mut request: Request<Body>, next: Next) -> Response {
    let request_id = Uuid::new_v4();
    request.extensions_mut().insert(RequestId(request_id));
    REQUEST_ID
        .scope(request_id, async move { next.run(request).await })
        .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::to_bytes;
    use serde_json::Value;

    #[tokio::test]
    async fn unauthorized_serializes_with_request_id() {
        let request_id = Uuid::new_v4();
        let response = REQUEST_ID
            .scope(request_id, async { ApiError::Unauthorized.into_response() })
            .await;

        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: Value = serde_json::from_slice(&body).expect("json body");

        assert_eq!(json["error"]["code"], "unauthorized");
        assert_eq!(json["error"]["request_id"], request_id.to_string());
        assert!(json["error"]["message"].is_string());
    }
}
