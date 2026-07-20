//! Identity key registration API (Stage 9.2).

use axum::{
    extract::State,
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use chrono::{DateTime, Utc};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::auth::AuthedAccount;
use crate::service::entitlements::{require_feature, Feature};
use crate::service::identity_challenge::{IdentityChallengeError, IdentityChallengeRepository};
use crate::service::identity_keys::{IdentityKeyError, IdentityKeyRepository};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct ChallengeResponse {
    pub challenge_id: Uuid,
    pub challenge: String,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize)]
pub struct RegisterKeyRequest {
    pub challenge_id: Uuid,
    pub public_key: String,
    pub signature: String,
    pub label: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisterKeyResponse {
    pub key_id: Uuid,
    pub fingerprint: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    code: String,
    message: String,
    request_id: String,
}

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/challenge", post(challenge_handler))
        .route("/register", post(register_handler))
}

fn error_response(status: StatusCode, code: &str, message: &str) -> Response {
    (
        status,
        Json(ErrorEnvelope {
            error: ErrorBody {
                code: code.to_string(),
                message: message.to_string(),
                request_id: Uuid::new_v4().to_string(),
            },
        }),
    )
        .into_response()
}

async fn challenge_handler(State(state): State<AppState>, auth: AuthedAccount) -> Response {
    if let Err(response) = check_identity_entitlement(&state, auth.account_id).await {
        return response;
    }

    match IdentityChallengeRepository::create(&state.db, auth.account_id).await {
        Ok(challenge) => (
            StatusCode::OK,
            Json(ChallengeResponse {
                challenge_id: challenge.id,
                challenge: challenge.challenge,
                expires_at: challenge.expires_at,
            }),
        )
            .into_response(),
        Err(IdentityChallengeError::Database(_)) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Internal server error",
        ),
        Err(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Internal server error",
        ),
    }
}

async fn register_handler(
    State(state): State<AppState>,
    auth: AuthedAccount,
    Json(body): Json<RegisterKeyRequest>,
) -> Response {
    if let Err(response) = check_identity_entitlement(&state, auth.account_id).await {
        return response;
    }

    let challenge = match IdentityChallengeRepository::find_by_id_and_account(
        &state.db,
        body.challenge_id,
        auth.account_id,
    )
    .await
    {
        Ok(Some(challenge)) => challenge,
        Ok(None) => {
            return error_response(
                StatusCode::NOT_FOUND,
                "challenge_not_found",
                "Challenge not found",
            );
        }
        Err(IdentityChallengeError::Database(_)) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Internal server error",
            );
        }
        Err(_) => unreachable!("find_by_id_and_account only returns NotFound or Database"),
    };

    if let Err(err) = IdentityChallengeRepository::validate(&challenge) {
        return map_challenge_error(err);
    }

    // Server-generated hex at create(); decode failure indicates internal corruption.
    let raw_challenge = hex::decode(&challenge.challenge)
        .expect("challenge stored by create() is always valid hex");

    if !verify_ed25519_signature(&body.public_key, &raw_challenge, &body.signature) {
        return error_response(
            StatusCode::UNAUTHORIZED,
            "invalid_signature",
            "Signature verification failed",
        );
    }

    let Some(fingerprint) =
        IdentityKeyRepository::fingerprint_from_public_key_hex(&body.public_key)
    else {
        return error_response(
            StatusCode::BAD_REQUEST,
            "invalid_request",
            "Invalid public key",
        );
    };

    let label = body
        .label
        .as_deref()
        .map(str::trim)
        .filter(|label| !label.is_empty());

    let key = match IdentityKeyRepository::create(
        &state.db,
        auth.account_id,
        &body.public_key,
        &fingerprint,
        label,
    )
    .await
    {
        Ok(key) => key,
        Err(IdentityKeyError::FingerprintAlreadyExists) => {
            return error_response(
                StatusCode::CONFLICT,
                "conflict",
                "Identity key already registered",
            );
        }
        Err(IdentityKeyError::EntitlementMissing) => {
            return error_response(
                StatusCode::FORBIDDEN,
                "entitlement_missing",
                "Identity feature not available on current plan",
            );
        }
        Err(IdentityKeyError::Database(_)) => {
            return error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Internal server error",
            );
        }
        Err(IdentityKeyError::KeyNotFound) => unreachable!("create does not return KeyNotFound"),
    };

    if let Err(err) = IdentityChallengeRepository::mark_used(&state.db, challenge.id).await {
        return match err {
            IdentityChallengeError::ChallengeAlreadyUsed => map_challenge_error(err),
            IdentityChallengeError::Database(_) => error_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal_error",
                "Internal server error",
            ),
            _ => unreachable!("mark_used only returns AlreadyUsed or Database"),
        };
    }

    (
        StatusCode::OK,
        Json(RegisterKeyResponse {
            key_id: key.id,
            fingerprint: key.fingerprint,
            created_at: key.created_at,
        }),
    )
        .into_response()
}

async fn check_identity_entitlement(state: &AppState, account_id: Uuid) -> Result<(), Response> {
    require_feature(&state.db, account_id, Feature::Identity)
        .await
        .map_err(|_| {
            error_response(
                StatusCode::FORBIDDEN,
                "entitlement_missing",
                "Identity feature not available on current plan",
            )
        })
}

fn map_challenge_error(err: IdentityChallengeError) -> Response {
    match err {
        IdentityChallengeError::ChallengeExpired => {
            error_response(StatusCode::GONE, "challenge_expired", "Challenge expired")
        }
        IdentityChallengeError::ChallengeAlreadyUsed => error_response(
            StatusCode::CONFLICT,
            "challenge_already_used",
            "Challenge already used",
        ),
        IdentityChallengeError::ChallengeNotFound => error_response(
            StatusCode::NOT_FOUND,
            "challenge_not_found",
            "Challenge not found",
        ),
        IdentityChallengeError::Database(_) => error_response(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal_error",
            "Internal server error",
        ),
    }
}

fn verify_ed25519_signature(public_key_hex: &str, message: &[u8], signature_hex: &str) -> bool {
    let Ok(pk_bytes) = hex::decode(public_key_hex) else {
        return false;
    };
    let Ok(sig_bytes) = hex::decode(signature_hex) else {
        return false;
    };
    let Ok(pk_array): Result<[u8; 32], _> = pk_bytes.try_into() else {
        return false;
    };
    let Ok(sig_array): Result<[u8; 64], _> = sig_bytes.try_into() else {
        return false;
    };
    let Ok(verifying_key) = VerifyingKey::from_bytes(&pk_array) else {
        return false;
    };
    let signature = Signature::from_bytes(&sig_array);
    verifying_key.verify(message, &signature).is_ok()
}
