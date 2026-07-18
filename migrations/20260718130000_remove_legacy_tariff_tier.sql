-- Stage 8.2a cleanup:
-- tariff_plan_id replaced legacy tariff_tier.

ALTER TABLE accounts
DROP COLUMN IF EXISTS tariff_tier;
