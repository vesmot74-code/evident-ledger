# Evident Ledger — Billing Model

**Status:** Implemented (Billing E2E complete — Stages 8.2a–8.2c + Paddle checkout).

This document defines subscription state, tariff change policy, Paddle webhook contract, and access control. Paddle Billing integration, webhook processing, subscription enforcement, and Dashboard checkout are implemented.

**Implementation summary:**

- **Paddle integration completed** — sandbox/live API client, customer linking, transaction checkout, Dashboard overlay (`Paddle.js`)
- **Webhook processing** — signed `POST /paddle/webhook`, idempotent event store, account linking for unknown customers
- **Subscription lifecycle** — create → active; update (upgrade immediate / downgrade deferred); past_due; canceled + lazy expiry to free

**Related documents:**

- [SECURITY.md](../SECURITY.md) — billing security invariants (§2.5 items 17–24) and billing model overview (§2.7)
- [SYSTEM_CONTRACT.md](../SYSTEM_CONTRACT.md) — subscription lifecycle summary (§17)
- [docs/AUTH_MODEL.md](AUTH_MODEL.md) — account ownership and API key authentication

**Tariff plans (current):** `free`, `legal`, `vault`, `identity` — see `tariff_plans` table. **Paid tier** means any plan where `name != 'free'`.

---

## 1. Subscription Dimensions

An account has three **orthogonal** billing dimensions:

| Dimension | Field(s) | Purpose |
|-----------|----------|---------|
| **Active tariff** | `tariff_plan_id` | Current plan in effect for limits and features |
| **Scheduled tariff** | `pending_tariff_plan_id` | Plan that takes effect after `current_period_end` (downgrades) |
| **Payment state** | `subscription_status` | Whether paid access is current |

### Subscription statuses

```
none       → no paid subscription (free-mode billing state)
active     → paid subscription current
past_due   → payment overdue (restricted write access on paid tiers)
canceled   → canceled; paid period may still run until current_period_end
```

**Rule:** `subscription_status` does not restrict the **free** tariff. When `tariff_plan_id` references the `free` plan, all subscription statuses are equivalent for access — the account always receives free-tier limits.

---

## 1.1 Schema (accounts)

Stage 8.2a adds:

| Column | Type | Purpose |
|--------|------|---------|
| `current_period_end` | `TIMESTAMPTZ NULL` | End of the current paid billing period |
| `pending_tariff_plan_id` | `UUID NULL` → `tariff_plans` | Scheduled plan after period end; `NULL` = no pending change |

Existing columns (Stage 8.1 / billing migration):

| Column | Purpose |
|--------|---------|
| `tariff_plan_id` | **Always** the active plan for limits and enforcement |
| `subscription_status` | Payment lifecycle state |
| `paddle_customer_id` | External Paddle customer reference (retained for audit) |

**Index:** `idx_accounts_pending_tariff_expiry` on `(current_period_end)` where `pending_tariff_plan_id IS NOT NULL` — for reconciliation, admin review, and subscription audits.

Migration: `migrations/20260718120000_add_billing_period_and_pending_plan.sql`.

---

## 2. State Transitions

```
none
  |
  | subscription.created
  v
active
  |
  +----------------------+
  |                      |
  | subscription.past_due| cancellation requested
  v                      v
past_due              canceled
  |                      |
  | subscription.updated | current_period_end reached (lazy)
  | (renewal / recovery) |
  +-----------> active   v
                      none (+ tariff → free)
past_due
  |
  | subscription.canceled
  v
canceled
  |
  | current_period_end reached (lazy)
  v
none (+ tariff → free)
```

### Canceled semantics

Until `current_period_end`, a **canceled** account keeps full paid-tier access. After `current_period_end`, the system **must** transition to `subscription_status = none` and `tariff_plan_id = free` (see §2.1).

---

## 2.1 Subscription Expiration (canceled → none)

**Condition:**

```
subscription_status = 'canceled'
AND current_period_end < now()
```

**Atomic outcome:**

```sql
tariff_plan_id = (SELECT plan_id FROM tariff_plans WHERE name = 'free')
pending_tariff_plan_id = NULL
subscription_status = 'none'
current_period_end = NULL
```

Applied via **lazy evaluation** on the first authenticated request after `current_period_end` (Stage 8.2c middleware). Billing history and `paddle_customer_id` are **not** deleted.

---

## 3. Tariff Change Policy

### Upgrade (immediate)

- `tariff_plan_id` updated immediately
- `pending_tariff_plan_id = NULL`
- New limits apply immediately
- `current_period_end` extended/updated for the new billing period

### Downgrade (deferred)

- `tariff_plan_id` unchanged (current paid plan)
- `pending_tariff_plan_id` = target plan
- Current limits remain until `current_period_end`
- After `current_period_end` (lazy evaluation on first request):
  - `tariff_plan_id = pending_tariff_plan_id`
  - `pending_tariff_plan_id = NULL`
  - `current_period_end` updated for the new period (implementation in 8.2b/8.2c)

### Lazy evaluation (no cron required)

Transitions `pending → active` and `canceled → none` run on the **first authenticated request** after `current_period_end`. Eligible triggers include any authenticated route — e.g. `POST /v1/events`, `GET /v1/verify/{event_id}`, `GET /accounts/me`.

**Normative pattern:** single atomic conditional `UPDATE … WHERE … RETURNING *` (not read-then-write). Example shape (Stage 8.2c):

```sql
UPDATE accounts
SET
    tariff_plan_id = COALESCE(
        pending_tariff_plan_id,
        (SELECT plan_id FROM tariff_plans WHERE name = 'free')
    ),
    pending_tariff_plan_id = NULL,
    subscription_status = CASE
        WHEN subscription_status = 'canceled' AND current_period_end < now() THEN 'none'
        ELSE subscription_status
    END,
    current_period_end = CASE
        WHEN pending_tariff_plan_id IS NOT NULL AND current_period_end < now()
            THEN current_period_end + interval '1 month'
        WHEN subscription_status = 'canceled' AND current_period_end < now() THEN NULL
        ELSE current_period_end
    END
WHERE
    account_id = $1
    AND (
        (pending_tariff_plan_id IS NOT NULL AND current_period_end < now())
        OR
        (subscription_status = 'canceled' AND current_period_end < now())
    )
RETURNING *;
```

Concurrent requests must apply the transition **at most once** (conditional update row count / `RETURNING`).

**Usage limits:** `usage_monthly` for the current period always follows **`tariff_plan_id`**, never `pending_tariff_plan_id`.

---

## Subscription Changes

- **New paid subscriptions** (`FREE` → `PAID`) are created automatically through Paddle checkout and activated by webhooks (`subscription.created` / `subscription.updated`).
- **Changing an active paid plan** (`PAID` → `PAID`) is **not** self-service in early access. Dashboard upgrade for accounts with `subscription_status = active` is rejected; customers change plans via support.
- Automatic self-service plan switching (subscription replacement / update in Paddle) may be added later when needed. Until then, do not implement cancel+recreate or in-place subscription replacement flows.

Webhook handling of `subscription.updated` (upgrade/downgrade semantics in §3) remains valid when plan changes are applied externally (e.g. by support or a future self-service flow).

---

## 4. Webhook Handling (Paddle) — Stage 8.2b

**Source of truth:**

```
Paddle is the external payment authority.
Local DB stores the last verified billing state.
```

Handled Paddle Billing event types (normalized `.` → `_` in the processor):

| Paddle `event_type` | Local action |
|-------|----------------|
| `subscription.created` | `subscription_status = active`; set plan + `current_period_end` |
| `subscription.updated` (upgrade) | Update `tariff_plan_id`, `current_period_end`; clear `pending_tariff_plan_id` |
| `subscription.updated` (downgrade) | Set `pending_tariff_plan_id`; do **not** change `tariff_plan_id` |
| `subscription.updated` (same plan / renewal) | `subscription_status = active`; refresh `current_period_end` |
| `subscription.past_due` | `subscription_status = past_due` |
| `subscription.canceled` | `subscription_status = canceled`; retain `current_period_end` |

Unrecognized event types are acknowledged with HTTP 200 (`ignored`) so Paddle does not retry forever. There is no `subscription.payment_succeeded` / `subscription.payment_failed` in Paddle Billing — renewals and recovery after `past_due` arrive as `subscription.updated`.

### Idempotency

Webhook processing **must** be idempotent. Repeated delivery of the same event **must not** apply state twice. Use Paddle `event_id` (table `paddle_webhook_events`).

### Signature verification

All webhook payloads **must** be signature-verified with Paddle's signing secret **before** any account mutation (`Paddle-Signature`: `{ts}:{raw_body}` HMAC-SHA256).

### Replay / out-of-order events

Stale events (timestamp older than current derived state) **should** be ignored or logged — not applied as downgrades to newer state.

---

## 5. Access Control — Stage 8.2c

`subscription_status` affects **paid tiers only**. Free tier is never blocked by billing status.

| `tariff_plan_id` | `subscription_status` | `/v1/*` writes | `/v1/*` reads | `/accounts/*` |
|------------------|----------------------|----------------|---------------|---------------|
| `free` | any | ✅ within free limits | ✅ | ✅ |
| paid (`legal`, `vault`, `identity`, …) | `active` | ✅ | ✅ | ✅ |
| paid | `past_due` | ❌ | ✅ | ✅ |
| paid | `canceled` (before period end) | ✅ | ✅ | ✅ |
| paid | `canceled` (after period end, before lazy eval) | → treated as `free` after lazy eval | → same | ✅ |

**Reads** include `GET /v1/verify`, `GET /v1/proof`, and other non-mutating owner operations — allowed under `past_due` because they do not create billable resources.

**Enforcement (8.2c):**

- Middleware evaluates **`tariff_plan_id`**, not `pending_tariff_plan_id`
- Runs lazy expiration/downgrade conditional update before limit checks when applicable
- Write paths reject `past_due` on paid tiers; free tier unaffected

---

## 6. Security Invariants

Normatively listed as items **17–24** in [SECURITY.md](../SECURITY.md) §2.5.

---

## Document status

**Implemented** — model freeze (8.2a), Paddle webhooks (8.2b), enforcement + lazy evaluation (8.2c), Dashboard checkout / Billing E2E.

Changes to subscription semantics, Paddle contract, or billing invariants require an explicit stage and updates to this document, [SECURITY.md](../SECURITY.md) §2.5–2.7, and [SYSTEM_CONTRACT.md](../SYSTEM_CONTRACT.md) §17 where lifecycle rules are affected — per Security Invariant 11.
