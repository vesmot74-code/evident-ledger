CREATE TABLE backups (
    backup_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    chain_id UUID NOT NULL REFERENCES chains(chain_id) ON DELETE CASCADE,
    account_id UUID NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    storage_path TEXT NOT NULL,
    event_count INT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_backups_account ON backups(account_id);
CREATE INDEX idx_backups_chain ON backups(chain_id);
