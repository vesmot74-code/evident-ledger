# Pilot Onboarding Runbook (Operator)

Step-by-step for the **operator**, not the end user. Related: [BILLING_MODEL.md](BILLING_MODEL.md), [PILOT_DEPLOYMENT_CHECKLIST.md](PILOT_DEPLOYMENT_CHECKLIST.md), [MANUAL_MONITORING.md](MANUAL_MONITORING.md), [SIGNING_KEY_OPERATIONS.md](SIGNING_KEY_OPERATIONS.md).

---

## 1. Create the pilot account

**Actual path today:** self-service registration.

```bash
curl -s -X POST http://127.0.0.1:3000/auth/register \
  -H 'content-type: application/json' \
  -d '{"email":"pilot.user@example.com","password":"<strong-password>"}'
```

Or: open Dashboard UI → register.

There is **no** separate admin “create user” CLI in the product. Manual SQL account creation is not the supported path for pilot.

Verify:

```bash
# After login cookie / session from POST /auth/login
curl -s http://127.0.0.1:3000/auth/me -H "Cookie: evident_session=…"
```

---

## 2. Assign / verify tariff plan

Default after register: **free** (`subscription_status` typically `none`).

For a **constrained first pilot**, keep **free** so evidence commits use machine TSA (paid plans currently hit Qualified TSA unavailability — see limitations below).

If a paid plan is required later:

1. Ensure `tariff_plans.paddle_price_id` is seeded ([DEPLOYMENT.md](DEPLOYMENT.md)).
2. Pilot uses Dashboard upgrade → Paddle checkout → webhook updates tariff ([BILLING_MODEL.md](BILLING_MODEL.md)).
3. **Paid → paid** plan changes are **not** self-service while `subscription_status=active` (Dashboard returns already-active / contact support). Change via support / ops process documented in BILLING_MODEL.md.

Check:

```bash
curl -s http://127.0.0.1:3000/account/capabilities -H "X-API-KEY: ev_…"
# or Dashboard → subscription
```

---

## 3. Issue the first API key

Via Dashboard: Account → API Keys → create.

Or authenticated API (`/dashboard/api-keys` with session cookie).

**Operator check:**

- Plaintext `ev_…` key is shown **once** at creation.
- Subsequent list endpoints must **not** return the full secret (Stage 11.6).
- Pilot stores key in `EVIDENT_API_KEY` or `~/.evident/api_key` for CLI.

---

## 4. Verify first commit / verify from the pilot

```bash
export EVIDENT_API_KEY=ev_…
evident new-chain                    # or POST /chains
evident commit ./document.pdf --chain <chain_id>
# Pin server identity once per deployment:
curl -s http://127.0.0.1:3000/identity | jq -r .public_key > ~/.evident/server_identity.pub
evident verify ~/.evident/proofs/<chain_id>/<event_id>.json
# Expect: OK: proof valid
```

Operator confirms: event row exists; proof file present; verify OK against pinned pubkey matching `/identity`.

---

## 5. If the pilot reports an error

**First places to look (no `/health` yet — see [MANUAL_MONITORING.md](MANUAL_MONITORING.md)):**

| Symptom | Check |
|---|---|
| Cannot register / login | Server log; `accounts` / `sessions` tables; rate limits on register/login |
| API `401` | Key revoked? Wrong `X-API-KEY`? |
| `402 payment_required` | `subscription_status=past_due` — expected on paid writes ([BILLING_MODEL.md](BILLING_MODEL.md) §5) |
| `500` on commit (paid / identity) | Likely Qualified TSA unavailable — known limitation |
| `403 entitlement_missing` (identity) | Plan lacks `identity_enabled` |
| Checkout fails `plan_not_purchasable` | `paddle_price_id` NULL — seed catalog |
| Verify `signature invalid` | CLI pin `server_identity.pub` mismatch vs current `/identity` |
| Webhook / plan not updating | `paddle_webhook_events` status; Stage 11.4 temporary vs permanent |

Useful SQL (read-only):

```sql
SELECT email, subscription_status,
       (SELECT name FROM tariff_plans t WHERE t.plan_id = a.tariff_plan_id) AS plan
FROM accounts a WHERE email = 'pilot.user@example.com';

SELECT status, event_type, created_at
FROM paddle_webhook_events
ORDER BY created_at DESC LIMIT 20;
```

---

## 6. Tell the pilot up front (known limitations)

1. **Identity + Qualified TSA:** on `identity` / other paid plans, event writes may return **`500 internal_error`** while a real Qualified TSA provider is unavailable. Prefer **free** for evidence commits during early pilot, or wait until Qualified TSA is enabled. Do **not** treat this as a billing bypass.
2. **Paid → paid plan change:** not self-service; contact support ([BILLING_MODEL.md](BILLING_MODEL.md)).
3. **No CLI identity register:** `evident identity` is not a command; register/revoke via Dashboard / HTTP (`/accounts/identity/keys/*`, `/v1/identity/keys/…`).
4. **CLI trust pin:** after any signing-key change (should be rare), refresh `~/.evident/server_identity.pub` from `GET /identity`.
5. **Past due:** paid writes (`/v1/*` events, `/events`, `/chains`, `/backup/create`) return `402 payment_required`; reads may still work.
