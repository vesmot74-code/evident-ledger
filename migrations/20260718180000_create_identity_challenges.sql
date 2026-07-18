-- Stage 9.2: proof-of-possession challenges for identity key registration.

CREATE TABLE IF NOT EXISTS identity_challenges (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id UUID NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    challenge TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL DEFAULT now() + interval '5 minutes',
    used_at TIMESTAMPTZ NULL
);

CREATE INDEX idx_identity_challenges_account_id ON identity_challenges(account_id);
CREATE INDEX idx_identity_challenges_expires_at ON identity_challenges(expires_at);
