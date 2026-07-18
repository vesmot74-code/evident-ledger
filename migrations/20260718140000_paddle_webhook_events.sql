-- Stage 8.2b: Paddle webhook idempotency and audit log.
CREATE TABLE paddle_webhook_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    paddle_event_id TEXT UNIQUE NOT NULL,
    event_type TEXT NOT NULL,
    payload_hash TEXT NOT NULL,
    account_id UUID NOT NULL REFERENCES accounts(account_id),
    subscription_id TEXT NULL,
    event_occurred_at TIMESTAMPTZ NOT NULL,
    status TEXT NOT NULL DEFAULT 'received'
        CHECK (status IN ('received', 'processing', 'processed', 'failed')),
    error_message TEXT NULL,
    processing_started_at TIMESTAMPTZ NULL,
    processed_at TIMESTAMPTZ NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_paddle_webhook_events_account_id
    ON paddle_webhook_events(account_id);
