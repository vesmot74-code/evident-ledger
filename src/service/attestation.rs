use sqlx::PgPool;
use uuid::Uuid;
use std::sync::Arc;
use crate::signing::ServerSigner;
use crate::service::verifier::verify_chain_hardened;
use crate::sac::*;

pub async fn build_attestation(
    pool: &PgPool,
    signer: &Arc<ServerSigner>,
    chain_id: Uuid,
) -> Result<SacDocument, sqlx::Error> {
    let report = verify_chain_hardened(pool, chain_id).await?;

    if report.blocks == 0 {
        return Ok(SacDocument {
            version: "1.0".into(),
            issued_at: chrono::Utc::now().to_rfc3339(),
            target: SacTarget::ChainId(chain_id.to_string()),
            state: None,
            tsa: None,
            verification: SacVerification {
                status: SacVerificationStatus::NotFound,
                signature: None,
                public_key_fingerprint: None,
                errors: report.errors,
            },
            exclusions: SacExclusions::default(),
        });
    }

    let head_event_id = report.head_event_id.expect("non-empty chain must have head");
    let merkle_root = report.merkle_recomputed.clone();
    let signature = signer.sign_root(&chain_id.to_string(), &merkle_root, &head_event_id.to_string());
    let public_key_fingerprint = signer.public_key_hex();

    let tsa_row = sqlx::query!(
        r#"SELECT tsa_timestamp, tsa_serial
           FROM tsa_tokens WHERE chain_id = $1 AND merkle_root = $2"#,
        chain_id, merkle_root
    )
    .fetch_optional(pool)
    .await?;

    let tsa = Some(match tsa_row {
        Some(row) => SacTsaSnapshot {
            status: SacTsaStatus::Present,
            provider: Some("freetsa.org/tsr".into()),
            timestamp: Some(row.tsa_timestamp),
            serial: Some(row.tsa_serial),
        },
        None => SacTsaSnapshot {
            status: SacTsaStatus::Missing,
            provider: None,
            timestamp: None,
            serial: None,
        },
    });

    let last_event_timestamp = report
        .last_event_created_at
        .map(|t| t.to_rfc3339())
        .unwrap_or_default();

    Ok(SacDocument {
        version: "1.0".into(),
        issued_at: chrono::Utc::now().to_rfc3339(),
        target: SacTarget::ChainId(chain_id.to_string()),
        state: Some(SacChainState {
            chain_id: chain_id.to_string(),
            merkle_root,
            head_event_id: head_event_id.to_string(),
            last_event_timestamp,
        }),
        tsa,
        verification: SacVerification {
            status: if report.valid { SacVerificationStatus::Verified } else { SacVerificationStatus::Failed },
            signature: if report.valid { Some(signature) } else { None },
            public_key_fingerprint: if report.valid { Some(public_key_fingerprint) } else { None },
            errors: report.errors,
        },
        exclusions: SacExclusions::default(),
    })
}
