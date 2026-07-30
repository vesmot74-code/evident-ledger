-- Stage: TSA read-path verification cache (RFC3161 re-verify + result).
-- Cache stores the outcome of an independent cryptographic check; it is not
-- itself a source of trust without the ability to reproduce verification.

ALTER TABLE tsa_tokens
    ADD COLUMN IF NOT EXISTS verified_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS verification_status TEXT,
    ADD COLUMN IF NOT EXISTS token_sha256 TEXT;

COMMENT ON COLUMN tsa_tokens.verification_status IS
    'verified | failed | unavailable — outcome of last cryptographic check';
COMMENT ON COLUMN tsa_tokens.token_sha256 IS
    'SHA-256 hex of tsa_token bytes used for the cached verification';
COMMENT ON COLUMN tsa_tokens.verified_at IS
    'When verification_status was last written after a fresh check';
