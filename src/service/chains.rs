use serde_json::{json, Value};
use sqlx::PgPool;
use uuid::Uuid;

pub async fn create_chain(pool: &PgPool) -> Result<Value, sqlx::Error> {
    let chain_id = Uuid::new_v4();

    sqlx::query!("INSERT INTO chains (chain_id) VALUES ($1)", chain_id)
        .execute(pool)
        .await?;

    Ok(json!({
        "chain_id": chain_id,
        "head_event_id": null,
        "status": "active"
    }))
}
