-- This table is a materialized public verification registry.
-- It is not a source of truth for evidence verification.
-- Core verification remains derived from ledger events and chain state.
CREATE TABLE public_proof_registry (
    id UUID PRIMARY KEY,
    file_hash TEXT NOT NULL,
    status TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_public_proof_registry_file_hash_anchored_created
    ON public_proof_registry (file_hash, created_at)
    WHERE status = 'Anchored';

CREATE TABLE public_proofs (
    id UUID PRIMARY KEY,
    public_id TEXT UNIQUE NOT NULL,
    proof_id UUID NOT NULL REFERENCES public_proof_registry(id),
    file_hash TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX idx_public_proof_registry_active_hash
    ON public_proofs (file_hash)
    WHERE enabled = true;
