-- Stage 8.1: store display prefix for API keys (secret is never persisted).
ALTER TABLE api_keys ADD COLUMN key_prefix TEXT;

UPDATE api_keys SET key_prefix = 'ev_legacy' WHERE key_prefix IS NULL;

ALTER TABLE api_keys ALTER COLUMN key_prefix SET NOT NULL;
