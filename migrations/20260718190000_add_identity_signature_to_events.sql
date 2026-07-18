-- Stage 9.3: optional user identity signatures on events.

ALTER TABLE events
ADD COLUMN identity_key_id UUID NULL
    REFERENCES identity_keys(id) ON DELETE RESTRICT,
ADD COLUMN identity_signature TEXT NULL,
ADD COLUMN identity_fingerprint TEXT NULL;

CREATE INDEX idx_events_identity_key_id ON events(identity_key_id);
