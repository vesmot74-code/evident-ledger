CREATE INDEX idx_events_file_hash ON events(file_hash);
CREATE INDEX idx_events_file_hash_chain ON events(file_hash, chain_id);
