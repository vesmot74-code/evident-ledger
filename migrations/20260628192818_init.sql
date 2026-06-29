CREATE TABLE chains (
    chain_id UUID PRIMARY KEY,
    head_event_id UUID NULL,
    status TEXT NOT NULL DEFAULT 'active',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE events (
    event_id UUID PRIMARY KEY,
    chain_id UUID NOT NULL,
    parent_event_id UUID NOT NULL,
    file_hash TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    signature TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),

    CONSTRAINT uniq_idem UNIQUE (chain_id, idempotency_key)
);

CREATE INDEX idx_events_chain ON events(chain_id);
