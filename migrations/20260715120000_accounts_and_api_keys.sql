CREATE TABLE accounts (
    account_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    email TEXT UNIQUE NOT NULL,
    tariff_tier INTEGER NOT NULL DEFAULT 1 CHECK (tariff_tier BETWEEN 1 AND 4),
    jurisdiction TEXT NOT NULL DEFAULT 'US' CHECK (jurisdiction IN ('US', 'EU')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE api_keys (
    api_key_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id UUID NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    key_hash TEXT NOT NULL UNIQUE,
    label TEXT NOT NULL DEFAULT 'default',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    revoked_at TIMESTAMPTZ NULL
);
CREATE INDEX idx_api_keys_account ON api_keys(account_id);

-- nullable: старые/тестовые цепочки останутся без владельца, это нормально
ALTER TABLE chains ADD COLUMN account_id UUID NULL REFERENCES accounts(account_id);
CREATE INDEX idx_chains_account ON chains(account_id);
