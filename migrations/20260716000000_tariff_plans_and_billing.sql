-- Тарифные планы: юридические возможности + технические лимиты, конфигурация не в коде
CREATE TABLE tariff_plans (
    plan_id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    name TEXT UNIQUE NOT NULL,               -- 'free' | 'legal' | 'vault' | 'identity'
    display_name TEXT NOT NULL,

    -- юридические возможности
    tsa_mode TEXT NOT NULL DEFAULT 'machine' CHECK (tsa_mode IN ('machine', 'qualified')),
    server_backup BOOLEAN NOT NULL DEFAULT false,
    history_recovery BOOLEAN NOT NULL DEFAULT false,
    identity_enabled BOOLEAN NOT NULL DEFAULT false,

    -- технические лимиты (NULL = без лимита)
    monthly_commits_limit INTEGER NULL,
    monthly_tsa_limit INTEGER NULL,
    rps_limit INTEGER NOT NULL DEFAULT 1,
    storage_retention_years INTEGER NULL,     -- NULL = бессрочно

    -- Paddle
    paddle_price_id TEXT NULL,

    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

INSERT INTO tariff_plans (name, display_name, tsa_mode, server_backup, history_recovery, identity_enabled, monthly_commits_limit, monthly_tsa_limit, rps_limit, storage_retention_years)
VALUES
    ('free',     'Free',     'machine',   false, false, false, 100,   100,   1,   NULL),
    ('legal',    'Legal',    'qualified', false, false, false, 5000,  5000,  10,  NULL),
    ('vault',    'Vault',    'qualified', true,  true,  false, 50000, 50000, 50,  7),
    ('identity', 'Identity', 'qualified', true,  true,  true,  NULL,  NULL,  100, NULL);

-- accounts: заменяем tariff_tier на ссылку на план + данные Paddle-подписки
ALTER TABLE accounts ADD COLUMN tariff_plan_id UUID REFERENCES tariff_plans(plan_id);
ALTER TABLE accounts ADD COLUMN paddle_customer_id TEXT NULL UNIQUE;
ALTER TABLE accounts ADD COLUMN subscription_status TEXT NOT NULL DEFAULT 'none'
    CHECK (subscription_status IN ('none', 'active', 'past_due', 'canceled'));

-- существующие аккаунты переводим на free-план
UPDATE accounts SET tariff_plan_id = (SELECT plan_id FROM tariff_plans WHERE name = 'free')
WHERE tariff_plan_id IS NULL;

ALTER TABLE accounts ALTER COLUMN tariff_plan_id SET NOT NULL;
ALTER TABLE accounts DROP COLUMN tariff_tier;
ALTER TABLE accounts DROP COLUMN jurisdiction;

-- usage считается только по серверным операциям (не по локальной работе GUI/CLI)
CREATE TABLE usage_monthly (
    account_id UUID NOT NULL REFERENCES accounts(account_id) ON DELETE CASCADE,
    period_start DATE NOT NULL,               -- первое число месяца, UTC
    server_commits INTEGER NOT NULL DEFAULT 0,
    tsa_requests INTEGER NOT NULL DEFAULT 0,
    storage_bytes BIGINT NOT NULL DEFAULT 0,
    PRIMARY KEY (account_id, period_start)
);
