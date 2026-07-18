-- Stage 9.1: user identity key storage.

CREATE TABLE IF NOT EXISTS identity_keys (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id UUID NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    public_key TEXT NOT NULL,
    fingerprint TEXT NOT NULL UNIQUE,
    label TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    verified_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ NULL
);

CREATE INDEX idx_identity_keys_account_id ON identity_keys(account_id);
