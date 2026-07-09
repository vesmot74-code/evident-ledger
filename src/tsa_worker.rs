use notary_tsa::{HashAlgorithm, Rfc3161Client};
use sqlx::PgPool;
use uuid::Uuid;

const FREETSA_URL: &str = "https://freetsa.org/tsr";

pub async fn stamp_chain(pool: &PgPool, chain_id: Uuid, merkle_root: &str, head_event_id: Uuid) {
    let hash_bytes = match hex::decode(merkle_root) {
        Ok(b) if b.len() == 32 => b,
        _ => {
            eprintln!("TSA: invalid merkle_root hex");
            return;
        }
    };

    let client = Rfc3161Client::new(FREETSA_URL, 30, 2, HashAlgorithm::Sha256);

    match client.timestamp(&hash_bytes).await {
        Ok(resp) => {
            let result = sqlx::query!(
                r#"
                INSERT INTO tsa_tokens (chain_id, event_id, merkle_root, tsa_token, tsa_timestamp, tsa_serial)
                VALUES ($1, $2, $3, $4, $5, $6)
                ON CONFLICT (chain_id, merkle_root) DO NOTHING
                "#,
                chain_id,
                head_event_id,
                merkle_root,
                resp.token.as_slice(),
                resp.timestamp as i64,
                resp.serial,
            )
            .execute(pool)
            .await;

            match result {
                Ok(_) => println!(
                    "TSA: stamped chain {} root {} ts={}",
                    chain_id, merkle_root, resp.timestamp
                ),
                Err(e) => eprintln!("TSA: db error: {}", e),
            }
        }
        Err(e) => eprintln!("TSA: request failed: {}", e),
    }
}
