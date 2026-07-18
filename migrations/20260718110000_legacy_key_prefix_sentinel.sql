-- Normalize first backfill sentinel to the canonical legacy marker.
UPDATE api_keys
SET key_prefix = 'legacy:no-prefix'
WHERE key_prefix = 'ev_legacy';
