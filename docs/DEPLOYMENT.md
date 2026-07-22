# Evident Ledger — Deployment Guide

**Stage 11.1 — Deployment readiness (pilot / first production).**

This document describes how to configure, migrate, build, and run Evident Ledger.
It does **not** cover Docker/Kubernetes automation.

Related: [DEPLOYMENT_FINDINGS.md](DEPLOYMENT_FINDINGS.md), [testing.md](testing.md), [BILLING_MODEL.md](BILLING_MODEL.md).

---

## Requirements

| Component | Guidance |
|-----------|----------|
| **Rust** | Stable toolchain, edition 2021. Verified with rustc 1.96+ locally; use a current stable release. |
| **PostgreSQL** | 14+ recommended (SQLx 0.7 / Postgres features used by migrations). |
| **OS** | Linux or macOS for pilot. Server binds `0.0.0.0:3000` (port not configurable via env today). |
| **sqlx-cli** | Required to apply migrations: `cargo install sqlx-cli --no-default-features --features rustls,postgres` |
| **Reverse proxy** | Recommended in production (TLS termination). Set `TRUST_PROXY_HEADERS=true` only if the proxy is trusted. |

Optional external services:

| Service | Role |
|---------|------|
| **Paddle Billing** | Checkout, subscriptions, webhooks |
| **FreeTSA** (`freetsa.org`) | Machine TSA stamps (hardcoded URL in current build; not configurable) |

---

## Configuration

Load variables from the environment or a local `.env` file (`dotenvy` at startup).  
Template: [`.env.example`](../.env.example).

### Required production variables

| Variable | Purpose |
|----------|---------|
| `DATABASE_URL` | Postgres connection string. Startup expects this; process panics if missing or unreachable. |
| `PADDLE_API_KEY` | Server-side Paddle API key. **Required** — panic if unset (non-test). |
| `PADDLE_WEBHOOK_SECRET` | HMAC secret for `POST /paddle/webhook`. **Required** — panic if unset. |
| `PADDLE_CLIENT_TOKEN` | Public Paddle.js client token for Dashboard overlay. **Required** — panic if unset. |

### Optional / recommended

| Variable | Default | Purpose |
|----------|---------|---------|
| `PADDLE_API_BASE_URL` | `https://api.paddle.com` | Use `https://sandbox-api.paddle.com` for sandbox. |
| `TRUST_PROXY_HEADERS` | `false` | Trust `X-Forwarded-For` / `X-Real-IP` for rate limits. |
| `EVIDENT_BACKUP_DIR` | `~/.evident/backups` | Server backup artifact directory. |
| `DEV_MODE` | `false` | **Must be unset/false in production.** |
| `APP_ENV` | unset | If set to `development`, enables the same flags as `DEV_MODE`. |
| `TEST_DATABASE_URL` | — | Tests only; never point at production. |

### Not environment-configured today

| Concern | Behavior |
|---------|----------|
| HTTP listen address/port | Hardcoded `0.0.0.0:3000` |
| Session cookie name / TTL | `evident_session`, 30 days (code constants) |
| Server signing key path | `signing_key.bin` in process **current working directory** |
| TSA endpoint | Hardcoded FreeTSA URL |

### Session / secrets model

- Web sessions are random tokens stored as hashes in Postgres (`sessions` table). There is **no** separate `SESSION_SECRET` env var.
- Session cookies are `HttpOnly; SameSite=Lax`. The `Secure` flag is set when **`DEV_MODE` is off**.
- API keys (`ev_…`) are created in Dashboard / Accounts API; CLI uses `EVIDENT_API_KEY` or `~/.evident/api_key` (client-side, not server env).

---

## Database Setup

Migrations live in `migrations/` and are applied with **sqlx-cli**.  
The application **does not** run migrations on startup.

### Empty database → migrate → start

```bash
# 1. Create database
createdb evident_ledger   # or equivalent

# 2. Apply all migrations in timestamp order
export DATABASE_URL=postgresql://USER:PASS@HOST:5432/evident_ledger
sqlx migrate run

# 3. Confirm
sqlx migrate info
```

No manual SQL steps are required beyond `sqlx migrate run` for a clean database.

Seed note: tariff plan rows and Paddle `paddle_price_id` values come from migrations / ops updates. Ensure production `tariff_plans.paddle_price_id` values match your Paddle catalog before enabling checkout.

---

## Build

```bash
# From repository root
cargo build --release --bin evident-ledger --bin evident --bin evident-verify
```

Binaries:

| Binary | Role |
|--------|------|
| `target/release/evident-ledger` | HTTP API + Dashboard server |
| `target/release/evident` | Operator / customer CLI |
| `target/release/evident-verify` | Offline proof verifier (used by `evident verify`) |

**Reproducible builds:** prefer committing `Cargo.lock` for deployment pins (see [DEPLOYMENT_FINDINGS.md](DEPLOYMENT_FINDINGS.md)).

---

## Run

```bash
# Ensure env is loaded (dotenv or export)
export DATABASE_URL=...
export PADDLE_API_KEY=...
export PADDLE_WEBHOOK_SECRET=...
export PADDLE_CLIENT_TOKEN=...
# export PADDLE_API_BASE_URL=https://api.paddle.com

# Run from a stable working directory (signing_key.bin is created/read here)
cd /var/lib/evident   # example
./path/to/evident-ledger
```

On success the process prints:

```text
Public key: <hex>
Evident Ledger running on http://0.0.0.0:3000
```

If `DEV_MODE` / `APP_ENV=development` is set, it also prints that dev mode is enabled — **that must not appear in production**.

Point Paddle’s webhook destination at:

```text
https://<your-public-host>/paddle/webhook
```

Minimum subscription events: `subscription.created`, `subscription.updated`, `subscription.canceled` (and optionally `subscription.past_due`).

---

## Health Check

There is **no** dedicated `/health` or `/ready` route yet.

Operational checks for a pilot:

```bash
# Process listening
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:3000/

# Expect 200 (landing HTML)

# Login page
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:3000/login

# Webhook path rejects unsigned bodies (expect 401)
curl -s -o /dev/null -w '%{http_code}\n' \
  -X POST http://127.0.0.1:3000/paddle/webhook \
  -H 'content-type: application/json' \
  -d '{}'
```

Database readiness: if `DATABASE_URL` is wrong, the process exits at startup (`DB connection failed`).

---

## Backup considerations

Preserve at least:

| Asset | Why |
|-------|-----|
| **PostgreSQL database** | Accounts, chains, events, proofs metadata, sessions, billing/webhook state, identity keys (public) |
| **Environment secrets** | `DATABASE_URL`, Paddle keys/secrets, any proxy config — store in a secrets manager, not git |
| **`signing_key.bin`** | Server Ed25519 signing key; losing it breaks signature continuity for new proofs |
| **Paddle Dashboard config** | Price IDs, webhook destination URL + secret, client token, default payment link |
| **`tariff_plans` catalog** | Especially `paddle_price_id` mappings |
| **Optional `EVIDENT_BACKUP_DIR`** | Chain backup JSON artifacts if server backups are used |

CLI local data (`~/.evident/`) is per-operator/client and is separate from server backups.

---

## Production checklist (short)

- [ ] `DEV_MODE` unset and `APP_ENV` not `development`
- [ ] TLS via reverse proxy; prefer `Secure` cookies (dev mode off)
- [ ] `TRUST_PROXY_HEADERS` only if proxy is trusted
- [ ] Migrations applied (`sqlx migrate run`)
- [ ] Paddle live/sandbox base URL matches credentials
- [ ] Webhook secret matches Paddle notification destination
- [ ] Stable CWD for `signing_key.bin`; key backed up
- [ ] `tariff_plans.paddle_price_id` match catalog
- [ ] Landing / login reachable; unsigned webhook returns 401
