# Pilot Deployment Checklist

Ordered runbook for a controlled pilot host. Detail lives in linked docs — this file is the **sequence**.

Related: [DEPLOYMENT.md](DEPLOYMENT.md), [DEPLOYMENT_FINDINGS.md](DEPLOYMENT_FINDINGS.md), [`.env.example`](../.env.example), [SIGNING_KEY_OPERATIONS.md](SIGNING_KEY_OPERATIONS.md).

---

## 1. Provision infrastructure (DB, host)

- [ ] PostgreSQL 14+ available; create an empty database dedicated to this deployment (not the shared dev DB). See [DEPLOYMENT.md — Requirements / Database Setup](DEPLOYMENT.md).
- [ ] Host can run release binaries; port **3000** free (listen address is hardcoded — see DEPLOYMENT.md).
- [ ] Reverse proxy / TLS planned if the host is exposed (set `TRUST_PROXY_HEADERS` only behind a trusted proxy).

```bash
# Example — adjust user/host/db name for your environment
createdb evident_ledger_pilot
```

---

## 2. Set required environment variables

Template: [`.env.example`](../.env.example). Full table: [DEPLOYMENT.md — Configuration](DEPLOYMENT.md).

Minimum for pilot production-like mode:

```bash
export ENVIRONMENT=production
export DEV_MODE=false          # must NOT be true
# unset APP_ENV or ensure it is not "development"
export DATABASE_URL=postgresql://USER:PASS@HOST:5432/evident_ledger_pilot
export SIGNING_KEY_PATH=/absolute/path/to/signing_key.bin
export PADDLE_API_KEY=...
export PADDLE_WEBHOOK_SECRET=...
export PADDLE_CLIENT_TOKEN=...
# Sandbox pilot:
# export PADDLE_API_BASE_URL=https://sandbox-api.paddle.com
```

- [ ] Confirm `DEV_MODE=true` is absent.
- [ ] Confirm `SIGNING_KEY_PATH` is absolute and points at the **intended** trust-anchor key ([SIGNING_KEY_OPERATIONS.md](SIGNING_KEY_OPERATIONS.md)).

---

## 3. Run migrations (before first start)

Application does **not** migrate on startup ([DEPLOYMENT_FINDINGS.md](DEPLOYMENT_FINDINGS.md)).

```bash
export DATABASE_URL=...   # same URL as the service will use
sqlx migrate run
sqlx migrate info
```

- [ ] All migrations applied with no error.
- [ ] Ops seed: set `tariff_plans.paddle_price_id` to match the Paddle sandbox/live catalog before enabling checkout ([DEPLOYMENT.md](DEPLOYMENT.md) seed note; Stage 11.6 finding).

---

## 4. Verify signing key location and backup **before** first start

- [ ] Key file exists at `SIGNING_KEY_PATH`; mode `0600`.
- [ ] Off-host backup exists and sha256 matches active key ([SIGNING_KEY_OPERATIONS.md](SIGNING_KEY_OPERATIONS.md)).
- [ ] No unmanaged CWD `./signing_key.bin` will be used by mistake (production uses `SIGNING_KEY_PATH` only).

```bash
test -f "$SIGNING_KEY_PATH"
shasum -a 256 "$SIGNING_KEY_PATH"
# compare to off-host backup sha256 in operator inventory
```

---

## 5. Build and start service

```bash
cargo build --release --bin evident-ledger --bin evident
./target/release/evident-ledger
# or: nohup ./target/release/evident-ledger >>/var/log/evident-ledger.log 2>&1 &
```

See [DEPLOYMENT.md — Build / Run](DEPLOYMENT.md).

---

## 6. Verify startup

Expect stdout (or log) similar to:

```text
Public key: <64-hex>
Signing key path: <absolute SIGNING_KEY_PATH>
Environment: production
Evident Ledger running on http://0.0.0.0:3000
```

- [ ] No panic (`DEV_MODE`/`SIGNING_KEY_PATH`/Paddle/DB).
- [ ] No `Dev mode: enabled` banner.
- [ ] No `WARNING: created new server signing key`.
- [ ] Public key matches the backed-up trust anchor.

```bash
curl -s http://127.0.0.1:3000/identity
curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:3000/
# expect 200
```

---

## 7. Smoke-check

```bash
# Register a throwaway account (Dashboard / API)
curl -s -X POST http://127.0.0.1:3000/auth/register \
  -H 'content-type: application/json' \
  -d '{"email":"pilot-smoke@example.com","password":"ReplaceMe1!"}'

# Public verify reachable (400 without valid hash is OK — proves route + rate-limit headers)
curl -sI 'http://127.0.0.1:3000/public/verify' | head
curl -s 'http://127.0.0.1:3000/public/verify?file_hash=aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa'
```

- [ ] Register returns success for a new email.
- [ ] `/public/verify` responds (see [MANUAL_MONITORING.md](MANUAL_MONITORING.md)).

Full scenario matrix: [audits/STAGE_11_6_PILOT_SMOKE_TEST.md](audits/STAGE_11_6_PILOT_SMOKE_TEST.md).

---

## 8. Confirm Paddle webhook endpoint from Paddle

- [ ] Notification destination URL reaches this host (TLS + reverse proxy / tunnel as required).
- [ ] Destination secret matches `PADDLE_WEBHOOK_SECRET`.
- [ ] Send a test notification or complete a sandbox checkout; confirm row in `paddle_webhook_events` and no stuck `failed` without retry (Stage 11.4 — [audits/STAGE_11_4_WEBHOOK_RELIABILITY.md](audits/STAGE_11_4_WEBHOOK_RELIABILITY.md)).

```text
POST https://<public-host>/paddle/webhook
```
