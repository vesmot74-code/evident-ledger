CREATE TABLE idempotency_records (
    id UUID PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    idempotency_key TEXT NOT NULL,
    request_hash TEXT NOT NULL,
    response_json JSONB NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    CONSTRAINT uniq_idempotency_account_key UNIQUE (account_id, idempotency_key)
);

CREATE INDEX idx_idempotency_records_expires_at ON idempotency_records(expires_at);
