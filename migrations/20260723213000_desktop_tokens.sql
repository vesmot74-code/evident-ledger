-- Stage 13.4: Desktop authentication tokens (hash-only storage).

CREATE TABLE desktop_tokens (
    id UUID PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts(account_id),
    token_hash TEXT NOT NULL UNIQUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    revoked_at TIMESTAMPTZ NULL,
    last_used_at TIMESTAMPTZ NULL
);

CREATE INDEX desktop_tokens_account_id_idx ON desktop_tokens(account_id);
CREATE INDEX desktop_tokens_lookup_idx ON desktop_tokens(token_hash)
    WHERE revoked_at IS NULL;
