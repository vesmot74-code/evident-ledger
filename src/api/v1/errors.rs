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
    InvalidVerifyFileHash,
    ProofNotReady,
    ProofGenerationFailed,
    PaymentRequired,
    UsageLimitExceeded,
    InvalidCredentials,
    RateLimited,
    EmailAlreadyRegistered,
    PasswordAlreadySet,
    Internal,
    NotImplemented,
    EntitlementMissing,
    IdentityKeyNotFound,
    IdentityKeyRevoked,
    IdentityKeyNotVerified,
    InvalidIdentitySignature,
}

impl ApiError {
    pub fn status_code(self) -> StatusCode {
        match self {
            ApiError::Unauthorized => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden => StatusCode::FORBIDDEN,
            ApiError::NotFound => StatusCode::NOT_FOUND,
            ApiError::Conflict => StatusCode::CONFLICT,
            ApiError::InvalidRequest => StatusCode::BAD_REQUEST,
            ApiError::InvalidVerifyFileHash => StatusCode::BAD_REQUEST,
            ApiError::ProofNotReady => StatusCode::CONFLICT,
            ApiError::ProofGenerationFailed => StatusCode::UNPROCESSABLE_ENTITY,
            ApiError::PaymentRequired => StatusCode::PAYMENT_REQUIRED,
            ApiError::UsageLimitExceeded => StatusCode::TOO_MANY_REQUESTS,
            ApiError::InvalidCredentials => StatusCode::UNAUTHORIZED,
            ApiError::RateLimited => StatusCode::TOO_MANY_REQUESTS,
            ApiError::EmailAlreadyRegistered => StatusCode::CONFLICT,
            ApiError::PasswordAlreadySet => StatusCode::CONFLICT,
            ApiError::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            ApiError::NotImplemented => StatusCode::NOT_IMPLEMENTED,
            ApiError::EntitlementMissing => StatusCode::FORBIDDEN,
            ApiError::IdentityKeyNotFound => StatusCode::NOT_FOUND,
            ApiError::IdentityKeyRevoked => StatusCode::FORBIDDEN,
            ApiError::IdentityKeyNotVerified => StatusCode::FORBIDDEN,
            ApiError::InvalidIdentitySignature => StatusCode::UNAUTHORIZED,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            ApiError::Unauthorized => "unauthorized",
            ApiError::Forbidden => "forbidden",
            ApiError::NotFound => "not_found",
            ApiError::Conflict => "conflict",
            ApiError::InvalidRequest => "invalid_request",
            ApiError::InvalidVerifyFileHash => "invalid_request",
            ApiError::ProofNotReady => "proof_not_ready",
            ApiError::ProofGenerationFailed => "proof_generation_failed",
            ApiError::PaymentRequired => "payment_required",
            ApiError::UsageLimitExceeded => "usage_limit_exceeded",
            ApiError::InvalidCredentials => "unauthorized",
            ApiError::RateLimited => "rate_limited",
            ApiError::EmailAlreadyRegistered => "conflict",
            ApiError::PasswordAlreadySet => "conflict",
            ApiError::Internal => "internal_error",
            ApiError::NotImplemented => "not_implemented",
            ApiError::EntitlementMissing => "entitlement_missing",
            ApiError::IdentityKeyNotFound => "identity_key_not_found",
            ApiError::IdentityKeyRevoked => "identity_key_revoked",
            ApiError::IdentityKeyNotVerified => "identity_key_not_verified",
            ApiError::InvalidIdentitySignature => "invalid_identity_signature",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            ApiError::Unauthorized => "Missing or invalid API key",
            ApiError::Forbidden => "Access denied",
            ApiError::NotFound => "Resource not found",
            ApiError::Conflict => "Request conflict",
            ApiError::InvalidRequest => "Invalid request",
            ApiError::InvalidVerifyFileHash => {
                "file_hash must be a valid SHA-256 hex string (64 chars, 0-9a-f)"
            }
            ApiError::ProofNotReady => "Proof material is not yet available for this event",
            ApiError::ProofGenerationFailed => "Proof generation failed for this event",
            ApiError::PaymentRequired => {
                "Subscription payment is past due. Write access is suspended until payment succeeds."
            }
            ApiError::UsageLimitExceeded => {
                "Monthly commit limit for your tariff plan has been reached."
            }
            ApiError::InvalidCredentials => "Invalid email or password",
            ApiError::RateLimited => "Too many login attempts. Please try again later.",
            ApiError::EmailAlreadyRegistered => "Email is already registered",
            ApiError::PasswordAlreadySet => "Password is already set for this account",
            ApiError::Internal => "Internal server error",
            ApiError::NotImplemented => "Not implemented",
            ApiError::EntitlementMissing => {
                "Identity feature not available on current plan"
            }
            ApiError::IdentityKeyNotFound => "Identity key not found",
            ApiError::IdentityKeyRevoked => "Identity key has been revoked",
            ApiError::IdentityKeyNotVerified => "Identity key is not verified",
            ApiError::InvalidIdentitySignature => "Signature verification failed",
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

    #[tokio::test]
    async fn proof_not_ready_serializes_with_request_id() {
        let request_id = Uuid::new_v4();
        let response = REQUEST_ID
            .scope(request_id, async {
                ApiError::ProofNotReady.into_response()
            })
            .await;

        assert_eq!(response.status(), StatusCode::CONFLICT);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: Value = serde_json::from_slice(&body).expect("json body");

        assert_eq!(json["error"]["code"], "proof_not_ready");
        assert_eq!(json["error"]["request_id"], request_id.to_string());
    }

    #[tokio::test]
    async fn proof_generation_failed_serializes_with_request_id() {
        let request_id = Uuid::new_v4();
        let response = REQUEST_ID
            .scope(request_id, async {
                ApiError::ProofGenerationFailed.into_response()
            })
            .await;

        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);

        let body = to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("response body");
        let json: Value = serde_json::from_slice(&body).expect("json body");

        assert_eq!(json["error"]["code"], "proof_generation_failed");
        assert_eq!(json["error"]["request_id"], request_id.to_string());
    }
}
