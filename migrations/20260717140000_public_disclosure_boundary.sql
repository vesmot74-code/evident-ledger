-- Stage 6.4: Public disclosure boundary — safe projection schema.
-- public_proof_registry is the only table queried by public endpoints.
-- public_proof_materialization is internal bookkeeping (never used by /public/*).

DROP TABLE IF EXISTS public_proofs;
DROP TABLE IF EXISTS public_proof_registry;

CREATE TABLE public_proof_registry (
    public_proof_id TEXT PRIMARY KEY,
    file_hash TEXT NOT NULL,
    proof_status TEXT NOT NULL,
    registered_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    tsa_class TEXT NOT NULL,
    integrity_state TEXT NOT NULL,
    enabled BOOLEAN NOT NULL DEFAULT true
);

CREATE UNIQUE INDEX idx_public_proof_registry_active_hash
    ON public_proof_registry (file_hash)
    WHERE enabled = true;

CREATE INDEX idx_public_proof_registry_file_hash
    ON public_proof_registry (file_hash);

-- Internal materialization state — not queried by public endpoints.
CREATE TABLE public_proof_materialization (
    internal_proof_id UUID PRIMARY KEY,
    file_hash TEXT NOT NULL,
    public_proof_id TEXT REFERENCES public_proof_registry (public_proof_id),
    materialized_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    sticky_disabled BOOLEAN NOT NULL DEFAULT false
);

CREATE INDEX idx_public_proof_materialization_file_hash
    ON public_proof_materialization (file_hash, materialized_at);
