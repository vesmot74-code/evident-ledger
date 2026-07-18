-- Stage 9.6: identity key audit trail.

CREATE TABLE identity_key_audit_events (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    key_id UUID NOT NULL REFERENCES identity_keys(id),
    actor_type TEXT NOT NULL CHECK (actor_type IN ('account')),
    actor_id UUID NOT NULL,
    action TEXT NOT NULL CHECK (action IN ('created', 'verified', 'revoked')),
    metadata JSONB NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_identity_key_audit_events_key_id ON identity_key_audit_events(key_id);
CREATE INDEX idx_identity_key_audit_events_actor ON identity_key_audit_events(actor_type, actor_id);
