-- Stage 8.2b: tariff priority for upgrade/downgrade detection.
ALTER TABLE tariff_plans
ADD COLUMN priority INTEGER NOT NULL DEFAULT 0 CHECK (priority >= 0);

UPDATE tariff_plans SET priority = 0 WHERE name = 'free';
UPDATE tariff_plans SET priority = 1 WHERE name = 'legal';
UPDATE tariff_plans SET priority = 2 WHERE name = 'vault';
UPDATE tariff_plans SET priority = 3 WHERE name = 'identity';
