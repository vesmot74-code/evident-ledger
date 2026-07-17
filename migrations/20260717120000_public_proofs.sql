-- Materialized internal proof registry for the public layer (Stage 6.1).
-- Populated when an internal proof transitions to Anchored via on_proof_anchored().
-- Not a duplicate of events/chains — runtime proof status in /v1 still uses events + resolve_proof_state().
CREATE TABLE proofs (
    id UUID PRIMARY KEY,
    file_hash TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_proofs_file_hash_anchored_created
    ON proofs (file_hash, created_at)
    WHERE status = 'Anchored';

CREATE TABLE public_proofs (
    id UUID PRIMARY KEY,
    public_id TEXT UNIQUE NOT NULL,
    proof_id UUID NOT NULL REFERENCES proofs(id),
    file_hash TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX idx_public_proofs_file_hash_active
    ON public_proofs (file_hash)
    WHERE enabled = true;
