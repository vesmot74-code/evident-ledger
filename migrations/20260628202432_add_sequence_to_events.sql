ALTER TABLE events ADD COLUMN sequence BIGSERIAL NOT NULL;
CREATE INDEX idx_events_chain_sequence ON events(chain_id, sequence ASC);
