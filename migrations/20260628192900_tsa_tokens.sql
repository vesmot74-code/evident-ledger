CREATE TABLE tsa_tokens (
    chain_id UUID NOT NULL REFERENCES chains(chain_id),
    event_id UUID NOT NULL REFERENCES events(event_id),
    merkle_root TEXT NOT NULL,
    tsa_token BYTEA NOT NULL,
    tsa_timestamp BIGINT NOT NULL,
    tsa_serial TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (chain_id, merkle_root)
);
