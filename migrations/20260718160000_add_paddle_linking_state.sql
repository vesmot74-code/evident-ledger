-- Stage 8.2d: Paddle account linking hardening.

ALTER TABLE paddle_webhook_events
    ALTER COLUMN account_id DROP NOT NULL;

ALTER TABLE paddle_webhook_events
    DROP CONSTRAINT IF EXISTS paddle_webhook_events_status_check;

ALTER TABLE paddle_webhook_events
    ADD CONSTRAINT paddle_webhook_events_status_check
    CHECK (status IN (
        'received',
        'processing',
        'processed',
        'failed',
        'waiting_for_account_link'
    ));

CREATE TABLE paddle_pending_links (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    paddle_customer_id TEXT NOT NULL,
    paddle_email TEXT NOT NULL,
    account_id UUID NULL REFERENCES accounts(account_id),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ NULL
);

CREATE INDEX idx_paddle_pending_links_customer_id
    ON paddle_pending_links(paddle_customer_id);

CREATE INDEX idx_paddle_pending_links_unresolved
    ON paddle_pending_links(paddle_customer_id)
    WHERE resolved_at IS NULL;
