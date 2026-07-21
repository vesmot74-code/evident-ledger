# Testing database policy

Evident Ledger uses **two separate Postgres databases** so integration tests cannot
corrupt the local development catalog (especially `tariff_plans.paddle_price_id`).

| Role | Database | Env var |
|------|----------|---------|
| Dev / E2E server (`cargo run`) | `ledger` | `DATABASE_URL` |
| In-process integration & unit DB tests | `ledger_test` | `TEST_DATABASE_URL` |

## Why not one shared database?

Several billing and webhook tests temporarily rewrite `tariff_plans.paddle_price_id`
(for example to `pri_vault_test`). When those tests ran against `ledger`, the
working server started sending fake price IDs to Paddle and checkout failed with
`400`. Isolating tests on `ledger_test` prevents that class of incident.

## Bootstrap `ledger_test`

```bash
PGPASSWORD=ledger createdb -h localhost -p 5433 -U ledger ledger_test

# Apply the same migrations the app uses:
DATABASE_URL=postgres://ledger:ledger@localhost:5433/ledger_test sqlx migrate run
```

Add to your local `.env` (never commit real secrets):

```bash
DATABASE_URL=postgres://ledger:ledger@localhost:5433/ledger
TEST_DATABASE_URL=postgres://ledger:ledger@localhost:5433/ledger_test
```

## Guard

`evident_ledger::db::require_test_database_url()` (used by `tests/common`) refuses
any URL that contains `/ledger` but not `/ledger_test`. Tests panic early instead
of mutating the wrong database.

## Live-server tests

A small set of tests hit a running process on `:3000` and therefore must use the
**same** database as that process (`DATABASE_URL` via
`common::live_server_database_url()`). Those tests must **not** mutate shared
catalog rows such as `tariff_plans.paddle_price_id`.

## Known gap

In-process tests are **not** wrapped in a per-test SQL transaction with automatic
rollback. Catalog mutations still rely on manual restore helpers where present.
Per-test transaction rollback remains a desirable follow-up.

## Commands

```bash
cargo test -- --skip dev_tariff_switcher_end_to_end

# Confirm the server DB was not rewritten by tests:
PGPASSWORD=ledger psql -d ledger -U ledger -h localhost -p 5433 \
  -c "SELECT name, paddle_price_id FROM tariff_plans ORDER BY name;"
```
