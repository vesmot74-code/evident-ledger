-- Stage 8.2a: billing period and scheduled downgrade (BILLING_MODEL.md freeze).
ALTER TABLE accounts
ADD COLUMN current_period_end TIMESTAMPTZ NULL;

ALTER TABLE accounts
ADD COLUMN pending_tariff_plan_id UUID NULL
REFERENCES tariff_plans(plan_id);

CREATE INDEX idx_accounts_pending_tariff_expiry
ON accounts(current_period_end)
WHERE pending_tariff_plan_id IS NOT NULL;
