use std::collections::HashMap;
use std::sync::Mutex;

use super::model::{AccountId, IdempotencyRecord};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdempotencyStoreError {
    Conflict,
}

pub trait IdempotencyRepository: Send + Sync {
    fn find(
        &self,
        account_id: AccountId,
        idempotency_key: &str,
    ) -> impl std::future::Future<Output = Result<Option<IdempotencyRecord>, IdempotencyStoreError>> + Send;

    fn insert(
        &self,
        record: IdempotencyRecord,
    ) -> impl std::future::Future<Output = Result<(), IdempotencyStoreError>> + Send;
}

/// In-memory implementation for unit tests and future wiring (Step 5b).
/// Not connected to PostgreSQL on this step.
pub struct InMemoryIdempotencyRepository {
    records: Mutex<HashMap<(AccountId, String), IdempotencyRecord>>,
}

impl InMemoryIdempotencyRepository {
    pub fn new() -> Self {
        Self {
            records: Mutex::new(HashMap::new()),
        }
    }
}

impl Default for InMemoryIdempotencyRepository {
    fn default() -> Self {
        Self::new()
    }
}

impl IdempotencyRepository for InMemoryIdempotencyRepository {
    async fn find(
        &self,
        account_id: AccountId,
        idempotency_key: &str,
    ) -> Result<Option<IdempotencyRecord>, IdempotencyStoreError> {
        let records = self
            .records
            .lock()
            .expect("idempotency repository lock poisoned");
        Ok(records
            .get(&(account_id, idempotency_key.to_string()))
            .cloned())
    }

    async fn insert(&self, record: IdempotencyRecord) -> Result<(), IdempotencyStoreError> {
        let mut records = self
            .records
            .lock()
            .expect("idempotency repository lock poisoned");
        let key = (record.account_id, record.idempotency_key.clone());
        if records.contains_key(&key) {
            return Err(IdempotencyStoreError::Conflict);
        }
        records.insert(key, record);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{Duration, Utc};
    use serde_json::json;
    use uuid::Uuid;

    fn sample_record(account_id: Uuid, key: &str, request_hash: &str) -> IdempotencyRecord {
        let now = Utc::now();
        IdempotencyRecord {
            id: Uuid::new_v4(),
            account_id,
            idempotency_key: key.to_string(),
            request_hash: request_hash.to_string(),
            response_json: json!({ "event_id": "event-1" }),
            created_at: now,
            expires_at: now + Duration::hours(24),
        }
    }

    #[tokio::test]
    async fn find_returns_none_for_missing_key() {
        let repo = InMemoryIdempotencyRepository::new();
        let found = repo
            .find(Uuid::new_v4(), "missing")
            .await
            .expect("find should succeed");
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn insert_and_find_roundtrip() {
        let repo = InMemoryIdempotencyRepository::new();
        let account_id = Uuid::new_v4();
        let record = sample_record(account_id, "key-1", "hash-a");

        repo.insert(record.clone())
            .await
            .expect("insert should succeed");

        let found = repo
            .find(account_id, "key-1")
            .await
            .expect("find should succeed")
            .expect("record should exist");

        assert_eq!(found.request_hash, "hash-a");
        assert_eq!(found.response_json, record.response_json);
    }

    #[tokio::test]
    async fn duplicate_insert_returns_conflict() {
        let repo = InMemoryIdempotencyRepository::new();
        let account_id = Uuid::new_v4();
        let record = sample_record(account_id, "key-1", "hash-a");

        repo.insert(record.clone())
            .await
            .expect("first insert should succeed");
        let err = repo.insert(record).await.unwrap_err();
        assert_eq!(err, IdempotencyStoreError::Conflict);
    }

    #[tokio::test]
    async fn same_key_different_accounts_do_not_collide() {
        let repo = InMemoryIdempotencyRepository::new();
        let account_a = Uuid::new_v4();
        let account_b = Uuid::new_v4();

        repo.insert(sample_record(account_a, "shared-key", "hash-a"))
            .await
            .expect("insert account_a");
        repo.insert(sample_record(account_b, "shared-key", "hash-b"))
            .await
            .expect("insert account_b");

        let found_a = repo
            .find(account_a, "shared-key")
            .await
            .expect("find a")
            .expect("record a");
        let found_b = repo
            .find(account_b, "shared-key")
            .await
            .expect("find b")
            .expect("record b");

        assert_eq!(found_a.request_hash, "hash-a");
        assert_eq!(found_b.request_hash, "hash-b");
    }
}
