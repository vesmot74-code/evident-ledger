ALTER TABLE events
    ADD CONSTRAINT events_chain_sequence_unique UNIQUE (chain_id, sequence);
