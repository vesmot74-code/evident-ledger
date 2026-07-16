pub mod canonical;
pub mod model;
pub mod postgres;
pub mod repository;

pub use canonical::canonical_json_sha256;
pub use model::{AccountId, IdempotencyRecord};
pub use postgres::{find_active_in_tx, insert_in_tx, PostgresIdempotencyRepository};
pub use repository::{IdempotencyRepository, IdempotencyStoreError, InMemoryIdempotencyRepository};

/// Idempotency record lifetime (see docs/API_IMPLEMENTATION_PLAN.md).
pub const IDEMPOTENCY_TTL_HOURS: i64 = 24;
