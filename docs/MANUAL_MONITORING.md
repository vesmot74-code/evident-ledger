# Manual Monitoring Guide (Interim)

Until a dedicated `/health` endpoint exists (Deferred — Stage 11.5 / M5, [DEPLOYMENT_FINDINGS.md](DEPLOYMENT_FINDINGS.md)), use these **existing** signals. Do **not** implement `/health` as part of this guide.

Related: [DEPLOYMENT.md](DEPLOYMENT.md), [audits/STAGE_11_4_WEBHOOK_RELIABILITY.md](audits/STAGE_11_4_WEBHOOK_RELIABILITY.md).

---

## 1. Liveness proxies (no new code)

| Check | Command / expectation |
|---|---|
| HTTP up | `curl -s -o /dev/null -w '%{http_code}\n' http://127.0.0.1:3000/` → `200` (landing) |
| Signing identity | `curl -s http://127.0.0.1:3000/identity` → JSON with `algorithm` + `public_key` |
| Public verify route | `GET /public/verify?file_hash=<64-hex>` → JSON existence payload or structured error; headers may include `x-ratelimit-*` |

`/identity` and the landing page are the best lightweight liveness proxies today. They are **not** full readiness (DB deep checks).

---

## 2. Startup log checklist

On each start, confirm:

```text
Public key: …
Signing key path: …          # matches SIGNING_KEY_PATH
Environment: production
Evident Ledger running on http://0.0.0.0:3000
```

**Red flags:**

| Log / behavior | Meaning |
|---|---|
| Panic: `DEV_MODE cannot be enabled in production` | Bad env mix |
| Panic: `SIGNING_KEY_PATH must be set` / refusing auto-create | Missing production key path |
| `WARNING: created new server signing key` | Unexpected new trust anchor — **stop** and investigate ([SIGNING_KEY_OPERATIONS.md](SIGNING_KEY_OPERATIONS.md)) |
| `Dev mode: enabled` | Must not appear in pilot production-like runs |
| `DB connection failed` | Postgres / `DATABASE_URL` |

---

## 3. Runtime signals

| Signal | Where |
|---|---|
| Process listening | `lsof -nP -iTCP:3000 -sTCP:LISTEN` |
| Public verification audit | Structured `public_verification_audit` log lines (outcome, rate_limit_action) |
| Auth / API errors | HTTP status from client reports; correlate timestamps in server log |
| Usage | `usage_monthly` for the pilot `account_id` |

---

## 4. Paddle webhook failures (human attention)

Classification from Stage 11.4 ([audits/STAGE_11_4_WEBHOOK_RELIABILITY.md](audits/STAGE_11_4_WEBHOOK_RELIABILITY.md)):

| Class | HTTP | Operator action |
|---|---|---|
| **Temporary** (DB, `PlanNotFound`, in-flight race, etc.) | `500 temporary_failure` | Paddle will retry. Ensure price↔plan mapping and DB health. Watch `paddle_webhook_events` rows in `failed` / `received`. |
| **Permanent** (bad signature, malformed payload, missing required fields) | `4xx` | Fix configuration / ignore poison messages; retries will not help until payload/secret is correct. |

```sql
SELECT event_id, event_type, status, created_at, updated_at
FROM paddle_webhook_events
WHERE status IN ('failed', 'received', 'processing')
ORDER BY created_at DESC
LIMIT 50;
```

Also confirm the Paddle notification destination shows successful deliveries after incidents.

---

## 5. Cadence suggestion (controlled pilot)

| Frequency | Action |
|---|---|
| After every deploy / restart | Startup log + `/identity` + landing `200` |
| During first pilot sessions | Watch logs while pilot runs first commit/verify |
| Daily (manual) | Landing + `/identity`; skim webhook table for `failed` |
| On billing complaints | Webhook table + account `subscription_status` / plan name |
