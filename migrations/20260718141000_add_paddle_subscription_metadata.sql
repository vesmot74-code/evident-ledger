-- Stage 8.2b: link accounts to Paddle subscriptions.
ALTER TABLE accounts
ADD COLUMN paddle_subscription_id TEXT NULL;

CREATE UNIQUE INDEX idx_accounts_paddle_subscription_id
    ON accounts(paddle_subscription_id)
    WHERE paddle_subscription_id IS NOT NULL;
